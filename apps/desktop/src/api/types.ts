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
  /** Messages in this conversation shown by the list (1, or missing, when not grouped). */
  threadCount?: number;
  /** Their ids, newest first, when grouped. Actions on the row apply to all of them. */
  threadIds?: string[];
}

/** How the in-app preview shows an attachment. */
export type PreviewKind = 'image' | 'pdf' | 'text';

export interface AttachmentMeta {
  index: number;
  fileName: string;
  mimeType: string;
  size: number;
  /** Null: no in-app preview; open it in another app or save it. */
  preview: PreviewKind | null;
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
  /** A meeting invitation, cancellation or answer in the message. */
  invitation?: InvitationView | null;
};

/** Your answer to an invitation. */
export type Answer = 'accepted' | 'tentative' | 'declined';

/** A meeting invitation in an email, with what your calendar knows of it. */
export interface InvitationView {
  /** `REQUEST` (invitation or update), `CANCEL`, `REPLY` (someone's answer), `PUBLISH`. */
  method: string;
  /** The event; `id`/`calendarId` are not set (it may not be in a calendar). */
  event: Omit<CalendarEvent, 'id' | 'calendarId' | 'accountId' | 'color'>;
  sequence: number;
  /** Your answer as listed in the invitation. */
  myStatus: string | null;
  /** For a `REPLY`: who answered, with their answer in `status`. */
  replier: EventPerson | null;
  /** The same event in your calendar. */
  calendarEventId: string | null;
  /** Your answer as saved in your calendar. */
  savedStatus: string | null;
  /** For a `REPLY`: your calendar doesn't show this answer yet. */
  replyPending: boolean;
  /** The account has a calendar to save to. */
  hasCalendar: boolean;
}

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
  /** Focused Inbox: only this half of inbox folders (other folders ignore it). */
  focus?: Focus | null;
}

/** What a mailto: link asks to fill in a new message with. */
export interface Mailto {
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
}

/** Focused Inbox halves: people, and newsletters and notifications. */
export type Focus = 'focused' | 'other';

export interface MessagePage {
  items: MessageListItem[];
  total: number;
}

export interface DraftAttachment {
  path: string;
  fileName: string;
  size: number;
}

/** A newer version of the app. `install`: the app can update itself; `download`: open `page`. */
export interface UpdateInfo {
  version: string;
  notes: string;
  how: 'install' | 'download';
  page: string;
}

/** A saved contact. `accountId` null: kept on this device only. */
export interface Contact {
  id: string;
  accountId: string | null;
  uid: string;
  name: string;
  emails: string[];
  phones: string[];
  organization: string;
  title: string;
  notes: string;
  /** `changed` or `deleted`: waiting to be sent to the server. */
  state: 'synced' | 'changed' | 'deleted';
}

/** A contact as typed in the editor. */
export interface ContactInput {
  id?: string;
  accountId: string | null;
  name: string;
  emails: string[];
  phones: string[];
  organization: string;
  title: string;
  notes: string;
}

/** Whether an account's contacts sync with its server. */
export interface AddressBookStatus {
  accountId: string;
  state: 'synced' | 'unavailable' | 'error';
  message: string | null;
}

/** A calendar on an account's mail server. */
export interface CalendarInfo {
  id: string;
  accountId: string;
  name: string;
  /** `#RRGGBB` or `#RRGGBBAA` from the server, if it has one. */
  color: string | null;
}

/** How an event repeats. `custom`: a rule the editor can't show (kept as it is). */
export type Repeat = 'none' | 'daily' | 'weekly' | 'monthly' | 'yearly' | 'custom';

/** Someone on an event. */
export interface EventPerson {
  email: string;
  name: string | null;
  /** `accepted`, `declined`, `tentative`, `needs-action`. */
  status: string | null;
}

/** One occurrence of an event: a single event, or one date of a series. */
export interface CalendarEvent {
  /** The stored event (a whole series shares it). */
  id: string;
  calendarId: string;
  accountId: string;
  color: string | null;
  uid: string;
  /** One date of a series: its original start, which names it. */
  recurrenceId: number | null;
  title: string;
  location: string;
  notes: string;
  /** Unix seconds. All-day: midnight UTC of the first day. */
  start: number;
  /** Unix seconds, exclusive. All-day: midnight UTC after the last day. */
  end: number;
  allDay: boolean;
  repeat: Repeat;
  recurring: boolean;
  organizer: EventPerson | null;
  attendees: EventPerson[];
  tentative: boolean;
}

/** An event from the editor. `repeat: 'keep'` leaves a series' rule alone. */
export interface EventInput {
  id?: string;
  calendarId: string;
  title: string;
  location: string;
  notes: string;
  allDay: boolean;
  start: number;
  end: number;
  repeat: Exclude<Repeat, 'custom'> | 'keep';
  /** The device's time zone, such as `Africa/Nairobi`. */
  timeZone: string;
}

/** Whether an account's calendars sync with its server. */
export interface CalendarStatus {
  accountId: string;
  state: 'synced' | 'unavailable' | 'error';
  message: string | null;
}

/** A picture under the signature (PNG, JPEG or GIF), sent inside the message. */
export interface SignatureImage {
  mimeType: string;
  /** Base64. */
  data: string;
  /** Width shown in messages, px. */
  width: number;
  naturalWidth: number;
  naturalHeight: number;
}

/** An email signature: cleaned HTML, the plain text compose shows, and an optional picture. */
export interface Signature {
  html: string;
  text: string;
  image?: SignatureImage;
}

/** One of an account's signatures. */
export type NamedSignature = Signature & { id: string; name: string };

/** An account's signatures, and which ones new messages and replies start with. */
export interface SignatureSet {
  items: NamedSignature[];
  defaultNew: string | null;
  defaultReply: string | null;
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
  /** Seconds Send waits so it can be undone; 0 sends at once. */
  undoSendSecs: number;
  /** When an opened message counts as read. */
  markRead: MarkReadSetting;
  /** Group replies with the message they answer, as one conversation. */
  threads: boolean;
  /** Split the Inbox into Focused and Other. */
  focusedInbox: boolean;
  /** Minutes between checks for servers that can't announce new mail. */
  checkEveryMins: number;
  /** Seconds a message must stay open to count as read (markRead "delay"). */
  markReadDelaySecs: number;
}

/** "open": as soon as it is opened; "delay": after a few seconds open; "manual": only with Mark as read. */
export type MarkReadSetting = 'open' | 'delay' | 'manual';

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
  'mail://contacts-changed': { books: AddressBookStatus[] };
  /** After a calendar sync (with each account's status) or a change made here (empty). */
  'mail://calendar-changed': { calendars: CalendarStatus[] };
  /** A mailto: link opened while the app runs (desktop). */
  'mail://mailto': { url: string };
}
export type MailEventName = keyof MailEvents;

/**
 * One method per backend command. Every method rejects with an AppError on
 * failure (see toAppError for normalising unexpected rejections).
 */
export interface MailApi {
  suggestServers(email: string): Promise<ServerSuggestion | null>;
  /** People written to or heard from whose name or address starts with `query`. */
  suggestRecipients(query: string): Promise<Mailbox[]>;
  addAccount(input: NewAccountInput): Promise<Account>;
  addOAuthAccount(input: OAuthAccountInput): Promise<Account>;
  updateAccount(id: string, patch: AccountPatch): Promise<Account>;
  removeAccount(id: string): Promise<void>;
  listAccounts(): Promise<Account[]>;
  listFolders(accountId: string): Promise<Folder[]>;
  /** A new folder on the server, at the top or inside `parentId`. */
  createFolder(accountId: string, name: string, parentId: string | null): Promise<void>;
  /** Rename one of the user's own folders. */
  renameFolder(folderId: string, name: string): Promise<void>;
  /** Delete one of the user's own folders and the mail in it. */
  deleteFolder(folderId: string): Promise<void>;
  /** With `threads`, one row per conversation (its newest message). */
  listMessages(query: ListQuery & { threads?: boolean }): Promise<MessagePage>;
  /** Always put mail from this sender (or anyone at its domain) in Focused or Other; null removes the rule. */
  setFocusRule(sender: string, focus: Focus | null, wholeDomain: boolean): Promise<void>;
  /** The mailto: link the app was started with, once (desktop). */
  takeMailto(): Promise<Mailto | null>;
  /** Read a mailto: link (from Android, or the mail://mailto event). */
  parseMailto(url: string): Promise<Mailto | null>;
  /** The HTML recipients will see for this compose text (formatting marks rendered). */
  previewBody(body: string): Promise<string>;
  /** Every message in this message's conversation, oldest first (Trash and Spam left out). */
  getThread(id: string): Promise<MessageListItem[]>;
  getMessage(id: string, allowRemote: boolean): Promise<MessageDetail>;
  setFlags(ids: string[], change: FlagChange): Promise<void>;
  moveMessages(ids: string[], target: MoveTarget): Promise<void>;
  deleteForever(ids: string[]): Promise<void>;
  /** Where it was saved, or null if the user cancelled. */
  saveAttachment(messageId: string, index: number): Promise<string | null>;
  /** Save the whole message as an .eml file; where it was saved, or null if cancelled. */
  saveMessage(messageId: string): Promise<string | null>;
  /** The message as received (headers and body), as text. */
  messageSource(messageId: string): Promise<string>;
  /** Android only: the system print screen (print or save as PDF) for a page. */
  printHtml(html: string, title: string): Promise<void>;
  /** The file's bytes for the in-app preview. Rejects with `invalid` if it isn't really a photo, PDF or text. */
  previewAttachment(messageId: string, index: number): Promise<ArrayBuffer>;
  /** Opens it in the device's app for that type (PDF reader, Word, Gallery). */
  openAttachment(messageId: string, index: number): Promise<void>;
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
  /** A newer version this install can move to, or null. Store installs always get null. */
  checkForUpdate(): Promise<UpdateInfo | null>;
  /** Download, verify and run the update; the app closes or restarts. */
  installUpdate(): Promise<void>;
  /** Saved contacts matching the text (name, email, phone, company), by name. */
  listContacts(query: string): Promise<Contact[]>;
  getContact(id: string): Promise<Contact>;
  /** Create or update; synced with the account's address book in the background. */
  saveContact(contact: ContactInput): Promise<Contact>;
  deleteContact(id: string): Promise<void>;
  /** Sync every account's address book now. */
  syncContacts(): Promise<AddressBookStatus[]>;
  listCalendars(): Promise<CalendarInfo[]>;
  /** Occurrences between two times (Unix seconds, at most about a year apart). */
  listEvents(from: number, to: number): Promise<CalendarEvent[]>;
  /** The event itself (a series' first date and rule), for editing. */
  getEvent(id: string): Promise<CalendarEvent>;
  /** Create or change an event on the server (needs a connection). Returns the event itself. */
  saveEvent(event: EventInput): Promise<CalendarEvent>;
  /** Delete an event or whole series; with `occurrence` (a `recurrenceId`), only that date. */
  deleteEvent(id: string, occurrence: number | null): Promise<void>;
  /** Sync every account's calendars now. */
  syncCalendars(): Promise<CalendarStatus[]>;
  /** Answer an invitation in a message: saved in your calendar (a decline removes it); with `notify`, the organiser is emailed. */
  answerInvitation(messageId: string, answer: Answer, notify: boolean): Promise<InvitationView>;
  /** Take a cancelled event out of your calendar. */
  removeCancelledEvent(messageId: string): Promise<InvitationView>;
  /** Record someone's answer to your invitation in your calendar. */
  applyInvitationReply(messageId: string): Promise<InvitationView>;
  /** The signature a new message (or, with `reply`, a reply or forward) starts with. */
  getSignature(accountId: string, reply?: boolean): Promise<Signature | null>;
  listSignatures(accountId: string): Promise<SignatureSet>;
  /** Create (`id` null) or change a signature's name and words (pasted HTML or typed). Keeps its picture. */
  saveSignature(accountId: string, id: string | null, name: string, html: string): Promise<SignatureSet>;
  deleteSignature(accountId: string, id: string): Promise<SignatureSet>;
  setSignatureDefaults(accountId: string, defaultNew: string | null, defaultReply: string | null): Promise<SignatureSet>;
  /** Choose a picture (PNG, JPG or GIF, up to 1 MB) for a signature; null if cancelled. */
  setSignatureImage(accountId: string, id: string): Promise<SignatureSet | null>;
  /** How wide a signature's picture shows, px. */
  setSignatureImageWidth(accountId: string, id: string, width: number): Promise<SignatureSet>;
  /** Drop a signature's picture, keep its words. */
  removeSignatureImage(accountId: string, id: string): Promise<SignatureSet>;
  getSettings(): Promise<Settings>;
  setSettings(settings: Settings): Promise<Settings>;
  openExternal(url: string): Promise<void>;
  /** Subscribe to a backend event. Returns an unsubscribe function. */
  on<E extends MailEventName>(event: E, handler: (payload: MailEvents[E]) => void): () => void;
}
