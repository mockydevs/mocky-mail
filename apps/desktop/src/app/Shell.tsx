import { useCallback, useEffect, useRef, useState } from 'react';

import { useMailApi } from '../api/MailApiContext';
import { errorTitle, toAppError } from '../api/errors';
import type { AppError, Draft, Focus, Folder, Mailto, MessageDetail, MessageListItem, OutboxEntry, ReadFilter, Settings, SyncStatus } from '../api/types';
import { AddAccountForm } from '../features/accounts/AddAccountForm';
import { Welcome } from '../features/accounts/Welcome';
import { ComposeView, type LeaveCompose } from '../features/compose/ComposeView';
import { blankDraft, draftFromMessage, forwardDraft, replyDraft, type ComposeKind } from '../features/compose/seed';
import { MessageList } from '../features/mailbox/MessageList';
import {
  applyOverrides,
  INBOX_VIEW,
  neighbourAfterRemoval,
  nextOverride,
  resolveSelection,
  rowHolds,
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
import { useSendStatus, type SendStatus } from '../features/outbox/useSendStatus';
import { playChime } from '../features/notifications/chime';
import { ReadingPane } from '../features/reader/ReadingPane';
import { SettingsView } from '../features/settings/SettingsView';
import { ContactsView } from '../features/contacts/ContactsView';
import { CalendarView } from '../features/calendar/CalendarView';
import { ManageFolders } from '../features/mailbox/ManageFolders';
import { HelpSheet } from '../features/shortcuts/HelpSheet';
import { shortcutFor, type ShortcutAction } from '../features/shortcuts/shortcuts';
import { nowSeconds } from '../lib/format';
import { useDebounced } from '../lib/useDebounced';
import { useMailEvent } from '../lib/useEvent';
import { useWakeSync } from '../lib/useWakeSync';
import { UpdateBanner } from '../features/updates/UpdateBanner';
import { useLoad } from '../lib/useLoad';
import { useTheme } from '../theme/useTheme';
import { ConfirmDialog } from '../ui/Modal';
import { Icon } from '../ui/Icon';
import { StatusBar } from './StatusBar';
import { syncSummary } from './sync';
import { usePhone, usePhoneBack } from './usePhone';
import { TopBar } from './TopBar';

export const SEARCH_DEBOUNCE_MS = 300;

type Screen =
  | { kind: 'mail' }
  | { kind: 'compose'; draft: Draft; title: string; compose: ComposeKind; attachmentsFrom: string | null; unsaved: boolean }
  | { kind: 'settings' }
  | { kind: 'contacts' }
  | { kind: 'calendar' }
  | { kind: 'addAccount' };

/** How long a confirmation toast stays up. */
const NOTICE_MS = 4000;

/** The messages a list row stands for: the whole conversation when grouped. */
function idsOf(item: MessageListItem | undefined, fallback = ''): string[] {
  if (!item) return fallback ? [fallback] : [];
  return item.threadIds && item.threadIds.length > 0 ? item.threadIds : [item.id];
}

/** The toast for a message the user just sent, if one is being followed. */
function sendNoticeOf(status: SendStatus | null, undo: (id: string) => void): Notice | null {
  switch (status?.phase) {
    case undefined:
      return null;
    case 'scheduled':
      return {
        tone: 'info',
        title: 'Sending…',
        text: `You have ${String(status.secs)} seconds to undo.`,
        sticky: true,
        action: {
          label: 'Undo',
          run: () => {
            undo(status.id);
          },
        },
      };
    case 'sending':
      return { tone: 'info', title: 'Sending…', text: 'Your message is on its way.', sticky: true };
    case 'sent':
      return { tone: 'info', title: 'Sent', text: 'Your message was delivered to your mail server.' };
    case 'waiting':
      return { tone: 'info', title: 'Not sent yet', text: 'There is no connection right now. It will send by itself when you are back online.' };
    case 'failed':
      return { tone: 'error', title: 'Not sent', text: `${status.message} The message is in the Outbox.` };
  }
}

interface Notice {
  tone: 'error' | 'info';
  title: string;
  text: string;
  /** Stays up until replaced or dismissed (an action still under way). */
  sticky?: boolean;
  /** One button beside the text, such as Undo. */
  action?: { label: string; run: () => void };
}

function errorNotice(error: AppError, what: string): Notice {
  return { tone: 'error', title: `${what}: ${errorTitle(error.kind)}`, text: error.message };
}

function Toast({ notice, onDismiss }: { notice: Notice; onDismiss: () => void }) {
  return (
    <div className={`toast toast--${notice.tone}`} role={notice.tone === 'error' ? 'alert' : 'status'}>
      <div>
        <p className="toast__title">{notice.title}</p>
        <p className="toast__text">{notice.text}</p>
      </div>
      {notice.action && (
        <button type="button" className="btn btn--secondary btn--small toast__action" onClick={notice.action.run}>
          {notice.action.label}
        </button>
      )}
      <button type="button" className="icon-btn" aria-label="Dismiss" onClick={onDismiss}>
        <Icon name="close" />
      </button>
    </div>
  );
}

const DEFAULT_SETTINGS: Settings = {
  theme: 'system',
  remoteImages: 'block',
  notifications: false,
  sound: false,
  undoSendSecs: 5,
  markRead: 'open',
  threads: true,
  focusedInbox: false,
  checkEveryMins: 2,
  markReadDelaySecs: 3,
  eventReminders: true,
  defaultReminderMins: 15,
  closeToTray: true,
  startAtLogin: false,
  device: { tray: false, startAtLogin: false },
};

/**
 * Back in compose behaves like Esc: it closes an untouched message, and asks
 * before throwing away changes. The key goes to whatever has focus inside
 * compose (or its discard dialog), else to the form itself.
 */
function pressEscape(form: Element | null): void {
  if (!form) return;
  const focused = document.activeElement;
  const target = focused && focused !== document.body ? focused : form;
  target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

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

  // Confirmations close by themselves; errors stay until dismissed.
  useEffect(() => {
    if (notice?.tone !== 'info' || notice.sticky) return;
    const timer = window.setTimeout(() => {
      setNotice(null);
    }, NOTICE_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [notice]);

  // Tell the user whether a message they sent actually went out.
  const sending = useSendStatus(api);
  /** Undo send: take the message back out of the Outbox into compose. */
  const undoSend = (id: string) => {
    sending.clear();
    api.outboxEdit(id).then(
      (draft) => {
        // It has left the Outbox, so compose holds the only copy.
        compose(draft, 'New message', 'draft', { unsaved: true });
      },
      (e: unknown) => {
        report(toAppError(e), 'Too late to undo; it is already sending');
      },
    );
  };
  const sendNotice = sendNoticeOf(sending.status, undoSend);
  const clearSending = sending.clear;
  const sendPhase = sending.status?.phase;
  useEffect(() => {
    if (sendPhase !== 'sent' && sendPhase !== 'waiting') return;
    const timer = window.setTimeout(clearSending, NOTICE_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [sendPhase, clearSending]);

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
  // Coming back (focus, wake, network) checks for new mail at once.
  const syncOnWake = useCallback(() => {
    api.syncNow(null).catch(() => undefined);
  }, [api]);
  useWakeSync(syncOnWake);
  useMailEvent(api, 'mail://outbox-changed', (payload) => {
    setOutbox(payload.entries);
  });
  useMailEvent(api, 'mail://sync-status', (status) => {
    setSync((all) => ({ ...all, [status.accountId]: status }));
  });
  // The backend shows the desktop notification itself; here new mail
  // refreshes the lists and unread counts and plays the chime.
  useMailEvent(api, 'mail://new-mail', () => {
    refresh();
    if (settings?.notifications && settings.sound) playChime();
  });
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
  const baseQuery = viewQuery(view, filter, search);
  const threads = settings?.threads ?? true;
  // Focused Inbox: inbox views show one half at a time.
  const [focusTab, setFocusTab] = useState<Focus>('focused');
  const inboxView =
    view.type === 'unified'
      ? view.role === 'inbox'
      : view.type === 'folder' && allFolders.some((f) => f.id === view.folderId && f.role === 'inbox');
  const focusSplit = (settings?.focusedInbox ?? false) && inboxView;
  const query = baseQuery && { ...baseQuery, threads, focus: focusSplit ? focusTab : null };
  const page = useMessagePage(api, query, revision);
  const [overrides, setOverrides] = useState<Record<string, Override>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  /** An unread message opened under the "delay" setting, waiting to count as read. */
  const [readPending, setReadPending] = useState<string | null>(null);
  const [pinned, setPinned] = useState<MessageListItem | null>(null);
  const [loaded, setLoaded] = useState<MessageDetail | null>(null);
  const [help, setHelp] = useState(false);
  // The account whose folders are being managed, if any.
  const [managing, setManaging] = useState<string | null>(null);
  const managedAccount = managing === null ? undefined : accounts.find((a) => a.id === managing);
  const phone = usePhone();
  /** Phones: the folder menu slid in over the list. */
  const [drawer, setDrawer] = useState(false);
  const [confirmForever, setConfirmForever] = useState<MessageListItem | null>(null);
  // An account removed in Settings takes its folders with it. Staying on one
  // of them would show an empty "Folder" and start new messages from an
  // account that no longer exists, so go back to the inbox.
  if (view.type === 'folder' && accountsLoad.status === 'ready' && !accounts.some((a) => a.id === view.accountId)) {
    setView(INBOX_VIEW);
    setFilter('all');
    setSelectedId(null);
    setPinned(null);
  }
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
  /**
   * The message the reader shows: the one the user opened. A conversation
   * row takes the id of a reply that arrives later; the reader stays on the
   * opened message (and any reply being typed) until the user picks again.
   */
  const readerId = active && selectedId !== null && rowHolds(active, selectedId) ? selectedId : activeId;
  const message = loaded?.id === readerId ? loaded : null;
  /** Phones show one pane: the list, or the message the user opened. */
  const reading = selectedId !== null && activeId !== null;
  /** The account a new message starts from: the open folder's, else the first. */
  const composeFrom = () => {
    const here = view.type === 'folder' ? accounts.find((a) => a.id === view.accountId) : undefined;
    return (here ?? accounts[0])?.id ?? '';
  };

  const closeMessage = () => {
    setSelectedId(null);
    setPinned(null);
  };
  usePhoneBack(phone ? Number(drawer) + Number(screen.kind !== 'mail') + Number(reading) : 0, () => {
    if (drawer) setDrawer(false);
    else if (screen.kind === 'compose') pressEscape(document.querySelector('form.compose'));
    else if (screen.kind === 'calendar') {
      // The calendar closes an open event or the editor first.
      const section = document.querySelector('section.calendar');
      const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
      if (!section || section.dispatchEvent(escape)) setScreen({ kind: 'mail' });
    }
    else if (screen.kind !== 'mail') setScreen({ kind: 'mail' });
    else closeMessage();
  });

  /** Apply a change at once; undo it and explain if the backend refuses. */
  const optimistic = (
    ids: string[],
    patch: Pick<Override, 'unread' | 'flagged' | 'removed'>,
    call: () => Promise<void>,
    what: string,
    onRollback?: () => void,
  ) => {
    const version = versionRef.current;
    setOverrides((o) => ({ ...o, ...Object.fromEntries(ids.map((id) => [id, nextOverride(o[id], patch, version)])) }));
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
    if (!item.unread) return;
    const when = settings?.markRead ?? 'open';
    if (when === 'open') markRead(id);
    // "delay": only if it is still open a moment later (see the effect below).
    else if (when === 'delay') setReadPending(id);
  };

  const markRead = (id: string) => {
    const ids = idsOf(items.find((m) => m.id === id), id);
    optimistic([id], { unread: false }, () => api.setFlags(ids, { seen: true }), 'Could not mark as read');
  };

  // "Mark as read after a few seconds": a quick peek leaves it unread.
  const readDelayMs = (settings?.markReadDelaySecs ?? 3) * 1000;
  const markReadRef = useRef(markRead);
  useEffect(() => {
    markReadRef.current = markRead;
  });
  useEffect(() => {
    if (readPending === null || readPending !== readerId) return;
    const timer = window.setTimeout(() => {
      markReadRef.current(readPending);
    }, readDelayMs);
    return () => {
      window.clearTimeout(timer);
    };
  }, [readPending, readerId, readDelayMs]);

  // Leaving compose for anywhere else goes through compose itself, which
  // asks to save or discard a message that has unsaved text.
  const [leaveCompose, setLeaveCompose] = useState<LeaveCompose | null>(null);
  const guardCompose = useCallback((ask: LeaveCompose | null) => {
    setLeaveCompose(() => ask);
  }, []);
  const leaving = (then: () => void) => {
    const ask = screen.kind === 'compose' ? leaveCompose : null;
    if (ask) ask(then);
    else then();
  };
  const show = (next: Screen) => {
    leaving(() => {
      setScreen(next);
    });
  };

  const goTo = (next: MailView) => {
    leaving(() => {
      setView(next);
      setFocusTab('focused');
      setFilter('all');
      setSelectedId(null);
      setPinned(null);
      setScreen({ kind: 'mail' });
    });
  };

  /** Show the neighbour now; go back to the message if the backend refuses. */
  const restore = (item: MessageListItem) => () => {
    setSelectedId(item.id);
  };

  const remove = (item: MessageListItem, target: 'archive' | 'trash') => {
    // On a phone, archiving or deleting the open message returns to the list.
    setSelectedId(phone ? null : neighbourAfterRemoval(items, item.id));
    setPinned(null);
    optimistic(
      [item.id],
      { removed: true },
      () => api.moveMessages(idsOf(item), { role: target }),
      target === 'archive' ? 'Could not archive' : 'Could not delete',
      restore(item),
    );
    setNotice({
      tone: 'info',
      title: target === 'archive' ? 'Archived' : 'Moved to Trash',
      text: item.subject || '(no subject)',
      action: {
        label: 'Undo',
        run: () => {
          undoRemove(item);
        },
      },
    });
  };

  /** Undo archive or delete: put the message back where it was. */
  const undoRemove = (item: MessageListItem) => {
    setNotice(null);
    optimistic(
      [item.id],
      { removed: false },
      () => api.moveMessages(idsOf(item), { folderId: item.folderId }),
      'Could not undo',
    );
  };

  const deleteForever = (item: MessageListItem) => {
    setSelectedId(phone ? null : neighbourAfterRemoval(items, item.id));
    setPinned(null);
    optimistic([item.id], { removed: true }, () => api.deleteForever(idsOf(item)), 'Could not delete', restore(item));
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
      () => api.setFlags(idsOf(item), { seen: item.unread }),
      'Could not change read state',
    );
  };

  /**
   * Open compose. `kind` says what is being written, which decides the
   * signature (see ComposeView). `attachmentsFrom` loads copies of that
   * message's attachments into the draft. `unsaved`: the draft exists nowhere
   * else (taken back out of the Outbox).
   */
  const compose = (
    draft: Draft,
    title: string,
    kind: ComposeKind,
    { attachmentsFrom = null, unsaved = false }: { attachmentsFrom?: string | null; unsaved?: boolean } = {},
  ) => {
    show({ kind: 'compose', draft, title, compose: kind, attachmentsFrom, unsaved });
  };

  /** Copies of a message's attachments are only worth loading when it has some. */
  const filesOf = (m: MessageDetail) => ({ attachmentsFrom: m.attachments.length > 0 ? m.id : null });

  const forward = (m: MessageDetail) => {
    compose(forwardDraft(m), 'Forward', 'forward', filesOf(m));
  };

  const continueDraft = (m: MessageDetail) => {
    compose(draftFromMessage(m), 'Edit draft', 'draft', filesOf(m));
  };

  const reply = (m: MessageDetail, all: boolean) => {
    const account = accounts.find((a) => a.id === m.accountId);
    if (account) compose(replyDraft(m, account, all), all ? 'Reply all' : 'Reply', 'reply');
  };

  const newMessage = () => {
    compose(blankDraft(composeFrom()), 'New message', 'new');
  };

  // mailto: links from other apps and web pages start a new message,
  // once accounts are loaded so it has a From.
  const pendingMailto = useRef<Mailto | null>(null);
  // The tray icon's "New message".
  const newMessageRef = useRef(newMessage);
  useEffect(() => {
    newMessageRef.current = newMessage;
  });
  useEffect(
    () =>
      api.on('mail://compose', () => {
        newMessageRef.current();
      }),
    [api],
  );
  const openMailtoRef = useRef<() => void>(() => undefined);
  useEffect(() => {
    openMailtoRef.current = () => {
      const link = pendingMailto.current;
      if (!link || accounts.length === 0) return;
      pendingMailto.current = null;
      compose(
        { ...blankDraft(composeFrom()), to: link.to, cc: link.cc, bcc: link.bcc, subject: link.subject, bodyText: link.body },
        'New message',
        'new',
      );
    };
  });
  const firstAccountId = accounts[0]?.id;
  useEffect(() => {
    openMailtoRef.current();
  }, [firstAccountId]);
  useEffect(() => {
    const show = (link: Mailto | null) => {
      if (!link) return;
      pendingMailto.current = link;
      openMailtoRef.current();
    };
    api.takeMailto().then(show, () => undefined);
    const off = api.on('mail://mailto', ({ url }) => {
      api.parseMailto(url).then(show, () => undefined);
    });
    // Android hands the link over through MockyIntents when the app opens or comes back.
    const bridge = (window as { MockyIntents?: { takeMailto: () => string | null } }).MockyIntents;
    const check = () => {
      const url = bridge?.takeMailto();
      if (url) api.parseMailto(url).then(show, () => undefined);
    };
    check();
    document.addEventListener('visibilitychange', check);
    window.addEventListener('focus', check);
    return () => {
      off();
      document.removeEventListener('visibilitychange', check);
      window.removeEventListener('focus', check);
    };
  }, [api]);

  // Keyboard shortcuts.
  const handlerRef = useRef<(action: ShortcutAction, event: KeyboardEvent) => void>(() => undefined);
  useEffect(() => {
    handlerRef.current = (action, event) => {
      if (action === 'close') {
        if (
          screen.kind === 'settings' ||
          screen.kind === 'contacts' ||
          screen.kind === 'calendar' ||
          screen.kind === 'addAccount'
        ) {
          setScreen({ kind: 'mail' });
        }
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
          if (message && role !== 'drafts') forward(message);
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
      // A dialog is open (help, view source, manage folders, a confirmation…):
      // keys belong to it, not to the mail underneath.
      if (document.querySelector('[aria-modal="true"]')) return;
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
          attachmentsFrom={screen.attachmentsFrom}
          kind={screen.compose}
          unsaved={screen.unsaved}
          onLeaveGuard={guardCompose}
          title={screen.title}
          accounts={accounts}
          offlineNote={offline}
          onClose={() => {
            setScreen({ kind: 'mail' });
          }}
          onSent={(entry) => {
            setScreen({ kind: 'mail' });
            sending.track(entry);
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
    case 'contacts':
      main = (
        <ContactsView
          accounts={accounts}
          phone={phone}
          onClose={() => {
            setScreen({ kind: 'mail' });
          }}
          onWrite={(name, address) => {
            const clean = name.replace(/["\\<>]/g, '').trim();
            const to = clean ? `"${clean}" <${address}>` : address;
            compose({ ...blankDraft(composeFrom()), to }, 'New message', 'new');
          }}
          onFindMail={(address) => {
            setScreen({ kind: 'mail' });
            goTo(INBOX_VIEW);
            setSearchInput(address);
            setSelectedId(null);
            setPinned(null);
          }}
        />
      );
      break;
    case 'calendar':
      main = (
        <CalendarView
          accounts={accounts}
          phone={phone}
          defaultReminder={settings?.defaultReminderMins ?? 15}
          onClose={() => {
            setScreen({ kind: 'mail' });
          }}
          onWrite={(to) => {
            compose({ ...blankDraft(composeFrom()), to }, 'New message', 'new');
          }}
        />
      );
      break;
    case 'addAccount':
      main = (
        <div className="panel panel--center">
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
              compose(draft, 'Edit message', 'draft', { unsaved: true });
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
              focus={focusSplit ? focusTab : null}
              onFocus={(next) => {
                setFocusTab(next);
                setSelectedId(null);
                setPinned(null);
              }}
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
              onRefresh={phone ? () => api.syncNow(null) : undefined}
            />
            <ReadingPane
              ref={readerRef}
              item={active}
              messageId={readerId}
              account={accounts.find((a) => a.id === active?.accountId)}
              role={roleOf(active)}
              alwaysLoadImages={settings?.remoteImages === 'allow'}
              theme={theme}
              onLoaded={setLoaded}
              focusTab={focusSplit ? focusTab : null}
              onFocusRule={(sender, focus) => {
                api.setFocusRule(sender, focus, false).then(
                  () => {
                    setSelectedId(null);
                    setPinned(null);
                    refresh();
                    setNotice({
                      tone: 'info',
                      title: focus === 'other' ? 'Moved to Other' : 'Moved to Focused',
                      text: `Mail from ${sender} will go there from now on.`,
                    });
                  },
                  (e: unknown) => {
                    report(toAppError(e), 'Could not move');
                  },
                );
              }}
              narrow={phone}
              onBack={closeMessage}
              onReply={reply}
              onForward={forward}
              onEdit={continueDraft}
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
    <div className={phone && reading && screen.kind === 'mail' ? 'app app--reading' : 'app'}>
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
        onMenu={
          phone
            ? () => {
                setDrawer(true);
              }
            : undefined
        }
      />
      <UpdateBanner />
      <div className="workspace">
        {(!phone || drawer) && (
          <div className={phone ? 'drawer' : 'sidebar-slot'}>
            <Sidebar
              view={view}
              accounts={accounts}
              folders={foldersByAccount}
              outboxCount={outboxActive}
              onView={(next) => {
                setDrawer(false);
                goTo(next);
              }}
              onCompose={() => {
                setDrawer(false);
                newMessage();
              }}
              onSettings={() => {
                setDrawer(false);
                show({ kind: 'settings' });
              }}
              onContacts={() => {
                setDrawer(false);
                show({ kind: 'contacts' });
              }}
              onCalendar={() => {
                setDrawer(false);
                show({ kind: 'calendar' });
              }}
              onManageFolders={(accountId) => {
                setDrawer(false);
                setManaging(accountId);
              }}
              onAddAccount={() => {
                setDrawer(false);
                show({ kind: 'addAccount' });
              }}
            />
            {phone && (
              <button
                type="button"
                className="drawer__scrim"
                aria-label="Close menu"
                onClick={() => {
                  setDrawer(false);
                }}
              />
            )}
          </div>
        )}
        <main className={phone ? `panes panes--phone ${reading ? 'panes--reading' : 'panes--listing'}` : 'panes'}>
          {main}
        </main>
        {phone && screen.kind === 'mail' && !reading && view.type !== 'outbox' && accounts.length > 0 && (
          <button type="button" className="fab" aria-label="New message" onClick={newMessage}>
            <Icon name="pen" />
          </button>
        )}
      </div>
      <StatusBar sync={summary} outbox={outboxActive} accountCount={accounts.length} />

      {(notice ?? sendNotice) && (
        // A send being followed and another message (Archived, an error) are
        // both shown, stacked, so neither hides the other.
        <div className="toasts">
          {notice && (
            <Toast
              notice={notice}
              onDismiss={() => {
                setNotice(null);
              }}
            />
          )}
          {sendNotice && <Toast notice={sendNotice} onDismiss={sending.clear} />}
        </div>
      )}
      {managedAccount && (
        <ManageFolders
          account={managedAccount}
          folders={foldersByAccount[managedAccount.id] ?? []}
          onClose={() => {
            setManaging(null);
          }}
        />
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
