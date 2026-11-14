// Types for the interface/backend contract in docs/ipc.md. Keep this file in
// step with that document; field names and tags must match exactly.

export const APP_ERROR_KINDS = [
  'auth',
  'connect',
  'tls',
  'config',
  'not_found',
  'invalid',
  'too_large',
  'storage',
  'keyring',
  'internal',
] as const;
export type AppErrorKind = (typeof APP_ERROR_KINDS)[number];

export interface AppError {
  kind: AppErrorKind;
  /** Safe to show; never contains passwords or tokens. */
  message: string;
}

export type Security = 'tls' | 'starttls';
export interface ServerConfig {
  host: string;
  port: number;
  security: Security;
}
export type AuthKind = 'password' | 'google' | 'microsoft';

export interface Account {
  id: string;
  email: string;
  displayName: string;
  auth: AuthKind;
  /** Unix seconds. */
  createdAt: number;
  /** Hex colour assigned by the backend from a fixed palette. */
  color: string;
  imap: ServerConfig;
  smtp: ServerConfig;
  sizeLimitBytes: number | null;
}

export type FolderRole = 'inbox' | 'sent' | 'drafts' | 'archive' | 'junk' | 'trash' | 'all' | 'flagged' | 'other';

export interface Folder {
  id: string;
  accountId: string;
  name: string;
  path: string;
  role: FolderRole;
  unread: number;
  total: number;
  /** Hierarchy separator: nest folders by splitting `path` on it. */
  delimiter: string | null;
}

export interface Mailbox {
  name: string | null;
  address: string;
}

export interface MessageListItem {
  id: string;
  accountId: string;
  folderId: string;
  from: Mailbox | null;
  to: Mailbox[];
  subject: string;
  preview: string;
  /** Unix seconds. */
  date: number | null;
  unread: boolean;
  flagged: boolean;
  answered: boolean;
  hasAttachments: boolean;
}

export interface AttachmentMeta {
  index: number;
  fileName: string;
  mimeType: string;
  size: number;
}

export type MessageDetail = MessageListItem & {
  cc: Mailbox[];
  replyTo: Mailbox[];
  /** Already sanitized. Render only in <iframe sandbox="allow-popups">; the backend routes link popups to the system browser. */
  html: string | null;
  text: string | null;
  blockedRemoteImages: number;
  attachments: AttachmentMeta[];
  messageIdHeader: string | null;
  /** In-Reply-To of this message (for continuing a saved reply draft). */
  inReplyTo: string | null;
  references: string[];
};

export type ReadFilter = 'all' | 'unread' | 'flagged';

export interface ListQuery {
  /** null = all accounts (unified). */
  accountId: string | null;
  folderId: string | null;
  role: FolderRole | null;
  filter: ReadFilter;
  search: string | null;
  offset: number;
  /** At most 200. */
  limit: number;
}

export interface MessagePage {
  items: MessageListItem[];
  total: number;
}

export interface DraftAttachment {
  path: string;
  fileName: string;
  size: number;
}

export interface Draft {
  accountId: string;
  /** As typed; the backend validates. */
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  bodyText: string;
  inReplyTo: string | null;
  references: string[];
  attachments: DraftAttachment[];
  /** Marks the original \Answered after sending. */
  replyToMessageId: string | null;
  /**
   * Message-ID of the saved draft this replaces: from save_draft, or the
   * message's messageIdHeader when continuing a draft from Drafts.
   */
  draftMessageId: string | null;
}

export interface DraftSaved {
  draftMessageId: string;
}

/** Tags come from mail-core's serde output, hence snake_case. */
export type OutboxState =
  | { state: 'queued'; next_attempt_at: number; maybe_sent: boolean }
  | { state: 'sending'; started_at: number }
  | { state: 'sent'; at: number }
  | { state: 'failed'; reason: 'rejected' | 'gave_up' | 'too_large' | 'unconfirmed'; message: string }
  | { state: 'cancelled' };

export interface OutboxEntry {
  id: string;
  accountId: string;
  to: Mailbox[];
  subject: string;
  preview: string;
  createdAt: number;
  attempts: number;
  status: OutboxState;
  /** 0..1 while sending, else null. */
  progress: number | null;
}

export type ThemeSetting = 'system' | 'light' | 'dark';

export interface Settings {
  theme: ThemeSetting;
  remoteImages: 'block' | 'allow';
  notifications: boolean;
  /** A short chime with each new-mail notification. */
  sound: boolean;
}

export interface SyncStatus {
  accountId: string;
  state: 'syncing' | 'idle' | 'offline' | 'error';
  lastSyncedAt: number | null;
  message: string | null;
}

export interface ServerSuggestion {
  imap: ServerConfig;
  smtp: ServerConfig;
  auth: AuthKind;
}

export interface NewAccountInput {
  email: string;
  displayName: string;
  password: string;
  imap: ServerConfig;
  smtp: ServerConfig;
}

export interface OAuthAccountInput {
  email: string;
  displayName: string;
  provider: 'google' | 'microsoft';
}

export interface AccountPatch {
  displayName?: string;
  password?: string;
}

export type MoveTarget = { role: FolderRole } | { folderId: string };

export interface FlagChange {
  seen?: boolean;
  flagged?: boolean;
}

export interface DraftCheck {
  estimatedBytes: number;
  limitBytes: number;
}

export interface MailEvents {
  'mail://sync-status': SyncStatus;
  'mail://changed': { accountId: string; folderIds: string[] };
  'mail://outbox-changed': { entries: OutboxEntry[] };
  /** The backend also shows the desktop notification when enabled. */
  'mail://new-mail': { accountId: string; count: number };
  'mail://accounts-changed': Record<string, never>;
}
export type MailEventName = keyof MailEvents;

/**
 * One method per backend command. Every method rejects with an AppError on
 * failure (see toAppError for normalising unexpected rejections).
 */
export interface MailApi {
  suggestServers(email: string): Promise<ServerSuggestion | null>;
  addAccount(input: NewAccountInput): Promise<Account>;
  addOAuthAccount(input: OAuthAccountInput): Promise<Account>;
  updateAccount(id: string, patch: AccountPatch): Promise<Account>;
  removeAccount(id: string): Promise<void>;
  listAccounts(): Promise<Account[]>;
  listFolders(accountId: string): Promise<Folder[]>;
  listMessages(query: ListQuery): Promise<MessagePage>;
  getMessage(id: string, allowRemote: boolean): Promise<MessageDetail>;
  setFlags(ids: string[], change: FlagChange): Promise<void>;
  moveMessages(ids: string[], target: MoveTarget): Promise<void>;
  deleteForever(ids: string[]): Promise<void>;
  saveAttachment(messageId: string, index: number): Promise<string | null>;
  pickAttachments(): Promise<DraftAttachment[]>;
  checkDraft(draft: Draft): Promise<DraftCheck>;
  sendMessage(draft: Draft): Promise<OutboxEntry>;
  /** Stores the draft in Drafts, replacing `draft.draftMessageId` if set. */
  saveDraft(draft: Draft): Promise<DraftSaved>;
  /** Copies of a message's attachments, usable in a Draft (forwarding, continuing a draft). */
  attachmentsForDraft(messageId: string): Promise<DraftAttachment[]>;
  listOutbox(): Promise<OutboxEntry[]>;
  outboxSendNow(id: string): Promise<void>;
  outboxCancel(id: string): Promise<void>;
  outboxEdit(id: string): Promise<Draft>;
  syncNow(accountId: string | null): Promise<void>;
  getSettings(): Promise<Settings>;
  setSettings(settings: Settings): Promise<Settings>;
  openExternal(url: string): Promise<void>;
  /** Subscribe to a backend event. Returns an unsubscribe function. */
  on<E extends MailEventName>(event: E, handler: (payload: MailEvents[E]) => void): () => void;
}
