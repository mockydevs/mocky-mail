import { appError, MailError, toAppError } from '../errors';
import { addressOf, isPlausibleAddress, parseRecipients, splitRecipients } from '../../features/compose/recipients';
import type {
  Account,
  AddressBookStatus,
  CalendarEvent,
  CalendarInfo,
  CalendarStatus,
  OutOfOffice,
  Contact,
  ContactGroup,
  ShowAs,
  Task,
  TaskList,
  AttachmentMeta,
  Category,
  Focus,
  ImageSender,
  ListQuery,
  Mailto,
  AppError,
  Draft,
  Signature,
  SignatureSet,
  UpdateInfo,
  DraftAttachment,
  DraftCheck,
  Folder,
  ImportProgress,
  ImportReport,
  Mailbox,
  MailApi,
  MailEventName,
  MailEvents,
  MessageDetail,
  MessageListItem,
  OutboxEntry,
  OpenedFile,
  RecipientCheck,
  Rule,
  RuleAction,
  ServerSuggestion,
  Settings,
} from '../types';
import { MOCK_CALENDARS, MOCK_EVENTS, mockOccurrences, mockSave, type MockEvent } from './mockCalendar';
import { dueReminders, MOCK_TASK_LISTS, MOCK_TASKS, mockFreeBusy, sortTasks } from './mockPlanner';
import {
  at,
  foldersFor,
  message as sampleMessage,
  MOCK_ACCOUNTS,
  MOCK_CATEGORIES,
  MOCK_FOLDERS,
  MOCK_MESSAGES,
  MOCK_NOW,
  MOCK_OUTBOX,
  MOCK_SETTINGS,
} from './mockData';
import { ruleMatches, serverStatusOf, validateRule } from './mockRules';
import { matchesSearch, namesFolder, parseSearchQuery } from './mockSearch';

/** A stored message; `remoteHtml` is what get_message returns when remote images are allowed. */
export type MockMessage = MessageDetail & { remoteHtml?: string };

type Method = Exclude<keyof MailApi, 'on'>;

export interface MockOptions {
  accounts?: Account[];
  messages?: MockMessage[];
  outbox?: OutboxEntry[];
  settings?: Settings;
  /** Unix seconds. */
  now?: () => number;
  oauthConfigured?: boolean;
  picked?: DraftAttachment[];
  /** A newer version for checkForUpdate to report. */
  update?: UpdateInfo;
  /** One signature per account (the default for new messages and replies), by account id. */
  signatures?: Record<string, Signature>;
  /** Full signature sets by account id; wins over `signatures`. */
  signatureSets?: Record<string, SignatureSet>;
  /** Saved contacts. */
  contacts?: Contact[];
  /** Calendars; default: sample calendars for the sample accounts. */
  calendars?: CalendarInfo[];
  /** Stored events; default: a few sample events. */
  events?: MockEvent[];
  /** Automatic replies by account id; missing: available and off. */
  outOfOffice?: Record<string, OutOfOffice>;
  /** Calendar status per account; default: every account synced. */
  calendarStatus?: CalendarStatus[];
  /** Address book status per account; default: every account synced. */
  books?: AddressBookStatus[];
  /** Dev preview: new messages go sending, then sent, like the real Outbox. */
  simulateDelivery?: boolean;
  /** Colour categories; default: a few samples. */
  categories?: Category[];
  /** Folder ids listed under Favourites. */
  favorites?: string[];
  /** Behave like the Android app: no message windows. */
  phone?: boolean;
  /** Mail rules, in order; default: none. */
  rules?: Rule[];
  /** Senders whose pictures always load; default: none. */
  imageSenders?: ImageSender[];
  /** Files the app was started with, for takeOpenedFiles. */
  openedFiles?: OpenedFile[];
  /** Milliseconds between import and export progress steps; default 10. */
  fileTickMs?: number;
  /** Contact groups; default: none. */
  groups?: ContactGroup[];
  /** To-do lists; default: a device list and a work list. */
  taskLists?: TaskList[];
  /** Tasks; default: a few samples. */
  tasks?: Task[];
  /**
   * Work out due reminders from events, tasks and flagged messages at `now()`
   * (the reminder clock); default off, so nothing pops up unasked.
   */
  reminders?: boolean;
  /** Find a Time: busy times by lower-case address (others are unknown); default: made-up times for everyone. */
  freeBusy?: Record<string, { start: number; end: number; showAs: ShowAs }[]>;
}

export interface MockMailApi extends MailApi {
  /** Fire a backend event, as the Rust side would. */
  emit<E extends MailEventName>(event: E, payload: MailEvents[E]): void;
  /** Make the next call to `method` reject with `error`. */
  failNext(method: Method, error: AppError): void;
  /** URLs passed to openExternal, for assertions. */
  readonly opened: string[];
  /** File names passed to openAttachment, for assertions. */
  readonly openedFiles: string[];
  /** Pages passed to printHtml, for assertions. */
  readonly printed: string[];
  /** Message ids passed to openMessageWindow, for assertions. */
  readonly openedWindows: string[];
  /** Keys of the compose windows opened with openComposeWindow, for assertions. */
  readonly openedComposeWindows: string[];
  /** Read receipts answered with answerReadReceipt, for assertions. */
  readonly answeredReceipts: { messageId: string; send: boolean }[];
  /** Make takeMailto return this link once, as if the app were opened with it. */
  launchWith(url: string): void;
  /** A new message arrives, as a sync would bring it in. */
  receive(message: MockMessage): void;
  /** The next file or save picker is cancelled (its command returns null). */
  cancelNextPick(): void;
  /** Imports and exports stop after their first progress step until the returned function is called. */
  holdFileTasks(): () => void;
  /** Events deleted with notify (cancellations sent), for assertions. */
  readonly cancellations: { eventId: string; occurrence: number | null }[];
  /** The reminder clock ticks: emit mail://reminders with what is due at now(). */
  tickReminders(): void;
}

/** 1x1 PNG, a one-line PDF and some text: what the dev preview shows. */
const MOCK_PNG_BASE64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
const MOCK_PNG = Uint8Array.from(
  atob(MOCK_PNG_BASE64),
  (c) => c.charCodeAt(0),
);
const MOCK_PDF = [
  '%PDF-1.4',
  '1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj',
  '2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj',
  '3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 160]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj',
  '4 0 obj<</Length 44>>stream',
  'BT /F1 18 Tf 40 80 Td (Sample attachment) Tj ET',
  'endstream endobj',
  '5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj',
  'trailer<</Root 1 0 R>>',
  '%%EOF',
].join('\n');

/** Name, or the first address for a contact without one. */
function contactSortKey(c: Contact): string {
  return c.name !== '' ? c.name : (c.emails[0] ?? '');
}

function mockBytes(kind: AttachmentMeta['preview']): ArrayBuffer {
  if (kind === 'image') return MOCK_PNG.slice().buffer;
  const text = kind === 'pdf' ? MOCK_PDF : 'Sample attachment text.\nSecond line.';
  return new TextEncoder().encode(text).slice().buffer;
}

const PALETTE = ['#2450C8', '#0F7B6C', '#8A3FB0', '#B5471B', '#1E6B3A', '#A1356B'];
const HEADER_ALLOWANCE = 16 * 1024;
/** Limit assumed when a server does not advertise one (docs/ipc.md: check_draft always returns a number). */
const DEFAULT_LIMIT = 25 * 1024 * 1024;

/** Same estimate as mail-core's limits::estimate_message_size. */
function encodedSize(raw: number): number {
  const encoded = Math.ceil(raw / 3) * 4;
  return encoded + Math.max(0, Math.ceil(encoded / 76) - 1) * 2;
}

const SUGGESTIONS: Record<string, ServerSuggestion> = {
  'gmail.com': {
    imap: { host: 'imap.gmail.com', port: 993, security: 'tls' },
    smtp: { host: 'smtp.gmail.com', port: 465, security: 'tls' },
    auth: 'google',
  },
  'outlook.com': {
    imap: { host: 'outlook.office365.com', port: 993, security: 'tls' },
    smtp: { host: 'smtp.office365.com', port: 587, security: 'starttls' },
    auth: 'microsoft',
  },
  'yahoo.com': {
    imap: { host: 'imap.mail.yahoo.com', port: 993, security: 'tls' },
    smtp: { host: 'smtp.mail.yahoo.com', port: 465, security: 'tls' },
    auth: 'password',
  },
  'mocky.co.ke': {
    imap: { host: 'mail.mailer.co.ke', port: 993, security: 'tls' },
    smtp: { host: 'mail.mailer.co.ke', port: 465, security: 'tls' },
    auth: 'password',
  },
};

/** The months a sync window may keep (null: everything). */
const SYNC_MONTHS = [1, 3, 6, 12, 24];

const HEX = /^#[0-9a-f]{6}$/i;

/** How long writing a rule to the mail server takes. */
const SERVER_WRITE_MS = 150;

/** Sort key of a list row for `sort`. */
function sortKey(m: MessageListItem, sort: NonNullable<ListQuery['sort']>): string | number {
  switch (sort) {
    case 'date':
      return m.date ?? 0;
    case 'size':
      return m.size;
    case 'from':
      return (m.from?.name ?? m.from?.address ?? '').toLocaleLowerCase();
    case 'subject':
      return m.subject.replace(/^((re|fwd?|aw|sv):\s*)+/i, '').toLocaleLowerCase();
  }
}

/** Order rows as list_messages does: by the sort, newest first on ties, pinned first in folder views. */
function ordered<T extends MessageListItem>(rows: T[], query: ListQuery): T[] {
  const sort = query.sort ?? 'date';
  const sign = query.descending === false ? 1 : -1;
  const pinnedFirst = !query.search?.trim();
  return [...rows].sort((a, b) => {
    if (pinnedFirst && a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    const ka = sortKey(a, sort);
    const kb = sortKey(b, sort);
    const by = typeof ka === 'number' && typeof kb === 'number' ? ka - kb : String(ka).localeCompare(String(kb));
    return by !== 0 ? by * sign : (b.date ?? 0) - (a.date ?? 0);
  });
}

function listItem(m: MockMessage): MessageListItem {
  return {
    id: m.id,
    accountId: m.accountId,
    folderId: m.folderId,
    from: m.from,
    to: m.to,
    subject: m.subject,
    preview: m.preview,
    date: m.date,
    unread: m.unread,
    flagged: m.flagged,
    answered: m.answered,
    hasAttachments: m.hasAttachments,
    pinned: m.pinned,
    importance: m.importance,
    categories: m.categories,
    flagDue: m.flagDue,
    size: m.size,
    threadCount: 1,
  };
}

/** Read a mailto: link the way the backend does (simplified). */
function parseMailtoLink(url: string): Mailto | null {
  if (!/^mailto:/i.test(url)) return null;
  const [addresses = '', query = ''] = url.slice(7).split('?');
  const params = new URLSearchParams(query.replace(/\+/g, '%2B'));
  const decode = (s: string) => decodeURIComponent(s);
  return {
    to: [decode(addresses), params.get('to') ?? ''].filter(Boolean).join(', '),
    cc: params.get('cc') ?? '',
    bcc: params.get('bcc') ?? '',
    subject: params.get('subject') ?? '',
    body: params.get('body') ?? '',
  };
}

/** The conversation a message belongs to, from its reply headers (as the backend does). */
function threadKey(m: MockMessage): string {
  return (m.references[0] ?? m.inReplyTo ?? m.messageIdHeader ?? m.id).toLowerCase();
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

/**
 * In-memory implementation of the whole contract, for tests and browser
 * development. Each call returns an isolated backend.
 */
export function createMockMailApi(options: MockOptions = {}): MockMailApi {
  let accounts = clone(options.accounts ?? MOCK_ACCOUNTS);
  let folders: Omit<Folder, 'unread' | 'total'>[] = MOCK_FOLDERS.filter((f) => accounts.some((a) => a.id === f.accountId));
  const known = (m: { accountId: string }) => accounts.some((a) => a.id === m.accountId);
  let messages = clone(options.messages ?? MOCK_MESSAGES).filter(known);
  let outbox = clone(options.outbox ?? MOCK_OUTBOX).filter(known);
  let settings = clone(options.settings ?? MOCK_SETTINGS);
  let categories = clone(options.categories ?? MOCK_CATEGORIES);
  const favoriteIds = new Set(options.favorites ?? []);
  folders = folders.map((f) => ({ ...f, favorite: favoriteIds.has(f.id) }));
  const openedWindows: string[] = [];
  const openedComposeWindows: string[] = [];
  const answeredReceipts: { messageId: string; send: boolean }[] = [];
  /** Drafts waiting for their compose window to take them, by key. */
  const composeDrafts = new Map<string, Draft>();
  const signatures = new Map<string, SignatureSet>(
    Object.entries(options.signatures ?? {}).map(([accountId, signature]) => [
      accountId,
      { items: [{ ...signature, id: 'main', name: 'Signature' }], defaultNew: 'main', defaultReply: 'main' },
    ]),
  );
  for (const [accountId, set] of Object.entries(options.signatureSets ?? {})) signatures.set(accountId, clone(set));
  const setOf = (accountId: string): SignatureSet =>
    signatures.get(accountId) ?? { items: [], defaultNew: null, defaultReply: null };
  /** Plain text of signature HTML, as the backend derives it. */
  const signatureText = (html: string): string => {
    const holder = document.createElement('div');
    holder.innerHTML = html.replace(/<script[\s\S]*?<\/script>/gi, '');
    holder.querySelectorAll('br').forEach((br) => {
      br.replaceWith('\n');
    });
    holder.querySelectorAll('div,p').forEach((block) => {
      block.append('\n');
    });
    return holder.textContent
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
      .join('\n');
  };
  let contacts: Contact[] = clone(options.contacts ?? []);
  let calendars: CalendarInfo[] = clone(options.calendars ?? MOCK_CALENDARS);
  let events: MockEvent[] = clone(options.events ?? MOCK_EVENTS);
  let groups: ContactGroup[] = clone(options.groups ?? []);
  let taskLists: TaskList[] = clone(options.taskLists ?? MOCK_TASK_LISTS);
  let tasks: Task[] = clone(options.tasks ?? MOCK_TASKS);
  const dismissedReminders = new Set<string>();
  const cancellations: { eventId: string; occurrence: number | null }[] = [];
  const snoozedReminders = new Map<string, number>();
  let calendarStatus: CalendarStatus[] | null = clone(options.calendarStatus ?? null);
  let bookStatus: AddressBookStatus[] | null = clone(options.books ?? null);
  const outOfOffice = new Map<string, OutOfOffice>(Object.entries(clone(options.outOfOffice ?? {})));
  const outboxDrafts = new Map<string, Draft>();
  /** Attachments of drafts saved in this session, by message id. */
  const draftFiles = new Map<string, DraftAttachment[]>();
  /** Opened .eml files read with readMessageFile, by the id they were shown with. */
  const openedMessages = new Map<string, MockMessage>();
  /** Files waiting for takeOpenedFiles. */
  let openedQueue: OpenedFile[] = clone(options.openedFiles ?? []);
  const fileTickMs = options.fileTickMs ?? 10;
  let cancelPick = false;
  /** While set, imports and exports wait here after their first step. */
  let fileGate: Promise<void> | null = null;
  const now = options.now ?? (() => MOCK_NOW);
  /** Play a sent message through sending and sent, then drop it, as Rust does. */
  const deliver = (id: string, holdSecs: number) => {
    const step = (status: OutboxEntry['status'], next?: () => void) => {
      outbox = outbox.map((e) => (e.id === id ? { ...e, status } : e));
      outboxChanged();
      next?.();
    };
    setTimeout(() => {
      // Undone meanwhile: nothing to deliver.
      if (!outbox.some((e) => e.id === id)) return;
      step({ state: 'sending', started_at: now() }, () =>
        setTimeout(() => {
          step({ state: 'sent', at: now() }, () => {
            outbox = outbox.filter((e) => e.id !== id);
            outboxChanged();
          });
        }, 1200),
      );
    }, 400 + holdSecs * 1000);
  };
  type Handler = (payload: never) => void;
  const handlers = new Map<MailEventName, Set<Handler>>();
  const failures = new Map<Method, AppError>();
  const opened: string[] = [];
  const openedFiles: string[] = [];
  const printed: string[] = [];
  let launchLink: string | null = null;
  /** Focused Inbox rules: address or @domain to focus. */
  const focusRules = new Map<string, Focus>();
  let seq = 1;
  /** Rules in the order they run, numbered from 0. */
  const renumber = (list: Rule[]) => list.map((r, order) => ({ ...r, order }));
  let rules: Rule[] = renumber(clone(options.rules ?? []));
  let imageSenders: ImageSender[] = clone(options.imageSenders ?? []);
  /** The latest server write started for each rule. */
  const serverWrites = new Map<string, number>();
  /** Whether pictures from this sender always load. */
  const trustsImages = (address: string | undefined) => {
    const a = (address ?? '').toLowerCase();
    const domain = a.slice(a.lastIndexOf('@') + 1);
    return (
      a !== '' &&
      imageSenders.some((s) => (s.wholeDomain ? domain === s.sender || domain.endsWith(`.${s.sender}`) : a === s.sender))
    );
  };

  const emit = <E extends MailEventName>(event: E, payload: MailEvents[E]) => {
    for (const handler of handlers.get(event) ?? []) (handler as (p: MailEvents[E]) => void)(clone(payload));
  };

  /** Run a command: honour failNext, copy results so callers cannot mutate state. */
  const run = <T>(method: Method, body: () => T): Promise<T> => {
    const failure = failures.get(method);
    if (failure) {
      failures.delete(method);
      return Promise.reject(toAppError(failure));
    }
    try {
      return Promise.resolve(clone(body()));
    } catch (error: unknown) {
      return Promise.reject(error instanceof MailError ? error : toAppError(error));
    }
  };

  /** Focused or Other, as the backend decides it (the mock has no list headers). */
  const focusOf = (m: MockMessage): Focus => {
    const address = (m.from?.address ?? '').toLowerCase();
    const rule = focusRules.get(address) ?? focusRules.get(address.slice(address.lastIndexOf('@')));
    if (rule) return rule;
    const writtenTo = messages.some(
      (s) => folderOf(s.folderId)?.role === 'sent' && s.to.some((t) => t.address.toLowerCase() === address),
    );
    if (writtenTo) return 'focused';
    return /^(noreply|no-reply|donotreply|notification|newsletter|news@|marketing|alerts?@|updates@)/.test(address)
      ? 'other'
      : 'focused';
  };

  const changed = (accountId: string, folderIds: string[]) => {
    emit('mail://changed', { accountId, folderIds });
  };
  const accountsChanged = () => {
    emit('mail://accounts-changed', {});
  };
  const outboxChanged = () => {
    emit('mail://outbox-changed', { entries: outbox });
  };

  const account = (id: string): Account => {
    const found = accounts.find((a) => a.id === id);
    if (!found) throw appError('not_found', 'That account no longer exists.');
    return found;
  };
  const folderOf = (id: string) => folders.find((f) => f.id === id);
  const stored = (id: string): MockMessage => {
    const found = messages.find((m) => m.id === id) ?? openedMessages.get(id);
    if (!found) throw appError('not_found', 'That message is no longer available.');
    return found;
  };

  /** Remove the saved draft with this Message-ID, as save_draft and send_message do. */
  const removeDraft = (draftMessageId: string | null) => {
    if (!draftMessageId) return;
    messages = messages.filter(
      (m) => !(m.messageIdHeader === draftMessageId && folderOf(m.folderId)?.role === 'drafts'),
    );
  };

  const check = (draft: Draft): DraftCheck => {
    const acc = account(draft.accountId);
    const to = parseRecipients(draft.to);
    const cc = parseRecipients(draft.cc);
    const bcc = parseRecipients(draft.bcc);
    const bad = [...to.invalid, ...cc.invalid, ...bcc.invalid];
    if (bad.length > 0) throw appError('invalid', `Not a valid address: ${bad.join(', ')}`);
    if (to.valid.length + cc.valid.length + bcc.valid.length === 0) throw appError('invalid', 'Add at least one recipient.');
    const estimatedBytes = draft.attachments.reduce(
      (sum, a) => sum + encodedSize(a.size),
      encodedSize(new TextEncoder().encode(draft.bodyText + (draft.bodyHtml ?? '')).length) + HEADER_ALLOWANCE,
    );
    const limitBytes = acc.sizeLimitBytes ?? DEFAULT_LIMIT;
    if (estimatedBytes > limitBytes) {
      throw appError(
        'too_large',
        `message is about ${String(estimatedBytes)} bytes once encoded; the server accepts ${String(limitBytes)}`,
      );
    }
    return { estimatedBytes, limitBytes };
  };

  const sameName = (a: string, b: string) => a.trim().toLocaleLowerCase() === b.trim().toLocaleLowerCase();
  /** The folder a message goes to for a role in its own account. */
  const roleFolder = (accountId: string, role: string) => {
    const dest = folders.find((f) => f.accountId === accountId && f.role === role);
    if (!dest) throw appError('not_found', 'That folder does not exist.');
    return dest;
  };
  const destination = (m: MockMessage, target: { role: string } | { folderId: string }) => {
    if ('folderId' in target) {
      const dest = folderOf(target.folderId);
      if (!dest) throw appError('not_found', 'That folder does not exist.');
      if (dest.accountId !== m.accountId) throw appError('invalid', 'Messages can only go to folders of their own account.');
      return dest;
    }
    return roleFolder(m.accountId, target.role);
  };
  const sortedAccounts = () => [...accounts].sort((a, b) => a.sortOrder - b.sortOrder);
  const calendarStatuses = (): CalendarStatus[] =>
    calendarStatus ?? accounts.map((a) => ({ accountId: a.id, state: 'synced' as const, message: null }));
  const books = (): AddressBookStatus[] =>
    bookStatus ?? accounts.map((a) => ({ accountId: a.id, state: 'synced' as const, message: null }));
  const shownCalendars = () => calendars.filter((c) => accounts.some((a) => a.id === c.accountId));
  const tasksChanged = () => {
    emit('mail://tasks-changed', {});
  };
  /** What the reminder clock says is due at now(). */
  const due = () => {
    if (!options.reminders) return [];
    const t = now();
    const shown = new Set(shownCalendars().map((c) => c.id));
    return dueReminders({
      now: t,
      events: mockOccurrences(
        events.filter((e) => shown.has(e.event.calendarId)),
        t - 86_400,
        t + 8 * 86_400,
      ),
      tasks,
      flags: messages,
      dismissed: dismissedReminders,
      snoozed: snoozedReminders,
    });
  };
  const ruleWorld = () => ({ accounts, folders, categories });

  const wait = (ms: number) =>
    new Promise<void>((resolve) => {
      setTimeout(resolve, ms);
    });
  /**
   * A long import or export, as the backend runs it: honours failNext, checks
   * what it can at once, then reports each step as mail://import-progress
   * before it finishes.
   */
  const slow = <T>(method: Method, steps: ImportProgress[], body: () => T, check?: () => void): Promise<T> => {
    const failure = failures.get(method);
    if (failure) {
      failures.delete(method);
      return Promise.reject(toAppError(failure));
    }
    try {
      check?.();
    } catch (error: unknown) {
      return Promise.reject(error instanceof MailError ? error : toAppError(error));
    }
    return (async () => {
      for (const [index, step] of steps.entries()) {
        await wait(fileTickMs);
        emit('mail://import-progress', step);
        if (index === 0 && fileGate) await fileGate;
      }
      await wait(fileTickMs);
      try {
        return clone(body());
      } catch (error: unknown) {
        throw error instanceof MailError ? error : toAppError(error);
      }
    })();
  };
  /** The user cancelled the file or save picker (see cancelNextPick). */
  const pickCancelled = () => {
    const was = cancelPick;
    cancelPick = false;
    return was;
  };
  const counting = (phase: string, total: number): ImportProgress[] =>
    Array.from({ length: total }, (_, i) => ({ phase, done: i + 1, total }));
  const needsServer = () => {
    if (settings.workOffline) throw appError('connect', 'Work Offline is on, so nothing can go to the mail server.');
  };
  const baseName = (path: string) => path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1);
  /** The mock's damaged files: any path naming itself broken or unreadable. */
  const readable = (path: string) => {
    if (/broken|unreadable/i.test(path)) {
      throw appError('invalid', `${baseName(path)} could not be read. It may be damaged, or not the kind of file its name says.`);
    }
  };
  const calendarOf = (id: string): CalendarInfo => {
    const found = calendars.find((c) => c.id === id && accounts.some((a) => a.id === c.accountId));
    if (!found) throw appError('not_found', 'That calendar no longer exists.');
    return found;
  };
  const folderFor = (id: string) => {
    const found = folderOf(id);
    if (!found) throw appError('not_found', 'That folder no longer exists.');
    return found;
  };
  const accountOrDevice = (accountId: string | null) => {
    if (accountId !== null && accountId !== '*') account(accountId);
  };
  /** Events from an imported file, a day apart from tomorrow. */
  const addEvents = (calendar: CalendarInfo, titles: string[]) => {
    const day = Math.floor(now() / 86_400) * 86_400 + 86_400;
    for (const [i, title] of titles.entries()) {
      const id = `e${String(seq++)}`;
      const start = day + i * 86_400 + 9 * 3600;
      const event: CalendarEvent = {
        id,
        calendarId: calendar.id,
        accountId: calendar.accountId,
        color: calendar.color,
        uid: `${id}@import`,
        recurrenceId: null,
        title,
        location: '',
        notes: '',
        start,
        end: start + 3600,
        allDay: false,
        repeat: 'none',
        recurring: false,
        organizer: null,
        attendees: [],
        tentative: false,
        reminders: [15],
        showAs: 'busy',
        private: false,
        repeatRule: null,
        isOrganizer: true,
      };
      events = [...events, { event, skipped: [] }];
    }
    emit('mail://calendar-changed', { calendars: [] });
  };
  /** Contacts from an imported file. */
  const addContacts = (accountId: string | null, people: [string, string][]) => {
    for (const [name, email] of people) {
      const id = `c${String(seq++)}`;
      contacts = [
        ...contacts,
        { id, uid: `u${id}`, accountId, name, emails: [email], phones: [], organization: '', title: '', notes: '', favorite: false, categories: [], photo: null, state: accountId ? 'changed' : 'synced' },
      ];
    }
    emit('mail://contacts-changed', {
      books: options.books ?? accounts.map((a) => ({ accountId: a.id, state: 'synced' as const, message: null })),
    });
  };
  /** Messages uploaded into a folder from imported files. */
  const addMessages = (folder: Omit<Folder, 'unread' | 'total'>, subjects: string[]) => {
    const date = now();
    for (const [i, subject] of subjects.entries()) {
      const id = `imp${String(seq++)}`;
      messages = [
        ...messages,
        sampleMessage({
          id,
          accountId: folder.accountId,
          folderId: folder.id,
          subject,
          from: { name: 'Imported sender', address: 'sender@imported.example' },
          preview: `${subject}, brought in from a file.`,
          text: `${subject}, brought in from a file.`,
          date: date - (i + 1) * 86_400,
        }),
      ];
    }
    changed(folder.accountId, [folder.id]);
  };
  /** The message in an opened .eml file. */
  const messageFile = (path: string): MockMessage =>
    sampleMessage({
      id: `file:${path}`,
      accountId: '',
      folderId: '',
      subject: 'Signed drawings for the Kilimani house',
      from: { name: 'Amina Hassan', address: 'amina@studio.example' },
      to: [{ name: null, address: 'you@work.example' }],
      preview: 'Here are the signed drawings. The client approved the kitchen layout.',
      text: 'Hi,\n\nHere are the signed drawings. The client approved the kitchen layout.\n\nAmina',
      date: at('2026-11-10T08:30:00Z'),
      attachments: [{ index: 0, fileName: 'Drawings-signed.pdf', mimeType: 'application/pdf', size: 240 * 1024, preview: 'pdf' }],
      messageIdHeader: '<drawings-77@studio.example>',
    });

  /** Do what a rule says to one message. */
  const applyAction = (id: string, action: RuleAction) => {
    const m = messages.find((x) => x.id === id);
    if (!m) return;
    const update = (patch: Partial<MockMessage>) => {
      messages = messages.map((x) => (x.id === id ? { ...x, ...patch } : x));
    };
    const target = () => (action.folderId ? { folderId: action.folderId } : { role: action.role ?? 'inbox' });
    switch (action.kind) {
      case 'move':
        update({ folderId: destination(m, target()).id });
        return;
      case 'copy':
        messages = [...messages, { ...clone(m), id: `copy${String(seq++)}`, folderId: destination(m, target()).id }];
        return;
      case 'categorize': {
        const name = categories.find((c) => sameName(c.name, action.category ?? ''))?.name;
        if (name && !m.categories.some((c) => sameName(c, name))) update({ categories: [...m.categories, name] });
        return;
      }
      case 'flag':
        update({ flagged: true });
        return;
      case 'clearFlag':
        update({ flagged: false, flagDue: null });
        return;
      case 'clearCategories':
        update({ categories: [] });
        return;
      case 'markRead':
        update({ unread: false });
        return;
      case 'setImportance':
        update({ importance: action.importance ?? 'normal' });
        return;
      case 'delete':
        update({ folderId: roleFolder(m.accountId, 'trash').id });
        return;
      case 'pin':
        update({ pinned: true });
        return;
      case 'notify':
        return;
    }
  };

  const api: MockMailApi = {
    opened,
    openedFiles,
    printed,
    openedWindows,
    openedComposeWindows,
    answeredReceipts,
    cancellations,
    launchWith: (url) => {
      launchLink = url;
    },
    receive: (message) => {
      messages = [...messages, clone(message)];
      changed(message.accountId, [message.folderId]);
    },
    tickReminders: () => {
      emit('mail://reminders', { reminders: due() });
    },
    emit,
    failNext(method, error) {
      failures.set(method, error);
    },
    cancelNextPick: () => {
      cancelPick = true;
    },
    holdFileTasks: () => {
      let release: () => void = () => undefined;
      fileGate = new Promise<void>((resolve) => {
        release = resolve;
      });
      return () => {
        fileGate = null;
        release();
      };
    },

    suggestRecipients: (query) =>
      run('suggestRecipients', () => {
        const q = query.trim().toLowerCase();
        if (!q) return [];
        const own = new Set(accounts.map((a) => a.email.toLowerCase()));
        const seen = new Map<string, Mailbox>();
        for (const m of messages) {
          for (const p of [m.from, ...m.to].filter((x): x is Mailbox => x !== null)) {
            const key = p.address.toLowerCase();
            const name = p.name?.toLowerCase() ?? '';
            const hit = key.startsWith(q) || name.startsWith(q) || name.includes(` ${q}`);
            if (hit && !own.has(key) && !seen.has(key)) seen.set(key, p);
          }
        }
        return [...seen.values()].slice(0, 8);
      }),

    suggestServers: (email) =>
      run('suggestServers', () => SUGGESTIONS[email.split('@')[1]?.toLowerCase() ?? ''] ?? null),

    addAccount: (input) =>
      run('addAccount', () => {
        if (!parseRecipients(input.email).valid.length) throw appError('invalid', 'Enter a valid email address.');
        if (input.imap.host.endsWith('.invalid')) throw appError('connect', `Could not connect to ${input.imap.host}.`);
        if (input.password === 'wrong') throw appError('auth', 'The server rejected the username or password.');
        if (accounts.some((a) => a.email.toLowerCase() === input.email.toLowerCase())) {
          throw appError('invalid', 'That account is already added.');
        }
        const id = `acc${String(seq++)}`;
        const created: Account = {
          id,
          email: input.email,
          displayName: input.displayName || input.email,
          auth: 'password',
          createdAt: now(),
          color: PALETTE[accounts.length % PALETTE.length] ?? '#2450C8',
          imap: input.imap,
          smtp: input.smtp,
          sizeLimitBytes: 25 * 1024 * 1024,
          sortOrder: Math.max(-1, ...accounts.map((a) => a.sortOrder)) + 1,
          syncMonths: null,
        };
        accounts = [...accounts, created];
        folders = [...folders, ...foldersFor(id)];
        accountsChanged();
        return created;
      }),

    addOAuthAccount: (input) =>
      run('addOAuthAccount', () => {
        if (!options.oauthConfigured) {
          const name = input.provider === 'google' ? 'Google' : 'Microsoft';
          throw appError('config', `${name} sign-in is not configured in this build yet.`);
        }
        const id = `acc${String(seq++)}`;
        // Like the real flow: with no address typed, the account picked in
        // the provider's chooser is used.
        const email = input.email || (input.provider === 'google' ? 'you@gmail.com' : 'you@outlook.com');
        const created: Account = {
          id,
          email,
          displayName: input.displayName || email,
          auth: input.provider,
          createdAt: now(),
          color: PALETTE[accounts.length % PALETTE.length] ?? '#2450C8',
          imap: { host: 'imap.gmail.com', port: 993, security: 'tls' },
          smtp: { host: 'smtp.gmail.com', port: 465, security: 'tls' },
          sizeLimitBytes: 25 * 1024 * 1024,
          sortOrder: Math.max(-1, ...accounts.map((a) => a.sortOrder)) + 1,
          syncMonths: null,
        };
        accounts = [...accounts, created];
        folders = [...folders, ...foldersFor(id)];
        accountsChanged();
        return created;
      }),

    updateAccount: (id, patch) =>
      run('updateAccount', () => {
        const acc = account(id);
        if (patch.password === 'wrong') throw appError('auth', 'The server rejected the new password.');
        if (patch.syncMonths !== undefined && patch.syncMonths !== null && !SYNC_MONTHS.includes(patch.syncMonths)) {
          throw appError('invalid', 'Choose 1, 3, 6, 12 or 24 months, or all mail.');
        }
        const next = {
          ...acc,
          ...(patch.displayName !== undefined ? { displayName: patch.displayName } : {}),
          ...(patch.syncMonths !== undefined ? { syncMonths: patch.syncMonths } : {}),
        };
        accounts = accounts.map((a) => (a.id === id ? next : a));
        accountsChanged();
        return next;
      }),

    reconnectAccount: (id) =>
      run('reconnectAccount', () => {
        const acc = account(id);
        if (acc.auth === 'password') throw appError('invalid', 'This account signs in with a password.');
        accountsChanged();
        return acc;
      }),

    removeAccount: (id) =>
      run('removeAccount', () => {
        account(id);
        accounts = accounts.filter((a) => a.id !== id);
        folders = folders.filter((f) => f.accountId !== id);
        messages = messages.filter((m) => m.accountId !== id);
        outbox = outbox.filter((o) => o.accountId !== id);
        accountsChanged();
        outboxChanged();
      }),

    listAccounts: () => run('listAccounts', sortedAccounts),

    reorderAccounts: (ids) =>
      run('reorderAccounts', () => {
        if (ids.length !== accounts.length || !accounts.every((a) => ids.includes(a.id))) {
          throw appError('invalid', 'List every account once.');
        }
        accounts = accounts.map((a) => ({ ...a, sortOrder: ids.indexOf(a.id) }));
        accountsChanged();
      }),

    createFolder: (accountId, name, parentId) =>
      run('createFolder', () => {
        const clean = name.trim().replace(/\s+/g, ' ');
        if (!clean) throw appError('invalid', 'Type a name for the folder.');
        const parent = parentId ? folders.find((f) => f.id === parentId) : undefined;
        const delimiter = parent?.delimiter ?? '/';
        if (clean.includes(delimiter)) throw appError('invalid', `Folder names can't contain "${delimiter}" on this server.`);
        const path = parent ? `${parent.path}${delimiter}${clean}` : clean;
        if (folders.some((f) => f.accountId === accountId && f.path.toLowerCase() === path.toLowerCase())) {
          throw appError('invalid', "There's already a folder with that name here.");
        }
        folders = [
          ...folders,
          { id: `${accountId}:${path}`, accountId, name: clean, path, role: 'other', delimiter, favorite: false },
        ];
        changed(accountId, []);
      }),

    renameFolder: (folderId, name) =>
      run('renameFolder', () => {
        const folder = folders.find((f) => f.id === folderId);
        if (!folder) throw appError('not_found', 'That folder no longer exists.');
        if (folder.role !== 'other') throw appError('invalid', `${folder.name} is a built-in folder, so it can't be renamed or deleted.`);
        const clean = name.trim();
        if (!clean) throw appError('invalid', 'Type a name for the folder.');
        const d = folder.delimiter ?? '/';
        const parent = folder.path.includes(d) ? folder.path.slice(0, folder.path.lastIndexOf(d) + 1) : '';
        const path = `${parent}${clean}`;
        folders = folders.map((f) =>
          f.id === folderId ? { ...f, name: clean, path } : f.path.startsWith(`${folder.path}${d}`) ? { ...f, path: path + f.path.slice(folder.path.length) } : f,
        );
        changed(folder.accountId, []);
      }),

    deleteFolder: (folderId) =>
      run('deleteFolder', () => {
        const folder = folders.find((f) => f.id === folderId);
        if (!folder) throw appError('not_found', 'That folder no longer exists.');
        if (folder.role !== 'other') throw appError('invalid', `${folder.name} is a built-in folder, so it can't be renamed or deleted.`);
        const d = folder.delimiter ?? '/';
        if (folders.some((f) => f.path.startsWith(`${folder.path}${d}`) && f.accountId === folder.accountId)) {
          throw appError('invalid', `${folder.name} has folders inside it. Delete or move those first.`);
        }
        folders = folders.filter((f) => f.id !== folderId);
        messages = messages.filter((m) => m.folderId !== folderId);
        changed(folder.accountId, []);
      }),

    setFolderFavorite: (folderId, favorite) =>
      run('setFolderFavorite', () => {
        const folder = folderOf(folderId);
        if (!folder) throw appError('not_found', 'That folder no longer exists.');
        folders = folders.map((f) => (f.id === folderId ? { ...f, favorite } : f));
        changed(folder.accountId, []);
      }),

    listFolders: (accountId) =>
      run('listFolders', () =>
        folders
          .filter((f) => f.accountId === accountId)
          .map((f) => {
            const inFolder = messages.filter((m) => m.folderId === f.id);
            return { ...f, unread: inFolder.filter((m) => m.unread).length, total: inFolder.length };
          }),
      ),

    listMessages: (query) =>
      run('listMessages', () => {
        if (query.limit > 200 || query.limit < 1) throw appError('invalid', 'limit must be 1 to 200');
        const searching = (query.search?.trim() ?? '') !== '';
        const { terms } = parseSearchQuery(query.search ?? '', now());
        // Where a search looks: the open view, it and the folders below it, the account, or everything.
        const asked = searching ? (query.searchScope ?? 'folder') : 'folder';
        // Naming a folder with in: looks in the whole account (Trash and Spam
        // included); otherwise wide searches leave Trash and Spam out.
        const named = namesFolder(terms);
        const scope = named && (asked === 'folder' || asked === 'subfolders') ? 'account' : asked;
        const open = query.folderId === null ? undefined : folderOf(query.folderId);
        const below = (folderId: string) => {
          const f = folderOf(folderId);
          const d = open?.delimiter ?? '/';
          return !!open && !!f && f.accountId === open.accountId && f.path.startsWith(`${open.path}${d}`);
        };
        const wide = scope === 'all' || scope === 'account';
        const matches = messages
          .filter((m) => scope === 'all' || query.accountId === null || m.accountId === query.accountId)
          .filter(
            (m) =>
              wide ||
              query.folderId === null ||
              m.folderId === query.folderId ||
              (scope === 'subfolders' && below(m.folderId)),
          )
          .filter((m) => wide || query.role === null || folderOf(m.folderId)?.role === query.role)
          .filter((m) => !wide || named || !['trash', 'junk'].includes(folderOf(m.folderId)?.role ?? ''))
          .filter((m) => query.filter !== 'unread' || m.unread)
          .filter((m) => query.filter !== 'flagged' || m.flagged)
          .filter((m) => query.filter !== 'attachments' || m.hasAttachments)
          .filter((m) => query.filter !== 'pinned' || m.pinned)
          .filter((m) => {
            if (query.filter !== 'toMe') return true;
            const own = accounts.find((a) => a.id === m.accountId)?.email.toLowerCase();
            return [...m.to, ...m.cc].some((t) => t.address.toLowerCase() === own);
          })
          .filter((m) => !query.focus || folderOf(m.folderId)?.role !== 'inbox' || focusOf(m) === query.focus)
          .filter((m) => matchesSearch(m, terms, { now: now(), folderOf }))
          .sort((a, b) => (b.date ?? 0) - (a.date ?? 0));
        if (!query.threads) {
          return {
            items: ordered(matches.map(listItem), query).slice(query.offset, query.offset + query.limit),
            total: matches.length,
          };
        }
        // One row per conversation: its newest message, unread if any is.
        const groups = new Map<string, MockMessage[]>();
        for (const m of matches) {
          const key = `${m.accountId}\u0000${threadKey(m)}`;
          groups.set(key, [...(groups.get(key) ?? []), m]);
        }
        const rows = [...groups.values()].map((group) => {
          const [newest] = group;
          if (!newest) throw appError('internal', 'empty conversation');
          return {
            ...listItem(newest),
            unread: group.some((m) => m.unread),
            pinned: group.some((m) => m.pinned),
            categories: [...new Set(group.flatMap((m) => m.categories))],
            threadCount: group.length,
            threadIds: group.map((m) => m.id),
          };
        });
        return { items: ordered(rows, query).slice(query.offset, query.offset + query.limit), total: rows.length };
      }),

    setFocusRule: (sender, focus, wholeDomain) =>
      run('setFocusRule', () => {
        const address = sender.trim().toLowerCase();
        const at = address.lastIndexOf('@');
        if (at < 1) throw appError('invalid', "That isn't an email address.");
        const key = wholeDomain ? address.slice(at) : address;
        if (focus) focusRules.set(key, focus);
        else focusRules.delete(key);
      }),

    resolveRecipients: (text) =>
      run('resolveRecipients', () => {
        const own = new Set(accounts.map((a) => a.email.toLowerCase()));
        // Everyone the mock knows: saved contacts, then people in the mail.
        const people = new Map<string, Mailbox>();
        for (const c of contacts.filter((x) => x.state !== 'deleted')) {
          for (const address of c.emails) people.set(address.toLowerCase(), { name: c.name || null, address });
        }
        for (const m of messages) {
          for (const p of [m.from, ...m.to, ...m.cc].filter((x): x is Mailbox => x !== null)) {
            const key = p.address.toLowerCase();
            if (!own.has(key) && !people.has(key)) people.set(key, p);
          }
        }
        const result: RecipientCheck = { resolved: [], ambiguous: [], invalid: [] };
        const add = (mailbox: Mailbox) => {
          if (!result.resolved.some((m) => m.address.toLowerCase() === mailbox.address.toLowerCase())) result.resolved.push(mailbox);
        };
        for (const entry of splitRecipients(text)) {
          const address = addressOf(entry);
          if (address !== null && isPlausibleAddress(address)) {
            const typed = /^(.*?)<[^<>]*>\s*$/.exec(entry)?.[1]?.trim().replace(/^"|"$/g, '');
            add({ name: typed !== undefined && typed !== '' ? typed : (people.get(address.toLowerCase())?.name ?? null), address });
            continue;
          }
          const q = entry.trim().toLowerCase();
          const candidates = [...people.values()].filter((p) => {
            const name = p.name?.toLowerCase() ?? '';
            return name !== '' && (name.startsWith(q) || name.split(/\s+/).some((word) => word.startsWith(q)));
          });
          // Of several, one whose name is exactly what was typed wins, as in the backend.
          const exact = candidates.filter((p) => p.name?.toLowerCase() === q);
          const one = candidates.length === 1 ? candidates[0] : exact.length === 1 ? exact[0] : undefined;
          if (one) add(one);
          else if (candidates.length > 0) result.ambiguous.push({ input: entry, candidates });
          else result.invalid.push(entry);
        }
        return result;
      }),

    contactCardAttachment: (contactId) =>
      run('contactCardAttachment', () => {
        const found = contacts.find((c) => c.id === contactId && c.state !== 'deleted');
        if (!found) throw appError('not_found', 'That contact no longer exists.');
        const name = (found.name !== '' ? found.name : (found.emails[0] ?? 'Contact')).replace(/[\\/:*?"<>|]/g, '_');
        return { path: `/tmp/mocky-mail/contacts/${found.id}.vcf`, fileName: `${name}.vcf`, size: 180 + name.length };
      }),

    messageAsAttachment: (messageId) =>
      run('messageAsAttachment', () => {
        const m = stored(messageId);
        const name = (m.subject !== '' ? m.subject : 'Message').replace(/[\\/:*?"<>|]/g, '_');
        return { path: `/tmp/mocky-mail/items/${m.id}.eml`, fileName: `${name}.eml`, size: m.size };
      }),

    answerReadReceipt: (messageId, send) =>
      run('answerReadReceipt', () => {
        const m = stored(messageId);
        if (!m.readReceipt) throw appError('invalid', 'This message did not ask for a read receipt.');
        if (m.readReceipt.answered) throw appError('invalid', 'The read receipt was already answered.');
        messages = messages.map((x) =>
          x.id === messageId && x.readReceipt ? { ...x, readReceipt: { ...x.readReceipt, answered: true } } : x,
        );
        answeredReceipts.push({ messageId, send });
      }),

    openComposeWindow: (draft) =>
      run('openComposeWindow', () => {
        if (options.phone) throw appError('invalid', 'Phones write one message at a time.');
        const key = `compose-${String(seq++)}`;
        composeDrafts.set(key, clone(draft));
        openedComposeWindows.push(key);
      }),

    takeComposeDraft: (key) =>
      run('takeComposeDraft', () => {
        const draft = composeDrafts.get(key) ?? null;
        composeDrafts.delete(key);
        return draft;
      }),

    importMailFiles: (folderId) => {
      if (pickCancelled()) return Promise.resolve(null);
      const subjects = ['Site visit notes, Karen', 'Invoice INV-0098', 'Holiday plans'];
      return slow(
        'importMailFiles',
        [{ phase: 'Reading files', done: 0, total: 0 }, ...counting('Uploading messages', subjects.length)],
        (): ImportReport => {
          const folder = folderFor(folderId);
          addMessages(folder, subjects);
          return {
            imported: subjects.length,
            skipped: 1,
            problems: ['newsletter.eml was left out: it is not an email message.'],
            folderId: folder.id,
          };
        },
        () => {
          folderFor(folderId);
          needsServer();
        },
      );
    },

    exportMail: (target, format) => {
      if (pickCancelled()) return Promise.resolve(null);
      const check = () => {
        if ('folderId' in target) {
          folderFor(target.folderId);
          if (format !== 'mbox') throw appError('invalid', 'A folder is saved as one .mbox file.');
        } else {
          if (format !== 'eml') throw appError('invalid', 'Chosen messages are saved as .eml files in a .zip.');
          if (target.ids.length === 0) throw appError('invalid', 'Choose at least one message.');
          for (const id of target.ids) stored(id);
        }
      };
      const count = 'folderId' in target ? messages.filter((m) => m.folderId === target.folderId).length : target.ids.length;
      return slow(
        'exportMail',
        [{ phase: 'Saving messages', done: 0, total: count }, ...counting('Saving messages', count)],
        () =>
          'folderId' in target
            ? `/home/you/Documents/${folderFor(target.folderId).name}.mbox`
            : `/home/you/Documents/Messages (${String(target.ids.length)}).zip`,
        check,
      );
    },

    importCalendarFile: (calendarId) => {
      if (pickCancelled()) return Promise.resolve(null);
      const titles = ['Board meeting', 'Site handover, Kilimani'];
      return slow(
        'importCalendarFile',
        [{ phase: 'Reading the calendar file', done: 0, total: 0 }, ...counting('Adding events', titles.length)],
        (): ImportReport => {
          const calendar = calendarOf(calendarId);
          if (!calendar.writable) throw appError('invalid', "Events can't be added to this calendar. Choose another one.");
          addEvents(calendar, titles);
          return { imported: titles.length, skipped: 1, problems: ['“Team lunch” is already in this calendar.'] };
        },
        () => {
          calendarOf(calendarId);
        },
      );
    },

    exportCalendar: (calendarId) => {
      if (pickCancelled()) return Promise.resolve(null);
      return slow(
        'exportCalendar',
        [{ phase: 'Saving events', done: 0, total: 0 }],
        () => `/home/you/Documents/${calendarOf(calendarId).name}.ics`,
        () => {
          calendarOf(calendarId);
        },
      );
    },

    importContactsFile: (accountId) => {
      if (pickCancelled()) return Promise.resolve(null);
      const people: [string, string][] = [
        ['Grace Otieno', 'grace@supplier.example'],
        ['Daniel Kiptoo', 'daniel@kiptoo.example'],
      ];
      return slow(
        'importContactsFile',
        [{ phase: 'Reading contacts', done: 0, total: 0 }, ...counting('Adding contacts', people.length)],
        (): ImportReport => {
          addContacts(accountId, people);
          return { imported: people.length, skipped: 0, problems: [] };
        },
        () => {
          if (accountId === '*') throw appError('invalid', 'Choose an account or this device.');
          accountOrDevice(accountId);
        },
      );
    },

    exportContacts: (accountId) => {
      if (pickCancelled()) return Promise.resolve(null);
      return slow(
        'exportContacts',
        [{ phase: 'Saving contacts', done: 0, total: 0 }],
        () => {
          const who = accountId === '*' ? 'All contacts' : accountId === null ? 'This device' : account(accountId).displayName;
          return `/home/you/Documents/${who}.vcf`;
        },
        () => {
          accountOrDevice(accountId);
        },
      );
    },

    importPst: (accountId) => {
      if (pickCancelled()) return Promise.resolve(null);
      const inbox = ['Quarterly report', 'Re: Kitchen tiles', 'Staff party'];
      const sent = ['Signed contract'];
      return slow(
        'importPst',
        [
          { phase: 'Reading the Outlook file', done: 0, total: 0 },
          ...counting('Uploading Inbox', inbox.length),
          { phase: 'Uploading Sent Items', done: 1, total: 1 },
          { phase: 'Adding contacts and calendar', done: 1, total: 1 },
        ],
        (): ImportReport => {
          const acc = account(accountId);
          const delimiter = folders.find((f) => f.accountId === acc.id)?.delimiter ?? '/';
          const top = 'Imported from Outlook';
          const made = (name: string, path: string) => {
            const id = `${acc.id}:${path}`;
            const existing = folders.find((f) => f.id === id);
            if (existing) return existing;
            const folder = { id, accountId: acc.id, name, path, role: 'other' as const, delimiter, favorite: false };
            folders = [...folders, folder];
            return folder;
          };
          const parent = made(top, top);
          addMessages(made('Inbox', `${top}${delimiter}Inbox`), inbox);
          addMessages(made('Sent Items', `${top}${delimiter}Sent Items`), sent);
          addContacts(acc.id, [['Peter Njoroge', 'peter@njoroge.example']]);
          const calendar = calendars.find((c) => c.accountId === acc.id);
          if (calendar) addEvents(calendar, ['Planning day']);
          const counts = { messages: inbox.length + sent.length, contacts: 1, events: calendar ? 1 : 0, tasks: 2, notes: 1 };
          return {
            imported: counts.messages + counts.contacts + counts.events + counts.tasks + counts.notes,
            skipped: 1,
            problems: ['“Scanned drawings” was left out: it is larger than the server accepts (25 MB).'],
            counts,
            folderId: parent.id,
          };
        },
        () => {
          account(accountId);
          needsServer();
        },
      );
    },

    takeOpenedFiles: () =>
      run('takeOpenedFiles', () => {
        const files = openedQueue;
        openedQueue = [];
        return files;
      }),

    readMessageFile: (path) =>
      run('readMessageFile', () => {
        readable(path);
        const found = messageFile(path);
        openedMessages.set(found.id, found);
        return found;
      }),

    importMessageFile: (path, folderId) =>
      run('importMessageFile', () => {
        readable(path);
        const folder = folderFor(folderId);
        needsServer();
        messages = [
          ...messages,
          { ...messageFile(path), id: `imp${String(seq++)}`, accountId: folder.accountId, folderId: folder.id },
        ];
        changed(folder.accountId, [folder.id]);
      }),

    importOpenedFile: (path, target) =>
      slow(
        'importOpenedFile',
        [{ phase: 'calendarId' in target ? 'Adding events' : 'Adding contacts', done: 0, total: 2 }, ...counting('calendarId' in target ? 'Adding events' : 'Adding contacts', 2)],
        (): ImportReport => {
          if ('calendarId' in target) addEvents(calendarOf(target.calendarId), ['Kilimani site meeting', 'Final inspection']);
          else
            addContacts(target.accountId, [
              ['Faith Wambui', 'faith@wambui.example'],
              ['Otieno Builders', 'office@otieno.example'],
            ]);
          return { imported: 2, skipped: 0, problems: [] };
        },
        () => {
          readable(path);
          if ('calendarId' in target) calendarOf(target.calendarId);
          else accountOrDevice(target.accountId);
        },
      ),
    connectCalendarAndContacts: (accountId) =>
      run('connectCalendarAndContacts', () => {
        const acc = account(accountId);
        if (acc.auth !== 'google' && acc.auth !== 'microsoft') {
          throw appError('invalid', 'Only Gmail and Microsoft accounts connect this way.');
        }
        const name = acc.auth === 'google' ? 'Google' : 'Microsoft';
        if (!options.oauthConfigured) {
          throw appError('config', `${name} calendar and contacts access is not set up in this build yet.`);
        }
        const fixed = (list: (CalendarStatus | AddressBookStatus)[]) =>
          list.map((s) => (s.accountId === accountId ? { accountId, state: 'synced' as const, message: null } : s));
        calendarStatus = fixed(calendarStatuses());
        bookStatus = fixed(books());
        if (!calendars.some((c) => c.accountId === accountId)) {
          calendars = [
            ...calendars,
            {
              id: `${accountId}:primary`,
              accountId,
              name: acc.email,
              color: acc.color,
              visible: true,
              writable: true,
              provider: acc.auth,
            },
          ];
        }
        if (!taskLists.some((l) => l.accountId === accountId)) {
          taskLists = [
            ...taskLists,
            { id: `${accountId}:tasks`, accountId, name: acc.auth === 'google' ? 'My Tasks' : 'Tasks', provider: acc.auth },
          ];
        }
        emit('mail://calendar-changed', { calendars: calendarStatus });
        emit('mail://contacts-changed', { books: bookStatus });
        tasksChanged();
      }),

    setCalendarVisible: (calendarId, visible) =>
      run('setCalendarVisible', () => {
        if (!calendars.some((c) => c.id === calendarId)) throw appError('not_found', 'That calendar no longer exists.');
        calendars = calendars.map((c) => (c.id === calendarId ? { ...c, visible } : c));
        return shownCalendars();
      }),

    getFreeBusy: (accountId, emails, from, to) =>
      run('getFreeBusy', () => {
        account(accountId);
        if (to <= from || to - from > 42 * 86_400) throw appError('invalid', 'That date range is too long.');
        const around = mockOccurrences(events, from, to);
        return emails.map((email) => mockFreeBusy(email, from, to, around, options.freeBusy));
      }),

    listDueReminders: () => run('listDueReminders', due),

    dismissReminder: (key) =>
      run('dismissReminder', () => {
        dismissedReminders.add(key);
        snoozedReminders.delete(key);
        return due();
      }),

    snoozeReminder: (key, minutes) =>
      run('snoozeReminder', () => {
        if (!Number.isFinite(minutes) || minutes <= 0) throw appError('invalid', 'Snooze for at least a minute.');
        snoozedReminders.set(key, now() + Math.round(minutes * 60));
        return due();
      }),

    listTaskLists: () =>
      run('listTaskLists', () => taskLists.filter((l) => l.accountId === null || accounts.some((a) => a.id === l.accountId))),

    listTasks: (listId, includeCompleted) =>
      run('listTasks', () => {
        if (listId !== null && !taskLists.some((l) => l.id === listId)) throw appError('not_found', 'That list no longer exists.');
        return sortTasks(
          tasks.filter(
            (t) =>
              t.state !== 'deleted' &&
              (listId === null || t.listId === listId) &&
              (includeCompleted || !t.completed) &&
              taskLists.some((l) => l.id === t.listId),
          ),
        );
      }),

    saveTask: (input) =>
      run('saveTask', () => {
        const title = input.title.trim();
        if (!title) throw appError('invalid', 'Give the task a title.');
        const list = taskLists.find((l) => l.id === input.listId);
        if (!list) throw appError('not_found', 'That list no longer exists.');
        const old = input.id ? tasks.find((t) => t.id === input.id) : undefined;
        if (input.id && !old) throw appError('not_found', 'That task no longer exists.');
        const saved: Task = {
          ...input,
          id: old?.id ?? `t${String(seq++)}`,
          title,
          notes: input.notes.trim(),
          completedAt: input.completed ? (old?.completed ? old.completedAt : now()) : null,
          state: list.provider === 'device' ? 'synced' : 'changed',
        };
        tasks = old ? tasks.map((t) => (t === old ? saved : t)) : [...tasks, saved];
        tasksChanged();
        return saved;
      }),

    deleteTask: (id) =>
      run('deleteTask', () => {
        if (!tasks.some((t) => t.id === id)) throw appError('not_found', 'That task no longer exists.');
        tasks = tasks.filter((t) => t.id !== id);
        tasksChanged();
      }),

    syncTasks: () =>
      run('syncTasks', () => {
        if (settings.workOffline) throw appError('invalid', 'Work Offline is on.');
        tasks = tasks.map((t) => ({ ...t, state: 'synced' as const }));
        tasksChanged();
      }),

    listContactGroups: () =>
      run('listContactGroups', () => [...groups].sort((a, b) => a.name.localeCompare(b.name))),

    saveContactGroup: (input) =>
      run('saveContactGroup', () => {
        const name = input.name.trim();
        if (!name) throw appError('invalid', 'Give the group a name.');
        if (groups.some((g) => g.id !== input.id && sameName(g.name, name))) {
          throw appError('invalid', `There is already a group called ${name}.`);
        }
        const old = input.id ? groups.find((g) => g.id === input.id) : undefined;
        if (input.id && !old) throw appError('not_found', 'That group no longer exists.');
        const memberIds = [...new Set(input.memberIds)].filter((id) => contacts.some((c) => c.id === id));
        const saved: ContactGroup = { id: old?.id ?? `g${String(seq++)}`, accountId: input.accountId, name, memberIds };
        groups = old ? groups.map((g) => (g === old ? saved : g)) : [...groups, saved];
        return [...groups].sort((a, b) => a.name.localeCompare(b.name));
      }),

    deleteContactGroup: (id) =>
      run('deleteContactGroup', () => {
        if (!groups.some((g) => g.id === id)) throw appError('not_found', 'That group no longer exists.');
        groups = groups.filter((g) => g.id !== id);
        return [...groups].sort((a, b) => a.name.localeCompare(b.name));
      }),

    takeMailto: () =>
      run('takeMailto', () => {
        const url = launchLink;
        launchLink = null;
        return url ? parseMailtoLink(url) : null;
      }),

    parseMailto: (url) => run('parseMailto', () => parseMailtoLink(url)),

    previewBody: (body) =>
      run('previewBody', () => {
        // The backend renders Markdown; the mock only shows bold and escapes the rest.
        const escaped = body.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
        return `<!doctype html><html><body>${escaped.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>').replace(/\n/g, '<br>')}</body></html>`;
      }),

    getThread: (id) =>
      run('getThread', () => {
        const message = stored(id);
        const role = folderOf(message.folderId)?.role;
        const hideBin = role !== 'trash' && role !== 'junk';
        return messages
          .filter((m) => m.accountId === message.accountId && threadKey(m) === threadKey(message))
          .filter((m) => !hideBin || !['trash', 'junk'].includes(folderOf(m.folderId)?.role ?? ''))
          .sort((a, b) => (a.date ?? 0) - (b.date ?? 0))
          .map(listItem);
      }),

    getMessage: (id, allowRemote) =>
      run('getMessage', () => {
        // Like the backend: with Always send, opening a message that asks for a
        // read receipt sends one at once (and it then counts as answered).
        const asked = stored(id).readReceipt;
        if (settings.readReceipts === 'always' && asked && !asked.answered) {
          messages = messages.map((x) => (x.id === id && x.readReceipt ? { ...x, readReceipt: { ...x.readReceipt, answered: true } } : x));
          answeredReceipts.push({ messageId: id, send: true });
        }
        const { remoteHtml, ...detail } = stored(id);
        const trusted = remoteHtml !== undefined && trustsImages(detail.from?.address);
        if (!remoteHtml || !(allowRemote || trusted)) return detail;
        return { ...detail, html: remoteHtml, blockedRemoteImages: 0, ...(trusted ? { imagesTrusted: true } : {}) };
      }),

    parseSearch: (query) => run('parseSearch', () => parseSearchQuery(query, now())),

    listRules: () => run('listRules', () => rules),

    saveRule: (input) =>
      run('saveRule', () => {
        validateRule(input, ruleWorld());
        const name = input.name.trim();
        const final = serverStatusOf(input);
        // Like the backend: a rule kept on the server is saved here at once,
        // and its server copy is written in the background; the outcome
        // comes in mail://rules-changed.
        const status: Pick<Rule, 'serverStatus' | 'serverNote'> = input.keepOnServer
          ? { serverStatus: 'local', serverNote: 'Being saved to the mail server.' }
          : final;
        const id = input.id ?? `rule${String(seq++)}`;
        if (input.id !== null) {
          const old = rules.find((r) => r.id === input.id);
          if (!old) throw appError('not_found', 'That rule no longer exists.');
          rules = rules.map((r) => (r.id === old.id ? { ...clone(input), id, name, order: old.order, ...status } : r));
        } else {
          rules = renumber([...rules, { ...clone(input), id, name, order: rules.length, ...status }]);
        }
        const write = seq++;
        serverWrites.set(id, write);
        if (input.keepOnServer) {
          setTimeout(() => {
            // Changed again (or deleted) meanwhile: that save writes its own copy.
            if (serverWrites.get(id) !== write || !rules.some((r) => r.id === id)) return;
            rules = rules.map((r) => (r.id === id ? { ...r, ...final } : r));
            emit('mail://rules-changed', { rules });
          }, SERVER_WRITE_MS);
        }
        return rules;
      }),

    deleteRule: (id) =>
      run('deleteRule', () => {
        if (!rules.some((r) => r.id === id)) throw appError('not_found', 'That rule no longer exists.');
        rules = renumber(rules.filter((r) => r.id !== id));
        return rules;
      }),

    reorderRules: (ids) =>
      run('reorderRules', () => {
        if (ids.length !== rules.length || new Set(ids).size !== ids.length || !rules.every((r) => ids.includes(r.id))) {
          throw appError('invalid', 'List every rule once.');
        }
        rules = renumber(ids.map((id) => rules.find((r) => r.id === id)).filter((r): r is Rule => r !== undefined));
        return rules;
      }),

    runRules: (ruleIds, folderId) =>
      run('runRules', () => {
        const folder = folderOf(folderId);
        if (!folder) throw appError('not_found', 'That folder no longer exists.');
        const own = account(folder.accountId).email;
        const chosen = rules.filter(
          (r) =>
            (r.accountId === null || r.accountId === folder.accountId) &&
            (ruleIds === null ? r.enabled : ruleIds.includes(r.id)),
        );
        const inFolder = messages.filter((m) => m.folderId === folderId).map((m) => m.id);
        let matched = 0;
        for (const id of inFolder) {
          let hit = false;
          for (const rule of chosen) {
            const current = messages.find((m) => m.id === id);
            if (!current || !ruleMatches(rule, current, own)) continue;
            hit = true;
            for (const action of rule.actions) applyAction(id, action);
            if (rule.stopProcessing) break;
          }
          if (hit) matched += 1;
        }
        if (matched > 0) changed(folder.accountId, []);
        return { checked: inFolder.length, matched };
      }),

    previewRule: (rule, folderId) =>
      run('previewRule', () => {
        const folder = folderOf(folderId);
        if (!folder) throw appError('not_found', 'That folder no longer exists.');
        const own = account(folder.accountId).email;
        return messages.filter((m) => m.folderId === folderId && ruleMatches(rule, m, own)).length;
      }),

    listImageSenders: () => run('listImageSenders', () => imageSenders),

    trustImageSender: (sender, wholeDomain) =>
      run('trustImageSender', () => {
        const clean = sender.trim().toLowerCase();
        const at = clean.lastIndexOf('@');
        const value = wholeDomain ? clean.slice(at + 1) : clean;
        if (wholeDomain ? !/^[^@\s]+\.[^@\s]+$/.test(value) : at < 1) {
          throw appError('invalid', wholeDomain ? "That isn't a domain." : "That isn't an email address.");
        }
        if (!imageSenders.some((s) => s.sender === value)) imageSenders = [...imageSenders, { sender: value, wholeDomain }];
        return imageSenders;
      }),

    untrustImageSender: (sender) =>
      run('untrustImageSender', () => {
        const clean = sender.trim().toLowerCase();
        imageSenders = imageSenders.filter((s) => s.sender !== clean);
        return imageSenders;
      }),

    setFlags: (ids, change) =>
      run('setFlags', () => {
        messages = messages.map((m) =>
          ids.includes(m.id)
            ? {
                ...m,
                ...(change.seen !== undefined ? { unread: !change.seen } : {}),
                ...(change.flagged !== undefined ? { flagged: change.flagged } : {}),
                ...(change.flagged === false ? { flagDue: null } : {}),
                ...(change.flagDue !== undefined ? { flagDue: change.flagDue } : {}),
                ...(change.flagDue !== undefined && change.flagDue !== null ? { flagged: true } : {}),
                ...(change.pinned !== undefined ? { pinned: change.pinned } : {}),
                ...(change.importance !== undefined ? { importance: change.importance } : {}),
              }
            : m,
        );
      }),

    moveMessages: (ids, target) =>
      run('moveMessages', () => {
        messages = messages.map((m) => (ids.includes(m.id) ? { ...m, folderId: destination(m, target).id } : m));
      }),

    copyMessages: (ids, target) =>
      run('copyMessages', () => {
        const copies = messages
          .filter((m) => ids.includes(m.id))
          .map((m) => ({ ...clone(m), id: `copy${String(seq++)}`, folderId: destination(m, target).id }));
        messages = [...messages, ...copies];
        for (const c of copies) changed(c.accountId, [c.folderId]);
      }),

    markJunk: (ids, junk) =>
      run('markJunk', () => {
        messages = messages.map((m) =>
          ids.includes(m.id) ? { ...m, folderId: roleFolder(m.accountId, junk ? 'junk' : 'inbox').id } : m,
        );
      }),

    listCategories: () => run('listCategories', () => categories),

    saveCategory: (category, previousName) =>
      run('saveCategory', () => {
        const name = category.name.trim().replace(/\s+/g, ' ');
        if (!name) throw appError('invalid', 'Type a name for the category.');
        if (name.length > 40) throw appError('invalid', 'Category names can be up to 40 characters.');
        if (!HEX.test(category.color)) throw appError('invalid', 'Pick a colour.');
        const old = previousName === null ? undefined : categories.find((c) => sameName(c.name, previousName));
        if (previousName !== null && !old) throw appError('not_found', 'That category no longer exists.');
        if (categories.some((c) => c !== old && sameName(c.name, name))) {
          throw appError('invalid', `There's already a category called ${name}.`);
        }
        const saved = { name, color: category.color.toLowerCase() };
        categories = old ? categories.map((c) => (c === old ? saved : c)) : [...categories, saved];
        if (old && old.name !== name) {
          messages = messages.map((m) => ({ ...m, categories: m.categories.map((c) => (sameName(c, old.name) ? name : c)) }));
        }
        return categories;
      }),

    deleteCategory: (name) =>
      run('deleteCategory', () => {
        categories = categories.filter((c) => !sameName(c.name, name));
        messages = messages.map((m) => ({ ...m, categories: m.categories.filter((c) => !sameName(c, name)) }));
        return categories;
      }),

    setCategories: (ids, add, remove) =>
      run('setCategories', () => {
        const unknown = add.find((name) => !categories.some((c) => sameName(c.name, name)));
        if (unknown) throw appError('not_found', `There's no category called ${unknown}.`);
        messages = messages.map((m) => {
          if (!ids.includes(m.id)) return m;
          const kept = m.categories.filter((c) => !remove.some((r) => sameName(r, c)));
          const added = add.filter((a) => !kept.some((c) => sameName(a, c)));
          return { ...m, categories: [...kept, ...added] };
        });
      }),

    openMessageWindow: (id) =>
      run('openMessageWindow', () => {
        if (options.phone) throw appError('invalid', 'Phones show one message at a time.');
        stored(id);
        openedWindows.push(id);
      }),

    deleteForever: (ids) =>
      run('deleteForever', () => {
        for (const id of ids) {
          const role = folderOf(stored(id).folderId)?.role;
          if (role !== 'trash' && role !== 'junk') {
            throw appError('invalid', 'Only messages in Trash or Spam can be deleted forever.');
          }
        }
        messages = messages.filter((m) => !ids.includes(m.id));
      }),

    saveAttachment: (messageId, index) =>
      run('saveAttachment', () => {
        const file = stored(messageId).attachments.find((a) => a.index === index);
        if (!file) throw appError('not_found', 'That attachment is no longer available.');
        return `/home/you/Downloads/${file.fileName}`;
      }),

    saveMessage: (messageId) =>
      run('saveMessage', () => `/home/you/Downloads/${stored(messageId).subject || 'message'}.eml`),

    messageSource: (messageId) =>
      run('messageSource', () => {
        const m = stored(messageId);
        return `From: ${m.from?.address ?? ''}\r\nSubject: ${m.subject}\r\n\r\n${m.text ?? ''}`;
      }),

    printHtml: (html) =>
      run('printHtml', () => {
        printed.push(html);
      }),

    previewAttachment: (messageId, index) =>
      run('previewAttachment', () => {
        const file = stored(messageId).attachments.find((a) => a.index === index);
        if (!file) throw appError('not_found', 'That attachment is no longer available.');
        if (file.preview === null) {
          throw appError('invalid', `${file.fileName} can't be shown here. Open it in another app or save it.`);
        }
        return mockBytes(file.preview);
      }),

    openAttachment: (messageId, index) =>
      run('openAttachment', () => {
        const file = stored(messageId).attachments.find((a) => a.index === index);
        if (!file) throw appError('not_found', 'That attachment is no longer available.');
        openedFiles.push(file.fileName);
      }),

    pickAttachments: () =>
      run(
        'pickAttachments',
        () => options.picked ?? [{ path: '/home/you/Documents/INV-0144.pdf', fileName: 'INV-0144.pdf', size: 186 * 1024 }],
      ),

    checkDraft: (draft) => run('checkDraft', () => check(draft)),

    sendMessage: (draft) =>
      run('sendMessage', () => {
        check(draft);
        const entry: OutboxEntry = {
          id: `q${String(seq++)}`,
          accountId: draft.accountId,
          to: parseRecipients(draft.to).valid.map((address) => ({ name: null, address })),
          subject: draft.subject,
          preview: draft.bodyText.replace(/\s+/g, ' ').trim().slice(0, 160),
          createdAt: now(),
          attempts: 0,
          // Held for the undo window, as the real Outbox does.
          status: { state: 'queued', next_attempt_at: now() + settings.undoSendSecs, maybe_sent: false },
          progress: null,
        };
        outboxDrafts.set(entry.id, clone({ ...draft, draftMessageId: null }));
        outbox = [...outbox, entry];
        if (draft.draftMessageId) {
          removeDraft(draft.draftMessageId);
          changed(draft.accountId, []);
        }
        if (draft.replyToMessageId) {
          messages = messages.map((m) => (m.id === draft.replyToMessageId ? { ...m, answered: true } : m));
        }
        outboxChanged();
        if (options.simulateDelivery) deliver(entry.id, settings.undoSendSecs);
        return entry;
      }),

    saveDraft: (draft) =>
      run('saveDraft', () => {
        const acc = account(draft.accountId);
        const folder = folders.find((f) => f.accountId === acc.id && f.role === 'drafts');
        if (!folder) throw appError('not_found', 'This account has no Drafts folder.');
        removeDraft(draft.draftMessageId);
        const id = `saved${String(seq++)}`;
        const draftMessageId = `<${id}@mock.example>`;
        messages = [
          ...messages,
          {
            id,
            accountId: acc.id,
            folderId: folder.id,
            from: { name: null, address: acc.email },
            to: parseRecipients(draft.to).valid.map((address) => ({ name: null, address })),
            subject: draft.subject,
            preview: draft.bodyText.slice(0, 160),
            date: now(),
            unread: false,
            flagged: false,
            answered: false,
            hasAttachments: draft.attachments.length > 0,
            pinned: false,
            importance: draft.importance ?? 'normal',
            categories: [],
            flagDue: null,
            size: 2048 + draft.bodyText.length + draft.attachments.reduce((sum, a) => sum + a.size, 0),
            cc: parseRecipients(draft.cc).valid.map((address) => ({ name: null, address })),
            bcc: parseRecipients(draft.bcc).valid.map((address) => ({ name: null, address })),
            replyTo: [],
            html: draft.bodyHtml ?? null,
            text: draft.bodyText,
            blockedRemoteImages: 0,
            attachments: draft.attachments.map((a, index) => ({
              index,
              fileName: a.fileName,
              mimeType: 'application/octet-stream',
              size: a.size,
              preview: null,
            })),
            messageIdHeader: draftMessageId,
            inReplyTo: draft.inReplyTo,
            references: draft.inReplyTo && !draft.references.includes(draft.inReplyTo)
              ? [...draft.references, draft.inReplyTo]
              : draft.references,
          },
        ];
        draftFiles.set(id, clone(draft.attachments));
        changed(acc.id, [folder.id]);
        return { draftMessageId };
      }),

    attachmentsForDraft: (messageId) =>
      run('attachmentsForDraft', () => {
        const m = stored(messageId);
        return (
          draftFiles.get(messageId) ??
          m.attachments.map((a) => ({
            path: `/tmp/mocky-mail/${m.id}/${String(a.index)}-${a.fileName}`,
            fileName: a.fileName,
            size: a.size,
          }))
        );
      }),

    listOutbox: () => run('listOutbox', () => outbox),

    outboxSendNow: (id) =>
      run('outboxSendNow', () => {
        outbox = outbox.map((o) => {
          if (o.id !== id) return o;
          if (o.status.state === 'queued' || o.status.state === 'failed') {
            return { ...o, attempts: o.attempts + 1, status: { state: 'sending', started_at: now() }, progress: 0 };
          }
          throw appError('invalid', 'This message cannot be sent right now.');
        });
        outboxChanged();
      }),

    outboxCancel: (id) =>
      run('outboxCancel', () => {
        const entry = outbox.find((o) => o.id === id);
        if (!entry) throw appError('not_found', 'That message is no longer in the Outbox.');
        if (entry.status.state === 'sending') throw appError('invalid', 'cannot cancel a message that is sending');
        outbox = outbox.filter((o) => o.id !== id);
        outboxChanged();
      }),

    outboxEdit: (id) =>
      run('outboxEdit', () => {
        const entry = outbox.find((o) => o.id === id);
        if (!entry) throw appError('not_found', 'That message is no longer in the Outbox.');
        if (entry.status.state === 'sending') throw appError('invalid', 'cannot edit a message that is sending');
        outbox = outbox.filter((o) => o.id !== id);
        outboxChanged();
        return (
          outboxDrafts.get(id) ?? {
            accountId: entry.accountId,
            to: entry.to.map((t) => t.address).join(', '),
            cc: '',
            bcc: '',
            subject: entry.subject,
            bodyText: entry.preview,
            inReplyTo: null,
            references: [],
            attachments: [],
            replyToMessageId: null,
            draftMessageId: null,
          }
        );
      }),

    syncNow: (accountId) =>
      run('syncNow', () => {
        if (settings.workOffline) throw appError('invalid', 'Work Offline is on.');
        for (const acc of accounts.filter((a) => accountId === null || a.id === accountId)) {
          emit('mail://sync-status', { accountId: acc.id, state: 'syncing', lastSyncedAt: null, message: null });
          emit('mail://sync-status', { accountId: acc.id, state: 'idle', lastSyncedAt: now(), message: null });
        }
      }),

    checkForUpdate: () => run('checkForUpdate', () => options.update ?? null),

    installUpdate: () => run('installUpdate', () => undefined),

    listContacts: (query) =>
      run('listContacts', () => {
        const q = query.trim().toLowerCase();
        return contacts
          .filter((c) => c.state !== 'deleted')
          .filter((c) =>
            [c.name, ...c.emails, ...c.phones, c.organization, c.title].some((f) => f.toLowerCase().includes(q)),
          )
          .sort((a, b) => contactSortKey(a).localeCompare(contactSortKey(b)));
      }),

    getContact: (id) =>
      run('getContact', () => {
        const found = contacts.find((c) => c.id === id && c.state !== 'deleted');
        if (!found) throw appError('not_found', 'That contact no longer exists.');
        return found;
      }),

    saveContact: (input) =>
      run('saveContact', () => {
        const emails = input.emails.map((e) => e.trim()).filter(Boolean);
        const bad = emails.find((e) => !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(e));
        if (bad) throw appError('invalid', `${bad} isn't an email address.`);
        const phones = input.phones.map((p) => p.trim()).filter(Boolean);
        if (!input.name.trim() && emails.length === 0 && phones.length === 0) {
          throw appError('invalid', 'Add a name, an email address or a phone number.');
        }
        const old = input.id ? contacts.find((c) => c.id === input.id) : undefined;
        if (input.id && !old) throw appError('not_found', 'That contact no longer exists.');
        const saved: Contact = {
          id: old?.id ?? `c${String(seq++)}`,
          uid: old?.uid ?? `u${String(seq)}`,
          accountId: old ? old.accountId : input.accountId,
          name: input.name.trim(),
          emails,
          phones,
          organization: input.organization.trim(),
          title: input.title.trim(),
          notes: input.notes.trim(),
          favorite: input.favorite ?? old?.favorite ?? false,
          categories: [...new Set((input.categories ?? old?.categories ?? []).map((c) => c.trim()).filter(Boolean))],
          photo: input.photo === undefined ? (old?.photo ?? null) : input.photo,
          state: input.accountId ? 'changed' : 'synced',
        };
        if (saved.photo !== null && !/^data:image\/(png|jpeg|gif|webp);base64,/.test(saved.photo)) {
          throw appError('invalid', 'That picture could not be used.');
        }
        contacts = [...contacts.filter((c) => c.id !== saved.id), saved];
        return saved;
      }),

    deleteContact: (id) =>
      run('deleteContact', () => {
        if (!contacts.some((c) => c.id === id)) throw appError('not_found', 'That contact no longer exists.');
        contacts = contacts.filter((c) => c.id !== id);
        groups = groups.map((g) => ({ ...g, memberIds: g.memberIds.filter((m) => m !== id) }));
      }),

    syncContacts: () =>
      run('syncContacts', () => {
        contacts = contacts.map((c) => ({ ...c, state: 'synced' as const }));
        return books();
      }),

    listCalendars: () =>
      run('listCalendars', shownCalendars),

    listEvents: (from, to) =>
      run('listEvents', () => {
        if (to <= from || to - from > 400 * 86_400) throw appError('invalid', 'That date range is too long.');
        const shown = new Set(calendars.filter((c) => accounts.some((a) => a.id === c.accountId)).map((c) => c.id));
        return mockOccurrences(
          events.filter((e) => shown.has(e.event.calendarId)),
          from,
          to,
        );
      }),

    getEvent: (id) =>
      run('getEvent', () => {
        const found = events.find((e) => e.event.id === id);
        if (!found) throw appError('not_found', 'That event no longer exists.');
        return found.event;
      }),

    saveEvent: (input) =>
      run('saveEvent', () => {
        if (input.end < input.start) throw appError('invalid', 'The event ends before it starts.');
        if (!calendars.some((c) => c.id === input.calendarId)) throw appError('not_found', 'That calendar no longer exists.');
        const calendar = calendars.find((c) => c.id === input.calendarId);
        const old = input.id ? events.find((e) => e.event.id === input.id) : undefined;
        if (input.id && !old) throw appError('not_found', 'That event no longer exists.');
        const own = accounts.find((a) => a.id === calendar?.accountId)?.email;
        if (calendar && !calendar.writable) {
          // Only your own reminders change on a calendar you can't write to.
          if (!old) throw appError('invalid', `${calendar.name} is read-only.`);
          const kept = { ...old.event, reminders: input.reminders ?? old.event.reminders };
          events = events.map((e) => (e === old ? { ...e, event: kept } : e));
          emit('mail://calendar-changed', { calendars: [] });
          return kept;
        }
        if (old && input.occurrence != null && input.scope === 'this') {
          // One date of a series: it leaves the series as an event of its own.
          if (!old.event.recurring) throw appError('invalid', "That event doesn't repeat.");
          const single = mockSave(
            undefined,
            { ...input, repeat: 'none', repeatRule: null },
            `e${String(seq++)}`,
            calendars,
            own,
          );
          const exception = {
            ...single,
            uid: old.event.uid,
            recurrenceId: input.occurrence,
            attendees: input.attendees ? single.attendees : old.event.attendees,
            organizer: input.attendees ? single.organizer : old.event.organizer,
            showAs: input.showAs ?? old.event.showAs,
            private: input.private ?? old.event.private,
          };
          events = [
            ...events.map((e) => (e === old ? { ...e, skipped: [...e.skipped, input.occurrence ?? 0] } : e)),
            { event: exception, skipped: [] },
          ];
          emit('mail://calendar-changed', { calendars: [] });
          return exception;
        }
        const saved = mockSave(old?.event, input, old?.event.id ?? `e${String(seq++)}`, calendars, own);
        events = old
          ? events.map((e) => (e === old ? { event: saved, skipped: e.skipped } : e))
          : [...events, { event: saved, skipped: [] }];
        emit('mail://calendar-changed', { calendars: [] });
        return saved;
      }),

    deleteEvent: (id, occurrence, notify) =>
      run('deleteEvent', () => {
        const found = events.find((e) => e.event.id === id);
        if (!found) throw appError('not_found', 'That event no longer exists.');
        if (calendars.find((c) => c.id === found.event.calendarId)?.writable === false) {
          throw appError('invalid', 'That calendar is read-only.');
        }
        events =
          occurrence !== null && found.event.recurring
            ? events.map((e) => (e === found ? { ...e, skipped: [...e.skipped, occurrence] } : e))
            : events.filter((e) => e !== found);
        if (notify) cancellations.push({ eventId: id, occurrence });
        emit('mail://calendar-changed', { calendars: [] });
      }),

    answerInvitation: (id, answer, notify) =>
      run('answerInvitation', () => {
        const m = stored(id);
        const inv = m.invitation;
        if (inv?.method !== 'REQUEST') throw appError('invalid', "This isn't an invitation you can answer.");
        const next = {
          ...inv,
          savedStatus: inv.hasCalendar && answer !== 'declined' ? answer : null,
          calendarEventId: inv.hasCalendar && answer !== 'declined' ? (inv.calendarEventId ?? `e${String(seq++)}`) : null,
        };
        messages = messages.map((x) => (x.id === id ? { ...x, invitation: next } : x));
        const organizer = inv.event.organizer;
        if (notify && organizer) {
          const word = answer === 'accepted' ? 'Accepted' : answer === 'tentative' ? 'Tentative' : 'Declined';
          outbox = [
            ...outbox,
            {
              id: `o${String(seq++)}`,
              accountId: m.accountId,
              to: [{ name: organizer.name, address: organizer.email }],
              subject: `${word}: ${inv.event.title}`,
              preview: `${word} this invitation.`,
              createdAt: now(),
              attempts: 0,
              status: { state: 'queued', next_attempt_at: now(), maybe_sent: false },
              progress: null,
            },
          ];
          emit('mail://outbox-changed', { entries: outbox });
        }
        return next;
      }),

    removeCancelledEvent: (id) =>
      run('removeCancelledEvent', () => {
        const inv = stored(id).invitation;
        if (inv?.method !== 'CANCEL') throw appError('invalid', "This event wasn't cancelled.");
        const next = { ...inv, calendarEventId: null, savedStatus: null };
        messages = messages.map((x) => (x.id === id ? { ...x, invitation: next } : x));
        return next;
      }),

    applyInvitationReply: (id) =>
      run('applyInvitationReply', () => {
        const inv = stored(id).invitation;
        if (!inv?.calendarEventId) throw appError('invalid', "This event isn't in your calendar.");
        const next = { ...inv, replyPending: false };
        messages = messages.map((x) => (x.id === id ? { ...x, invitation: next } : x));
        return next;
      }),

    getOutOfOffice: (accountId) =>
      run('getOutOfOffice', () => {
        if (!accounts.some((a) => a.id === accountId)) throw appError('not_found', 'That account was removed.');
        return (
          outOfOffice.get(accountId) ?? {
            accountId,
            available: true,
            message: null,
            active: false,
            settings: {
              enabled: false,
              subject: 'Out of office',
              message: '',
              start: null,
              end: null,
              days: 1,
              ...(account(accountId).auth === 'microsoft' ? { externalMessage: null, externalAudience: 'all' as const } : {}),
            },
          }
        );
      }),

    setOutOfOffice: (accountId, settings) =>
      run('setOutOfOffice', () => {
        const current = outOfOffice.get(accountId);
        if (current && !current.available) throw appError('invalid', current.message ?? 'Not available.');
        if (settings.enabled && !settings.message.trim()) throw appError('invalid', 'Write the reply people will get.');
        if (settings.start && settings.end && settings.end < settings.start) {
          throw appError('invalid', 'The last day is before the first.');
        }
        const saved: OutOfOffice = {
          accountId,
          available: true,
          message: null,
          active: settings.enabled,
          settings: {
            ...settings,
            subject: settings.subject.trim() || 'Out of office',
            message: settings.message.trim(),
            ...(settings.externalMessage !== undefined ? { externalMessage: settings.externalMessage?.trim() ? settings.externalMessage.trim() : null } : {}),
          },
        };
        outOfOffice.set(accountId, saved);
        return saved;
      }),

    syncCalendars: () =>
      run(
        'syncCalendars',
        calendarStatuses,
      ),

    getSignature: (accountId, reply) =>
      run('getSignature', () => {
        const set = setOf(accountId);
        const id = reply ? set.defaultReply : set.defaultNew;
        const found = set.items.find((s) => s.id === id);
        if (!found) return null;
        const { html, text, image } = found;
        return image ? { html, text, image } : { html, text };
      }),

    listSignatures: (accountId) => run('listSignatures', () => setOf(accountId)),

    saveSignature: (accountId, id, name, html) =>
      run('saveSignature', () => {
        const set = setOf(accountId);
        let items = set.items;
        let target = id;
        if (target === null) {
          target = `s${String(seq++)}`;
          items = [...items, { id: target, name: `Signature ${String(items.length + 1)}`, html: '', text: '' }];
        }
        const old = items.find((s) => s.id === target);
        if (!old) throw appError('not_found', 'That signature no longer exists.');
        const clean = html.replace(/<script[\s\S]*?<\/script>/gi, '').trim();
        const text = signatureText(clean);
        const picture = old.image ? account(accountId).displayName : '';
        const next = {
          ...old,
          name: name.trim() || old.name,
          html: text ? clean : '',
          text: text || picture,
        };
        const first = set.items.length === 0;
        const saved: SignatureSet = {
          items: items.map((s) => (s.id === target ? next : s)),
          defaultNew: first ? target : set.defaultNew,
          defaultReply: first ? target : set.defaultReply,
        };
        signatures.set(accountId, saved);
        return saved;
      }),

    deleteSignature: (accountId, id) =>
      run('deleteSignature', () => {
        const set = setOf(accountId);
        const saved: SignatureSet = {
          items: set.items.filter((s) => s.id !== id),
          defaultNew: set.defaultNew === id ? null : set.defaultNew,
          defaultReply: set.defaultReply === id ? null : set.defaultReply,
        };
        signatures.set(accountId, saved);
        return saved;
      }),

    setSignatureDefaults: (accountId, defaultNew, defaultReply) =>
      run('setSignatureDefaults', () => {
        const saved = { ...setOf(accountId), defaultNew, defaultReply };
        signatures.set(accountId, saved);
        return saved;
      }),

    setSignatureImage: (accountId, id) =>
      run('setSignatureImage', () => {
        const image = { mimeType: 'image/png', data: MOCK_PNG_BASE64, width: 320, naturalWidth: 640, naturalHeight: 160 };
        const set = setOf(accountId);
        if (!set.items.some((s) => s.id === id)) throw appError('not_found', 'That signature no longer exists.');
        const saved: SignatureSet = {
          ...set,
          items: set.items.map((s) =>
            s.id === id ? { ...s, image, text: s.html ? s.text : account(accountId).displayName } : s,
          ),
        };
        signatures.set(accountId, saved);
        return saved;
      }),

    setSignatureImageWidth: (accountId, id, width) =>
      run('setSignatureImageWidth', () => {
        const set = setOf(accountId);
        const saved: SignatureSet = {
          ...set,
          items: set.items.map((s) =>
            s.id === id && s.image
              ? { ...s, image: { ...s.image, width: Math.min(Math.max(width, 40), 600, s.image.naturalWidth) } }
              : s,
          ),
        };
        signatures.set(accountId, saved);
        return saved;
      }),

    removeSignatureImage: (accountId, id) =>
      run('removeSignatureImage', () => {
        const set = setOf(accountId);
        const saved: SignatureSet = {
          ...set,
          items: set.items.map((s) => {
            if (s.id !== id) return s;
            return { id: s.id, name: s.name, html: s.html, text: s.html ? s.text : '' };
          }),
        };
        signatures.set(accountId, saved);
        return saved;
      }),

    getSettings: () => run('getSettings', () => settings),

    setSettings: (next) =>
      run('setSettings', () => {
        settings = clone(next);
        return settings;
      }),

    openExternal: (url) =>
      run('openExternal', () => {
        if (!/^(https?:|mailto:)/i.test(url)) throw appError('invalid', 'Only web and email links can be opened.');
        opened.push(url);
      }),

    on(event, handler) {
      const set = handlers.get(event) ?? new Set<Handler>();
      const stored: Handler = handler;
      set.add(stored);
      handlers.set(event, set);
      return () => {
        set.delete(stored);
      };
    },
  };
  return api;
}
