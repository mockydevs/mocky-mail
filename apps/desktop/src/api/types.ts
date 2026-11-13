export const FOLDER_IDS = ['inbox', 'outbox', 'drafts', 'sent', 'archive', 'spam', 'trash'] as const;
export type FolderId = (typeof FOLDER_IDS)[number];

export type Hue = 'blue' | 'teal' | 'orange' | 'purple' | 'green';

export interface Account {
  id: string;
  label: string;
  address: string;
  hue: Hue;
  /** Largest message the account's server accepts, in bytes. */
  maxMessageBytes: number;
}

export interface Label {
  id: string;
  name: string;
  hue: Hue;
}

export interface Person {
  name: string;
  address: string;
}

export type OutboxStatus =
  | { kind: 'retrying'; retryInSeconds: number }
  | { kind: 'sending'; progress: number }
  | { kind: 'failed'; reason: 'too_large'; estimatedBytes: number; limitBytes: number }
  | { kind: 'failed'; reason: 'rejected' | 'gave_up'; detail: string };

export interface Attachment {
  name: string;
  mimeType: string;
  sizeBytes: number;
}

export interface MessageSummary {
  id: string;
  folder: FolderId;
  accountId: string;
  direction: 'incoming' | 'outgoing';
  /** Sender for incoming mail, first recipient for outgoing mail. */
  counterpart: Person;
  subject: string;
  preview: string;
  /** ISO 8601 timestamp. */
  date: string;
  unread: boolean;
  flagged: boolean;
  hasAttachments: boolean;
  labelIds: readonly string[];
  outbox?: OutboxStatus;
}

export type MessageBody = { kind: 'text'; text: string } | { kind: 'html'; html: string };

export interface MessageDetail extends MessageSummary {
  body: MessageBody;
  attachments: readonly Attachment[];
}

export interface FolderCounts {
  inboxUnread: number;
  unreadByAccount: Readonly<Record<string, number>>;
  outbox: number;
  drafts: number;
}

export interface ConnectionStatus {
  state: 'online' | 'weak' | 'offline';
  /** ISO 8601 timestamp of the last full sync, if any. */
  lastSyncAt: string | null;
}

export interface Draft {
  accountId: string;
  to: readonly string[];
  cc: readonly string[];
  bcc: readonly string[];
  subject: string;
  body: string;
}

/**
 * Everything the UI needs from the mail engine. The desktop build backs this
 * with Tauri commands into the Rust core; tests and the browser preview use
 * the in-memory mock.
 */
export interface MailApi {
  listAccounts(): Promise<readonly Account[]>;
  listLabels(): Promise<readonly Label[]>;
  listFolder(folder: FolderId): Promise<readonly MessageSummary[]>;
  getMessage(id: string): Promise<MessageDetail | null>;
  getCounts(): Promise<FolderCounts>;
  getConnection(): Promise<ConnectionStatus>;
  syncNow(): Promise<void>;
  setFlagged(id: string, flagged: boolean): Promise<void>;
  markRead(id: string): Promise<void>;
  moveToFolder(id: string, folder: FolderId): Promise<void>;
  queueMessage(draft: Draft): Promise<void>;
  outbox: {
    sendNow(id: string): Promise<void>;
    cancel(id: string): Promise<void>;
  };
}
