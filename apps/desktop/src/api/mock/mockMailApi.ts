import { appError, MailError, toAppError } from '../errors';
import { parseRecipients } from '../../features/compose/recipients';
import type {
  Account,
  AppError,
  Draft,
  DraftAttachment,
  DraftCheck,
  Folder,
  MailApi,
  MailEventName,
  MailEvents,
  MessageDetail,
  MessageListItem,
  OutboxEntry,
  ServerSuggestion,
  Settings,
} from '../types';
import { foldersFor, MOCK_ACCOUNTS, MOCK_FOLDERS, MOCK_MESSAGES, MOCK_NOW, MOCK_OUTBOX, MOCK_SETTINGS } from './mockData';

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
}

export interface MockMailApi extends MailApi {
  /** Fire a backend event, as the Rust side would. */
  emit<E extends MailEventName>(event: E, payload: MailEvents[E]): void;
  /** Make the next call to `method` reject with `error`. */
  failNext(method: Method, error: AppError): void;
  /** URLs passed to openExternal, for assertions. */
  readonly opened: string[];
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
  };
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
  const outboxDrafts = new Map<string, Draft>();
  const now = options.now ?? (() => MOCK_NOW);
  type Handler = (payload: never) => void;
  const handlers = new Map<MailEventName, Set<Handler>>();
  const failures = new Map<Method, AppError>();
  const opened: string[] = [];
  let seq = 1;

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
    const found = messages.find((m) => m.id === id);
    if (!found) throw appError('not_found', 'That message is no longer available.');
    return found;
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
      encodedSize(new TextEncoder().encode(draft.bodyText).length) + HEADER_ALLOWANCE,
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

  const api: MockMailApi = {
    opened,
    emit,
    failNext(method, error) {
      failures.set(method, error);
    },

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
        const created: Account = {
          id,
          email: input.email,
          displayName: input.displayName || input.email,
          auth: input.provider,
          createdAt: now(),
          color: PALETTE[accounts.length % PALETTE.length] ?? '#2450C8',
          imap: { host: 'imap.gmail.com', port: 993, security: 'tls' },
          smtp: { host: 'smtp.gmail.com', port: 465, security: 'tls' },
          sizeLimitBytes: 25 * 1024 * 1024,
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
        const next = { ...acc, ...(patch.displayName !== undefined ? { displayName: patch.displayName } : {}) };
        accounts = accounts.map((a) => (a.id === id ? next : a));
        accountsChanged();
        return next;
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

    listAccounts: () => run('listAccounts', () => accounts),

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
        const needle = query.search?.trim().toLowerCase() ?? '';
        const matches = messages
          .filter((m) => query.accountId === null || m.accountId === query.accountId)
          .filter((m) => query.folderId === null || m.folderId === query.folderId)
          .filter((m) => query.role === null || folderOf(m.folderId)?.role === query.role)
          .filter((m) => query.filter !== 'unread' || m.unread)
          .filter((m) => query.filter !== 'flagged' || m.flagged)
          .filter(
            (m) =>
              !needle ||
              [m.subject, m.preview, m.text ?? '', m.from?.name ?? '', m.from?.address ?? '', ...m.to.map((t) => t.address)]
                .join(' ')
                .toLowerCase()
                .includes(needle),
          )
          .sort((a, b) => (b.date ?? 0) - (a.date ?? 0));
        return {
          items: matches.slice(query.offset, query.offset + query.limit).map(listItem),
          total: matches.length,
        };
      }),

    getMessage: (id, allowRemote) =>
      run('getMessage', () => {
        const { remoteHtml, ...detail } = stored(id);
        return allowRemote && remoteHtml ? { ...detail, html: remoteHtml, blockedRemoteImages: 0 } : detail;
      }),

    setFlags: (ids, change) =>
      run('setFlags', () => {
        messages = messages.map((m) =>
          ids.includes(m.id)
            ? {
                ...m,
                ...(change.seen !== undefined ? { unread: !change.seen } : {}),
                ...(change.flagged !== undefined ? { flagged: change.flagged } : {}),
              }
            : m,
        );
      }),

    moveMessages: (ids, target) =>
      run('moveMessages', () => {
        messages = messages.map((m) => {
          if (!ids.includes(m.id)) return m;
          const dest =
            'folderId' in target
              ? folderOf(target.folderId)
              : folders.find((f) => f.accountId === m.accountId && f.role === target.role);
          if (!dest) throw appError('not_found', 'That folder does not exist.');
          return { ...m, folderId: dest.id };
        });
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
          status: { state: 'queued', next_attempt_at: now(), maybe_sent: false },
          progress: null,
        };
        outboxDrafts.set(entry.id, clone(draft));
        outbox = [...outbox, entry];
        if (draft.replyToMessageId) {
          messages = messages.map((m) => (m.id === draft.replyToMessageId ? { ...m, answered: true } : m));
        }
        outboxChanged();
        return entry;
      }),

    saveDraft: (draft) =>
      run('saveDraft', () => {
        const acc = account(draft.accountId);
        const folder = folders.find((f) => f.accountId === acc.id && f.role === 'drafts');
        if (!folder) throw appError('not_found', 'This account has no Drafts folder.');
        const id = `d${String(seq++)}`;
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
            cc: [],
            replyTo: [],
            html: null,
            text: draft.bodyText,
            blockedRemoteImages: 0,
            attachments: [],
            messageIdHeader: null,
            references: draft.references,
          },
        ];
        changed(acc.id, [folder.id]);
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
          }
        );
      }),

    syncNow: (accountId) =>
      run('syncNow', () => {
        for (const acc of accounts.filter((a) => accountId === null || a.id === accountId)) {
          emit('mail://sync-status', { accountId: acc.id, state: 'syncing', lastSyncedAt: null, message: null });
          emit('mail://sync-status', { accountId: acc.id, state: 'idle', lastSyncedAt: now(), message: null });
        }
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
