import { Suspense, useCallback, useEffect, useRef, useState, type DragEvent } from 'react';

import { useMailApi } from '../api/MailApiContext';
import { toAppError } from '../api/errors';
import type {
  AppError,
  Category,
  Draft,
  Folder,
  Mailto,
  MessageDetail,
  MessageListItem,
  MoveTarget,
  OutboxEntry,
  RuleInput,
  SearchScope,
  Settings,
  SyncStatus,
} from '../api/types';
import { AddAccountForm } from '../features/accounts/AddAccountForm';
import { Welcome } from '../features/accounts/Welcome';
import { ComposeView, type LeaveCompose } from '../features/compose/ComposeView';
import { preloadEditor } from '../features/compose/editor/loader';
import { blankDraft, draftFromMessage, forwardDraft, replyDraft, type ComposeKind } from '../features/compose/seed';
import { FolderPicker } from '../features/mailbox/FolderPicker';
import { groupRows } from '../features/mailbox/grouping';
import { ListTools } from '../features/mailbox/ListTools';
import { MessageList, type PickMode } from '../features/mailbox/MessageList';
import { RowActions } from '../features/mailbox/RowActions';
import {
  applyOverrides,
  folderLabel,
  INBOX_VIEW,
  resolveSelection,
  rowHolds,
  viewInfo,
  viewQuery,
  withPinned,
  type MailView,
} from '../features/mailbox/selectors';
import { SelectionBar } from '../features/mailbox/SelectionBar';
import { useListView } from '../features/mailbox/useListView';
import { countLabel, idsOf, useMessageActions, type FlagValue } from '../features/mailbox/useMessageActions';
import { useSelection } from '../features/mailbox/useSelection';
import { Sidebar } from '../features/mailbox/Sidebar';
import { useMessagePage } from '../features/mailbox/useMessagePage';
import { OutboxView } from '../features/outbox/OutboxView';
import { isActive } from '../features/outbox/outboxCopy';
import { useSendStatus, type SendStatus } from '../features/outbox/useSendStatus';
import { playChime } from '../features/notifications/chime';
import { ReadingPane } from '../features/reader/ReadingPane';
import { SettingsView } from '../features/settings/SettingsView';
import { ContactsView } from '../features/contacts/ContactsView';
import { RemindersPanel } from '../features/reminders/RemindersPanel';
import { ManageFolders } from '../features/mailbox/ManageFolders';
import { RuleEditor } from '../features/rules/RuleEditor';
import { ruleFromMessages } from '../features/rules/ruleText';
import { SearchStrip, useParsedSearch } from '../features/search/SearchStrip';
import { HelpSheet } from '../features/shortcuts/HelpSheet';
import { shortcutFor, type ShortcutAction } from '../features/shortcuts/shortcuts';
import { nowSeconds } from '../lib/format';
import { useMailEvent } from '../lib/useEvent';
import { useWakeSync } from '../lib/useWakeSync';
import { UpdateBanner } from '../features/updates/UpdateBanner';
import { useLoad } from '../lib/useLoad';
import { useTheme } from '../theme/useTheme';
import { ConfirmDialog } from '../ui/Modal';
import { Icon } from '../ui/Icon';
import { StatusBar } from './StatusBar';
import { syncSummary, type SyncSummary } from './sync';
import { usePhone, usePhoneBack } from './usePhone';
import { Toast } from './Toast';
import { TopBar } from './TopBar';
import { NOTICE_MS, useToasts, type Notice } from './useToasts';
import { CalendarScreen, preloadScreens, TasksScreen } from './screens';

type Screen =
  | { kind: 'mail' }
  | { kind: 'compose'; draft: Draft; title: string; compose: ComposeKind; attachmentsFrom: string | null; unsaved: boolean }
  | { kind: 'settings' }
  | { kind: 'contacts' }
  | { kind: 'calendar'; focus?: { eventId: string; at: number } }
  | { kind: 'tasks'; focusTaskId?: string }
  | { kind: 'addAccount' };

/** While a screen's chunk loads. */
const screenLoading = (
  <div className="panel panel--center muted" role="status">
    Loading…
  </div>
);

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
  readingPane: 'right',
  workOffline: false,
  readReceipts: 'ask',
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

/** The row to show once `removed` leave the list: the next one after them, else the one before. */
function neighbourOf(rows: readonly MessageListItem[], removed: ReadonlySet<string>): string | null {
  const last = rows.reduce((at, m, i) => (removed.has(m.id) ? i : at), -1);
  if (last < 0) return null;
  const after = rows.slice(last + 1).find((m) => !removed.has(m.id));
  if (after) return after.id;
  return [...rows.slice(0, last)].reverse().find((m) => !removed.has(m.id))?.id ?? null;
}

/** "Wanjiru's quotation" for one message, "3 messages" for several. */
function whatOf(rows: readonly MessageListItem[]): string {
  const [first] = rows;
  return rows.length === 1 && first ? first.subject || '(no subject)' : `${String(rows.length)} messages`;
}

/** Work Offline wins over whatever the accounts last reported. */
function withWorkOffline(summary: SyncSummary, on: boolean): SyncSummary {
  if (!on) return summary;
  return {
    pill: 'Working offline',
    tone: 'warn',
    status: 'Working offline. Mail is not checked or sent until you switch Work Offline off.',
    lastSync: summary.lastSync,
  };
}

export function Shell({ now = nowSeconds }: { now?: () => number }) {
  const api = useMailApi();
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => {
    setRevision((r) => r + 1);
  }, []);

  // Accounts, folders, settings, outbox and sync status.
  const accountsLoad = useLoad('accounts', revision, () => api.listAccounts());
  const accounts = accountsLoad.status === 'ready' ? [...accountsLoad.value].sort((a, b) => a.sortOrder - b.sortOrder) : [];
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
  const [categories, setCategories] = useState<Category[]>([]);
  const toasts = useToasts();
  const { notice } = toasts;
  const setNotice = toasts.show;
  /** Work Offline is on (kept in a ref so callbacks see it without changing). */
  const offlineRef = useRef(false);
  /**
   * Explain an error. While working offline the backend refuses anything that
   * needs the server with a connection error; say so plainly instead.
   */
  const report = (error: AppError, what: string) => {
    if (settings?.workOffline && error.kind === 'connect') {
      setNotice({ tone: 'info', title: "You're working offline", text: `${what}. Turn off Work Offline to connect.` });
    } else {
      toasts.report(error, what);
    }
  };
  const reportLoad = toasts.report;

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

  // The compose editor is a chunk of its own: fetch it when the app is idle,
  // where the web view can say so (otherwise compose loads it when opened).
  useEffect(() => {
    if (typeof window.requestIdleCallback !== 'function') return;
    const idle = window.requestIdleCallback(() => {
      preloadEditor().catch(() => undefined);
    });
    return () => {
      window.cancelIdleCallback(idle);
    };
  }, []);
  // The calendar and tasks screens are small chunks: fetch them straight away.
  useEffect(preloadScreens, []);

  useEffect(() => {
    api.getSettings().then(setSettings, (e: unknown) => {
      setSettings(DEFAULT_SETTINGS);
      reportLoad(toAppError(e), 'Could not load settings');
    });
  }, [api, reportLoad]);

  useEffect(() => {
    api.listOutbox().then(setOutbox, (e: unknown) => {
      reportLoad(toAppError(e), 'Could not load the Outbox');
    });
  }, [api, revision, reportLoad]);

  // Categories are few and shared by all accounts; a rename elsewhere comes with mail://changed.
  useEffect(() => {
    api.listCategories().then(setCategories, () => undefined);
  }, [api, revision]);

  useMailEvent(api, 'mail://changed', refresh);
  // Coming back (focus, wake, network) checks for new mail at once.
  useEffect(() => {
    offlineRef.current = settings?.workOffline ?? false;
  });
  const syncOnWake = useCallback(() => {
    if (offlineRef.current) return;
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
  const currentSettings = settings ?? DEFAULT_SETTINGS;
  const workOffline = currentSettings.workOffline;

  const theme = useTheme(settings?.theme ?? 'system');

  // What the list shows.
  const [screen, setScreen] = useState<Screen>({ kind: 'mail' });
  const listView = useListView();
  const { view, setView, filter, setFilter, searchInput, setSearchInput, search, focusTab, setFocusTab, order } = listView;
  const parsedSearch = useParsedSearch(api, search);
  const searching = search.trim() !== '';
  // Where a search can look from this view: a folder can widen to the folders
  // below it, its account or everything; a unified view to everything.
  const viewFolder = view.type === 'folder' ? allFolders.find((f) => f.id === view.folderId) : undefined;
  const hasSubfolders =
    viewFolder !== undefined &&
    allFolders.some(
      (f) => f.accountId === viewFolder.accountId && f.path.startsWith(`${viewFolder.path}${viewFolder.delimiter ?? '/'}`),
    );
  const scopes: SearchScope[] =
    view.type === 'folder' ? ['folder', ...(hasSubfolders ? (['subfolders'] as const) : []), 'account', 'all'] : ['folder', 'all'];
  const searchScope = scopes.includes(listView.searchScope) ? listView.searchScope : 'folder';
  const wideSearch = searching && (searchScope === 'account' || searchScope === 'all');
  const baseQuery = viewQuery(view, filter, search);
  const threads = settings?.threads ?? true;
  // Focused Inbox: inbox views show one half at a time.
  const inboxView =
    view.type === 'unified'
      ? view.role === 'inbox'
      : view.type === 'folder' && allFolders.some((f) => f.id === view.folderId && f.role === 'inbox');
  const focusSplit = (settings?.focusedInbox ?? false) && inboxView;
  const query = baseQuery && {
    ...baseQuery,
    threads,
    focus: focusSplit ? focusTab : null,
    sort: order.sort,
    descending: order.descending,
    ...(searching ? { searchScope } : {}),
  };
  const page = useMessagePage(api, query, revision);
  const roleOfFolder = (folderId: string) => allFolders.find((f) => f.id === folderId)?.role ?? null;
  const actions = useMessageActions({ api, version: page.version, refresh, report, roleOfFolder });
  const selection = useSelection();
  const { selectedId, setSelectedId, kept, setKept } = selection;
  /** An unread message opened under the "delay" setting, waiting to count as read. */
  const [readPending, setReadPending] = useState<string | null>(null);
  const [loaded, setLoaded] = useState<MessageDetail | null>(null);
  const [help, setHelp] = useState(false);
  // The account whose folders are being managed, if any.
  const [managing, setManaging] = useState<string | null>(null);
  const managedAccount = managing === null ? undefined : accounts.find((a) => a.id === managing);
  const phone = usePhone();
  /** Phones: the folder menu slid in over the list. */
  const [drawer, setDrawer] = useState(false);
  /** Messages waiting for "Delete forever?" to be answered. */
  const [confirmForever, setConfirmForever] = useState<MessageListItem[] | null>(null);
  /** "Create rule…" is open with this rule. */
  const [ruleDraft, setRuleDraft] = useState<RuleInput | null>(null);
  /** "Move to…" or "Copy to…" is open for these messages. */
  const [picker, setPicker] = useState<{ mode: 'move' | 'copy'; rows: MessageListItem[] } | null>(null);
  /** Messages being dragged to a folder (computers). */
  const [dragging, setDragging] = useState<MessageListItem[] | null>(null);
  // An account removed in Settings takes its folders with it. Staying on one
  // of them would show an empty "Folder" and start new messages from an
  // account that no longer exists, so go back to the inbox.
  if (view.type === 'folder' && accountsLoad.status === 'ready' && !accounts.some((a) => a.id === view.accountId)) {
    setView(INBOX_VIEW);
    setFilter('all');
    setSelectedId(null);
    setKept(null);
  }
  const searchRef = useRef<HTMLInputElement>(null);
  const readerRef = useRef<HTMLElement>(null);

  const filtering = filter !== 'all' || search.trim() !== '';
  const items = applyOverrides(filtering ? withPinned(page.items, kept) : page.items, actions.overrides, page.version);
  const info = viewInfo(view, accounts, allFolders);
  const outgoing = info.role === 'sent' || info.role === 'drafts';
  const groups = groupRows(items, order.groupBy, now(), outgoing);
  /** The rows in the order they are shown (grouping can reorder the page). */
  const shown = groups.flatMap((g) => g.items);
  const activeId = resolveSelection(shown, selectedId);
  const active = shown.find((m) => m.id === activeId) ?? null;
  const checkedRows = shown.filter((m) => selection.checked.has(m.id));
  const selecting = checkedRows.length > 0;
  const roleOf = (item: MessageListItem | null) =>
    item ? (allFolders.find((f) => f.id === item.folderId)?.role ?? info.role) : info.role;
  const outboxActive = (outbox ?? []).filter(isActive).length;
  const summary = withWorkOffline(syncSummary(Object.values(sync), accounts), workOffline);
  const offline = summary.tone === 'warn' || summary.tone === 'error';
  /** Where the open message shows: phones always use the full screen. */
  const pane = phone ? 'right' : currentSettings.readingPane;
  /**
   * The message the reader shows: the one the user opened. A conversation
   * row takes the id of a reply that arrives later; the reader stays on the
   * opened message (and any reply being typed) until the user picks again.
   */
  const readerId = active && selectedId !== null && rowHolds(active, selectedId) ? selectedId : activeId;
  const message = loaded?.id === readerId ? loaded : null;
  /** Phones (and the reading pane switched off) show one pane: the list, or the message the user opened. */
  const reading = selectedId !== null && activeId !== null;
  /** The account a new message starts from: the open folder's, else the first. */
  const composeFrom = () => {
    const here = view.type === 'folder' ? accounts.find((a) => a.id === view.accountId) : undefined;
    return (here ?? accounts[0])?.id ?? '';
  };

  const closeMessage = () => {
    setSelectedId(null);
    setKept(null);
  };
  usePhoneBack(phone ? Number(drawer) + Number(screen.kind !== 'mail') + Number(reading) + Number(selecting) : 0, () => {
    if (drawer) setDrawer(false);
    else if (screen.kind === 'compose') pressEscape(document.querySelector('form.compose'));
    else if (screen.kind === 'calendar') {
      // The calendar closes an open event or the editor first.
      const section = document.querySelector('section.calendar');
      const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
      if (!section || section.dispatchEvent(escape)) setScreen({ kind: 'mail' });
    }
    else if (screen.kind !== 'mail') setScreen({ kind: 'mail' });
    else if (selecting) selection.clearChecked();
    else closeMessage();
  });

  /** The user opened a message: select it and mark it read. */
  const open = (id: string) => {
    selection.clearChecked();
    setSelectedId(id);
    const item = shown.find((m) => m.id === id);
    if (!item) return;
    setKept(item);
    if (!item.unread) return;
    const when = settings?.markRead ?? 'open';
    if (when === 'open') markRead(id);
    // "delay": only if it is still open a moment later (see the effect below).
    else if (when === 'delay') setReadPending(id);
  };

  /** Opening a message reads it; that is not something to undo. */
  const markRead = (id: string) => {
    const ids = idsOf(shown.find((m) => m.id === id), id);
    actions.optimistic([id], { unread: false }, () => api.setFlags(ids, { seen: true }), 'Could not mark as read');
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
      listView.goTo(next);
      setSelectedId(null);
      setKept(null);
      selection.clearChecked();
      setScreen({ kind: 'mail' });
    });
  };

  /** The messages an action applies to: the ticked rows, else the open one. */
  const targets = (): MessageListItem[] => (selecting ? checkedRows : active ? [active] : []);

  /**
   * Rows are leaving the list. If the open message is one of them, show the
   * next one (phones, and the reading pane switched off, go back to the
   * list). Returns how to put the selection back if the backend refuses.
   */
  const leave = (rows: readonly MessageListItem[]): (() => void) => {
    const before = selectedId;
    const openRow = active;
    selection.clearChecked();
    if (openRow && rows.some((r) => r.id === openRow.id)) {
      setSelectedId(phone || pane === 'off' ? null : neighbourOf(shown, new Set(rows.map((r) => r.id))));
      setKept(null);
    }
    return () => {
      if (openRow && rows.some((r) => r.id === openRow.id)) setSelectedId(before ?? openRow.id);
    };
  };

  /** A toast for a change that took messages away, with Undo. */
  const undoable = (title: string, rows: readonly MessageListItem[], entry: ReturnType<typeof actions.move>) => {
    setNotice({
      tone: 'info',
      title,
      text: whatOf(rows),
      action: {
        label: 'Undo',
        run: () => {
          setNotice(null);
          actions.history.undoEntry(entry);
        },
      },
    });
  };

  const remove = (rows: readonly MessageListItem[], target: 'archive' | 'trash') => {
    if (rows.length === 0) return;
    const rollback = leave(rows);
    const title = target === 'archive' ? 'Archived' : 'Moved to Trash';
    const entry = actions.move(
      rows,
      { role: target },
      countLabel(title, rows),
      target === 'archive' ? 'Could not archive' : 'Could not delete',
      rollback,
    );
    undoable(title, rows, entry);
  };

  const moveTo = (rows: readonly MessageListItem[], target: MoveTarget, name: string) => {
    const moving = 'folderId' in target ? rows.filter((r) => r.folderId !== target.folderId) : rows;
    if (moving.length === 0) return;
    const rollback = leave(moving);
    const entry = actions.move(moving, target, countLabel(`Moved to ${name}`, moving), 'Could not move', rollback);
    undoable(`Moved to ${name}`, moving, entry);
  };

  const copyTo = (rows: readonly MessageListItem[], target: MoveTarget, name: string) => {
    if (rows.length === 0) return;
    selection.clearChecked();
    actions.copy(rows, target);
    setNotice({ tone: 'info', title: `Copied to ${name}`, text: whatOf(rows) });
  };

  const junk = (rows: readonly MessageListItem[], isJunk: boolean) => {
    if (rows.length === 0) return;
    const rollback = leave(rows);
    const entry = actions.markJunk(rows, isJunk, rollback);
    undoable(isJunk ? 'Marked as junk' : 'Not junk: moved to the Inbox', rows, entry);
  };

  const deleteForever = (rows: readonly MessageListItem[]) => {
    const rollback = leave(rows);
    actions.deleteForever(rows, rollback);
  };

  const deleteRows = (rows: readonly MessageListItem[]) => {
    const [first] = rows;
    if (!first) return;
    const role = roleOf(first);
    if (role === 'trash' || role === 'junk') setConfirmForever([...rows]);
    else remove(rows, 'trash');
  };

  const flagRows = (rows: readonly MessageListItem[], flag: FlagValue) => {
    if (rows.length > 0) actions.setFlag(rows, flag);
  };
  const toggleFlag = (item: MessageListItem) => {
    flagRows([item], { flagged: !item.flagged });
  };
  const toggleRead = (item: MessageListItem) => {
    actions.setRead([item], item.unread);
  };
  const togglePin = (item: MessageListItem) => {
    actions.setPinned([item], !item.pinned);
  };

  /** "New category…" made one: remember it and put it on the messages. */
  const categoryCreated = (rows: readonly MessageListItem[], all: Category[], name: string) => {
    setCategories(all);
    if (rows.length > 0) actions.setCategory(rows, name, true);
  };

  const openWindow = (id: string) => {
    if (phone) return;
    api.openMessageWindow(id).catch((e: unknown) => {
      report(toAppError(e), 'Could not open a new window');
    });
  };

  /** Ctrl+Z and Ctrl+Y, with a word about what changed. */
  const undo = () => {
    const entry = actions.history.undo();
    setNotice(
      entry
        ? { tone: 'info', title: 'Undone', text: entry.label }
        : { tone: 'info', title: 'Nothing to undo', text: 'Changes to messages can be undone here.' },
    );
  };
  const redo = () => {
    const entry = actions.history.redo();
    setNotice(
      entry
        ? { tone: 'info', title: 'Redone', text: entry.label }
        : { tone: 'info', title: 'Nothing to redo', text: 'Undo something first.' },
    );
  };

  const pick = (id: string, mode: PickMode) => {
    if (mode === 'range') selection.extendTo(shown, id, activeId);
    else if (mode === 'toggle' && !phone) selection.toggle(id, selectedId !== null ? activeId : null);
    else selection.toggle(id);
  };

  const dragRows = (id: string, event: DragEvent<HTMLElement>) => {
    const row = shown.find((m) => m.id === id);
    if (!row) return;
    const rows = selection.checked.has(id) ? checkedRows : [row];
    setDragging(rows);
    const transfer = event.dataTransfer as DataTransfer | null | undefined;
    if (transfer) {
      transfer.effectAllowed = 'copyMove';
      transfer.setData('text/plain', whatOf(rows));
    }
  };
  const dragAccount = dragging && new Set(dragging.map((r) => r.accountId)).size === 1 ? (dragging[0]?.accountId ?? null) : null;

  /**
   * Open a dialog only when nothing else is in the way.
   */
  const openPicker = (mode: 'move' | 'copy') => {
    const rows = targets();
    if (rows.length > 0) setPicker({ mode, rows });
  };

  /**
   * Compose. `kind` says what is being written, which decides the
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

  /** Show a message wherever it is (a task's or a reminder's "Open"). */
  const openMessage = (id: string) => {
    api.getMessage(id, false).then(
      (m) => {
        goTo({ type: 'folder', accountId: m.accountId, folderId: m.folderId });
        setSelectedId(m.id);
        setKept(m);
      },
      (e: unknown) => {
        report(toAppError(e), 'Could not open the message');
      },
    );
  };

  /** "Create task" on a message: a task titled with its subject, linked to it. */
  const createTask = (m: MessageDetail) => {
    api
      .listTaskLists()
      .then((lists) => {
        const list = lists.find((l) => l.provider === 'device') ?? lists[0];
        if (!list) throw new Error('There is no task list to add to yet.');
        return api.saveTask({
          id: null,
          listId: list.id,
          title: m.subject.trim() || '(no subject)',
          notes: '',
          due: null,
          reminder: null,
          importance: m.importance,
          completed: false,
          messageId: m.id,
          categories: m.categories,
        });
      })
      .then(
        (task) => {
          setNotice({
            tone: 'info',
            title: 'Task added',
            text: task.title,
            action: {
              label: 'Open',
              run: () => {
                setNotice(null);
                show({ kind: 'tasks', focusTaskId: task.id });
              },
            },
          });
        },
        (e: unknown) => {
          report(toAppError(e), 'Could not add the task');
        },
      );
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
          screen.kind === 'tasks' ||
          screen.kind === 'addAccount'
        ) {
          setScreen({ kind: 'mail' });
        } else if (screen.kind === 'mail' && selecting) {
          selection.clearChecked();
        } else if (screen.kind === 'mail' && pane === 'off' && reading) {
          closeMessage();
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
      if (action === 'undo') {
        undo();
        return;
      }
      if (action === 'redo') {
        redo();
        return;
      }
      if (view.type === 'outbox') return;
      if (action === 'selectAll') {
        selection.selectAll(shown);
        return;
      }
      const index = shown.findIndex((m) => m.id === activeId);
      if (action === 'next' || action === 'previous') {
        const target = shown[action === 'next' ? index + 1 : index - 1];
        if (target) open(target.id);
        else if (action === 'next' && page.hasMore) page.loadMore();
        return;
      }
      if (action === 'open') {
        readerRef.current?.focus();
        return;
      }
      const rows = targets();
      const [first] = rows;
      if (!first || !active) return;
      const role = roleOf(first);
      switch (action) {
        case 'reply':
        case 'replyAll':
          if (message && roleOf(active) !== 'drafts') reply(message, action === 'replyAll');
          return;
        case 'forward':
          if (message && roleOf(active) !== 'drafts') forward(message);
          return;
        case 'archive':
          if (role !== 'archive' && role !== 'trash' && role !== 'junk' && role !== 'drafts') remove(rows, 'archive');
          return;
        case 'delete':
          deleteRows(rows);
          return;
        case 'flag':
          flagRows(rows, { flagged: !rows.every((r) => r.flagged) });
          return;
        case 'toggleRead':
          actions.setRead(rows, rows.some((r) => r.unread));
          return;
        case 'move':
          openPicker('move');
          return;
      }
    };
  });
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const action = shortcutFor(event);
      if (!action) return;
      // A dialog is open (help, view source, manage folders, a confirmation…):
      // keys belong to it, not to the mail underneath. So do they in a menu.
      if (document.querySelector('[aria-modal="true"]')) return;
      if (event.target instanceof Element && event.target.closest('[role="menu"]')) return;
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

  const selectionBar = selecting && (
    <SelectionBar
      rows={checkedRows}
      shown={shown.length}
      role={info.role}
      categories={categories}
      onClear={selection.clearChecked}
      onSelectAll={() => {
        selection.selectAll(shown);
      }}
      onRead={(read) => {
        actions.setRead(checkedRows, read);
      }}
      onFlag={(flagged) => {
        flagRows(checkedRows, { flagged });
      }}
      onPin={(pinned) => {
        actions.setPinned(checkedRows, pinned);
      }}
      onCategory={(name, add) => {
        actions.setCategory(checkedRows, name, add);
      }}
      onCategoryCreated={(all, name) => {
        categoryCreated(checkedRows, all, name);
      }}
      onMove={() => {
        openPicker('move');
      }}
      onCopy={() => {
        openPicker('copy');
      }}
      onArchive={() => {
        remove(checkedRows, 'archive');
      }}
      onDelete={() => {
        deleteRows(checkedRows);
      }}
      onJunk={(isJunk) => {
        junk(checkedRows, isJunk);
      }}
      onCreateRule={() => {
        setRuleDraft(ruleFromMessages(checkedRows, allFolders));
      }}
    />
  );

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
          phone={phone}
          // Computers: carry on in a window of its own; it holds the message now.
          onPoppedOut={
            phone
              ? undefined
              : () => {
                  setScreen({ kind: 'mail' });
                }
          }
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
          settings={currentSettings}
          accounts={accounts}
          onChange={changeSettings}
          onAccountsChanged={refresh}
          categories={categories}
          onCategoriesChanged={(next) => {
            setCategories(next);
            refresh();
          }}
          phone={phone}
          folders={foldersByAccount}
          onNotice={setNotice}
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
            setKept(null);
          }}
          onWriteMany={(to) => {
            compose({ ...blankDraft(composeFrom()), to }, 'New message', 'new');
          }}
        />
      );
      break;
    case 'calendar':
      main = (
        <Suspense fallback={screenLoading}>
          <CalendarScreen
            accounts={accounts}
            phone={phone}
            defaultReminder={settings?.defaultReminderMins ?? 15}
            focus={screen.focus ?? null}
            onClose={() => {
              setScreen({ kind: 'mail' });
            }}
            onWrite={(to) => {
              compose({ ...blankDraft(composeFrom()), to }, 'New message', 'new');
            }}
          />
        </Suspense>
      );
      break;
    case 'tasks':
      main = (
        <Suspense fallback={screenLoading}>
          <TasksScreen
            accounts={accounts}
            phone={phone}
            now={now()}
            categories={categories}
            focusTaskId={screen.focusTaskId ?? null}
            onClose={() => {
              setScreen({ kind: 'mail' });
            }}
            onOpenMessage={openMessage}
          />
        </Suspense>
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
    case 'mail': {
      if (view.type === 'outbox') {
        main = (
          <OutboxView
            entries={outbox}
            accounts={accounts}
            now={now()}
            onEdit={(draft) => {
              compose(draft, 'Edit message', 'draft', { unsaved: true });
            }}
          />
        );
        break;
      }
      const list = (
        <MessageList
          title={info.title}
          subtitle={info.subtitle}
          role={info.role}
          status={page.status}
          groups={groups}
          total={page.total}
          error={page.error}
          filtered={filtering}
          filter={filter}
          focus={focusSplit ? focusTab : null}
          onFocus={(next) => {
            setFocusTab(next);
            setSelectedId(null);
            setKept(null);
          }}
          selectedId={activeId}
          accounts={accounts}
          categories={categories}
          now={now()}
          hasMore={page.hasMore}
          loadingMore={page.loadingMore}
          onFilter={(next) => {
            setFilter(next);
            setSelectedId(null);
            setKept(null);
            selection.clearChecked();
          }}
          onSelect={open}
          onLoadMore={page.loadMore}
          onRetry={refresh}
          onRefresh={phone ? () => api.syncNow(null) : undefined}
          phone={phone}
          checked={selection.checked}
          onPick={pick}
          onLongPress={(id) => {
            if (!selection.checked.has(id)) selection.toggle(id);
          }}
          onToggleRead={(id) => {
            const row = shown.find((m) => m.id === id);
            if (row) toggleRead(row);
          }}
          onOpenWindow={phone ? undefined : openWindow}
          onDragRows={phone ? undefined : dragRows}
          onDragEnd={() => {
            setDragging(null);
          }}
          rowActions={
            phone
              ? undefined
              : (item) => (
                  <RowActions
                    item={item}
                    role={roleOf(item)}
                    onPin={() => {
                      togglePin(item);
                    }}
                    onFlag={() => {
                      toggleFlag(item);
                    }}
                    onRead={() => {
                      toggleRead(item);
                    }}
                    onJunk={(isJunk) => {
                      junk([item], isJunk);
                    }}
                    onArchive={() => {
                      remove([item], 'archive');
                    }}
                    onDelete={() => {
                      deleteRows([item]);
                    }}
                  />
                )
          }
          selectionBar={selectionBar || undefined}
          folderOf={
            wideSearch
              ? (item) => {
                  const folder = allFolders.find((f) => f.id === item.folderId);
                  return folder ? folderLabel(folder) : null;
                }
              : undefined
          }
          tools={
            <ListTools
              order={order}
              onOrder={listView.setOrder}
              pane={phone ? null : pane}
              onPane={(next) => {
                changeSettings({ ...currentSettings, readingPane: next });
              }}
            />
          }
        />
      );
      const reader = (
        <ReadingPane
          ref={readerRef}
          item={active}
          messageId={readerId}
          account={accounts.find((a) => a.id === active?.accountId)}
          role={roleOf(active)}
          alwaysLoadImages={settings?.remoteImages === 'allow'}
          // Nothing is sent or asked until the setting is known.
          readReceipts={settings ? settings.readReceipts : 'never'}
          theme={theme}
          onLoaded={setLoaded}
          focusTab={focusSplit ? focusTab : null}
          onFocusRule={(sender, focus) => {
            api.setFocusRule(sender, focus, false).then(
              () => {
                setSelectedId(null);
                setKept(null);
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
          workOffline={workOffline}
          showBack={!phone && pane === 'off'}
          onBack={closeMessage}
          categories={categories}
          now={now()}
          onReply={reply}
          onForward={forward}
          onEdit={continueDraft}
          onArchive={() => {
            if (active) remove([active], 'archive');
          }}
          onDelete={() => {
            if (active) deleteRows([active]);
          }}
          onDeleteForever={() => {
            if (active) setConfirmForever([active]);
          }}
          onToggleFlag={() => {
            if (active) toggleFlag(active);
          }}
          onFlagDue={(flag) => {
            if (active) flagRows([active], flag);
          }}
          onToggleRead={() => {
            if (active) toggleRead(active);
          }}
          onTogglePin={() => {
            if (active) togglePin(active);
          }}
          onCategory={(name, add) => {
            if (active) actions.setCategory([active], name, add);
          }}
          onCategoryCreated={(all, name) => {
            categoryCreated(active ? [active] : [], all, name);
          }}
          onImportance={(importance) => {
            if (active) actions.setImportance([active], importance);
          }}
          onJunk={(isJunk) => {
            if (active) junk([active], isJunk);
          }}
          onOpenWindow={
            phone
              ? undefined
              : () => {
                  if (readerId) openWindow(readerId);
                }
          }
          onCreateRule={(m) => {
            setRuleDraft(ruleFromMessages([m], allFolders));
          }}
          onCreateTask={createTask}
        />
      );
      main = pane === 'off' ? (reading ? reader : list) : (
        <>
          {list}
          {reader}
        </>
      );
      break;
    }
  }

  const panesClass = phone
    ? `panes panes--phone ${reading ? 'panes--reading' : 'panes--listing'}`
    : `panes${screen.kind === 'mail' && pane !== 'right' ? ` panes--${pane}` : ''}`;

  return (
    <div className={phone && reading && screen.kind === 'mail' ? 'app app--reading' : 'app'}>
      <TopBar
        search={searchInput}
        searchField={searchRef}
        sync={summary}
        theme={theme}
        workOffline={workOffline}
        categories={categories}
        folders={allFolders}
        onSearch={(value) => {
          setSearchInput(value);
          setSelectedId(null);
          setKept(null);
          selection.clearChecked();
        }}
        onSync={() => api.syncNow(null)}
        onToggleTheme={() => {
          changeSettings({ ...currentSettings, theme: theme === 'dark' ? 'light' : 'dark' });
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
      {screen.kind === 'mail' && view.type !== 'outbox' && searchInput.trim() !== '' && (
        <SearchStrip
          query={searchInput}
          parsed={parsedSearch}
          scope={searchScope}
          scopes={scopes}
          {...(view.type === 'unified'
            ? { labels: { folder: `${info.title} of every account`, all: 'Every folder of every account' } }
            : {})}
          onScope={(next) => {
            listView.setSearchScope(next);
            setSelectedId(null);
            setKept(null);
            selection.clearChecked();
          }}
          onQuery={(next) => {
            setSearchInput(next);
            setSelectedId(null);
            setKept(null);
            selection.clearChecked();
          }}
        />
      )}
      <UpdateBanner />
      <div className="workspace">
        {(!phone || drawer) && (
          <div className={phone ? 'drawer' : 'sidebar-slot'}>
            <Sidebar
              view={view}
              accounts={accounts}
              folders={foldersByAccount}
              outboxCount={outboxActive}
              drag={dragging ? { accountId: dragAccount } : null}
              onDropFolder={(folder, copy) => {
                const rows = dragging ?? [];
                setDragging(null);
                if (copy) copyTo(rows, { folderId: folder.id }, folderLabel(folder));
                else moveTo(rows, { folderId: folder.id }, folderLabel(folder));
              }}
              onFavorite={(folder, favorite) => {
                api.setFolderFavorite(folder.id, favorite).then(refresh, (e: unknown) => {
                  report(toAppError(e), favorite ? 'Could not add to Favourites' : 'Could not remove from Favourites');
                });
              }}
              onReorderAccounts={(ids) => {
                api.reorderAccounts(ids).then(refresh, (e: unknown) => {
                  report(toAppError(e), 'Could not change the order of accounts');
                });
              }}
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
              onTasks={() => {
                setDrawer(false);
                show({ kind: 'tasks' });
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
        <main className={panesClass}>{main}</main>
        {phone && screen.kind === 'mail' && !reading && !selecting && view.type !== 'outbox' && accounts.length > 0 && (
          <button type="button" className="fab" aria-label="New message" onClick={newMessage}>
            <Icon name="pen" />
          </button>
        )}
      </div>
      <StatusBar
        sync={summary}
        outbox={outboxActive}
        accountCount={accounts.length}
        workOffline={workOffline}
        onWorkOffline={(on) => {
          changeSettings({ ...currentSettings, workOffline: on });
        }}
      />

      <RemindersPanel
        phone={phone}
        now={now}
        onOpen={(r) => {
          if (r.kind === 'event' && r.eventId) show({ kind: 'calendar', focus: { eventId: r.eventId, at: r.at } });
          else if (r.kind === 'task' && r.taskId) show({ kind: 'tasks', focusTaskId: r.taskId });
          else if (r.messageId) openMessage(r.messageId);
        }}
      />
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
      {picker && (
        <FolderPicker
          mode={picker.mode}
          rows={picker.rows}
          accounts={accounts}
          folders={foldersByAccount}
          onClose={() => {
            setPicker(null);
          }}
          onPick={(target, name) => {
            const { mode, rows } = picker;
            setPicker(null);
            if (mode === 'move') moveTo(rows, target, name);
            else copyTo(rows, target, name);
          }}
        />
      )}
      {ruleDraft && (
        <RuleEditor
          initial={ruleDraft}
          accounts={accounts}
          folders={foldersByAccount}
          categories={categories}
          onClose={() => {
            setRuleDraft(null);
          }}
          onSaved={(_rules, saved) => {
            setRuleDraft(null);
            setNotice({
              tone: 'info',
              title: 'Rule saved',
              text: `“${saved.name.trim()}” sorts new mail from now on. Run it on mail you already have from Settings › Rules.`,
            });
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
          title={confirmForever.length === 1 ? 'Delete forever?' : `Delete ${String(confirmForever.length)} messages forever?`}
          description={
            confirmForever.length === 1
              ? 'This message is removed from the server and cannot be recovered.'
              : `These ${String(confirmForever.length)} messages are removed from the server and cannot be recovered.`
          }
          confirmLabel="Delete forever"
          danger
          onCancel={() => {
            setConfirmForever(null);
          }}
          onConfirm={() => {
            const rows = confirmForever;
            setConfirmForever(null);
            deleteForever(rows);
          }}
        />
      )}
    </div>
  );
}
