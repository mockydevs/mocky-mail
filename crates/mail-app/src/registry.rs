//! Every command by name, for programs that drive the app without its
//! interface (the command line, the MCP server, the browser version's
//! server). The names and the camelCase argument names are the ones the
//! interface uses (`docs/ipc.md`); [`dispatch`] runs one with its
//! arguments as a JSON object, and [`commands`] lists them all, with their
//! arguments (and a JSON schema for each, [`CommandInfo::input_schema`]).
//!
//! The interface itself calls the Tauri wrappers in the desktop app, which
//! call the same functions.

use base64::Engine as _;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::{
    accounts, attachments, automation, calendar, compose, connections, contacts, files, folders,
    invitations, mail, mailto, message_tools, message_window, opened, organize, out_of_office,
    outbox, pst, read_receipts, reminders, rules, security, settings, signatures, tasks, updates,
};

/// One command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    /// Its name, as the interface calls it (`list_messages`).
    pub name: &'static str,
    /// What it does, in one line.
    pub description: &'static str,
    /// Whether it changes anything (mail, settings, files, the server, or
    /// what the device shows); `false` for commands that only read.
    pub writes: bool,
    /// Whether a program outside the app may run it (the command line and
    /// the MCP server, over the automation channel). `false` for commands
    /// that need the person at the computer: file pickers and save dialogs,
    /// windows, browser sign-in, opening links and files, installing
    /// updates, and what the app was started with.
    pub remote: bool,
    /// Its arguments, by their camelCase names.
    pub args: &'static [ArgInfo],
}

/// One argument of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArgInfo {
    /// The camelCase name it is sent under.
    pub name: &'static str,
    /// The Rust type it is read as (`Option<String>`, `outbox::Draft`).
    pub rust_type: &'static str,
    /// It must be sent (and not be null).
    pub required: bool,
}

impl ArgInfo {
    /// A JSON schema for the argument's value.
    #[must_use]
    pub fn schema(&self) -> Value {
        schema_for(self.rust_type)
    }
}

impl Serialize for ArgInfo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        json!({
            "name": self.name,
            "required": self.required,
            "schema": self.schema(),
        })
        .serialize(serializer)
    }
}

impl CommandInfo {
    /// The command's arguments as one JSON schema (an object with a
    /// property per argument), for MCP tools.
    #[must_use]
    pub fn input_schema(&self) -> Value {
        let properties: serde_json::Map<String, Value> = self
            .args
            .iter()
            .map(|arg| (arg.name.to_owned(), arg.schema()))
            .collect();
        let required: Vec<&str> = self
            .args
            .iter()
            .filter(|arg| arg.required)
            .map(|arg| arg.name)
            .collect();
        json!({ "type": "object", "properties": properties, "required": required })
    }
}

/// Every command, in the order of `docs/ipc.md`'s modules.
#[must_use]
pub fn commands() -> &'static [CommandInfo] {
    COMMANDS
}

/// One command, by name.
#[must_use]
pub fn command(name: &str) -> Option<&'static CommandInfo> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// Run a command by name. `args` is a JSON object with the command's
/// arguments under their camelCase names (or null when it takes none); an
/// argument left out counts as null. The result is the command's result as
/// JSON (`preview_attachment`'s bytes as a base64 string).
///
/// # Errors
/// `invalid` for an unknown command or arguments that don't fit; otherwise
/// whatever the command returns.
pub async fn dispatch(ctx: &Ctx, name: &str, args: Value) -> AppResult<Value> {
    run(ctx, name, &args).await
}

/// One argument, by its camelCase name.
fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> AppResult<T> {
    let value = match args {
        Value::Object(map) => map.get(name).cloned().unwrap_or(Value::Null),
        Value::Null => Value::Null,
        _ => return Err(AppError::invalid("The arguments must be a JSON object.")),
    };
    serde_json::from_value(value).map_err(|e| AppError::invalid(format!("Argument {name}: {e}")))
}

/// A command's result as JSON.
fn reply<T: Serialize>(result: AppResult<T>) -> AppResult<Value> {
    result.and_then(value)
}

/// A value as JSON.
fn value<T: Serialize>(value: T) -> AppResult<Value> {
    serde_json::to_value(value).map_err(|e| AppError::internal(e.to_string()))
}

/// Bytes as a base64 string.
fn bytes(result: AppResult<Vec<u8>>) -> AppResult<Value> {
    result.map(|data| Value::String(base64::engine::general_purpose::STANDARD.encode(data)))
}

/// Whether a type (as `stringify!` writes it) may be left out.
const fn optional(rust_type: &str) -> bool {
    matches!(
        rust_type.as_bytes(),
        [b'O', b'p', b't', b'i', b'o', b'n', ..]
    )
}

/// The last segment of a path (`outbox :: Draft` → `Draft`).
fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path).trim()
}

/// A JSON schema for a Rust argument type (as `stringify!` writes it).
/// Primitive types map to their JSON types; the interface's own types get
/// a short schema of their main fields, or a pointer to `docs/ipc.md`.
fn schema_for(rust_type: &str) -> Value {
    let compact: String = rust_type.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(inner) = compact
        .strip_prefix("Option<")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        return schema_for(inner);
    }
    if let Some(inner) = compact
        .strip_prefix("Vec<")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        return json!({ "type": "array", "items": schema_for(inner) });
    }
    let name = last_segment(&compact);
    match name {
        "String" | "str" | "&str" => json!({ "type": "string" }),
        "bool" => json!({ "type": "boolean" }),
        "i64" | "i32" | "u64" | "u32" | "usize" => json!({ "type": "integer" }),
        "f64" | "f32" => json!({ "type": "number" }),
        "Value" => json!({}),
        _ => interface_type_schema(name),
    }
}

/// Schemas for the interface types tools send most; the rest point to
/// `docs/ipc.md` (`apps/desktop/src/api/types.ts` has them all).
fn interface_type_schema(name: &str) -> Value {
    let role = json!({
        "type": "string",
        "enum": ["inbox", "sent", "drafts", "archive", "junk", "trash", "all", "flagged", "other"]
    });
    match name {
        "MoveTarget" => json!({
            "description": "Where messages go: a folder role in each message's account, or one folder.",
            "oneOf": [
                { "type": "object", "properties": { "role": role }, "required": ["role"] },
                { "type": "object", "properties": { "folderId": { "type": "string" } }, "required": ["folderId"] }
            ]
        }),
        "ListQuery" => json!({
            "type": "object",
            "description": "Which messages: every selector is optional and they combine; filter, offset and limit are required.",
            "properties": {
                "accountId": { "type": ["string", "null"] },
                "folderId": { "type": ["string", "null"] },
                "role": role,
                "filter": { "type": "string", "enum": ["all", "unread", "flagged", "attachments", "toMe", "pinned"] },
                "search": { "type": ["string", "null"], "description": "Search words and operators (from:, to:, subject:, has:attachment, before:, after:)." },
                "searchScope": { "type": ["string", "null"], "enum": ["folder", "subfolders", "account", "all", null] },
                "offset": { "type": "integer", "minimum": 0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
            },
            "required": ["filter", "offset", "limit"]
        }),
        "Draft" => json!({
            "type": "object",
            "description": "A message to send or save. Addresses are comma-separated text.",
            "properties": {
                "accountId": { "type": "string" },
                "to": { "type": "string" },
                "cc": { "type": "string" },
                "bcc": { "type": "string" },
                "subject": { "type": "string" },
                "bodyText": { "type": "string" },
                "bodyHtml": { "type": ["string", "null"] },
                "inReplyTo": { "type": ["string", "null"] },
                "references": { "type": "array", "items": { "type": "string" } },
                "attachments": { "type": "array", "items": { "type": "object" } },
                "replyToMessageId": { "type": ["string", "null"] },
                "importance": { "type": "string", "enum": ["low", "normal", "high"] }
            },
            "required": ["accountId", "to", "cc", "bcc", "subject", "bodyText", "references", "attachments"]
        }),
        "Answer" => json!({ "type": "string", "enum": ["accepted", "tentative", "declined"] }),
        "Focus" => json!({ "type": "string", "enum": ["focused", "other"] }),
        "SecurityKind" => json!({ "type": "string", "enum": ["smime", "openpgp"] }),
        other => json!({
            "type": "object",
            "description": format!("A {other} object, as described in docs/ipc.md.")
        }),
    }
}

/// The `let`s that read a command's arguments: none when they are read
/// whole (`[...]`, the command takes `args` itself).
macro_rules! bind_args {
    ($args:ident, ($($arg:ident: $ty:ty),*)) => {
        $(let $arg: $ty = arg($args, stringify!($arg))?;)*
    };
    ($args:ident, [$($arg:ident: $ty:ty),*]) => {};
}

/// A command's [`ArgInfo`]s.
macro_rules! arg_infos {
    ($(($($arg:ident: $ty:ty),*))? $([$($whole:ident: $whole_ty:ty),*])?) => {
        &[
            $($(ArgInfo { name: stringify!($arg), rust_type: stringify!($ty), required: !optional(stringify!($ty)) },)*)?
            $($(ArgInfo { name: stringify!($whole), rust_type: stringify!($whole_ty), required: !optional(stringify!($whole_ty)) },)*)?
        ]
    };
}

/// Whether an access level writes, and whether it may be run remotely.
macro_rules! access {
    (read) => {
        (false, true)
    };
    (write) => {
        (true, true)
    };
    (app_read) => {
        (false, false)
    };
    (app_write) => {
        (true, false)
    };
}

/// The table: each command's name, access, description, arguments and how
/// it is called. One list, so [`commands`] and [`dispatch`] can't drift.
///
/// Access is `read` (changes nothing), `write`, or `app_read`/`app_write`
/// for commands only the app's own window may run (see
/// [`CommandInfo::remote`]). Arguments in `(…)` are read by name into
/// variables of those names; in `[…]` they are documented only and the
/// command reads `args` itself.
macro_rules! commands {
    (($ctx:ident, $args:ident) $($name:literal, $access:ident, $description:literal, $params:tt => $call:expr;)*) => {
        const COMMANDS: &[CommandInfo] = &[
            $(CommandInfo {
                name: $name,
                description: $description,
                writes: access!($access).0,
                remote: access!($access).1,
                args: arg_infos!($params),
            },)*
        ];

        // The variables carry the interface's camelCase argument names.
        #[allow(non_snake_case)]
        async fn run($ctx: &Ctx, name: &str, $args: &Value) -> AppResult<Value> {
            match name {
                $($name => {
                    bind_args!($args, $params);
                    $call
                })*
                _ => Err(AppError::invalid(format!("There is no command called {name}."))),
            }
        }
    };
}

commands! {
    (ctx, args)
    "suggest_servers", read, "Server settings to pre-fill for an email address (known providers, then DNS, then a guess).", (email: String) => value(Box::pin(accounts::suggest_servers(email)).await);
    "list_accounts", read, "Every account, in sidebar order.", () => reply(accounts::list_accounts(ctx));
    "add_account", write, "Add an account with a password, after a test login to both servers.", (input: accounts::AddAccountInput) => reply(Box::pin(accounts::add_account(ctx, input)).await);
    "add_oauth_account", app_write, "Add a Google or Microsoft account by signing in in the browser.", (input: accounts::AddOAuthInput) => reply(Box::pin(accounts::add_oauth_account(ctx, input)).await);
    "update_account", write, "Rename an account, change its colour or sync window, or replace its password.", (id: String, patch: accounts::AccountPatch) => reply(Box::pin(accounts::update_account(ctx, id, patch)).await);
    "reconnect_account", app_write, "Sign in again to a Google or Microsoft account whose sign-in expired.", (id: String) => reply(Box::pin(accounts::reconnect_account(ctx, id)).await);
    "add_exchange_account", write, "Add an account on an organisation's own Exchange Server (finds the server with Autodiscover when ewsUrl is null), after a test sign-in.", (input: accounts::ExchangeAccountInput) => reply(Box::pin(accounts::add_exchange_account(ctx, input)).await);
    "remove_account", write, "Remove an account and its local mail; mail on the server stays.", (id: String) => reply(accounts::remove_account(ctx, id));
    "preview_attachment", read, "An attachment's bytes (base64 here) for an in-app preview; photos, PDFs and text only.", (messageId: String, index: usize) => bytes(Box::pin(attachments::preview_attachment(ctx, messageId, index)).await);
    "open_attachment", app_write, "Open an attachment in the app the device uses for its type.", (messageId: String, index: usize) => reply(Box::pin(attachments::open_attachment(ctx, messageId, index)).await);
    "save_attachment", app_write, "Ask where to save an attachment and write it there.", (messageId: String, index: usize) => reply(Box::pin(attachments::save_attachment(ctx, messageId, index)).await);
    "pick_attachments", app_write, "Show the open-file dialog; only files picked here may be attached.", () => reply(Box::pin(attachments::pick_attachments(ctx)).await);
    "get_automation_info", read, "How scripts and AI tools reach the app (the automation channel, its secret, the command line).", () => reply(automation::get_automation_info(ctx));
    "reset_automation_token", app_write, "Make a new secret for the command line and MCP server; the old one stops working.", () => reply(automation::reset_automation_token(ctx));
    "list_calendars", read, "Every account's calendars.", () => reply(calendar::list_calendars(ctx));
    "set_calendar_visible", write, "Show or hide a calendar (this device only).", (calendarId: String, visible: bool) => reply(calendar::set_calendar_visible(ctx, calendarId, visible));
    "list_events", read, "Every event occurrence between two times (Unix seconds).", (from: i64, to: i64) => reply(calendar::list_events(ctx, from, to));
    "get_event", read, "An event itself (a series' first date and its rule).", (id: String) => reply(calendar::get_event(ctx, id));
    "save_event", write, "Create or change an event on the server, then here.", (event: calendar::EventSave) => reply(Box::pin(calendar::save_event(ctx, event)).await);
    "delete_event", write, "Delete an event, a whole series, or one occurrence.", (id: String, occurrence: Option<i64>, notify: Option<bool>) => reply(Box::pin(calendar::delete_event(ctx, id, occurrence, notify)).await);
    "get_free_busy", read, "When people are busy between two times, for Find a Time.", (accountId: String, emails: Vec<String>, from: i64, to: i64) => reply(Box::pin(calendar::get_free_busy(ctx, accountId, emails, from, to)).await);
    "sync_calendars", read, "Sync every account's calendars now.", () => reply(Box::pin(calendar::sync_calendars(ctx)).await);
    "resolve_recipients", read, "Check Names for one recipient field.", (text: String) => reply(compose::resolve_recipients(ctx, text));
    "contact_card_attachment", read, "A contact as a vCard file ready to attach.", (contactId: String) => reply(Box::pin(compose::contact_card_attachment(ctx, contactId)).await);
    "message_as_attachment", read, "A message, whole, as an .eml file ready to attach.", (messageId: String) => reply(Box::pin(compose::message_as_attachment(ctx, messageId)).await);
    "open_compose_window", app_write, "Continue writing a draft in a window of its own (computers).", (draft: outbox::Draft) => reply(Box::pin(compose::open_compose_window(ctx, draft)).await);
    "take_compose_draft", app_write, "The draft a compose window was opened with, once.", (key: String) => value(compose::take_compose_draft(ctx, key));
    "connect_calendar_and_contacts", app_write, "Sign in again asking for calendar, contacts and tasks access, then sync them.", (accountId: String) => reply(Box::pin(connections::connect_calendar_and_contacts(ctx, accountId)).await);
    "list_contacts", read, "Saved contacts matching a query.", (query: String) => reply(contacts::list_contacts(ctx, query));
    "get_contact", read, "One contact.", (id: String) => reply(contacts::get_contact(ctx, id));
    "save_contact", write, "Create or update a contact.", (contact: contacts::ContactInput) => reply(contacts::save_contact(ctx, contact));
    "delete_contact", write, "Delete a contact (from the server on the next sync).", (id: String) => reply(contacts::delete_contact(ctx, id));
    "list_contact_groups", read, "Every contact group, by name.", () => reply(contacts::list_contact_groups(ctx));
    "save_contact_group", write, "Create or change a contact group.", (group: contacts::GroupInput) => reply(contacts::save_contact_group(ctx, group));
    "delete_contact_group", write, "Delete a contact group (its people stay).", (id: String) => reply(contacts::delete_contact_group(ctx, id));
    "sync_contacts", read, "Sync every account's address book now.", () => reply(Box::pin(contacts::sync_contacts(ctx)).await);
    "import_mail_files", app_write, "Pick .eml and .mbox files and upload their messages into a folder.", (folderId: String) => reply(Box::pin(files::import_mail_files(ctx, folderId)).await);
    "import_message_file", app_write, "Add an opened .eml file to a folder.", (path: String, folderId: String) => reply(Box::pin(files::import_message_file(ctx, path, folderId)).await);
    "export_mail", app_write, "Save a folder as one .mbox file, or messages as .eml files in a .zip.", (target: files::ExportTarget, format: String) => reply(Box::pin(files::export_mail(ctx, target, format)).await);
    "import_calendar_file", app_write, "Pick an .ics file and add its events to a calendar.", (calendarId: String) => reply(Box::pin(files::import_calendar_file(ctx, calendarId)).await);
    "export_calendar", app_write, "Save a calendar as one .ics file.", (calendarId: String) => reply(Box::pin(files::export_calendar(ctx, calendarId)).await);
    "import_contacts_file", app_write, "Pick a .vcf file and add its contacts.", (accountId: Option<String>) => reply(Box::pin(files::import_contacts_file(ctx, accountId)).await);
    "export_contacts", app_write, "Save contacts as one .vcf file.", (accountId: Option<String>) => reply(Box::pin(files::export_contacts(ctx, accountId)).await);
    "import_opened_file", app_write, "Add an opened .ics or .vcf file's items.", (path: String, target: files::OpenedTarget) => reply(Box::pin(files::import_opened_file(ctx, path, target)).await);
    "read_message_file", app_read, "An opened .eml file, read for showing.", (path: String) => reply(files::read_message_file(ctx, path));
    "create_folder", write, "Create a folder, at the top or inside another.", (accountId: String, name: String, parentId: Option<String>) => reply(Box::pin(folders::create_folder(ctx, accountId, name, parentId)).await);
    "rename_folder", write, "Rename one of the user's own folders.", (folderId: String, name: String) => reply(Box::pin(folders::rename_folder(ctx, folderId, name)).await);
    "delete_folder", write, "Delete one of the user's own folders and the mail in it.", (folderId: String) => reply(Box::pin(folders::delete_folder(ctx, folderId)).await);
    "answer_invitation", write, "Accept, tentatively accept or decline a meeting invitation.", (id: String, answer: mail_core::ical::Answer, notify: bool) => reply(Box::pin(invitations::answer_invitation(ctx, id, answer, notify)).await);
    "remove_cancelled_event", write, "Take a cancelled event out of the calendar.", (id: String) => reply(Box::pin(invitations::remove_cancelled_event(ctx, id)).await);
    "apply_invitation_reply", write, "Record someone's answer to an invitation you sent.", (id: String) => reply(Box::pin(invitations::apply_invitation_reply(ctx, id)).await);
    "list_folders", read, "An account's folders.", (accountId: String) => reply(mail::list_folders(ctx, &accountId));
    "list_messages", read, "A page of messages (or conversations) matching a query.", (query: mail_store::ListQuery, threads: Option<bool>) => reply(mail::list_messages(ctx, query, threads));
    "set_focus_rule", write, "Always put mail from a sender (or domain) in Focused or Other.", (sender: String, focus: Option<mail_core::focus::Focus>, wholeDomain: bool) => reply(mail::set_focus_rule(ctx, sender, focus, wholeDomain));
    "preview_body", read, "The HTML recipients will see for a typed message body.", (body: String) => reply(mail::preview_body(body));
    "get_thread", read, "Every message in a message's conversation, oldest first.", (id: String) => reply(mail::get_thread(ctx, id));
    "get_message", read, "A message with its cleaned body (downloaded if needed).", (id: String, allowRemote: bool) => reply(Box::pin(mail::get_message(ctx, id, allowRemote)).await);
    "set_flags", write, "Set read, flagged, pinned, importance or follow-up date on messages.", [ids: Vec<String>, seen: Option<bool>, flagged: Option<bool>, pinned: Option<bool>, importance: Option<String>, flagDue: Option<i64>] => reply(mail::set_flags(ctx, args));
    "suggest_recipients", read, "People to offer while typing an address.", (query: String) => reply(mail::suggest_recipients(ctx, query));
    "move_messages", write, "Move messages to Archive, Trash, Junk or any folder.", (ids: Vec<String>, target: mail::MoveTarget) => reply(mail::move_messages(ctx, ids, target));
    "delete_forever", write, "Delete messages for good (only from Trash or Junk).", (ids: Vec<String>) => reply(mail::delete_forever(ctx, ids));
    "sync_now", read, "Check for mail now (one account, or all) and send what waits.", (accountId: Option<String>) => {
        mail::sync_now(ctx, accountId);
        value(())
    };
    "open_external", app_write, "Open a web or mail link in the user's own apps.", (url: String) => reply(mail::open_external(ctx, url));
    "take_mailto", app_write, "The mailto: link the app was started with, once.", () => value(mailto::take_mailto());
    "parse_mailto", read, "Read a mailto: link.", (url: String) => value(mailto::parse_mailto(url));
    "save_message", app_write, "Save a whole message as an .eml file.", (messageId: String) => reply(Box::pin(message_tools::save_message(ctx, messageId)).await);
    "message_source", read, "A message as received (headers and body), as text.", (messageId: String) => reply(Box::pin(message_tools::message_source(ctx, messageId)).await);
    "print_html", app_write, "Print a page with the system's print screen (Android).", (html: String, title: String) => reply(Box::pin(message_tools::print_html(ctx, html, title)).await);
    "open_message_window", app_write, "Show a message in a window of its own (computers).", (id: String) => reply(Box::pin(message_window::open_message_window(ctx, id)).await);
    "take_opened_files", app_write, "The files the app was asked to open, once each.", () => value(opened::take_opened_files());
    "copy_messages", write, "Copy messages to another folder.", (ids: Vec<String>, target: mail::MoveTarget) => reply(organize::copy_messages(ctx, ids, target));
    "mark_junk", write, "Move messages to Junk, or out of it.", (ids: Vec<String>, junk: bool) => reply(organize::mark_junk(ctx, ids, junk));
    "list_categories", read, "Every category.", () => reply(organize::list_categories(ctx));
    "save_category", write, "Add a category, change its colour, or rename it.", (category: mail_store::Category, previousName: Option<String>) => reply(organize::save_category(ctx, category, previousName));
    "delete_category", write, "Delete a category, from every message too.", (name: String) => reply(organize::delete_category(ctx, name));
    "set_categories", write, "Add and remove categories on messages.", (ids: Vec<String>, add: Vec<String>, remove: Vec<String>) => reply(organize::set_categories(ctx, ids, add, remove));
    "set_folder_favorite", write, "List a folder under Favourites (this device only).", (folderId: String, favorite: bool) => reply(organize::set_folder_favorite(ctx, folderId, favorite));
    "reorder_accounts", write, "Put the accounts in a new order.", (ids: Vec<String>) => reply(organize::reorder_accounts(ctx, ids));
    "check_draft", read, "Check recipients and size before sending.", (draft: outbox::Draft) => reply(Box::pin(outbox::check_draft(ctx, draft)).await);
    "send_message", write, "Queue a message for sending (Undo Send applies).", (draft: outbox::Draft) => reply(Box::pin(outbox::send_message(ctx, draft)).await);
    "save_draft", write, "Save a draft in the account's Drafts folder.", (draft: outbox::Draft) => reply(Box::pin(outbox::save_draft(ctx, draft)).await);
    "list_outbox", read, "Messages waiting to be sent, and those that failed.", () => reply(outbox::list_outbox(ctx));
    "outbox_send_now", write, "Send a queued message now, or retry a failed one.", (id: String) => reply(outbox::outbox_send_now(ctx, id));
    "outbox_cancel", write, "Cancel a queued message.", (id: String) => reply(outbox::outbox_cancel(ctx, id));
    "outbox_edit", write, "Take a queued or failed message back into compose.", (id: String) => reply(Box::pin(outbox::outbox_edit(ctx, id)).await);
    "attachments_for_draft", read, "A message's attachments, ready to attach to a draft.", (messageId: String) => reply(Box::pin(outbox::attachments_for_draft(ctx, messageId)).await);
    "get_out_of_office", read, "An account's automatic replies.", (accountId: String) => reply(Box::pin(out_of_office::get_out_of_office(ctx, accountId)).await);
    "set_out_of_office", write, "Save automatic replies and switch them on or off.", (accountId: String, settings: mail_core::vacation::Vacation) => reply(Box::pin(out_of_office::set_out_of_office(ctx, accountId, settings)).await);
    "import_pst", app_write, "Pick an Outlook .pst file and bring in its mail, contacts, calendar and tasks.", (accountId: String) => reply(Box::pin(pst::import_pst(ctx, accountId)).await);
    "answer_read_receipt", write, "Send or decline the read receipt a message asks for.", (messageId: String, send: bool) => reply(Box::pin(read_receipts::answer_read_receipt(ctx, messageId, send)).await);
    "list_due_reminders", read, "Reminders due now, soonest first.", () => reply(reminders::list_due_reminders(ctx));
    "dismiss_reminder", write, "Dismiss a reminder.", (key: String) => reply(reminders::dismiss_reminder(ctx, key));
    "snooze_reminder", write, "Show a reminder again in some minutes.", (key: String, minutes: u32) => reply(reminders::snooze_reminder(ctx, key, minutes));
    "parse_search", read, "What a search box query means.", (query: String) => value(rules::parse_search(query));
    "list_rules", read, "Every mail rule, in order.", () => reply(rules::list_rules(ctx));
    "save_rule", write, "Add or change a mail rule.", (rule: mail_core::rules::RuleInput) => reply(rules::save_rule(ctx, rule));
    "delete_rule", write, "Delete a mail rule.", (id: String) => reply(rules::delete_rule(ctx, id));
    "reorder_rules", write, "Put the rules in a new order.", (ids: Vec<String>) => reply(rules::reorder_rules(ctx, ids));
    "run_rules", write, "Run rules now on a folder's newest messages.", (ruleIds: Option<Vec<String>>, folderId: String) => reply(rules::run_rules(ctx, ruleIds, folderId));
    "preview_rule", read, "How many of a folder's newest messages a rule would match.", (rule: mail_core::rules::RuleInput, folderId: String) => reply(rules::preview_rule(ctx, rule, folderId));
    "list_image_senders", read, "Senders whose pictures always load.", () => reply(rules::list_image_senders(ctx));
    "trust_image_sender", write, "Always load pictures from an address or domain.", (sender: String, wholeDomain: bool) => reply(rules::trust_image_sender(ctx, sender, wholeDomain));
    "untrust_image_sender", write, "Stop always loading pictures from an address or domain.", (sender: String) => reply(rules::untrust_image_sender(ctx, sender));
    "list_keys", read, "Every S/MIME certificate and OpenPGP key known.", () => reply(security::list_keys(ctx));
    "import_key", app_write, "Pick a key or certificate file and add what is in it.", (password: Option<String>) => reply(Box::pin(security::import_key(ctx, password)).await);
    "export_public_key", app_write, "Save a certificate or public key to share.", (id: String) => reply(Box::pin(security::export_public_key(ctx, id)).await);
    "remove_key", write, "Remove a key or certificate.", (id: String) => reply(security::remove_key(ctx, id));
    "set_key_trusted", write, "Say whether a key really is its owner's.", (id: String, trusted: bool) => reply(security::set_key_trusted(ctx, id, trusted));
    "set_account_key", write, "Choose the key an account signs and decrypts with.", (accountId: String, kind: mail_sync::security::SecurityKind, keyId: Option<String>) => reply(security::set_account_key(ctx, accountId, kind, keyId));
    "get_account_keys", read, "An account's chosen keys.", (accountId: String) => reply(security::get_account_keys(ctx, accountId));
    "set_preferred_kind", write, "Choose S/MIME or OpenPGP for an account.", (accountId: String, kind: mail_sync::security::SecurityKind) => reply(security::set_preferred_kind(ctx, accountId, kind));
    "check_encryption", read, "Which recipients' keys are known, for encrypting.", (accountId: String, addresses: Vec<String>, kind: mail_sync::security::SecurityKind) => reply(Box::pin(security::check_encryption(ctx, accountId, addresses, kind)).await);
    "get_settings", read, "The app settings.", () => reply(settings::get_settings(ctx));
    "set_settings", write, "Save the app settings.", (settings: settings::Settings) => reply(settings::set_settings(ctx, settings));
    "get_signature", read, "The signature a new message (or a reply) starts with.", (accountId: String, reply: Option<bool>) => self::reply(signatures::get_signature(ctx, accountId, reply));
    "list_signatures", read, "An account's signatures and its defaults.", (accountId: String) => reply(signatures::list_signatures(ctx, accountId));
    "save_signature", write, "Create or change a signature.", (accountId: String, id: Option<String>, name: String, html: String) => reply(signatures::save_signature(ctx, accountId, id, name, html));
    "delete_signature", write, "Delete a signature.", (accountId: String, id: String) => reply(signatures::delete_signature(ctx, accountId, id));
    "set_signature_defaults", write, "Choose the signatures new messages and replies start with.", (accountId: String, defaultNew: Option<String>, defaultReply: Option<String>) => reply(signatures::set_signature_defaults(ctx, accountId, defaultNew, defaultReply));
    "set_signature_image", app_write, "Pick a picture for a signature.", (accountId: String, id: String) => reply(Box::pin(signatures::set_signature_image(ctx, accountId, id)).await);
    "set_signature_image_width", write, "Set how wide a signature's picture shows.", (accountId: String, id: String, width: u32) => reply(signatures::set_signature_image_width(ctx, accountId, id, width));
    "remove_signature_image", write, "Remove a signature's picture.", (accountId: String, id: String) => reply(signatures::remove_signature_image(ctx, accountId, id));
    "list_task_lists", read, "The device's task list, then each account's.", () => reply(tasks::list_task_lists(ctx));
    "list_tasks", read, "The tasks in one list, or every list.", (listId: Option<String>, includeCompleted: bool) => reply(tasks::list_tasks(ctx, listId, includeCompleted));
    "save_task", write, "Create or change a task.", (task: tasks::TaskInput) => reply(tasks::save_task(ctx, task));
    "delete_task", write, "Delete a task.", (id: String) => reply(tasks::delete_task(ctx, id));
    "sync_tasks", read, "Sync every account's task lists now.", () => reply(Box::pin(tasks::sync_tasks(ctx)).await);
    "check_for_update", read, "The newer version this install can move to, if any.", () => reply(Box::pin(updates::check_for_update(ctx)).await);
    "install_update", app_write, "Download, check and run the newest installer.", () => reply(Box::pin(updates::install_update(ctx)).await);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// The command names the interface calls (`call('name', ...)` in
    /// `apps/desktop/src/api/tauriMailApi.ts`).
    fn interface_calls() -> Vec<String> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/desktop/src/api/tauriMailApi.ts"
        );
        let source = std::fs::read_to_string(path).unwrap_or_default();
        let mut names = Vec::new();
        let mut rest = source.as_str();
        while let Some(at) = rest.find("call") {
            rest = rest.get(at.saturating_add(4)..).unwrap_or_default();
            // `call('name'` or `call<Type>('name'`, perhaps over lines.
            let after_type = if rest.starts_with('<') {
                rest.find('>')
                    .and_then(|end| rest.get(end.saturating_add(1)..))
            } else {
                Some(rest)
            };
            let Some(open) = after_type.and_then(|s| s.strip_prefix('(')) else {
                continue;
            };
            let open = open.trim_start();
            let Some(quote) = open
                .chars()
                .next()
                .filter(|c| matches!(c, '\'' | '"' | '`'))
            else {
                continue;
            };
            let quoted = open.get(1..).unwrap_or_default();
            if let Some(end) = quoted.find(quote) {
                let name = quoted.get(..end).unwrap_or_default();
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    names.push(name.to_owned());
                }
            }
        }
        names
    }

    #[test]
    fn every_command_the_interface_calls_is_registered() {
        let called = interface_calls();
        assert!(called.len() > 100, "found only {} calls", called.len());
        let registered: HashSet<&str> = commands().iter().map(|c| c.name).collect();
        for name in &called {
            assert!(
                registered.contains(name.as_str()),
                "{name} isn't in the registry"
            );
        }
    }

    #[test]
    fn names_are_unique_and_described() {
        let mut seen = HashSet::new();
        for command in commands() {
            assert!(seen.insert(command.name), "{} twice", command.name);
            assert!(!command.description.trim().is_empty(), "{}", command.name);
        }
    }

    #[test]
    fn reads_and_writes_are_marked() {
        let writes = |name: &str| commands().iter().find(|c| c.name == name).map(|c| c.writes);
        assert_eq!(writes("list_messages"), Some(false));
        assert_eq!(writes("get_message"), Some(false));
        assert_eq!(writes("send_message"), Some(true));
        assert_eq!(writes("delete_forever"), Some(true));
    }

    fn context() -> Ctx {
        let state = crate::AppState::new(
            mail_store::Store::open_in_memory().unwrap(),
            std::sync::Arc::new(mail_sync::secrets::MemoryStore::default()),
            std::env::temp_dir(),
        );
        Ctx::new(
            std::sync::Arc::new(state),
            std::sync::Arc::new(crate::host::NoHost),
            tokio::runtime::Handle::current(),
        )
    }

    #[tokio::test]
    async fn runs_commands_by_name() {
        let ctx = context();
        let accounts = dispatch(&ctx, "list_accounts", Value::Null).await.unwrap();
        assert_eq!(accounts, serde_json::json!([]));
        let parsed = dispatch(
            &ctx,
            "parse_mailto",
            serde_json::json!({ "url": "mailto:a@example.com?subject=Hi" }),
        )
        .await
        .unwrap();
        assert_eq!(parsed["subject"], "Hi");
        let missing = dispatch(&ctx, "get_contact", serde_json::json!({})).await;
        assert_eq!(missing.map_err(|e| e.kind), Err("invalid"));
        let unknown = dispatch(&ctx, "launch_rockets", Value::Null).await;
        assert_eq!(unknown.map_err(|e| e.kind), Err("invalid"));
    }

    #[tokio::test]
    async fn platform_commands_say_they_are_not_available() {
        let ctx = context();
        let picked = dispatch(&ctx, "pick_attachments", Value::Null).await;
        assert_eq!(picked.map_err(|e| e.kind), Err("invalid"));
        let window = dispatch(
            &ctx,
            "open_message_window",
            serde_json::json!({ "id": "x" }),
        )
        .await;
        assert_eq!(window.map_err(|e| e.kind), Err("invalid"));
    }

    #[test]
    fn arguments_are_listed_with_schemas() {
        let get = |name: &str| command(name).unwrap();
        let message = get("get_message");
        assert_eq!(
            message
                .args
                .iter()
                .map(|a| (a.name, a.required))
                .collect::<Vec<_>>(),
            vec![("id", true), ("allowRemote", true)]
        );
        assert_eq!(
            message.input_schema(),
            serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "string" }, "allowRemote": { "type": "boolean" } },
                "required": ["id", "allowRemote"]
            })
        );
        let delete = get("delete_event");
        assert_eq!(delete.input_schema()["required"], serde_json::json!(["id"]));
        assert_eq!(
            delete.input_schema()["properties"]["occurrence"]["type"],
            "integer"
        );
        // Read whole, but still documented.
        let flags = get("set_flags").input_schema();
        assert_eq!(flags["properties"]["ids"]["type"], "array");
        assert_eq!(flags["required"], serde_json::json!(["ids"]));
        let list = get("list_messages").input_schema();
        assert_eq!(list["required"], serde_json::json!(["query"]));
        assert_eq!(
            list["properties"]["query"]["required"],
            serde_json::json!(["filter", "offset", "limit"])
        );
        assert!(get("move_messages").input_schema()["properties"]["target"]["oneOf"].is_array());
        // Every command has an object schema whose required names are its own.
        for command in commands() {
            let schema = command.input_schema();
            assert_eq!(schema["type"], "object", "{}", command.name);
            for name in schema["required"].as_array().unwrap() {
                assert!(
                    schema["properties"].get(name.as_str().unwrap()).is_some(),
                    "{}",
                    command.name
                );
            }
        }
    }

    #[test]
    fn only_the_apps_own_window_runs_dialogs_and_sign_in() {
        let remote = |name: &str| command(name).map(|c| c.remote);
        for local in [
            "pick_attachments",
            "save_attachment",
            "open_external",
            "add_oauth_account",
            "import_pst",
            "reset_automation_token",
            "install_update",
            "open_message_window",
        ] {
            assert_eq!(remote(local), Some(false), "{local}");
        }
        for allowed in [
            "list_messages",
            "send_message",
            "add_exchange_account",
            "get_automation_info",
            "set_settings",
        ] {
            assert_eq!(remote(allowed), Some(true), "{allowed}");
        }
    }

    #[test]
    fn arguments_are_read_by_camel_case_name() {
        let args = serde_json::json!({ "messageId": "m1", "index": 2 });
        assert_eq!(
            arg::<String>(&args, "messageId").ok().as_deref(),
            Some("m1")
        );
        assert_eq!(arg::<usize>(&args, "index").ok(), Some(2));
        assert_eq!(arg::<Option<String>>(&args, "accountId").ok(), Some(None));
        assert_eq!(
            arg::<String>(&args, "accountId").map_err(|e| e.kind),
            Err("invalid")
        );
        assert_eq!(
            arg::<String>(&Value::from(3), "id").map_err(|e| e.kind),
            Err("invalid")
        );
    }
}
