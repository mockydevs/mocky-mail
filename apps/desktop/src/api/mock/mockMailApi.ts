import { appError, MailError, toAppError } from '../errors';
import { parseRecipients } from '../../features/compose/recipients';
import type {
  Account,
  AddressBookStatus,
  Contact,
  AttachmentMeta,
  Focus,
  AppError,
  Draft,
  Signature,
  SignatureSet,
  UpdateInfo,
  DraftAttachment,
  DraftCheck,
  Folder,
  Mailbox,
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
  /** A newer version for checkForUpdate to report. */
  update?: UpdateInfo;
  /** One signature per account (the default for new messages and replies), by account id. */
  signatures?: Record<string, Signature>;
  /** Full signature sets by account id; wins over `signatures`. */
  signatureSets?: Record<string, SignatureSet>;
  /** Saved contacts. */
  contacts?: Contact[];
  /** Address book status per account; default: every account synced. */
  books?: AddressBookStatus[];
  /** Dev preview: new messages go sending, then sent, like the real Outbox. */
  simulateDelivery?: boolean;
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
    threadCount: 1,
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
  const outboxDrafts = new Map<string, Draft>();
  /** Attachments of drafts saved in this session, by message id. */
  const draftFiles = new Map<string, DraftAttachment[]>();
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
  /** Focused Inbox rules: address or @domain to focus. */
  const focusRules = new Map<string, Focus>();
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
    const found = messages.find((m) => m.id === id);
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
    openedFiles,
    printed,
    emit,
    failNext(method, error) {
      failures.set(method, error);
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
        folders = [...folders, { id: `${accountId}:${path}`, accountId, name: clean, path, role: 'other', delimiter }];
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
          .filter((m) => !query.focus || folderOf(m.folderId)?.role !== 'inbox' || focusOf(m) === query.focus)
          .filter(
            (m) =>
              !needle ||
              [m.subject, m.preview, m.text ?? '', m.from?.name ?? '', m.from?.address ?? '', ...m.to.map((t) => t.address)]
                .join(' ')
                .toLowerCase()
                .includes(needle),
          )
          .sort((a, b) => (b.date ?? 0) - (a.date ?? 0));
        if (!query.threads) {
          return {
            items: matches.slice(query.offset, query.offset + query.limit).map(listItem),
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
            threadCount: group.length,
            threadIds: group.map((m) => m.id),
          };
        });
        return { items: rows.slice(query.offset, query.offset + query.limit), total: rows.length };
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
            cc: parseRecipients(draft.cc).valid.map((address) => ({ name: null, address })),
            replyTo: [],
            html: null,
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
          state: input.accountId ? 'changed' : 'synced',
        };
        contacts = [...contacts.filter((c) => c.id !== saved.id), saved];
        return saved;
      }),

    deleteContact: (id) =>
      run('deleteContact', () => {
        if (!contacts.some((c) => c.id === id)) throw appError('not_found', 'That contact no longer exists.');
        contacts = contacts.filter((c) => c.id !== id);
      }),

    syncContacts: () =>
      run('syncContacts', () => {
        contacts = contacts.map((c) => ({ ...c, state: 'synced' as const }));
        return options.books ?? accounts.map((a) => ({ accountId: a.id, state: 'synced' as const, message: null }));
      }),

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
