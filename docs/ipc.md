# Interface ↔ backend contract

The interface calls Tauri commands with `invoke(name, args)` and listens to
events with `listen(name, cb)`. Argument keys are camelCase on the JS side
(Tauri maps them to snake_case Rust parameters). Every command that can fail
rejects with an `AppError`. IDs are opaque strings.

```ts
type AppError = {
  kind: "auth" | "connect" | "tls" | "config" | "not_found" | "invalid" | "too_large" | "storage" | "keyring" | "internal";
  message: string; // safe to show the user; never contains passwords or tokens
};

type Security = "tls" | "starttls";
type ServerConfig = { host: string; port: number; security: Security };
type AuthKind = "password" | "google" | "microsoft";

type Account = {
  id: string; email: string; displayName: string; auth: AuthKind; createdAt: number;
  color: string;            // hex, assigned by backend from a fixed palette
  imap: ServerConfig; smtp: ServerConfig; sizeLimitBytes: number | null;
  sortOrder: number;        // place in the sidebar from 0; list_accounts is sorted by it. Until the user
                            // reorders, oldest first; accounts added after a reorder go last
  syncMonths: number | null; // months of mail kept in full (bodies downloaded); null: current behaviour
};

type FolderRole = "inbox" | "sent" | "drafts" | "archive" | "junk" | "trash" | "all" | "flagged" | "other";
type Folder = {
  id: string; accountId: string; name: string; path: string; role: FolderRole; unread: number; total: number;
  delimiter: string | null;  // hierarchy separator; nest folders by splitting `path` on it. A parent that
                             // isn't itself a folder (server container) is shown as a plain label.
  favorite: boolean;         // listed under Favourites; this device only (kept while the server lists it)
};

type Mailbox = { name: string | null; address: string };

type Importance = "low" | "normal" | "high";
type Category = { name: string; color: string };  // name unique ignoring case, across accounts; color "#rrggbb"

type MessageListItem = {
  id: string; accountId: string; folderId: string;
  from: Mailbox | null; to: Mailbox[]; subject: string; preview: string;
  date: number | null;      // unix seconds
  unread: boolean; flagged: boolean; answered: boolean; hasAttachments: boolean;
  pinned: boolean;          // this device only; true on a conversation row if any of it is pinned
  importance: Importance;   // from Importance / X-Priority / X-MSMail-Priority, unless set with set_flags
                            // (then local only; headers no longer change it)
  categories: string[];     // category names in the order added (see "Categories" below)
  flagDue: number | null;   // follow-up date of a flagged message, unix seconds; this device only
  size: number;             // bytes, as the server reported
  threadCount: number;      // 1 unless grouped
  threadIds?: string[];     // when grouped
};

type AttachmentMeta = {
  index: number; fileName: string; mimeType: string; size: number;
  preview: 'image' | 'pdf' | 'text' | null; // in-app preview offered, judged by name and type (never SVG or HTML); null: open or save instead
};

type MessageDetail = MessageListItem & {
  cc: Mailbox[]; replyTo: Mailbox[];
  bcc: Mailbox[];           // only on the user's own drafts and sent mail (read from the Bcc header saved with them)
  html: string | null;      // already sanitized, inline images embedded; render only in <iframe sandbox="allow-popups allow-same-origin"> (never allow-scripts); link popups are routed to the system browser by the backend
  text: string | null;
  blockedRemoteImages: number;
  attachments: AttachmentMeta[];
  messageIdHeader: string | null; inReplyTo: string | null; references: string[];
  invitation: InvitationView | null;  // from the first text/calendar part (iMIP)
};

type InvitationView = {
  method: string;                     // REQUEST, CANCEL, REPLY, PUBLISH
  event: CalendarEvent;               // without id, calendarId, accountId, color
  sequence: number;
  myStatus: string | null;            // your PARTSTAT in it, lower case
  replier: { email, name, status } | null;   // for REPLY
  calendarEventId: string | null;     // the same UID in your calendar
  savedStatus: string | null;         // your answer as saved there
  replyPending: boolean;              // REPLY not yet in your calendar copy
  hasCalendar: boolean;
};

type ListQuery = {
  accountId: string | null;          // null = all accounts (unified)
  folderId: string | null;           // a specific folder, or
  role: FolderRole | null;           // a role across accounts (unified Inbox etc.)
  filter: "all" | "unread" | "flagged" | "attachments" | "toMe" | "pinned";
                                     // toMe: the account's own address is in To or Cc
  search: string | null;             // full-text over subject, people, body
  offset: number; limit: number;     // limit <= 200
  focus?: "focused" | "other" | null;
  sort?: "date" | "from" | "subject" | "size";  // default date; from = sender name, else address
  descending?: boolean;              // default true (newest, Z-A, largest first); ties newest first
  // Pinned messages come first, then the sort, in every view except search results
  // (folder, unified role, Focused/Other, threads).
};
type MessagePage = { items: MessageListItem[]; total: number };

type DraftAttachment = { path: string; fileName: string; size: number };
type Draft = {
  accountId: string; to: string; cc: string; bcc: string;   // as typed; backend validates
  subject: string; bodyText: string;
  inReplyTo: string | null; references: string[];
  attachments: DraftAttachment[];
  replyToMessageId: string | null;   // marks original \Answered after send
  draftMessageId: string | null;     // Message-ID of the saved draft this replaces (from save_draft, or
                                     // MessageDetail.messageIdHeader when continuing a draft from Drafts)
  importance?: Importance;           // high: "Importance: High" + "X-Priority: 1 (Highest)"; low: "Low" +
                                     // "5 (Lowest)"; normal (default): no headers. outbox_edit returns it.
};

type OutboxState =
  | { state: "queued"; next_attempt_at: number; maybe_sent: boolean }
  | { state: "sending"; started_at: number }
  | { state: "sent"; at: number }
  | { state: "failed"; reason: "rejected" | "gave_up" | "too_large" | "unconfirmed"; message: string }
  | { state: "cancelled" };
type OutboxEntry = {
  id: string; accountId: string; to: Mailbox[]; subject: string; preview: string;
  createdAt: number; attempts: number; status: OutboxState;
  progress: number | null;            // 0..1 while sending, else null
};

type Settings = { theme: "system" | "light" | "dark"; remoteImages: "block" | "allow"; notifications: boolean;
  readingPane: "right" | "bottom" | "off";  // default right; stored for the interface
  workOffline: boolean;                     // default false; see "Work Offline" below
  /* …and the others in Settings (types.ts) */ };
type SyncStatus = { accountId: string; state: "syncing" | "idle" | "offline" | "error"; lastSyncedAt: number | null; message: string | null };
```

**Work Offline** (`Settings.workOffline`): while on, the sync engines and the outbox worker
make no connections. Every account reports `state: "offline"` with the message "Working
offline. Turn off Work Offline to sync and send."; changes (flags, moves, copies,
categories, junk) are queued as they are without a network, and sending waits in the Outbox.
Opening a message whose body isn't on the device and `save_draft` reject `connect`.
Switching it off wakes every engine and the outbox.

**Categories** are kept on the server where it can: on Gmail accounts (Google sign-in, or IMAP
host `imap.gmail.com`) as labels (`X-GM-LABELS`, modified UTF-7), and sync reads labels back as
categories (system labels such as `\Inbox`, `\Important`, `\Sent`, `\Starred`, `\Draft` are
skipped; a new label becomes a category with a palette colour). Elsewhere each category is an
IMAP keyword: `$Cat_` + the name with every character that isn't an ASCII letter or digit
written as `_` and two upper-case hex digits per UTF-8 byte ("Red team" → `$Cat_Red_20team`,
"Café" → `$Cat_Caf_C3_A9`); keywords in that form are read back as categories. A folder whose
`PERMANENTFLAGS` lacks `\*` keeps them on this device only (and sync leaves them alone).

## Commands

| Command | Args | Returns |
|---|---|---|
| `suggest_servers` | `{ email }` | `{ imap: ServerConfig; smtp: ServerConfig; auth: AuthKind } \| null` |
| `add_account` | `{ input: { email, displayName, password, imap, smtp } }` | `Account` (logs in to IMAP and SMTP first; nothing saved on failure) |
| `add_oauth_account` | `{ input: { email, displayName, provider: "google" \| "microsoft" } }` | `Account` (opens the system browser at the provider's account chooser; `email` may be `""`, and the account picked there is used; may reject `config` until OAuth is configured) |
| `update_account` | `{ id, patch: { displayName?, password?, syncMonths?: number \| null } }` | `Account` (`syncMonths` 1–240, or null for the default; with a window, headers are synced for the newest 1000 messages per folder plus every message inside it (IMAP `SEARCH SINCE`) and bodies under 2 MB inside it are downloaded, 500 per folder per pass; changing it wakes that account's sync; bodies already downloaded stay) |
| `reorder_accounts` | `{ ids }` | `void` (every account id exactly once, in the new sidebar order, else `invalid`; local only; emits `mail://accounts-changed`) |
| `reconnect_account` | `{ id }` (Google and Microsoft accounts: sign in again, same address) | `Account` |
| `remove_account` | `{ id }` | `void` (deletes local mail and keyring entry, not server mail) |
| `list_accounts` | | `Account[]` (by `sortOrder`) |
| `list_folders` | `{ accountId }` | `Folder[]` (sorted by role, then name) |
| `create_folder` | `{ accountId, name, parentId: string \| null }` | `void` (IMAP CREATE now, online only; name encoded as modified UTF-7; top-level folders go under `INBOX.` on servers that keep everything there; the folder list is re-read and `mail://changed` emitted) |
| `rename_folder` | `{ folderId, name }` | `void` (own folders only; built-in ones are refused with `invalid`) |
| `delete_folder` | `{ folderId }` | `void` (own folders only, and not while they contain folders; the mail in it is deleted on the server) |
| `set_folder_favorite` | `{ folderId, favorite }` | `void` (local only; emits `mail://changed` for the folder) |
| `list_messages` | `{ query: ListQuery, threads?: boolean }` | `MessagePage` (`query.focus` `'focused' \| 'other'` shows one half of inbox folders: the user's rule for the sender wins, then people the user has written to are Focused, then mail with List-Unsubscribe, List-Id, Precedence bulk or Auto-Submitted, or from noreply@, notifications@, newsletter@ and similar, is Other; other folders ignore it. With `threads`, one row per conversation: its newest matching message, `unread` if any is, `threadCount` and `threadIds` (newest first; actions on the row use all of them); `total` counts conversations. A conversation is the first id in References, else In-Reply-To, else the Message-ID) |
| `set_focus_rule` | `{ sender, focus: 'focused' \| 'other' \| null, wholeDomain }` | `void` (Focused Inbox: always put this address, or with `wholeDomain` its `@domain`, in that half; null removes the rule) |
| `preview_body` | `{ body }` | `string` (the HTML recipients will see for this compose text: Markdown-style marks such as `**bold**`, `*italic*`, `- ` lists, `1. ` lists, `> ` quotes, `` `code` `` and `[text](url)` rendered, typed HTML escaped. Sending adds this HTML part whenever the body uses such marks; the text part is kept as typed) |
| `take_mailto` | | `Mailto \| null` (`{ to, cc, bcc, subject, body }` from the `mailto:` link the app was started with, once; Linux `.desktop` and Windows MSIX register the app for `mailto:`, and a link opened while it runs arrives as `mail://mailto` `{ url }`) |
| `parse_mailto` | `{ url }` | `Mailto \| null` (Android hands its SENDTO/VIEW link to the page through `window.MockyIntents.takeMailto()`; links only fill in a new message, control characters are dropped from fields) |
| `get_thread` | `{ id }` | `MessageListItem[]` (every message in its conversation in the account, sent ones included, oldest first; Trash and Spam left out unless the message is there) |
| `get_message` | `{ id, allowRemote: boolean }` | `MessageDetail` (fetches body if not cached; marks nothing read) |
| `set_flags` | `{ ids, seen?: boolean, flagged?: boolean, pinned?: boolean, importance?: Importance, flagDue?: number \| null }` | `void` (local at once. `seen`/`flagged` reach the server when online; `pinned`, `importance` and `flagDue` stay on this device. A `flagDue` number (unix seconds) also flags the message; null clears the date; unflagging, here or on another device, clears it too. At the due time the app shows one notification "Follow up: <subject>") |
| `move_messages` | `{ ids, target: { role: FolderRole } \| { folderId: string } }` | `void` (a message moved again before the server gave it a new UID is found by Message-ID) |
| `copy_messages` | `{ ids, target: { role: FolderRole } \| { folderId: string } }` | `void` (same account only; IMAP `UID COPY`, queued like moves so it works offline; nothing changes locally until the target folder syncs, when the copies appear as new messages) |
| `mark_junk` | `{ ids, junk: boolean }` | `void` (true: moved to the account's Junk folder (role `junk`) and marked `$Junk`, `$NotJunk` removed; false: moved to the Inbox, marked `$NotJunk`, `$Junk` removed. The keyword is set before the move. No Junk folder: `invalid` with a plain message, and nothing changes) |
| `list_categories` | | `Category[]` (by name) |
| `save_category` | `{ category: Category, previousName: string \| null }` | `Category[]` (adds one, or sets the colour of the one with that name; with `previousName`, renames it on every message too, queuing the server change: old label/keyword removed, new one added. A name another category has: `invalid`; unknown `previousName`: `not_found`. Names: 1–60 characters, not starting with `\`; colours `#rrggbb`) |
| `delete_category` | `{ name }` | `Category[]` (removed from every message too, and from the server) |
| `set_categories` | `{ ids, add: string[], remove: string[] }` | `void` (case is ignored; names not yet known become categories with a palette colour; changed messages get the server change queued) |
| `open_message_window` | `{ id }` | `void` (computers: a window `message-<id>` (900×700, titled with the subject) loading the app with `?message=<url-encoded id>`, or the existing one focused; same permissions and link rules as the main window; closing it never hides to the tray. Phones: `invalid`) |
| `suggest_recipients` | `{ query }` | `Mailbox[]` (up to 8 people written to or heard from, best first; own addresses and no-reply senders left out) |
| `delete_forever` | `{ ids }` | `void` (only allowed for messages in Trash or Junk) |
| `save_attachment` | `{ messageId, index }` | `string \| null` (shows a save dialog, Android's "Save to" screen on phones; where it was saved, null if cancelled) |
| `save_message` | `{ messageId }` | `string \| null` (the whole message as an `.eml` file named after its subject; save dialog, Android's "Save to" screen on phones; null if cancelled) |
| `message_source` | `{ messageId }` | `string` (the message as received, headers and body; cut after 2 MB with a note) |
| `print_html` | `{ html, title }` | `void` (Android only: the system print screen, which can also save a PDF; desktop prints the page from a sandboxed frame instead and gets `invalid` here) |
| `preview_attachment` | `{ messageId, index }` | raw bytes (`ArrayBuffer`) for the in-app preview; rejects `invalid` unless the file's own bytes are a PNG/JPEG/GIF/WebP photo, a PDF or plain text, up to 30 MB |
| `open_attachment` | `{ messageId, index }` | `void` (copies it to the app cache and opens it in the device's app for its type; programs and scripts are refused with `invalid`) |
| `pick_attachments` | | `DraftAttachment[]` (open-file dialog; on Android the system picker, each file copied into the app cache) |
| `check_draft` | `{ draft }` | `{ estimatedBytes: number; limitBytes: number }` or rejects `invalid`/`too_large` |
| `send_message` | `{ draft }` | `OutboxEntry` (validated, queued and held for `Settings.undoSendSecs` so `outbox_edit`/`outbox_cancel` can undo it; outbox worker then sends; removes the saved draft `draft.draftMessageId` if set) |
| `attachments_for_draft` | `{ messageId }` | `DraftAttachment[]` (copies of a message's attachments, usable in a Draft: forwarding, continuing a draft) |
| `save_draft` | `{ draft }` | `{ draftMessageId: string }` (stored in Drafts; replaces `draft.draftMessageId` if set; pass the returned id on the next save) |
| `list_outbox` | | `OutboxEntry[]` |
| `outbox_send_now` | `{ id }` | `void` |
| `outbox_cancel` | `{ id }` | `void` |
| `outbox_edit` | `{ id }` | `Draft` (removes it from the outbox) |
| `sync_now` | `{ accountId: string \| null }` | `void` |
| `get_settings` / `set_settings` | `{ settings }` | `Settings`. `readingPane` (`right`, `bottom`, `off`; anything else becomes `right`) is only stored; changing `workOffline` wakes every sync engine and the outbox. On computers also `closeToTray` (closing the window hides it to the tray icon: Open, New message (emits `mail://compose`), Check for mail, Quit) and `startAtLogin` (applied to the system through tauri-plugin-autostart, launching with `--hidden` into the tray); `device: { tray, startAtLogin }` is read only (start at login is off inside a Store package). |
| `list_contacts` | `{ query }` | `Contact[]` (saved contacts matching name, email, phone or company, by name; `Contact = { id, accountId: string \| null, uid, name, emails, phones, organization, title, notes, state: 'synced' \| 'changed' \| 'deleted' }`) |
| `get_contact` | `{ id }` | `Contact` |
| `save_contact` | `{ contact: { id?, accountId: string \| null, name, emails, phones, organization, title, notes } }` | `Contact` (saved here at once; a contact of an account syncs to its server's address book in the background; null `accountId` keeps it on this device) |
| `delete_contact` | `{ id }` | `void` (gone here at once; deleted on the server at the next sync) |
| `sync_contacts` | | `{ accountId, state: 'synced' \| 'unavailable' \| 'error', message }[]`. Password accounts sync with CardDAV on their IMAP host (`https://<imap host>/.well-known/carddav`, e.g. SOGo), Basic auth over HTTPS only; local changes are sent first (newest edit wins), then server changes are read; fields the app doesn't show (photos, addresses) are kept. Also runs at start-up and every 15 minutes, and emits `mail://contacts-changed` `{ books }`. Gmail and Microsoft accounts are `unavailable` for now. `suggest_recipients` lists saved contacts before people from mail. |
| `list_calendars` | | `{ id, accountId, name, color: string \| null }[]` (each password account's calendars on its server) |
| `list_events` | `{ from, to }` (Unix seconds, at most 400 days apart) | `CalendarEvent[]`: every occurrence in the window, all-day first then by start. `CalendarEvent = { id (the stored event; a series shares it), calendarId, accountId, color, uid, recurrenceId: number \| null (a series date's original start), title, location, notes, start, end (exclusive), allDay, repeat: 'none' \| 'daily' \| 'weekly' \| 'monthly' \| 'yearly' \| 'custom', recurring, organizer, attendees: { email, name, status }[], tentative, reminders: number[] (minutes before the start, from the event's DISPLAY/AUDIO alarms) }`. All-day `start`/`end` are midnight UTC of the first day and the day after the last. Repeating events are expanded on the device (RRULE, EXDATE, RDATE, changed or cancelled single dates), in the event's own time zone. |
| `get_event` | `{ id }` | `CalendarEvent` (the event itself: a series' first date and rule) |
| `save_event` | `{ event: { id?, calendarId, title, location, notes, allDay, start, end, repeat: … \| 'keep', timeZone, reminders: number[] \| null } }` | `CalendarEvent`. Saved on the server first (needs a connection; `If-Match` on edits), then here. New events are written in the device's time zone with a `VTIMEZONE`; edits keep everything the app doesn't show (alarms, attendees, other properties). Choosing another calendar moves the event. |
| `delete_event` | `{ id, occurrence: number \| null }` | `void`. With `occurrence` (a `recurrenceId`), only that date of a series is removed (an `EXDATE`); otherwise the event or whole series is deleted. |
| `sync_calendars` | | `{ accountId, state: 'synced' \| 'unavailable' \| 'error', message }[]`. Password accounts sync with CalDAV on their IMAP host (`https://<imap host>/.well-known/caldav`, e.g. SOGo), Basic auth over HTTPS only; calendars that hold events are listed, changed events fetched, and events deleted elsewhere removed. Also runs at start-up and every 15 minutes, and emits `mail://calendar-changed` `{ calendars }` (empty after a change made here). Gmail and Microsoft accounts are `unavailable` for now. |
| (reminders) | | The app checks every 30 s for reminders that came due since its last check (kept in the database, at most a day back) and shows a system notification ("In 15 minutes · 10:00–10:30 · place"), skipping declined and finished events. Flagged messages whose `flagDue` came in that time get "Follow up: <subject>" (always, whatever `eventReminders` says). `reminders` on `save_event` replaces the event's alarms (null keeps them). Settings `eventReminders` (on) and `defaultReminderMins` (15; -1 none) control it. |
| `answer_invitation` | `{ id (message), answer: 'accepted' \| 'tentative' \| 'declined', notify }` | `InvitationView`. For a `METHOD:REQUEST` invitation in the message: the event is saved in the account's calendar with your `PARTSTAT` (into the calendar already holding its `UID`, else the account's personal calendar); a decline removes it. Calendar writes carry `Schedule-Reply: F` so the server doesn't email the organiser as well. With `notify`, a `METHOD:REPLY` email (text note plus `text/calendar; method=REPLY`) to the organiser is queued in the Outbox to send at once. |
| `remove_cancelled_event` | `{ id }` | `InvitationView` (for a `METHOD:CANCEL`: deletes the event with that `UID` from your calendar) |
| `apply_invitation_reply` | `{ id }` | `InvitationView` (for a `METHOD:REPLY` to your invitation: the replier's `PARTSTAT` is written into your calendar copy) |
| `get_out_of_office` | `{ accountId }` | `{ accountId, available, message, active, settings: { enabled, subject, message, start: 'YYYY-MM-DD' \| null, end, days } }`. Read from the account's ManageSieve server (IMAP host, port 4190, STARTTLS required, PLAIN login with the mail password). `available` false (with `message`) for Gmail/Microsoft accounts or servers without ManageSieve or the `vacation` extension. |
| `set_out_of_office` | `{ accountId, settings }` | Same. Writes the script `mocky-mail` (`vacation :days N :subject … :addresses [account email]`, inside `currentdate` tests when dated; the settings are kept as JSON in a comment). A script that was running before is kept running through `include :personal`, and becomes active again when replies are switched off. |
| `get_signature` | `{ accountId, reply?: boolean }` | `Signature \| null` (the account's default signature for new messages, or with `reply` for replies and forwards; `{ html, text, image? }`, image `{ mimeType, data (base64), width, naturalWidth, naturalHeight }`) |
| `list_signatures` | `{ accountId }` | `SignatureSet` (`{ items: (Signature & { id, name })[], defaultNew: string \| null, defaultReply: string \| null }`; the single signature older versions stored is moved into the list on first read) |
| `save_signature` | `{ accountId, id: string \| null, name, html }` | `SignatureSet` (null `id` creates one; the HTML is sanitized like incoming mail; the picture is kept; the first signature becomes both defaults. Sending adds an HTML alternative with the full signature, and its picture as a `multipart/related` part `cid:signature-image@mocky.mail`, for whichever of the account's signatures still has its plain block in the body) |
| `delete_signature` | `{ accountId, id }` | `SignatureSet` |
| `set_signature_defaults` | `{ accountId, defaultNew, defaultReply }` | `SignatureSet` (null: no signature) |
| `set_signature_image` | `{ accountId, id }` | `SignatureSet \| null` (picker; PNG, JPEG or GIF checked by its bytes, up to 1 MB; null if cancelled; a picture-only signature uses the account name as its text) |
| `set_signature_image_width` | `{ accountId, id, width }` | `SignatureSet` (40 to 600 px, never wider than the picture) |
| `remove_signature_image` | `{ accountId, id }` | `SignatureSet` (keeps the words) |
| `check_for_update` | — | `UpdateInfo \| null` (`{ version, notes, how: 'install' \| 'download', page }`); reads `latest.json` on the download site; null for store installs |
| `install_update` | — | `void`; downloads this install's installer, checks its SHA-256, runs it (the app closes or restarts) |
| `open_external` | `{ url }` | `void` (http, https, mailto only) |

## Events

| Event | Payload |
|---|---|
| `mail://sync-status` | `SyncStatus` |
| `mail://changed` | `{ accountId: string; folderIds: string[] }` (refetch lists and counts) |
| `mail://outbox-changed` | `{ entries: OutboxEntry[] }` |
| `mail://new-mail` | `{ accountId: string; count: number }` (the backend also shows the desktop notification when enabled) |
| `mail://accounts-changed` | `{}` (refetch accounts) |
