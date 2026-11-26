import { useCallback, useEffect, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, Category, Draft, FlagChange, MessageDetail, MessageListItem, Settings } from '../../api/types';
import { nowSeconds } from '../../lib/format';
import { useLoad } from '../../lib/useLoad';
import { useTheme } from '../../theme/useTheme';
import { ConfirmDialog } from '../../ui/Modal';
import { ComposeView } from '../compose/ComposeView';
import { draftFromMessage, forwardDraft, replyDraft, type ComposeKind } from '../compose/seed';
import { ReadingPane } from './ReadingPane';

/** The message id a window was opened for (`?message=<id>`), or null for the main window. */
export function messageWindowId(search: string): string | null {
  const id = new URLSearchParams(search).get('message');
  return id && id.trim() !== '' ? id : null;
}

interface Writing {
  draft: Draft;
  title: string;
  kind: ComposeKind;
  attachmentsFrom: string | null;
}

/**
 * A message in a window of its own (computers): the reading pane with its
 * actions, and no sidebar or list. Reply and forward open compose in this
 * window; once the message is moved away the window says so and can close.
 */
export function MessageWindow({ messageId, now = nowSeconds }: { messageId: string; now?: () => number }) {
  const api = useMailApi();
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => {
    setRevision((r) => r + 1);
  }, []);
  const accountsLoad = useLoad('accounts', 0, () => api.listAccounts());
  const accounts = accountsLoad.status === 'ready' ? accountsLoad.value : [];
  const messageLoad = useLoad(`window:${messageId}`, revision, () => api.getMessage(messageId, false));
  const detail = messageLoad.status === 'ready' ? messageLoad.value : null;
  const accountId = detail?.accountId ?? null;
  const foldersLoad = useLoad(`folders:${accountId ?? ''}`, revision, () =>
    accountId ? api.listFolders(accountId) : Promise.resolve([]),
  );
  const folders = foldersLoad.status === 'ready' ? foldersLoad.value : [];
  const [settings, setSettings] = useState<Settings | null>(null);
  const [categories, setCategories] = useState<Category[]>([]);
  const theme = useTheme(settings?.theme ?? 'system');
  const [writing, setWriting] = useState<Writing | null>(null);
  const [gone, setGone] = useState<string | null>(null);
  const [confirmForever, setConfirmForever] = useState(false);
  const [error, setError] = useState<{ error: AppError; what: string } | null>(null);
  /** Changes made here, shown before the backend's next answer. */
  const [patch, setPatch] = useState<Partial<MessageListItem>>({});

  useEffect(() => {
    api.getSettings().then(setSettings, () => undefined);
    api.listCategories().then(setCategories, () => undefined);
  }, [api]);

  useEffect(() => {
    const offChanged = api.on('mail://changed', refresh);
    return offChanged;
  }, [api, refresh]);

  useEffect(() => {
    if (detail) document.title = detail.subject || '(no subject)';
  }, [detail]);

  if (messageLoad.status === 'loading') {
    return (
      <div className="splash" aria-busy="true">
        Opening the message
      </div>
    );
  }
  if (messageLoad.status === 'error' || !detail) {
    return (
      <div className="splash" role="alert">
        <p className="empty__title">This message could not be opened</p>
        <p className="empty__text">{messageLoad.status === 'error' ? messageLoad.message : ''}</p>
      </div>
    );
  }

  const item: MessageListItem = { ...detail, ...patch };
  const role = folders.find((f) => f.id === detail.folderId)?.role ?? null;
  const account = accounts.find((a) => a.id === detail.accountId);

  const failed = (what: string) => (e: unknown) => {
    setPatch({});
    setError({ error: toAppError(e), what });
  };

  const flags = (change: FlagChange, shown: Partial<MessageListItem>, what: string) => {
    setError(null);
    setPatch((p) => ({ ...p, ...shown }));
    api.setFlags([detail.id], change).then(() => {
      setPatch({});
      refresh();
    }, failed(what));
  };

  const away = (work: Promise<void>, done: string, what: string) => {
    setError(null);
    work.then(() => {
      setGone(done);
    }, failed(what));
  };

  const write = (draft: Draft, title: string, kind: ComposeKind, m: MessageDetail, withFiles: boolean) => {
    setWriting({ draft, title, kind, attachmentsFrom: withFiles && m.attachments.length > 0 ? m.id : null });
  };

  if (writing) {
    return (
      <div className="app app--window">
        <main className="panes panes--window">
          <ComposeView
            key={JSON.stringify(writing.draft)}
            initial={writing.draft}
            attachmentsFrom={writing.attachmentsFrom}
            kind={writing.kind}
            title={writing.title}
            accounts={accounts}
            offlineNote={settings?.workOffline ?? false}
            onClose={() => {
              setWriting(null);
            }}
            onSent={() => {
              setWriting(null);
              refresh();
            }}
          />
        </main>
      </div>
    );
  }

  if (gone) {
    return (
      <div className="splash" role="status">
        <p className="empty__title">{gone}</p>
        <p className="empty__text">{detail.subject || '(no subject)'}</p>
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => {
            window.close();
          }}
        >
          Close window
        </button>
      </div>
    );
  }

  return (
    <div className="app app--window">
      {error && (
        <div className="toast toast--error window-error" role="alert">
          <div>
            <p className="toast__title">{error.what}</p>
            <p className="toast__text">{error.error.message}</p>
          </div>
        </div>
      )}
      <main className="panes panes--window">
        <ReadingPane
          item={item}
          account={account}
          role={role}
          alwaysLoadImages={settings?.remoteImages === 'allow'}
          theme={theme}
          categories={categories}
          now={now()}
          onLoaded={() => undefined}
          onReply={(m, all) => {
            if (account) write(replyDraft(m, account, all), all ? 'Reply all' : 'Reply', 'reply', m, false);
          }}
          onForward={(m) => {
            write(forwardDraft(m), 'Forward', 'forward', m, true);
          }}
          onEdit={(m) => {
            write(draftFromMessage(m), 'Edit draft', 'draft', m, true);
          }}
          onArchive={() => {
            away(api.moveMessages([detail.id], { role: 'archive' }), 'Archived', 'Could not archive');
          }}
          onDelete={() => {
            if (role === 'trash' || role === 'junk') setConfirmForever(true);
            else away(api.moveMessages([detail.id], { role: 'trash' }), 'Moved to Trash', 'Could not delete');
          }}
          onDeleteForever={() => {
            setConfirmForever(true);
          }}
          onToggleFlag={() => {
            flags({ flagged: !item.flagged }, { flagged: !item.flagged }, 'Could not flag');
          }}
          onFlagDue={(flag) => {
            flags(
              flag.flagged && flag.due !== undefined ? { flagged: true, flagDue: flag.due } : { flagged: flag.flagged },
              { flagged: flag.flagged, flagDue: flag.flagged ? (flag.due ?? item.flagDue) : null },
              'Could not flag',
            );
          }}
          onToggleRead={() => {
            flags({ seen: item.unread }, { unread: !item.unread }, 'Could not change read state');
          }}
          onTogglePin={() => {
            flags({ pinned: !item.pinned }, { pinned: !item.pinned }, 'Could not pin');
          }}
          onImportance={(importance) => {
            flags({ importance }, { importance }, 'Could not change importance');
          }}
          onCategory={(name, add) => {
            setError(null);
            setPatch((p) => ({
              ...p,
              categories: add ? [...item.categories, name] : item.categories.filter((c) => c !== name),
            }));
            api.setCategories([detail.id], add ? [name] : [], add ? [] : [name]).then(() => {
              setPatch({});
              refresh();
            }, failed('Could not change categories'));
          }}
          onCategoryCreated={(all, name) => {
            setCategories(all);
            api.setCategories([detail.id], [name], []).then(refresh, failed('Could not change categories'));
          }}
          onJunk={(junk) => {
            away(api.markJunk([detail.id], junk), junk ? 'Marked as junk' : 'Moved to the Inbox', 'Could not move');
          }}
        />
      </main>
      {confirmForever && (
        <ConfirmDialog
          title="Delete forever?"
          description="This message is removed from the server and cannot be recovered."
          confirmLabel="Delete forever"
          danger
          onCancel={() => {
            setConfirmForever(false);
          }}
          onConfirm={() => {
            setConfirmForever(false);
            away(api.deleteForever([detail.id]), 'Deleted forever', 'Could not delete');
          }}
        />
      )}
    </div>
  );
}
