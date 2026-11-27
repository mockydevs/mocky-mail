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
  html: string | null;      // already sanitized, inline images embedded; render only in <iframe sandbox="allow-popups allow-same-origin"> (never allow-scripts); link popups are routed to the system browser by the backend.
                            // Every link has a `title` with its real address (the sender's own title is dropped); when the
                            // link text is a web address or domain on another site, the title starts
                            // "Link goes to <host>, not the address shown." then a line break and the address
  text: string | null;
  blockedRemoteImages: number;
  imagesTrusted?: boolean;  // true when the sender (or their domain) is in the trusted list: pictures load even with allowRemote false
  attachments: AttachmentMeta[];
  messageIdHeader: string | null; inReplyTo: string | null; references: string[];
  invitation: InvitationView | null;  // from the first text/calendar part (iMIP)
  readReceipt: { requestedBy: string; answered: boolean } | null;
                            // the sender asked for a read receipt (see "Read receipts"); answered: sent or
                            // declined here already. null: none asked for, or one not to answer
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
  search: string | null;             // the query language in "Search" below
  searchScope?: "folder" | "subfolders" | "account" | "all";  // where `search` looks (default folder); see "Search"
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
  bodyHtml?: string | null;          // rich-text compose: the editor's HTML (see "Rich-text compose"); then
                                     // bodyText is only a fallback. null/missing: plain bodyText as before
  requestReadReceipt?: boolean;      // "Disposition-Notification-To: <account address>"; default false
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
  readReceipts: "ask" | "always" | "never"; // default ask; see "Read receipts" below
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

**Search** (`ListQuery.search`, `parse_search`). Plain words must all match, each as the start
of a word (`invoic` finds "invoice"), anywhere in the subject, sender, To, Cc, preview or
downloaded text; accents and case are ignored. `"quoted phrases"` match as written. A leading
`-` leaves out what a word, phrase or operator matches (`-newsletter`, `-from:boss`). `OR` in
capitals between two terms matches either (`from:jane OR from:peter`; chains work; anywhere
else it is the word "OR"). Operators (names any case; values with spaces in double quotes):

| Operator | Matches |
|---|---|
| `from:` `to:` `cc:` `subject:` | words (as word starts) in that field; `from:` and `to:`/`cc:` cover names and addresses (`from:mocky.co.ke`) |
| `body:` | words in the downloaded text, or the preview when the text isn't downloaded |
| `has:attachment` | messages with attachments |
| `is:unread` `is:read` `is:flagged` `is:pinned` `is:answered` | that state (`starred`, `replied`, `new`, `seen` also work) |
| `category:` | that category, ignoring case (`category:"Red team"`) |
| `after:` / `before:` | dated on or after / before the start of that local day: `YYYY-MM-DD` (or `YYYY/MM/DD`), `today`, `yesterday`. Undated mail never matches |
| `in:` | `inbox`, `sent`, `drafts`, `archive`, `junk`/`spam`, `trash` (by role), else a folder name or path (ignoring case) |
| `larger:` / `smaller:` | size in bytes, `k`, `m` or `g` (×1024; `kb`/`mb` too, decimals allowed): `larger:500k`, `smaller:2m` |

Any other `word:` is searched as plain text (`10:30`, `re:`, URLs). Up to 16 terms are used.
A bad value (`after:soon`, `is:sleepy`, `larger:big`) is left out and explained in
`parse_search`'s `errors`; if nothing usable is left, the search matches nothing. Words and
names never become SQL: text terms go to the FTS5 index as quoted literals, everything else is
a bound parameter. Search results are not pinned-first.

Scope (`ListQuery.searchScope`, only while `search` has text): `folder` (default) is the open
view (folder, or role across accounts, Focused/Other as chosen). `subfolders` adds every folder
whose path starts with the view's folder path plus its delimiter (for a unified role view, below
each account's folder with that role). `account` is the view's account (the `accountId`, or the
folder's account; a unified view without one is every account) and `all` every account; both
leave out Trash and Junk and ignore Focused/Other. Any `in:` term (not negated) also widens a
`folder`/`subfolders` search to the account, and then Trash and Junk are searched too, so
`in:trash invoice` works from any folder.

**Rules** (`Rule`, `RuleInput` in `types.ts`; `save_rule` and friends). A rule matches a
message when its conditions do (`match: 'all'`: every one; `'any'`: one is enough; no
conditions: every message) and none of its exceptions does. Text conditions (`from`, `to`,
`cc`, `toOrCc`, `subject`, `body`, `subjectOrBody`) take comma-separated words or addresses;
any one found anywhere in the field, ignoring case, matches (`from` and `to`/`cc` look at
`Name <address>`). `header` is `Name: words` (just `Name`: the header is there); `importance`
is `high`/`normal`/`low` as the list shows it; `sizeOver`/`sizeUnder` are kilobytes; `category`
a category name; `toMe` the account's address in To or Cc; `onlyToMe` the account's address
alone in To and no Cc; `hasAttachment` as the list shows it. Rules run in `order`; a matching
rule with `stopProcessing` ends the run for that message. What every matching rule does is
gathered first, then done in this order: read/flag (`markRead`, `flag`, `clearFlag`; later rules
win), pin and importance (this device), categories (`clearCategories` then `categorize`; queued to
the server like `set_categories`), copies (`copy`, queued like `copy_messages`), and last the
move: the first `move` or `delete` (to the account's Trash) of the matching rules decides where
the message goes; later ones are ignored. `notify` shows a desktop notification with the sender
and subject (more than three at once: one saying how many). Rules for every account
(`accountId` null) move and copy by `role`, to that folder in each account. Rule actions never
set off rules again: rules run once per new message, and a copy a rule makes (same Message-ID in
the account) and mail moved here by the app are skipped when the server's copy arrives.

Rules run by themselves on mail new in an account's inbox since the last sync (not on the first
sync of an account or folder), right after each inbox sync, using the account's rules and the
rules for every account that are on. Bodies of new inbox mail (under 2 MB) are downloaded in
that same sync before the rules run, so `body`/`subjectOrBody` look at the text, and `header`
at the downloaded headers; for a message not downloaded, `body` looks at the preview and
`header` conditions don't match. What the rules move out of the inbox or mark read is left out
of the new-mail notification. In other folders and unified views rules only run with
`run_rules`.

`keepOnServer` also writes the rule to the mail server, so it runs while every device is off:
password accounts with ManageSieve (IMAP host, port 4190, as for out of office), into one
script, `mocky-mail-rules` (`header :contains`, `address :is`, `size`, `fileinto`,
`fileinto :copy`, `addflag`, `stop`; the folder's UTF-8 name). Writes happen in the background
(retried after 20 s, 2 and 10 minutes, and again after the account's next full sync or the next
start); `save_rule` returns at once with `serverStatus: 'local'` and the note "Being saved to
the mail server." while that happens, then `mail://rules-changed`. The server can't run
`body`, `subjectOrBody`, `hasAttachment`, `category`, `onlyToMe`, `categorize`,
`clearCategories`, `clearFlag`, `setImportance`, `notify` or `pin`, nor move/copy/flag where it
lacks `fileinto`/`copy`/`imap4flags`; such rules are `unsupported` with a `serverNote` saying
why, and still run here. Gmail and Microsoft accounts are always `unsupported` ("Gmail and
Outlook keep their own filters, so this rule runs on this device."). A rule that is off isn't
written (`local`, "Turned off, so it isn't on the mail server."). For a rule for every account,
`server` means at least one account took it (the note names them and says why the others
didn't). Rules on the server also run here; that is harmless (what they move never reaches the
inbox here). Only one script runs on a server: when out of office (`mocky-mail`) is on, it
`include`s the rules script; a script that was running before (webmail filters) keeps running
after the rules through `include :personal` and runs alone again when no rule is kept on the
server. Without the `include` extension, rules don't go on a server that already runs another
script (`unsupported`, with a note).

**Rich-text compose** (`Draft.bodyHtml`). The HTML is cleaned before it is sent or saved: kept are
`p br div span b strong i em u s strike del ins sub sup small big font code pre ol ul li blockquote a
table thead tbody tfoot tr td th caption colgroup col img hr h1 h2 h3`; attributes `style`, `dir`,
`align` everywhere, `href` on `a` (http, https, mailto only), `src alt width height` on `img`, `face
size color` on `font`, simple table attributes (`width border cellpadding cellspacing bgcolor colspan
rowspan valign nowrap`), `start`/`type` on lists. `style` keeps colour, background colour, font
family/size/weight/style, text decoration and alignment, indent, line height, margins, padding,
borders, width/height; any declaration with `url(`, `image`, `expression`, `var(`, `@`, `\`… is
dropped. Scripts, event handlers, forms (with their contents), iframes, objects, media, SVG,
stylesheets, comments, classes and ids go; pictures stay only with `cid:` or `data:image/png|jpeg|gif|webp;base64`
sources (others are removed), and `data:` pictures are sent as `multipart/related` parts by
`cid:` (checked by their bytes; over 5 MB each rejects `too_large`). Lists nest 8 deep at most
(deeper ones are flattened). The message goes as `multipart/alternative`: a text part made from the
HTML (paragraphs and line breaks kept, `- ` and `1. ` list items indented by level, links as
`text (address)` unless the text is the address, quotes prefixed `> `, table cells split by tabs,
pictures as `[alt]`) and the HTML in a document whose head has
`<meta name="mocky-compose" content="rich">`. Size checks count the cleaned HTML, the text part,
the pictures and attachments. Two marks steer it:
- `data-mocky-quote` on the element holding the quoted original (ideally a wrapper with the
  "On …, … wrote:" line and the `<blockquote>` inside: `<div data-mocky-quote><p>On …</p><blockquote>…</blockquote></div>`).
  The signature goes right before it; with no such element, at the end.
- `data-mocky-signature="<signature id>"` on an element standing for the signature (empty, or
  showing a preview): it is replaced, whole, by that signature as saved (its HTML, remote logo and
  picture as `cid:signature-image@mocky.mail`), wrapped in
  `<div class="signature" data-mocky-signature="<id>">`, which is also how a sent or saved message
  carries it, so a continued draft gets the current signature again. Without the mark, a signature
  whose plain `-- ` block is still in `bodyText` is put in (as before 0.7.0), unless the HTML
  already shows its words. Put the signature in the editor only inside a marked element, or it is
  sent as ordinary text (its remote logo removed).

Both marks survive `get_message` (so `MessageDetail.html` of a saved draft can be loaded back into
the editor; its pictures arrive as `data:` URLs). `outbox_edit` returns `bodyHtml` (cleaned, pictures
as `data:`) only for messages written in rich-text compose, and `requestReadReceipt`.

**Read receipts.** `Draft.requestReadReceipt` asks for one. For received mail,
`MessageDetail.readReceipt` is set when `Disposition-Notification-To` names exactly one valid address,
the message is not itself a receipt, is not in Sent, Drafts or Junk, and neither its sender nor that
address is the account's own; with `readReceipts: 'never'` it is always null. `answered` is kept on
this device per account and Message-ID (moving the message doesn't ask again). With `'always'`,
`get_message` queues the receipt the first time (once, even if opened in two windows) and returns
`answered: true`, unless the message's `Return-Path` is at another domain than the address (or `<>`),
when it is left to the user as with `'ask'` (RFC 8098). A receipt is an RFC 8098 report from the
account to the address, through the Outbox (sent at once, no undo delay; a copy goes to Sent like any
message): `Subject: Read: <subject>`, In-Reply-To/References the original, `multipart/report;
report-type=disposition-notification` with a short text note and a `message/disposition-notification`
part (`Reporting-UA`, `Final-Recipient: rfc822;<account>`, `Original-Message-ID`, `Disposition:
manual-action/MDN-sent-manually; displayed`, or `MDN-sent-automatically` with `'always'`).

**Pictures from trusted senders.** `trust_image_sender` keeps an address, or a domain that
covers its subdomains; `get_message` then loads remote pictures for mail from it and sets
`imagesTrusted`. The global `remoteImages: 'allow'` loads them for everyone anyway.

## Commands

| Command | Args | Returns |
|---|---|---|
| `suggest_servers` | `{ email }` | `{ imap: ServerConfig; smtp: ServerConfig; auth: AuthKind } \| null` |
| `add_account` | `{ input: { email, displayName, password, imap, smtp } }` | `Account` (logs in to IMAP and SMTP first; nothing saved on failure) |
| `add_oauth_account` | `{ input: { email, displayName, provider: "google" \| "microsoft" } }` | `Account` (opens the system browser at the provider's account chooser; `email` may be `""`, and the account picked there is used; may reject `config` until OAuth is configured) |
| `update_account` | `{ id, patch: { displayName?, password?, syncMonths?: number \| null } }` | `Account` (`syncMonths` 1–240, or null for the default; with a window, headers are synced for the newest 1000 messages per folder plus every message inside it (IMAP `SEARCH SINCE`) and bodies under 2 MB inside it are downloaded, 500 per folder per pass; changing it wakes that account's sync; bodies already downloaded stay) |
| `reorder_accounts` | `{ ids }` | `void` (every account id exactly once, in the new sidebar order, else `invalid`; local only; emits `mail://accounts-changed`) |
| `reconnect_account` | `{ id }` (Google and Microsoft accounts: sign in again, same address) | `Account`. A Gmail account that connected calendar and contacts asks for its granted Google scopes (from `account_connections`) again with mail (`include_granted_scopes=true`; on Android, through Play services), so signing in again never drops calendar, contacts, tasks or automatic-reply access. Microsoft's Graph grant is separate and left as it is. |
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
| `get_message` | `{ id, allowRemote: boolean }` | `MessageDetail` (fetches body if not cached; marks nothing read. Remote pictures load when `allowRemote`, when `Settings.remoteImages` is `allow`, or when the sender is trusted (`imagesTrusted: true`; see `trust_image_sender`). `readReceipt` as in "Read receipts"; with `readReceipts: 'always'` opening queues the receipt once) |
| `answer_read_receipt` | `{ messageId, send }` | `void` (`send: true` queues the read receipt (see "Read receipts") from the message's account; false declines it; either way `answered` becomes true and it isn't asked again; answering again does nothing. Works whatever `readReceipts` says, since the user chose. `invalid` when the message asks for none (or is one not to answer)) |
| `resolve_recipients` | `{ text }` | `RecipientCheck` (`{ resolved: Mailbox[], ambiguous: { input, candidates: Mailbox[] }[], invalid: string[] }`: Check Names for one To/Cc/Bcc field, split on `,`/`;` as when sending. An address (`a@b.co` or `Name <a@b.co>`) is resolved, its name filled from contacts (then people from mail) when missing; anything else is looked up like `suggest_recipients` (contacts first, up to 8): one match resolves it; of several, the one whose name, address or address before `@` is exactly the entry (ignoring case); otherwise `ambiguous`; none: `invalid` (as typed). Resolved addresses are listed once) |
| `contact_card_attachment` | `{ contactId }` | `DraftAttachment` (the contact as a vCard 3.0 file `<name>.vcf` in the app cache, attachable like picked files; sent as `text/vcard`; `not_found` for a deleted contact) |
| `message_as_attachment` | `{ messageId }` | `DraftAttachment` (the message as received, `<subject>.eml` (as `save_message` names it), in the app cache; sent as `message/rfc822`, 7bit when its lines allow, else base64; body downloaded if needed) |
| `open_compose_window` | `{ draft }` | `void` (computers: keeps the draft in memory under a random key and opens a window `compose-<key>` (900×760, titled with the subject or "New message") loading the app with `?compose=<key>`; same permissions and link rules as the main window; closing it never hides to the tray; the draft is held until taken (at most 32 wait). Phones: `invalid`) |
| `take_compose_draft` | `{ key }` | `Draft \| null` (the draft a compose window was opened with, once; null after that or for an unknown key) |
| `parse_search` | `{ query }` | `ParsedSearch` (`{ terms: SearchTerm[], errors: string[] }`; never rejects. Terms joined by `OR` carry `orWithNext: true` on the first of the two (absent otherwise). Terms with a bad value are left out and explained in `errors`, e.g. "after: needs a date like 2026-10-01 (or today, yesterday).") |
| `list_rules` | | `Rule[]` (in order) |
| `save_rule` | `{ rule: RuleInput }` | `Rule[]` (`id` null adds it last; otherwise changes it in place. `invalid` with a plain message for: no name, no action, a text condition with nothing to look for, a bad header/importance/size value, move/copy without a folder, a folder of another account, a rule for every account using a folder instead of a `role`, a category that doesn't exist (names are matched ignoring case and stored as the category is spelled). Up to 30 conditions, 30 exceptions and 20 actions. Never waits for the mail server: `serverStatus`/`serverNote` say what is known now, and `mail://rules-changed` follows when the server write finishes) |
| `delete_rule` | `{ id }` | `Rule[]` (`not_found` if it's gone; taken off the server too) |
| `reorder_rules` | `{ ids }` | `Rule[]` (every rule id exactly once, else `invalid`; the server script follows the new order) |
| `run_rules` | `{ ruleIds: string[] \| null, folderId }` | `RuleRunResult` (`{ checked, matched }`: the folder's newest 5,000 messages; `ruleIds` null runs every enabled rule for the folder's account, else those rules (even if off) that are for that account or every account, in their order. Works on any folder, Trash included) |
| `preview_rule` | `{ rule: RuleInput, folderId }` | `number` (how many of the folder's newest 5,000 messages the conditions and exceptions match; name, account and actions aren't needed yet; a bad condition value rejects `invalid`) |
| `list_image_senders` | | `ImageSender[]` (by name) |
| `trust_image_sender` | `{ sender, wholeDomain }` | `ImageSender[]` (`sender` an address; with `wholeDomain` its domain is stored (a bare domain is accepted too), and covers subdomains: `bank.example` covers `mail.bank.example`. Stored in lower case; `invalid` if it isn't an address/domain) |
| `untrust_image_sender` | `{ sender }` | `ImageSender[]` (as listed; nothing happens if it isn't there) |
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
| `save_draft` | `{ draft }` | `{ draftMessageId: string }` (stored in Drafts, built exactly as it would be sent (a rich-text draft keeps its HTML, marks and pictures); replaces `draft.draftMessageId` if set; pass the returned id on the next save) |
| `list_outbox` | | `OutboxEntry[]` |
| `outbox_send_now` | `{ id }` | `void` |
| `outbox_cancel` | `{ id }` | `void` |
| `outbox_edit` | `{ id }` | `Draft` (removes it from the outbox; `bodyHtml` for a rich-text message, `requestReadReceipt` as it was) |
| `sync_now` | `{ accountId: string \| null }` | `void` |
| `get_settings` / `set_settings` | `{ settings }` | `Settings`. `readReceipts` (`ask`, `always`, `never`; anything else becomes `ask`). `readingPane` (`right`, `bottom`, `off`; anything else becomes `right`) is only stored; changing `workOffline` wakes every sync engine and the outbox. On computers also `closeToTray` (closing the window hides it to the tray icon: Open, New message (emits `mail://compose`), Check for mail, Quit) and `startAtLogin` (applied to the system through tauri-plugin-autostart, launching with `--hidden` into the tray); `device: { tray, startAtLogin }` is read only (start at login is off inside a Store package). |
| `list_contacts` | `{ query }` | `Contact[]` (saved contacts matching name, email, phone or company; favourites first, then by name; `Contact = { id, accountId: string \| null, uid, name, emails, phones, organization, title, notes, favorite: boolean, categories: string[], photo: string \| null (a `data:image/…;base64,` URL), state: 'synced' \| 'changed' \| 'deleted' }`) |
| `get_contact` | `{ id }` | `Contact` |
| `save_contact` | `{ contact: { id?, accountId: string \| null, name, emails, phones, organization, title, notes, favorite?, categories?, photo? } }` | `Contact` (saved here at once; a contact of an account syncs to its server in the background; null `accountId` keeps it on this device). `favorite` and `categories` missing: as they were. `photo` missing: as it was; `null`: removed; a `data:` URL of a JPEG, PNG, GIF or WebP up to 200 KB (checked by its bytes), else `invalid`. CardDAV writes `PHOTO;ENCODING=b` (vCard 3.0), `CATEGORIES` and `X-MOCKY-FAVORITE:1`; a server's picture given by address (`PHOTO;VALUE=uri`) or over 200 KB isn't fetched or shown and stays on the card. Google and Microsoft: the picture and favourite are kept on this device only (Google's CardDAV and Graph contacts aren't sent them); categories sync with Microsoft (`categories`) and Google (`CATEGORIES`). |
| `delete_contact` | `{ id }` | `void` (gone here at once; deleted on the server at the next sync) |
| `sync_contacts` | | `AddressBookStatus[]` = `{ accountId, state: 'synced' \| 'unavailable' \| 'error', message, connectable }`. Password accounts sync with CardDAV on their IMAP host (`https://<imap host>/.well-known/carddav`, e.g. SOGo), Basic auth over HTTPS only. Connected Gmail accounts sync with Google's CardDAV (`https://www.googleapis.com/carddav/v1/principals/<email>/lists/default/`, Bearer token). Connected Microsoft accounts sync with Graph (`/me/contacts`, created/changed/deleted with POST/PATCH/DELETE; read with `/me/contacts/delta`, the delta link kept per account, a full read again if it expires). Local changes are sent first (newest edit wins), then server changes are read; fields the app doesn't show (addresses, birthdays) are kept. Also runs at start-up and every 15 minutes, and emits `mail://contacts-changed` `{ books }`. Gmail and Microsoft accounts not connected are `unavailable` with `connectable: true` (their contacts stay on this device). `suggest_recipients` lists the people in groups whose name matches, then saved contacts (favourites first), then people from mail. |
| `list_contact_groups` | | `ContactGroup[]` = `{ id, accountId: string \| null, name, memberIds: string[] }`, by name; members that were deleted are left out |
| `save_contact_group` | `{ group: { id: string \| null, accountId: string \| null, name, memberIds } }` | `ContactGroup[]` (all groups). Null `accountId`: kept on this device, may hold anyone. A group of an account may only hold that account's contacts and syncs with its address book: CardDAV as an Apple-style group card (vCard 3.0, `X-ADDRESSBOOKSERVER-KIND:group`, `X-ADDRESSBOOKSERVER-MEMBER:urn:uuid:<member UID>`; vCard 4 `KIND:group`/`MEMBER` cards from the server are read too; a server that refuses group cards leaves the group waiting here); Gmail through the People API (`contactGroups`, members matched to contacts by email address, so contacts without one can't be in a Google group). Microsoft has no groups the app can use (`contactFolders` are folders, not lists): an account group for a Microsoft account is `invalid`; use a device group. Typing a group's name in a recipient field (`resolve_recipients`) adds everyone in it (their first address). |
| `delete_contact_group` | `{ id }` | `ContactGroup[]` (the people stay; deleted on the server at the next sync) |
| `connect_calendar_and_contacts` | `{ accountId }` | `void`. Gmail and Microsoft accounts: opens the provider's sign-in in the browser (PKCE, loopback redirect, as at sign-in) asking for calendar, contacts, tasks and automatic replies. Gmail: the mail scope plus `calendar`, `carddav`, `contacts`, `tasks`, `gmail.settings.basic`, with `include_granted_scopes=true`; the new refresh token replaces the account's one in the keyring (it covers mail too). Microsoft: Graph `Calendars.ReadWrite Contacts.ReadWrite Tasks.ReadWrite MailboxSettings.ReadWrite offline_access` as a second grant (Microsoft issues tokens per resource); its refresh token is kept as `<accountId>:graph`, the mail one is untouched. The granted scopes are recorded per account (`account_connections`); then calendars, contacts and tasks sync in the background and `mail://accounts-changed` is emitted. Errors: `config` "Google hasn't enabled calendar access for Mocky Mail yet…" / "Microsoft hasn't allowed calendar access…" when the provider refuses the permissions (`invalid_scope`, `unauthorized_client`, `consent_required`), `auth` when a different account was chosen or calendar access was unticked. Password accounts: `invalid`. On Android, Gmail accounts added with the phone's own Google sign-in connect through Google Play services instead (`GoogleAuthPlugin` `token` with `interactive: true` and the extra scopes: incremental authorization, Google's consent sheet, one access token for mail and the rest; no refresh token, later tokens are asked for with the granted scopes and no screen). Microsoft works there through the browser. |
| `list_calendars` | | `CalendarInfo[]` = `{ id, accountId, name, color: string \| null, visible, writable, provider: 'caldav' \| 'google' \| 'microsoft' }`. `visible` is this device's choice (kept across syncs); `writable` from the server (`current-user-privilege-set` write/write-content/all/bind for CalDAV, `accessRole` owner/writer for Google, `canEdit` for Microsoft). |
| `set_calendar_visible` | `{ calendarId, visible }` | `CalendarInfo[]` (all calendars); emits `mail://calendar-changed`. Hidden calendars' events are left out of `list_events` (reminders still come). |
| `list_events` | `{ from, to }` (Unix seconds, at most 400 days apart) | `CalendarEvent[]`: every occurrence in the window in the calendars that are shown, all-day first then by start. `CalendarEvent = { id (the stored event; a series shares it), calendarId, accountId, color, uid, recurrenceId: number \| null (a series date's original start), title, location, notes, start, end (exclusive), allDay, repeat: 'none' \| 'daily' \| 'weekly' \| 'monthly' \| 'yearly' \| 'custom', recurring, organizer, attendees: { email, name, status }[], tentative, reminders: number[] (minutes before the start, from the event's DISPLAY/AUDIO alarms), showAs: 'busy' \| 'free' \| 'tentative' (Microsoft calendars are left alone: Exchange puts invitations in them itself, so only the answer is emailed; a connected Gmail account saves into its own calendar over Google's CalDAV)\| 'away', private, repeatRule: RepeatRule \| null, isOrganizer }`. `showAs` from `X-MICROSOFT-CDO-BUSYSTATUS` (BUSY, FREE, TENTATIVE, OOF), else `TRANSP:TRANSPARENT` (free), else `STATUS:TENTATIVE`; `private` is `CLASS:PRIVATE` or `CONFIDENTIAL`; `isOrganizer`: no `ORGANIZER`, or it is the account's address. `RepeatRule = { freq: 'daily' \| 'weekly' \| 'monthly' \| 'yearly', interval, byDay: ('MO'…'SU')[], monthDay: number \| null (1–31, -1 last), month: number \| null (1–12), setPos: number \| null (1–4, -1: "the second Tuesday", with one byDay), count: number \| null, until: number \| null (midnight UTC of the last day) }`, read from the RRULE (FREQ, INTERVAL, BYDAY incl. `2TU`, BYMONTHDAY, BYMONTH, BYSETPOS, COUNT, UNTIL, WKST); a rule with anything else (hourly, several month days, BYWEEKNO, …) gives `repeatRule: null` and `repeat: 'custom'` and is kept as it is. Repeating events are expanded on the device (RRULE, EXDATE, RDATE, changed or cancelled single dates), in the event's own time zone. |
| `get_event` | `{ id }` | `CalendarEvent` (the event itself: a series' first date and rule) |
| `save_event` | `{ event: { id?, calendarId, title, location, notes, allDay, start, end, repeat: … \| 'keep', timeZone, reminders: number[] \| null, repeatRule?, showAs?, private?, attendees?: string[], notifyAttendees?, occurrence?: number \| null, scope?: 'this' \| 'all' } }` | `CalendarEvent`. Saved on the server first (needs a connection; `If-Match` on edits), then here; a calendar that isn't `writable` is `invalid`. `repeatRule` is used instead of `repeat` when set. `showAs` writes `TRANSP` and `X-MICROSOFT-CDO-BUSYSTATUS`, `private` writes `CLASS:PRIVATE`/`PUBLIC` (missing: as they were). `attendees` (addresses, up to 100; missing: as they were) sets the `ATTENDEE`s: people already invited keep their line and answer, new ones get `PARTSTAT=NEEDS-ACTION;RSVP=TRUE`, and an `ORGANIZER` (the account) is added if there is none. `occurrence` (a `recurrenceId`) with `scope: 'this'` changes only that date: a `RECURRENCE-ID` copy is written (or rewritten) beside the series. New events are written in the device's time zone with a `VTIMEZONE`; edits keep everything the app doesn't show. Choosing another calendar moves the event (not to or from a Microsoft calendar). `notifyAttendees` (when you organise it): if the CalDAV server schedules itself (`calendar-auto-schedule` in its `DAV` header, e.g. SOGo, Google) it sends the invitations; otherwise an iMIP `METHOD:REQUEST` email (the event without your alarms) to everyone on it, and `METHOD:CANCEL` to people taken off, are queued in the Outbox. Microsoft calendars go through Graph (`POST /me/calendars/{id}/events`, `PATCH /me/events/{id}`; one date of a series: its instance from `/instances`), and Microsoft sends the invitations and updates itself whenever attendees are set, whatever `notifyAttendees` says. A Microsoft series can't repeat on the last day of the month (`invalid`). |
| `delete_event` | `{ id, occurrence: number \| null, notify?: boolean }` | `void`. With `occurrence` (a `recurrenceId`), only that date of a series is removed (an `EXDATE`); otherwise the event or whole series is deleted. With `notify` (default false) and a meeting you organise on a CalDAV server that doesn't schedule itself, a `METHOD:CANCEL` (of the series, or of that date) is emailed to the attendees through the Outbox. Microsoft: Graph `DELETE` (the instance for one date); Microsoft sends the cancellation itself for meetings you organise. |
| `get_free_busy` | `{ accountId, emails, from, to }` (at most 62 days, 50 people) | `FreeBusy[]` = `{ email, busy: { start, end, showAs }[], unknown }`. Addresses of your own accounts come from the calendars kept here (events not `free`, not declined). Others are asked through `accountId`'s server: CalDAV scheduling (the principal's `schedule-outbox-URL`, POST of a `VFREEBUSY` `METHOD:REQUEST`, RFC 6638; each recipient's `request-status` 2.x and `FREEBUSY` periods, `FBTYPE` BUSY-TENTATIVE → tentative, BUSY-UNAVAILABLE → away), Google `POST calendar/v3/freeBusy`, Microsoft `POST /me/calendar/getSchedule` (busy, tentative, oof). `unknown: true` (with no periods) when the server has no scheduling, the person's calendar can't be read, the account isn't connected, or there is no connection. |
| `sync_calendars` | | `CalendarStatus[]` = `{ accountId, state: 'synced' \| 'unavailable' \| 'error', message, connectable }`. Password accounts sync with CalDAV on their IMAP host (`https://<imap host>/.well-known/caldav`, e.g. SOGo), Basic auth over HTTPS only; calendars that hold events are listed, changed events fetched, and events deleted elsewhere removed. Connected Gmail accounts: the calendar list from `calendar/v3/users/me/calendarList`, each calendar's events over Google's CalDAV (`https://apidata.googleusercontent.com/caldav/v2/<calendar id>/events/`, Bearer token), the same way. Connected Microsoft accounts: `/me/calendars`, then each calendar's `calendarView` from 90 days back to a year on (UTC times); single events are kept as one-event iCalendar resources, a series as one resource with its RRULE (from Graph's `recurrence`), its changed dates as `RECURRENCE-ID` copies and its cancelled dates as `EXDATE`s (href = Graph id, `X-MOCKY-GRAPH-ID`); a series is only rebuilt (its master read) when one of its dates changed. Also runs at start-up and every 15 minutes (with tasks), and emits `mail://calendar-changed` `{ calendars }` (empty after a change made here). Gmail and Microsoft accounts not connected are `unavailable` with `connectable: true`. Setup problems come back as readable `error` messages ("Google hasn't enabled calendar and contacts access for Mocky Mail yet…", "Microsoft didn't give Mocky Mail access…"). |
| `list_due_reminders` | | `DueReminder[]` = `{ key, kind: 'event' \| 'task' \| 'flag', title, at, location, eventId, recurrenceId, taskId, messageId }`, soonest first: event dates whose earliest alarm has gone off and that haven't ended (all-day: local midnight to midnight; declined left out; only with setting `eventReminders`), open tasks whose `reminder` has come, flagged messages whose `flagDue` came in the last 30 days, less those dismissed or snoozed. Keys: `event:<eventId>:<start>`, `task:<taskId>:<reminder>`, `flag:<messageId>:<flagDue>`, so a moved event or a new reminder time comes due again. |
| `dismiss_reminder` | `{ key }` | `DueReminder[]` (the new list; also emitted). Kept in the database (`reminder_states`, 60 days), so it stays dismissed after a restart. |
| `snooze_reminder` | `{ key, minutes }` (1 to 10080) | `DueReminder[]`; listed (and notified) again when the snooze ends; kept in the database. |
| (reminders) | | Every 30 s the app looks for reminders that went off since its last look (kept in the database, at most a day back) and shows one system notification for each: an event alarm ("In 15 minutes · 10:00–10:30 · place"), a task reminder ("Task reminder"), a flag ("Follow up: <subject>"), a snooze that ended. Several alarms of one event each notify; one look covering several notifies once. Event alarms follow the `eventReminders` setting; tasks and flags always notify. Whenever the due list changes (a reminder came due, was dismissed or snoozed, a task changed), `mail://reminders` `{ reminders }` is emitted with the whole list. `reminders` on `save_event` replaces the event's alarms (null keeps them). Settings `eventReminders` (on) and `defaultReminderMins` (15; -1 none) control it. |
| `list_task_lists` | | `TaskList[]` = `{ id, accountId: string \| null, name, provider: 'device' \| 'caldav' \| 'google' \| 'microsoft' }`: the device's own list first (`id: 'device'`, "Tasks", always there), then accounts' lists: CalDAV calendars that say they hold `VTODO` (`supported-calendar-component-set`) and are writable, Google Tasks lists (connected Gmail accounts granted `tasks`), Microsoft To Do lists (`/me/todo/lists`). |
| `list_tasks` | `{ listId: string \| null, includeCompleted }` | `Task[]` = `{ id, listId, title, notes, due: number \| null (midnight UTC of the day, or a time), reminder: number \| null, importance: 'low' \| 'normal' \| 'high', completed, completedAt, messageId: string \| null, categories, state: 'synced' \| 'changed' \| 'deleted' }`; `listId` null: every list. Open tasks first by due date, then done ones. |
| `save_task` | `{ task: TaskInput }` (`Task` without `completedAt` and `state`; `id: null` for a new one) | `Task`. Saved here at once (title required, up to 500 characters; notes 8000; 20 categories) and sent on the next sync; `completedAt` is set when it is first completed. `messageId` (the flagged message it was made from) is kept on this device only. Moving it to another list deletes it from the old one. CalDAV: a `VTODO` (`SUMMARY`, `DESCRIPTION`, `DUE` as a date or UTC time, `PRIORITY` 1 high / 9 low, `STATUS`, `COMPLETED`, `PERCENT-COMPLETE`, `CATEGORIES`, an absolute `VALARM` for the reminder), keeping what the app doesn't show. Google Tasks: title, notes, due (a day) and status only; its reminder, importance and categories stay on this device. Microsoft To Do: title, body, `dueDateTime`, `reminderDateTime`/`isReminderOn`, importance, status, `completedDateTime`, categories. Emits `mail://tasks-changed`. |
| `delete_task` | `{ id }` | `void` (gone here at once, on the server at the next sync); emits `mail://tasks-changed` |
| `sync_tasks` | | `void`. Sends the changes made here (newest edit wins), then reads every list (CalDAV: `calendar-query` for `VTODO` hrefs and etags, then `calendar-multiget`; Google: `tasks/v1/lists/<id>/tasks?showCompleted&showHidden`; Microsoft: `/me/todo/lists/<id>/tasks`); tasks gone from the server are dropped unless changed here. Runs with the calendar every 15 minutes and soon after a change; emits `mail://tasks-changed`. |
| `answer_invitation` | `{ id (message), answer: 'accepted' \| 'tentative' \| 'declined', notify }` | `InvitationView`. For a `METHOD:REQUEST` invitation in the message: the event is saved in the account's calendar with your `PARTSTAT` (into the calendar already holding its `UID`, else the account's personal calendar, or a connected Gmail account's own calendar over Google's CalDAV; Microsoft calendars are left alone, as Exchange puts invitations in them itself, so for those only the answer is emailed); a decline removes it. Calendar writes carry `Schedule-Reply: F` so the server doesn't email the organiser as well. With `notify`, a `METHOD:REPLY` email (text note plus `text/calendar; method=REPLY`) to the organiser is queued in the Outbox to send at once. |
| `remove_cancelled_event` | `{ id }` | `InvitationView` (for a `METHOD:CANCEL`: deletes the event with that `UID` from your calendar) |
| `apply_invitation_reply` | `{ id }` | `InvitationView` (for a `METHOD:REPLY` to your invitation: the replier's `PARTSTAT` is written into your calendar copy) |
| `get_out_of_office` | `{ accountId }` | `{ accountId, available, message, active, settings: { enabled, subject, message, start: 'YYYY-MM-DD' \| null, end, days, externalMessage?, externalAudience? } }`. Password accounts: read from the account's ManageSieve server (IMAP host, port 4190, STARTTLS required, PLAIN login with the mail password); `available` false (with `message`) for servers without ManageSieve or the `vacation` extension. Connected Gmail accounts (granted `gmail.settings.basic`): Gmail's `users/me/settings/vacation` (`subject`, plain-text `message`, `start`/`end` as days where the device is; Gmail has no `days`: it is 1). Connected Microsoft accounts: Graph `mailboxSettings/automaticRepliesSetting` (`status` disabled / alwaysEnabled / scheduled with the days, `internalReplyMessage` as `message`, `externalReplyMessage` as `externalMessage` (null when the same), `externalAudience` none / contactsOnly / all as `'none' \| 'contacts' \| 'all'`; Outlook has no subject or `days`). `active`: on, and today within the days. Gmail and Microsoft accounts not connected: `available` false with "Connect this account's calendar and contacts to set its automatic replies here." |
| `set_out_of_office` | `{ accountId, settings }` | Same. Password accounts: writes the script `mocky-mail` (`vacation :days N :subject … :addresses [account email]`, inside `currentdate` tests when dated; the settings are kept as JSON in a comment). A script that was running before is kept running through `include :personal`, and becomes active again when replies are switched off. With rules kept on the server, that script is `mocky-mail-rules`, which keeps any older one running in turn (see "Rules"). Gmail: `PUT …/settings/vacation` (`startTime`/`endTime` in ms, the end day included). Microsoft: `PATCH /me/mailboxSettings` (a scheduled reply with only a first day ends a year on; messages are sent as simple HTML). `externalMessage`/`externalAudience` are used by Microsoft only and ignored elsewhere. |
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
| `mail://accounts-changed` | `{}` (refetch accounts; also after `connect_calendar_and_contacts`) |
| `mail://calendar-changed` | `{ calendars: CalendarStatus[] }` (after a sync: each account's state, with `connectable`; empty after a change made here or `set_calendar_visible`: refetch calendars and events) |
| `mail://contacts-changed` | `{ books: AddressBookStatus[] }` (after a contacts sync, with `connectable`; refetch contacts and groups) |
| `mail://reminders` | `{ reminders: DueReminder[] }` (the whole due list, whenever it changes: a reminder came due, was dismissed or snoozed, or tasks changed) |
| `mail://tasks-changed` | `{}` (tasks changed: a sync, or a change here; refetch task lists and tasks) |
| `mail://rules-changed` | `{ rules: Rule[] }` (a background write of rules to a mail server finished or gave up, and some `serverStatus`/`serverNote` changed; the full list, in order) |
