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
  /** Place in the sidebar; accounts are listed by it, lowest first. */
  sortOrder: number;
  /** Months of mail kept in full on this device; older mail keeps its headers and preview. null: all. */
  syncMonths: number | null;
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
  /** Listed under Favourites at the top of the sidebar. */
  favorite: boolean;
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
  /** Kept at the top of its folder. */
  pinned: boolean;
  /** From the Importance / X-Priority headers, or set here. */
  importance: Importance;
  /** Category names (see `Category`), in the order they were added. */
  categories: string[];
  /** Flagged with a due date: Unix seconds; a reminder comes then. */
  flagDue: number | null;
  /** Size of the whole message in bytes. */
  size: number;
  /** Signed (S/MIME or OpenPGP), whatever the result of checking it. */
  signed?: boolean;
  /** Encrypted (S/MIME or OpenPGP). */
  encrypted?: boolean;
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
  /** Only a draft still has its Bcc. */
  bcc?: Mailbox[];
  replyTo: Mailbox[];
  /** Already sanitized. Render only in <iframe sandbox="allow-popups">; the backend routes link popups to the system browser. */
  html: string | null;
  text: string | null;
  blockedRemoteImages: number;
  /** Pictures load because the sender (or their domain) is in the trusted list. */
  imagesTrusted?: boolean;
  /** The digital signature on the message and what checking it found; null when unsigned. */
  signature?: SignatureCheck | null;
  /** The message was encrypted: `decrypted` (shown here), or `locked` with why (no key, PIN refused…). */
  encryption?: { kind: SecurityKind; state: 'decrypted' | 'locked'; detail: string | null } | null;
  /** The sender asked for a read receipt. `answered`: one was sent or declined here already. */
  readReceipt?: { requestedBy: string; answered: boolean } | null;
  attachments: AttachmentMeta[];
  messageIdHeader: string | null;
  /** In-Reply-To of this message (for continuing a saved reply draft). */
  inReplyTo: string | null;
  references: string[];
  /** A meeting invitation, cancellation or answer in the message. */
  invitation?: InvitationView | null;
};

/** Automatic replies (out of office), kept on the mail server. */
export interface Vacation {
  enabled: boolean;
  subject: string;
  message: string;
  /** First day, `YYYY-MM-DD`; null from now. */
  start: string | null;
  /** Last day, `YYYY-MM-DD`; null until switched off. */
  end: string | null;
  /** Days before the same sender gets the reply again. */
  days: number;
}

/** An account's automatic replies. */
export interface OutOfOffice {
  accountId: string;
  /** The server can send automatic replies for this account. */
  available: boolean;
  /** Why not, in plain words. */
  message: string | null;
  /** Replies are on at the server now. */
  active: boolean;
  settings: Vacation;
}

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

export type ReadFilter = 'all' | 'unread' | 'flagged' | 'attachments' | 'toMe' | 'pinned';

/** How the list is ordered. Pinned messages come first in folder views either way. */
export type ListSort = 'date' | 'from' | 'subject' | 'size';

export type Importance = 'low' | 'normal' | 'high';

export type SearchScope = 'folder' | 'subfolders' | 'account' | 'all';

/** One piece of a parsed search, for showing the query back as chips. */
export interface SearchTerm {
  /** `text` for plain words and phrases; otherwise the operator without its colon. */
  field: 'text' | 'from' | 'to' | 'cc' | 'subject' | 'body' | 'has' | 'is' | 'category' | 'before' | 'after' | 'in' | 'larger' | 'smaller';
  value: string;
  negated: boolean;
  /** Joined to the next term by OR (absent otherwise). */
  orWithNext?: boolean;
}

/** The result of parsing a search box query. */
export interface ParsedSearch {
  terms: SearchTerm[];
  /** Plain-language problems, e.g. "after: needs a date like 2026-10-01". */
  errors: string[];
}

/** What a rule looks at. */
export type RuleConditionKind =
  | 'from' | 'to' | 'cc' | 'toOrCc' | 'subject' | 'body' | 'subjectOrBody' | 'header'
  | 'hasAttachment' | 'importance' | 'sizeOver' | 'sizeUnder' | 'category' | 'toMe' | 'onlyToMe';

export interface RuleCondition {
  kind: RuleConditionKind;
  /**
   * Words or addresses for the text kinds (several, comma-separated: any of
   * them matches; case-insensitive "contains"); a header name and words as
   * `Name: words` for `header`; `high`/`normal`/`low` for `importance`;
   * kilobytes for the size kinds; the category name for `category`. Unused
   * for `hasAttachment`, `toMe`, `onlyToMe`.
   */
  value: string;
}

export type RuleActionKind =
  | 'move' | 'copy' | 'categorize' | 'flag' | 'clearFlag' | 'clearCategories'
  | 'markRead' | 'setImportance' | 'delete' | 'notify' | 'pin';

export interface RuleAction {
  kind: RuleActionKind;
  /** For move/copy. A folder of the rule's account (rules for all accounts can only use `role`). */
  folderId?: string | null;
  /** For move/copy in a rule for all accounts: the folder with this role in each account. */
  role?: FolderRole | null;
  /** For categorize. */
  category?: string | null;
  /** For setImportance. */
  importance?: Importance | null;
}

export interface Rule {
  id: string;
  name: string;
  enabled: boolean;
  /** null: every account. */
  accountId: string | null;
  /** `all`: every condition must match; `any`: one is enough. No conditions: every message. */
  match: 'all' | 'any';
  conditions: RuleCondition[];
  /** The rule skips a message that matches any of these. */
  exceptions: RuleCondition[];
  actions: RuleAction[];
  /** Later rules don't run on a message this rule matched. */
  stopProcessing: boolean;
  /** Also write it to the mail server (Sieve), so it runs when this device is off. */
  keepOnServer: boolean;
  /** Read only. `local`: runs here only; `server`: also on the server; `unsupported`: the server can't run it (says why in `serverNote`). */
  serverStatus: 'local' | 'server' | 'unsupported';
  serverNote: string | null;
  /** Lower runs first. */
  order: number;
}

export type RuleInput = Omit<Rule, 'id' | 'serverStatus' | 'serverNote' | 'order'> & { id: string | null };

export interface RuleRunResult {
  /** Messages looked at. */
  checked: number;
  /** Messages at least one rule matched. */
  matched: number;
}

/** A sender whose remote pictures always load. */
/** Check Names: what the recipients typed in a field resolve to. */
export interface RecipientCheck {
  /** Recognised addresses, with names from your contacts filled in. */
  resolved: Mailbox[];
  /** A name that matches more than one contact: pick one. */
  ambiguous: { input: string; candidates: Mailbox[] }[];
  /** Neither an address nor a known contact. */
  invalid: string[];
}

export type SecurityKind = 'smime' | 'openpgp';

/** What checking a message's digital signature found. */
export interface SignatureCheck {
  kind: SecurityKind;
  /**
   * `verified`: valid, and the signer is trusted (a certificate from a trusted
   * authority for S/MIME; a key you trust for OpenPGP). `unverifiedSigner`:
   * the signature is valid but who made it isn't proven. `changed`: the
   * message was changed after signing. `unknown`: it couldn't be checked
   * (missing key, unsupported algorithm, revoked or expired certificate…).
   */
  result: 'verified' | 'unverifiedSigner' | 'changed' | 'unknown';
  /** Who signed, as far as the signature says. */
  signer: { name: string | null; address: string | null; fingerprint: string | null; issuer: string | null } | null;
  /** The signer's address doesn't match the From address. */
  fromMismatch: boolean;
  /** Plain-language explanation of the result. */
  detail: string;
  /** When it was signed (Unix seconds), if the signature says. */
  signedAt: number | null;
}

/** A certificate or key on this device, yours or someone else's. */
export interface SecurityKey {
  id: string;
  kind: SecurityKind;
  /** The name on it. */
  owner: string;
  addresses: string[];
  fingerprint: string;
  /** Unix seconds; null: doesn't expire. */
  expires: number | null;
  canSign: boolean;
  canEncrypt: boolean;
  /** Has the private part (it is yours). */
  isOwn: boolean;
  /** Where it lives: the system store, a hardware token, gpg-agent, or Mocky Mail's own store. */
  source: 'windows' | 'keychain' | 'pkcs11' | 'gpg' | 'android' | 'mocky';
  /** For other people's keys: you've said it really is theirs. */
  trusted: boolean;
}

/** Whether a message to these recipients can be encrypted. */
export interface EncryptionCheck {
  kind: SecurityKind;
  /** Each recipient, and whether their key or certificate is known. */
  recipients: { address: string; canEncrypt: boolean }[];
  /** The account has a key of this kind to sign and to encrypt to itself. */
  ownKey: boolean;
}

export interface ImageSender {
  /** An address, or a domain when `wholeDomain`. */
  sender: string;
  wholeDomain: boolean;
}

/** A colour category. Names are unique (case-insensitive) across all accounts. */
export interface Category {
  name: string;
  /** Hex colour, `#rrggbb`. */
  color: string;
}

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
  /** Default `date`. */
  sort?: ListSort;
  /** Default true: newest, Z–A, largest first. */
  descending?: boolean;
  /**
   * Where `search` looks. Default `folder`: the open view only. `subfolders`
   * adds the folders below it, `account` the whole account, `all` every
   * account. `search` takes the query language in docs/ipc.md ("Search").
   */
  searchScope?: SearchScope;
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
  /** Reminders, minutes before the start (0: at the start). */
  reminders: number[];
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
  /** Reminders, minutes before; null leaves an edited event's reminders as they are. */
  reminders: number[] | null;
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
  /**
   * The message as written in the rich-text editor (HTML). When set, it is
   * what is sent (sanitized by the backend, with a plain-text part made from
   * it) and `bodyText` is only a plain-text fallback; null or missing: plain
   * `bodyText` with the old inline marks, as before 0.7.0.
   */
  bodyHtml?: string | null;
  inReplyTo: string | null;
  references: string[];
  attachments: DraftAttachment[];
  /** Marks the original \Answered after sending. */
  replyToMessageId: string | null;
  /** Sent as the Importance and X-Priority headers; default normal. */
  importance?: Importance;
  /** Ask for a read receipt (Disposition-Notification-To: the From address). */
  requestReadReceipt?: boolean;
  /** Sign the message (with the account's default key of `securityKind`). */
  sign?: boolean;
  /** Encrypt it to every recipient (and to yourself, so Sent stays readable). */
  encrypt?: boolean;
  /** S/MIME or OpenPGP; default the account's preferred kind. */
  securityKind?: SecurityKind;
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
  /** Notifications before calendar events. */
  eventReminders: boolean;
  /** Reminder for new events, minutes before; -1 for none. */
  defaultReminderMins: number;
  /** Computers: closing the window keeps the app running in the tray. */
  closeToTray: boolean;
  /** Computers: open at login, into the tray. */
  startAtLogin: boolean;
  /** Where the open message shows on computers (phones always use the full screen). */
  readingPane: 'right' | 'bottom' | 'off';
  /** Pause all syncing and sending until switched off. */
  workOffline: boolean;
  /** When a message you open asks for a read receipt. */
  readReceipts: 'ask' | 'always' | 'never';
  /** Sign every new message when the account has a key. */
  signByDefault: boolean;
  /** Encrypt when every recipient's key is known. */
  encryptWhenPossible: boolean;
  /** Read only: what this device offers. */
  device: { tray: boolean; startAtLogin: boolean };
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
  /** null: keep all mail in full. */
  syncMonths?: number | null;
}

export type MoveTarget = { role: FolderRole } | { folderId: string };

export interface FlagChange {
  seen?: boolean;
  flagged?: boolean;
  pinned?: boolean;
  importance?: Importance;
  /** Unix seconds, or null to clear the due date (a date also sets `flagged`). */
  flagDue?: number | null;
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
  /** A rule's server copy was written, or writing it gave up. */
  'mail://rules-changed': { rules: Rule[] };
  'mail://contacts-changed': { books: AddressBookStatus[] };
  /** After a calendar sync (with each account's status) or a change made here (empty). */
  'mail://calendar-changed': { calendars: CalendarStatus[] };
  /** A mailto: link opened while the app runs (desktop). */
  'mail://mailto': { url: string };
  /** The tray icon's New message. */
  'mail://compose': Record<string, never>;
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
  /** Google and Microsoft accounts: sign in again after the sign-in expired; it must be the same address. */
  reconnectAccount(id: string): Promise<Account>;
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
  /** Parse a search box query without running it (for chips and help). */
  parseSearch(query: string): Promise<ParsedSearch>;
  listRules(): Promise<Rule[]>;
  /** Adds (id null) or changes a rule; returns them all, in order. */
  saveRule(rule: RuleInput): Promise<Rule[]>;
  deleteRule(id: string): Promise<Rule[]>;
  /** Every rule id, in the new order. */
  reorderRules(ids: string[]): Promise<Rule[]>;
  /** Run rules now on a folder's messages. `ruleIds` null: every enabled rule of that account. */
  runRules(ruleIds: string[] | null, folderId: string): Promise<RuleRunResult>;
  /** How many messages in a folder an unsaved rule would match (for the editor). */
  previewRule(rule: RuleInput, folderId: string): Promise<number>;
  listImageSenders(): Promise<ImageSender[]>;
  /** Always load pictures from this sender (or their whole domain). */
  trustImageSender(sender: string, wholeDomain: boolean): Promise<ImageSender[]>;
  untrustImageSender(sender: string): Promise<ImageSender[]>;
  /** Check Names for one recipient field's text. */
  resolveRecipients(text: string): Promise<RecipientCheck>;
  /** A contact as a .vcf file to attach (Insert › Contact card). */
  contactCardAttachment(contactId: string): Promise<DraftAttachment>;
  /** A message as an attached .eml file (Insert › Item). */
  messageAsAttachment(messageId: string): Promise<DraftAttachment>;
  /** Send (true) or decline (false) the read receipt a message asked for. */
  answerReadReceipt(messageId: string, send: boolean): Promise<void>;
  /** Computers: continue writing this draft in a window of its own. Phones reject `invalid`. */
  openComposeWindow(draft: Draft): Promise<void>;
  /** The draft a compose window was opened with (`?compose=<key>` in its address); null once taken. */
  takeComposeDraft(key: string): Promise<Draft | null>;
  /** Your keys and certificates, and the ones collected from other people. */
  listKeys(): Promise<SecurityKey[]>;
  /**
   * Pick a key or certificate file (.p12/.pfx, .cer/.pem, .asc/.gpg) and add it.
   * `password` unlocks a .p12/.pfx or a protected OpenPGP secret key; it is
   * used once and not kept. null: cancelled.
   */
  importKey(password: string | null): Promise<SecurityKey[] | null>;
  /** Save your public key or certificate to share (.cer or .asc). */
  exportPublicKey(id: string): Promise<string | null>;
  removeKey(id: string): Promise<SecurityKey[]>;
  /** Say another person's key really is theirs (OpenPGP), or stop trusting it. */
  setKeyTrusted(id: string, trusted: boolean): Promise<SecurityKey[]>;
  /** The key an account signs with and encrypts to itself with, per kind (null: none). */
  setAccountKey(accountId: string, kind: SecurityKind, keyId: string | null): Promise<void>;
  /** The account's chosen keys. */
  getAccountKeys(accountId: string): Promise<{ smime: string | null; openpgp: string | null; preferred: SecurityKind }>;
  setPreferredKind(accountId: string, kind: SecurityKind): Promise<void>;
  /** Before sending encrypted: which recipients' keys are known (looks up OpenPGP keys by WKD as well). */
  checkEncryption(accountId: string, addresses: string[], kind: SecurityKind): Promise<EncryptionCheck>;
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
  /** The originals stay; copies go to `target`. */
  copyMessages(ids: string[], target: MoveTarget): Promise<void>;
  /** true: to Junk, marked $Junk. false: back to the Inbox, marked $NotJunk. */
  markJunk(ids: string[], junk: boolean): Promise<void>;
  listCategories(): Promise<Category[]>;
  /** Adds or changes one; `previousName` renames it on every message. Returns them all. */
  saveCategory(category: Category, previousName: string | null): Promise<Category[]>;
  /** Removes it from every message too. Returns the rest. */
  deleteCategory(name: string): Promise<Category[]>;
  setCategories(ids: string[], add: string[], remove: string[]): Promise<void>;
  setFolderFavorite(folderId: string, favorite: boolean): Promise<void>;
  /** Every account id, in the new order. */
  reorderAccounts(ids: string[]): Promise<void>;
  /** Computers: the message in a window of its own. Phones reject `invalid`. */
  openMessageWindow(id: string): Promise<void>;
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
  /** An account's automatic replies, read from its mail server. */
  getOutOfOffice(accountId: string): Promise<OutOfOffice>;
  /** Save automatic replies on the mail server (switching them on or off). */
  setOutOfOffice(accountId: string, settings: Vacation): Promise<OutOfOffice>;
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
