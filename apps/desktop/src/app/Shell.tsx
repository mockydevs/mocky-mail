import { useCallback, useEffect, useRef, useState } from 'react';

import { useMailApi } from '../api/MailApiContext';
import { errorTitle, toAppError } from '../api/errors';
import type { AppError, Draft, Folder, MessageDetail, MessageListItem, OutboxEntry, ReadFilter, Settings, SyncStatus } from '../api/types';
import { AddAccountForm } from '../features/accounts/AddAccountForm';
import { Welcome } from '../features/accounts/Welcome';
import { ComposeView } from '../features/compose/ComposeView';
import { blankDraft, draftFromMessage, forwardDraft, replyDraft } from '../features/compose/seed';
import { MessageList } from '../features/mailbox/MessageList';
import {
  applyOverrides,
  INBOX_VIEW,
  neighbourAfterRemoval,
  resolveSelection,
  viewInfo,
  viewQuery,
  withPinned,
  type MailView,
  type Override,
} from '../features/mailbox/selectors';
import { Sidebar } from '../features/mailbox/Sidebar';
import { useMessagePage } from '../features/mailbox/useMessagePage';
import { OutboxView } from '../features/outbox/OutboxView';
import { isActive } from '../features/outbox/outboxCopy';
import { ReadingPane } from '../features/reader/ReadingPane';
import { SettingsView } from '../features/settings/SettingsView';
import { HelpSheet } from '../features/shortcuts/HelpSheet';
import { shortcutFor, type ShortcutAction } from '../features/shortcuts/shortcuts';
import { nowSeconds } from '../lib/format';
import { useDebounced } from '../lib/useDebounced';
import { useMailEvent } from '../lib/useEvent';
import { useLoad } from '../lib/useLoad';
import { useTheme } from '../theme/useTheme';
import { ConfirmDialog } from '../ui/Modal';
import { Icon } from '../ui/Icon';
import { StatusBar } from './StatusBar';
import { syncSummary } from './sync';
import { TopBar } from './TopBar';

export const SEARCH_DEBOUNCE_MS = 300;

type Screen =
  | { kind: 'mail' }
  | { kind: 'compose'; draft: Draft; title: string }
  | { kind: 'settings' }
  | { kind: 'addAccount' };

interface Notice {
  tone: 'error' | 'info';
  title: string;
  text: string;
}

function errorNotice(error: AppError, what: string): Notice {
  return { tone: 'error', title: `${what}: ${errorTitle(error.kind)}`, text: error.message };
}

const DEFAULT_SETTINGS: Settings = { theme: 'system', remoteImages: 'block', notifications: false };

export function Shell({ now = nowSeconds }: { now?: () => number }) {
  const api = useMailApi();
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => {
    setRevision((r) => r + 1);
  }, []);

  // Accounts, folders, settings, outbox and sync status.
  const accountsLoad = useLoad('accounts', revision, () => api.listAccounts());
  const accounts = accountsLoad.status === 'ready' ? accountsLoad.value : [];
  const accountKey = accounts.map((a) => a.id).join(',');
  const foldersLoad = useLoad(`folders:${accountKey}`, revision, async () => {
    const lists = await Promise.all(accounts.map((a) => api.listFolders(a.id)));
    const byAccount: Record<string, Folder[]> = Object.fromEntries(accounts.map((a, i) => [a.id, lists[i] ?? []]));
    return byAccount;
  });
  const foldersByAccount = foldersLoad.status === 'ready' ? foldersLoad.value : {};
  const allFolders = Object.values(foldersByAccount).flat();

  const [settings, setSettings] = useState<Settings | null>(null);
  const [outbox, setOutbox] = useState<OutboxEntry[] | null>(null);
  const [sync, setSync] = useState<Record<string, SyncStatus>>({});
  const [notice, setNotice] = useState<Notice | null>(null);

  const report = (error: AppError, what: string) => {
    setNotice(errorNotice(error, what));
  };

  useEffect(() => {
    api.getSettings().then(setSettings, (e: unknown) => {
      setSettings(DEFAULT_SETTINGS);
      setNotice(errorNotice(toAppError(e), 'Could not load settings'));
    });
  }, [api]);

  useEffect(() => {
    api.listOutbox().then(setOutbox, (e: unknown) => {
      setNotice(errorNotice(toAppError(e), 'Could not load the Outbox'));
    });
  }, [api, revision]);

  useMailEvent(api, 'mail://changed', refresh);
  useMailEvent(api, 'mail://outbox-changed', (payload) => {
    setOutbox(payload.entries);
  });
  useMailEvent(api, 'mail://sync-status', (status) => {
    setSync((all) => ({ ...all, [status.accountId]: status }));
  });
  // The backend shows the desktop notification itself; here new mail only
  // refreshes the lists and unread counts.
  useMailEvent(api, 'mail://new-mail', refresh);
  useMailEvent(api, 'mail://accounts-changed', refresh);

  const changeSettings = (next: Settings) => {
    const previous = settings;
    setSettings(next);
    api.setSettings(next).then(setSettings, (e: unknown) => {
      setSettings(previous);
      report(toAppError(e), 'Could not save settings');
    });
  };

  const theme = useTheme(settings?.theme ?? 'system');

  // What the list shows.
  const [screen, setScreen] = useState<Screen>({ kind: 'mail' });
  const [view, setView] = useState<MailView>(INBOX_VIEW);
  const [filter, setFilter] = useState<ReadFilter>('all');
  const [searchInput, setSearchInput] = useState('');
  const search = useDebounced(searchInput, SEARCH_DEBOUNCE_MS);
  const query = viewQuery(view, filter, search);
  const page = useMessagePage(api, query, revision);
  const [overrides, setOverrides] = useState<Record<string, Override>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pinned, setPinned] = useState<MessageListItem | null>(null);
  const [loaded, setLoaded] = useState<MessageDetail | null>(null);
  const [help, setHelp] = useState(false);
  const [confirmForever, setConfirmForever] = useState<MessageListItem | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const readerRef = useRef<HTMLElement>(null);
  const versionRef = useRef(page.version);
  useEffect(() => {
    versionRef.current = page.version;
  });

  const filtering = filter !== 'all' || search.trim() !== '';
  const items = applyOverrides(filtering ? withPinned(page.items, pinned) : page.items, overrides, page.version);
  const activeId = resolveSelection(items, selectedId);
  const active = items.find((m) => m.id === activeId) ?? null;
  const info = viewInfo(view, accounts, allFolders);
  const roleOf = (item: MessageListItem | null) =>
    item ? (allFolders.find((f) => f.id === item.folderId)?.role ?? info.role) : info.role;
  const outboxActive = (outbox ?? []).filter(isActive).length;
  const summary = syncSummary(Object.values(sync), accounts);
  const offline = summary.tone === 'warn' || summary.tone === 'error';
  const message = loaded?.id === activeId ? loaded : null;

  /** Apply a change at once; undo it and explain if the backend refuses. */
  const optimistic = (
    ids: string[],
    patch: Pick<Override, 'unread' | 'flagged' | 'removed'>,
    call: () => Promise<void>,
    what: string,
    onRollback?: () => void,
  ) => {
    const version = versionRef.current;
    setOverrides((o) => ({ ...o, ...Object.fromEntries(ids.map((id) => [id, { ...o[id], ...patch, version, settled: false }])) }));
    call().then(
      () => {
        setOverrides((o) => {
          const next = { ...o };
          for (const id of ids) {
            const entry = next[id];
            if (entry) next[id] = { ...entry, settled: true, version: versionRef.current };
          }
          return next;
        });
        refresh();
      },
      (e: unknown) => {
        setOverrides((o) => {
          const next = { ...o };
          for (const id of ids) Reflect.deleteProperty(next, id);
          return next;
        });
        onRollback?.();
        report(toAppError(e), what);
      },
    );
  };

  /** The user opened a message: select it and mark it read. */
  const open = (id: string) => {
    setSelectedId(id);
    const item = items.find((m) => m.id === id);
    if (!item) return;
    setPinned(item);
    if (item.unread) {
      optimistic([id], { unread: false }, () => api.setFlags([id], { seen: true }), 'Could not mark as read');
    }
  };

  const goTo = (next: MailView) => {
    setView(next);
    setFilter('all');
    setSelectedId(null);
    setPinned(null);
    setScreen({ kind: 'mail' });
  };

  /** Show the neighbour now; go back to the message if the backend refuses. */
  const restore = (item: MessageListItem) => () => {
    setSelectedId(item.id);
  };

  const remove = (item: MessageListItem, target: 'archive' | 'trash') => {
    setSelectedId(neighbourAfterRemoval(items, item.id));
    setPinned(null);
    optimistic(
      [item.id],
      { removed: true },
      () => api.moveMessages([item.id], { role: target }),
      target === 'archive' ? 'Could not archive' : 'Could not delete',
      restore(item),
    );
  };

  const deleteForever = (item: MessageListItem) => {
    setSelectedId(neighbourAfterRemoval(items, item.id));
    setPinned(null);
    optimistic([item.id], { removed: true }, () => api.deleteForever([item.id]), 'Could not delete', restore(item));
  };

  const deleteItem = (item: MessageListItem) => {
    const role = roleOf(item);
    if (role === 'trash' || role === 'junk') setConfirmForever(item);
    else remove(item, 'trash');
  };

  const toggleFlag = (item: MessageListItem) => {
    optimistic([item.id], { flagged: !item.flagged }, () => api.setFlags([item.id], { flagged: !item.flagged }), 'Could not flag');
  };

  const toggleRead = (item: MessageListItem) => {
    optimistic(
      [item.id],
      { unread: !item.unread },
      () => api.setFlags([item.id], { seen: item.unread }),
      'Could not change read state',
    );
  };

  const compose = (draft: Draft, title: string) => {
    setScreen({ kind: 'compose', draft, title });
  };

  const reply = (m: MessageDetail, all: boolean) => {
    const account = accounts.find((a) => a.id === m.accountId);
    if (account) compose(replyDraft(m, account, all), all ? 'Reply all' : 'Reply');
  };

  const newMessage = () => {
    const accountId = view.type === 'folder' ? view.accountId : (accounts[0]?.id ?? '');
    compose(blankDraft(accountId), 'New message');
  };

  // Keyboard shortcuts.
  const handlerRef = useRef<(action: ShortcutAction, event: KeyboardEvent) => void>(() => undefined);
  useEffect(() => {
    handlerRef.current = (action, event) => {
      if (help || confirmForever) return;
      if (action === 'close') {
        if (screen.kind === 'settings' || screen.kind === 'addAccount') setScreen({ kind: 'mail' });
        return;
      }
      if (screen.kind !== 'mail') return;
      if (action === 'help') {
        setHelp(true);
        return;
      }
      if (action === 'search') {
        event.preventDefault();
        searchRef.current?.focus();
        return;
      }
      if (action === 'compose') {
        event.preventDefault();
        newMessage();
        return;
      }
      if (view.type === 'outbox') return;
      const index = items.findIndex((m) => m.id === activeId);
      if (action === 'next' || action === 'previous') {
        const target = items[action === 'next' ? index + 1 : index - 1];
        if (target) open(target.id);
        else if (action === 'next' && page.hasMore) page.loadMore();
        return;
      }
      if (action === 'open') {
        readerRef.current?.focus();
        return;
      }
      if (!active) return;
      const role = roleOf(active);
      switch (action) {
        case 'reply':
        case 'replyAll':
          if (message && role !== 'drafts') reply(message, action === 'replyAll');
          return;
        case 'forward':
          if (message && role !== 'drafts') compose(forwardDraft(message), 'Forward');
          return;
        case 'archive':
          if (role !== 'archive' && role !== 'trash' && role !== 'junk' && role !== 'drafts') remove(active, 'archive');
          return;
        case 'delete':
          deleteItem(active);
          return;
        case 'flag':
          toggleFlag(active);
          return;
        case 'toggleRead':
          toggleRead(active);
          return;
      }
    };
  });
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const action = shortcutFor(event);
      if (!action) return;
      // Stop the key from also being typed into a field the shortcut focuses
      // (for example the body of the reply that "r" opens).
      if (action !== 'close') event.preventDefault();
      handlerRef.current(action, event);
    };
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('keydown', onKey);
    };
  }, []);

  if (accountsLoad.status === 'loading' && accounts.length === 0) {
    return (
      <div className="splash" aria-busy="true">
        Loading Mocky Mail
      </div>
    );
  }
  if (accountsLoad.status === 'error') {
    return (
      <div className="splash" role="alert">
        <p className="empty__title">Mocky Mail could not start</p>
        <p className="empty__text">{accountsLoad.message}</p>
        <button type="button" className="btn btn--primary" onClick={refresh}>
          Try again
        </button>
      </div>
    );
  }
  if (accounts.length === 0) {
    return (
      <Welcome
        onAdded={() => {
          refresh();
          goTo(INBOX_VIEW);
        }}
      />
    );
  }

  let main;
  switch (screen.kind) {
    case 'compose':
      main = (
        <ComposeView
          key={JSON.stringify(screen.draft)}
          initial={screen.draft}
          title={screen.title}
          accounts={accounts}
          offlineNote={offline}
          onClose={() => {
            setScreen({ kind: 'mail' });
          }}
          onSent={() => {
            setScreen({ kind: 'mail' });
            setNotice({ tone: 'info', title: 'Message queued', text: 'It is in the Outbox and will send automatically.' });
            refresh();
          }}
        />
      );
      break;
    case 'settings':
      main = (
        <SettingsView
          settings={settings ?? DEFAULT_SETTINGS}
          accounts={accounts}
          onChange={changeSettings}
          onAccountsChanged={refresh}
          onAddAccount={() => {
            setScreen({ kind: 'addAccount' });
          }}
          onClose={() => {
            setScreen({ kind: 'mail' });
          }}
        />
      );
      break;
    case 'addAccount':
      main = (
        <div className="panel">
          <AddAccountForm
            onAdded={() => {
              refresh();
              goTo(INBOX_VIEW);
            }}
            onCancel={() => {
              setScreen({ kind: 'mail' });
            }}
          />
        </div>
      );
      break;
    case 'mail':
      main =
        view.type === 'outbox' ? (
          <OutboxView
            entries={outbox}
            accounts={accounts}
            now={now()}
            onEdit={(draft) => {
              compose(draft, 'Edit message');
            }}
          />
        ) : (
          <>
            <MessageList
              title={info.title}
              subtitle={info.subtitle}
              role={info.role}
              status={page.status}
              items={items}
              total={page.total}
              error={page.error}
              filtered={filtering}
              filter={filter}
              selectedId={activeId}
              accounts={accounts}
              now={now()}
              hasMore={page.hasMore}
              loadingMore={page.loadingMore}
              onFilter={(next) => {
                setFilter(next);
                setSelectedId(null);
                setPinned(null);
              }}
              onSelect={open}
              onLoadMore={page.loadMore}
              onRetry={refresh}
            />
            <ReadingPane
              ref={readerRef}
              item={active}
              account={accounts.find((a) => a.id === active?.accountId)}
              role={roleOf(active)}
              alwaysLoadImages={settings?.remoteImages === 'allow'}
              onLoaded={setLoaded}
              onReply={reply}
              onForward={(m) => {
                compose(forwardDraft(m), 'Forward');
              }}
              onEdit={(m) => {
                compose(draftFromMessage(m), 'Edit draft');
              }}
              onArchive={() => {
                if (active) remove(active, 'archive');
              }}
              onDelete={() => {
                if (active) deleteItem(active);
              }}
              onDeleteForever={() => {
                if (active) setConfirmForever(active);
              }}
              onToggleFlag={() => {
                if (active) toggleFlag(active);
              }}
              onToggleRead={() => {
                if (active) toggleRead(active);
              }}
            />
          </>
        );
      break;
  }

  return (
    <div className="app">
      <TopBar
        search={searchInput}
        searchField={searchRef}
        sync={summary}
        theme={theme}
        onSearch={(value) => {
          setSearchInput(value);
          setSelectedId(null);
          setPinned(null);
        }}
        onSync={() => api.syncNow(null)}
        onToggleTheme={() => {
          changeSettings({ ...(settings ?? DEFAULT_SETTINGS), theme: theme === 'dark' ? 'light' : 'dark' });
        }}
        onHelp={() => {
          setHelp(true);
        }}
      />
      <div className="workspace">
        <Sidebar
          view={view}
          accounts={accounts}
          folders={foldersByAccount}
          outboxCount={outboxActive}
          onView={goTo}
          onCompose={newMessage}
          onSettings={() => {
            setScreen({ kind: 'settings' });
          }}
          onAddAccount={() => {
            setScreen({ kind: 'addAccount' });
          }}
        />
        <main className="panes">{main}</main>
      </div>
      <StatusBar sync={summary} outbox={outboxActive} accountCount={accounts.length} />

      {notice && (
        <div className={`toast toast--${notice.tone}`} role={notice.tone === 'error' ? 'alert' : 'status'}>
          <div>
            <p className="toast__title">{notice.title}</p>
            <p className="toast__text">{notice.text}</p>
          </div>
          <button
            type="button"
            className="icon-btn"
            aria-label="Dismiss"
            onClick={() => {
              setNotice(null);
            }}
          >
            <Icon name="close" />
          </button>
        </div>
      )}
      {help && (
        <HelpSheet
          onClose={() => {
            setHelp(false);
          }}
        />
      )}
      {confirmForever && (
        <ConfirmDialog
          title="Delete forever?"
          description="This message is removed from the server and cannot be recovered."
          confirmLabel="Delete forever"
          danger
          onCancel={() => {
            setConfirmForever(null);
          }}
          onConfirm={() => {
            const item = confirmForever;
            setConfirmForever(null);
            deleteForever(item);
          }}
        />
      )}
    </div>
  );
}
