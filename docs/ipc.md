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
};

type FolderRole = "inbox" | "sent" | "drafts" | "archive" | "junk" | "trash" | "all" | "flagged" | "other";
type Folder = {
  id: string; accountId: string; name: string; path: string; role: FolderRole; unread: number; total: number;
  delimiter: string | null;  // hierarchy separator; nest folders by splitting `path` on it. A parent that
                             // isn't itself a folder (server container) is shown as a plain label.
};

type Mailbox = { name: string | null; address: string };

type MessageListItem = {
  id: string; accountId: string; folderId: string;
  from: Mailbox | null; to: Mailbox[]; subject: string; preview: string;
  date: number | null;      // unix seconds
  unread: boolean; flagged: boolean; answered: boolean; hasAttachments: boolean;
};

type AttachmentMeta = { index: number; fileName: string; mimeType: string; size: number };

type MessageDetail = MessageListItem & {
  cc: Mailbox[]; replyTo: Mailbox[];
  html: string | null;      // already sanitized, inline images embedded; render only in <iframe sandbox="allow-popups">; link popups are routed to the system browser by the backend
  text: string | null;
  blockedRemoteImages: number;
  attachments: AttachmentMeta[];
  messageIdHeader: string | null; inReplyTo: string | null; references: string[];
};

type ListQuery = {
  accountId: string | null;          // null = all accounts (unified)
  folderId: string | null;           // a specific folder, or
  role: FolderRole | null;           // a role across accounts (unified Inbox etc.)
  filter: "all" | "unread" | "flagged";
  search: string | null;             // full-text over subject, people, body
  offset: number; limit: number;     // limit <= 200
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

type Settings = { theme: "system" | "light" | "dark"; remoteImages: "block" | "allow"; notifications: boolean };
type SyncStatus = { accountId: string; state: "syncing" | "idle" | "offline" | "error"; lastSyncedAt: number | null; message: string | null };
```

## Commands

| Command | Args | Returns |
|---|---|---|
| `suggest_servers` | `{ email }` | `{ imap: ServerConfig; smtp: ServerConfig; auth: AuthKind } \| null` |
| `add_account` | `{ input: { email, displayName, password, imap, smtp } }` | `Account` (logs in to IMAP and SMTP first; nothing saved on failure) |
| `add_oauth_account` | `{ input: { email, displayName, provider: "google" \| "microsoft" } }` | `Account` (opens system browser; may reject `config` until OAuth is configured) |
| `update_account` | `{ id, patch: { displayName?, password? } }` | `Account` |
| `remove_account` | `{ id }` | `void` (deletes local mail and keyring entry, not server mail) |
| `list_accounts` | | `Account[]` |
| `list_folders` | `{ accountId }` | `Folder[]` (sorted by role, then name) |
| `list_messages` | `{ query: ListQuery }` | `MessagePage` (newest first) |
| `get_message` | `{ id, allowRemote: boolean }` | `MessageDetail` (fetches body if not cached; marks nothing read) |
| `set_flags` | `{ ids, seen?: boolean, flagged?: boolean }` | `void` (local at once, server when online) |
| `move_messages` | `{ ids, target: { role: FolderRole } \| { folderId: string } }` | `void` |
| `delete_forever` | `{ ids }` | `void` (only allowed for messages in Trash or Junk) |
| `save_attachment` | `{ messageId, index }` | `string \| null` (shows a save dialog; null if cancelled) |
| `pick_attachments` | | `DraftAttachment[]` (open-file dialog) |
| `check_draft` | `{ draft }` | `{ estimatedBytes: number; limitBytes: number }` or rejects `invalid`/`too_large` |
| `send_message` | `{ draft }` | `OutboxEntry` (validated, queued; outbox worker sends; removes the saved draft `draft.draftMessageId` if set) |
| `attachments_for_draft` | `{ messageId }` | `DraftAttachment[]` (copies of a message's attachments, usable in a Draft: forwarding, continuing a draft) |
| `save_draft` | `{ draft }` | `{ draftMessageId: string }` (stored in Drafts; replaces `draft.draftMessageId` if set; pass the returned id on the next save) |
| `list_outbox` | | `OutboxEntry[]` |
| `outbox_send_now` | `{ id }` | `void` |
| `outbox_cancel` | `{ id }` | `void` |
| `outbox_edit` | `{ id }` | `Draft` (removes it from the outbox) |
| `sync_now` | `{ accountId: string \| null }` | `void` |
| `get_settings` / `set_settings` | `{ settings }` | `Settings` |
| `open_external` | `{ url }` | `void` (http, https, mailto only) |

## Events

| Event | Payload |
|---|---|
| `mail://sync-status` | `SyncStatus` |
| `mail://changed` | `{ accountId: string; folderIds: string[] }` (refetch lists and counts) |
| `mail://outbox-changed` | `{ entries: OutboxEntry[] }` |
| `mail://new-mail` | `{ accountId: string; count: number }` (the backend also shows the desktop notification when enabled) |
| `mail://accounts-changed` | `{}` (refetch accounts) |
