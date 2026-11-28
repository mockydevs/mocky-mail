//! Every command by name, for programs that drive the app without its
//! interface (the command line, the MCP server, the browser version's
//! server). The names and the camelCase argument names are the ones the
//! interface uses (`docs/ipc.md`); [`dispatch`] runs one with its
//! arguments as a JSON object, and [`commands`] lists them all.
//!
//! The interface itself calls the Tauri wrappers in the desktop app, which
//! call the same functions.

use base64::Engine as _;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::{
    accounts, attachments, calendar, compose, connections, contacts, files, folders, invitations,
    mail, mailto, message_tools, message_window, opened, organize, out_of_office, outbox, pst,
    read_receipts, reminders, rules, security, settings, signatures, tasks, updates,
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
}

/// Every command, in the order of `docs/ipc.md`'s modules.
#[must_use]
pub fn commands() -> &'static [CommandInfo] {
    COMMANDS
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

/// The table: each command's name, whether it writes, its description, and
/// how it is called. One list, so [`commands`] and [`dispatch`] can't drift.
macro_rules! commands {
    (($ctx:ident, $args:ident) $($name:literal, $writes:literal, $description:literal => $call:expr;)*) => {
        const COMMANDS: &[CommandInfo] = &[
            $(CommandInfo { name: $name, description: $description, writes: $writes },)*
        ];

        async fn run($ctx: &Ctx, name: &str, $args: &Value) -> AppResult<Value> {
            match name {
                $($name => $call,)*
                _ => Err(AppError::invalid(format!("There is no command called {name}."))),
            }
        }
    };
}

commands! {
    (ctx, args)
    "suggest_servers", false, "Server settings to pre-fill for an email address (known providers, then DNS, then a guess)." => value(Box::pin(accounts::suggest_servers(arg(args, "email")?)).await);
    "list_accounts", false, "Every account, in sidebar order." => reply(accounts::list_accounts(ctx));
    "add_account", true, "Add an account with a password, after a test login to both servers." => reply(Box::pin(accounts::add_account(ctx, arg(args, "input")?)).await);
    "add_oauth_account", true, "Add a Google or Microsoft account by signing in in the browser." => reply(Box::pin(accounts::add_oauth_account(ctx, arg(args, "input")?)).await);
    "update_account", true, "Rename an account, change its colour or sync window, or replace its password." => reply(Box::pin(accounts::update_account(ctx, arg(args, "id")?, arg(args, "patch")?)).await);
    "reconnect_account", true, "Sign in again to a Google or Microsoft account whose sign-in expired." => reply(Box::pin(accounts::reconnect_account(ctx, arg(args, "id")?)).await);
    "add_exchange_account", true, "Add an account on an organisation's own Exchange Server (finds the server with Autodiscover when ewsUrl is null), after a test sign-in." => reply(Box::pin(accounts::add_exchange_account(ctx, arg(args, "input")?)).await);
    "remove_account", true, "Remove an account and its local mail; mail on the server stays." => reply(accounts::remove_account(ctx, arg(args, "id")?));
    "preview_attachment", false, "An attachment's bytes (base64 here) for an in-app preview; photos, PDFs and text only." => bytes(Box::pin(attachments::preview_attachment(ctx, arg(args, "messageId")?, arg(args, "index")?)).await);
    "open_attachment", true, "Open an attachment in the app the device uses for its type." => reply(Box::pin(attachments::open_attachment(ctx, arg(args, "messageId")?, arg(args, "index")?)).await);
    "save_attachment", true, "Ask where to save an attachment and write it there." => reply(Box::pin(attachments::save_attachment(ctx, arg(args, "messageId")?, arg(args, "index")?)).await);
    "pick_attachments", true, "Show the open-file dialog; only files picked here may be attached." => reply(Box::pin(attachments::pick_attachments(ctx)).await);
    "list_calendars", false, "Every account's calendars." => reply(calendar::list_calendars(ctx));
    "set_calendar_visible", true, "Show or hide a calendar (this device only)." => reply(calendar::set_calendar_visible(ctx, arg(args, "calendarId")?, arg(args, "visible")?));
    "list_events", false, "Every event occurrence between two times (Unix seconds)." => reply(calendar::list_events(ctx, arg(args, "from")?, arg(args, "to")?));
    "get_event", false, "An event itself (a series' first date and its rule)." => reply(calendar::get_event(ctx, arg(args, "id")?));
    "save_event", true, "Create or change an event on the server, then here." => reply(Box::pin(calendar::save_event(ctx, arg(args, "event")?)).await);
    "delete_event", true, "Delete an event, a whole series, or one occurrence." => reply(Box::pin(calendar::delete_event(ctx, arg(args, "id")?, arg(args, "occurrence")?, arg(args, "notify")?)).await);
    "get_free_busy", false, "When people are busy between two times, for Find a Time." => reply(Box::pin(calendar::get_free_busy(ctx, arg(args, "accountId")?, arg(args, "emails")?, arg(args, "from")?, arg(args, "to")?)).await);
    "sync_calendars", false, "Sync every account's calendars now." => reply(Box::pin(calendar::sync_calendars(ctx)).await);
    "resolve_recipients", false, "Check Names for one recipient field." => reply(compose::resolve_recipients(ctx, arg(args, "text")?));
    "contact_card_attachment", false, "A contact as a vCard file ready to attach." => reply(Box::pin(compose::contact_card_attachment(ctx, arg(args, "contactId")?)).await);
    "message_as_attachment", false, "A message, whole, as an .eml file ready to attach." => reply(Box::pin(compose::message_as_attachment(ctx, arg(args, "messageId")?)).await);
    "open_compose_window", true, "Continue writing a draft in a window of its own (computers)." => reply(Box::pin(compose::open_compose_window(ctx, arg(args, "draft")?)).await);
    "take_compose_draft", true, "The draft a compose window was opened with, once." => value(compose::take_compose_draft(ctx, arg(args, "key")?));
    "connect_calendar_and_contacts", true, "Sign in again asking for calendar, contacts and tasks access, then sync them." => reply(Box::pin(connections::connect_calendar_and_contacts(ctx, arg(args, "accountId")?)).await);
    "list_contacts", false, "Saved contacts matching a query." => reply(contacts::list_contacts(ctx, arg(args, "query")?));
    "get_contact", false, "One contact." => reply(contacts::get_contact(ctx, arg(args, "id")?));
    "save_contact", true, "Create or update a contact." => reply(contacts::save_contact(ctx, arg(args, "contact")?));
    "delete_contact", true, "Delete a contact (from the server on the next sync)." => reply(contacts::delete_contact(ctx, arg(args, "id")?));
    "list_contact_groups", false, "Every contact group, by name." => reply(contacts::list_contact_groups(ctx));
    "save_contact_group", true, "Create or change a contact group." => reply(contacts::save_contact_group(ctx, arg(args, "group")?));
    "delete_contact_group", true, "Delete a contact group (its people stay)." => reply(contacts::delete_contact_group(ctx, arg(args, "id")?));
    "sync_contacts", false, "Sync every account's address book now." => reply(Box::pin(contacts::sync_contacts(ctx)).await);
    "import_mail_files", true, "Pick .eml and .mbox files and upload their messages into a folder." => reply(Box::pin(files::import_mail_files(ctx, arg(args, "folderId")?)).await);
    "import_message_file", true, "Add an opened .eml file to a folder." => reply(Box::pin(files::import_message_file(ctx, arg(args, "path")?, arg(args, "folderId")?)).await);
    "export_mail", true, "Save a folder as one .mbox file, or messages as .eml files in a .zip." => reply(Box::pin(files::export_mail(ctx, arg(args, "target")?, arg(args, "format")?)).await);
    "import_calendar_file", true, "Pick an .ics file and add its events to a calendar." => reply(Box::pin(files::import_calendar_file(ctx, arg(args, "calendarId")?)).await);
    "export_calendar", true, "Save a calendar as one .ics file." => reply(Box::pin(files::export_calendar(ctx, arg(args, "calendarId")?)).await);
    "import_contacts_file", true, "Pick a .vcf file and add its contacts." => reply(Box::pin(files::import_contacts_file(ctx, arg(args, "accountId")?)).await);
    "export_contacts", true, "Save contacts as one .vcf file." => reply(Box::pin(files::export_contacts(ctx, arg(args, "accountId")?)).await);
    "import_opened_file", true, "Add an opened .ics or .vcf file's items." => reply(Box::pin(files::import_opened_file(ctx, arg(args, "path")?, arg(args, "target")?)).await);
    "read_message_file", false, "An opened .eml file, read for showing." => reply(files::read_message_file(ctx, arg(args, "path")?));
    "create_folder", true, "Create a folder, at the top or inside another." => reply(Box::pin(folders::create_folder(ctx, arg(args, "accountId")?, arg(args, "name")?, arg(args, "parentId")?)).await);
    "rename_folder", true, "Rename one of the user's own folders." => reply(Box::pin(folders::rename_folder(ctx, arg(args, "folderId")?, arg(args, "name")?)).await);
    "delete_folder", true, "Delete one of the user's own folders and the mail in it." => reply(Box::pin(folders::delete_folder(ctx, arg(args, "folderId")?)).await);
    "answer_invitation", true, "Accept, tentatively accept or decline a meeting invitation." => reply(Box::pin(invitations::answer_invitation(ctx, arg(args, "id")?, arg(args, "answer")?, arg(args, "notify")?)).await);
    "remove_cancelled_event", true, "Take a cancelled event out of the calendar." => reply(Box::pin(invitations::remove_cancelled_event(ctx, arg(args, "id")?)).await);
    "apply_invitation_reply", true, "Record someone's answer to an invitation you sent." => reply(Box::pin(invitations::apply_invitation_reply(ctx, arg(args, "id")?)).await);
    "list_folders", false, "An account's folders." => reply(mail::list_folders(ctx, &arg::<String>(args, "accountId")?));
    "list_messages", false, "A page of messages (or conversations) matching a query." => reply(mail::list_messages(ctx, arg(args, "query")?, arg(args, "threads")?));
    "set_focus_rule", true, "Always put mail from a sender (or domain) in Focused or Other." => reply(mail::set_focus_rule(ctx, arg(args, "sender")?, arg(args, "focus")?, arg(args, "wholeDomain")?));
    "preview_body", false, "The HTML recipients will see for a typed message body." => reply(mail::preview_body(arg(args, "body")?));
    "get_thread", false, "Every message in a message's conversation, oldest first." => reply(mail::get_thread(ctx, arg(args, "id")?));
    "get_message", false, "A message with its cleaned body (downloaded if needed)." => reply(Box::pin(mail::get_message(ctx, arg(args, "id")?, arg(args, "allowRemote")?)).await);
    "set_flags", true, "Set read, flagged, pinned, importance or follow-up date on messages." => reply(mail::set_flags(ctx, args));
    "suggest_recipients", false, "People to offer while typing an address." => reply(mail::suggest_recipients(ctx, arg(args, "query")?));
    "move_messages", true, "Move messages to Archive, Trash, Junk or any folder." => reply(mail::move_messages(ctx, arg(args, "ids")?, arg(args, "target")?));
    "delete_forever", true, "Delete messages for good (only from Trash or Junk)." => reply(mail::delete_forever(ctx, arg(args, "ids")?));
    "sync_now", false, "Check for mail now (one account, or all) and send what waits." => {
        mail::sync_now(ctx, arg(args, "accountId")?);
        value(())
    };
    "open_external", true, "Open a web or mail link in the user's own apps." => reply(mail::open_external(ctx, arg(args, "url")?));
    "take_mailto", true, "The mailto: link the app was started with, once." => value(mailto::take_mailto());
    "parse_mailto", false, "Read a mailto: link." => value(mailto::parse_mailto(arg(args, "url")?));
    "save_message", true, "Save a whole message as an .eml file." => reply(Box::pin(message_tools::save_message(ctx, arg(args, "messageId")?)).await);
    "message_source", false, "A message as received (headers and body), as text." => reply(Box::pin(message_tools::message_source(ctx, arg(args, "messageId")?)).await);
    "print_html", true, "Print a page with the system's print screen (Android)." => reply(Box::pin(message_tools::print_html(ctx, arg(args, "html")?, arg(args, "title")?)).await);
    "open_message_window", true, "Show a message in a window of its own (computers)." => reply(Box::pin(message_window::open_message_window(ctx, arg(args, "id")?)).await);
    "take_opened_files", true, "The files the app was asked to open, once each." => value(opened::take_opened_files());
    "copy_messages", true, "Copy messages to another folder." => reply(organize::copy_messages(ctx, arg(args, "ids")?, arg(args, "target")?));
    "mark_junk", true, "Move messages to Junk, or out of it." => reply(organize::mark_junk(ctx, arg(args, "ids")?, arg(args, "junk")?));
    "list_categories", false, "Every category." => reply(organize::list_categories(ctx));
    "save_category", true, "Add a category, change its colour, or rename it." => reply(organize::save_category(ctx, arg(args, "category")?, arg(args, "previousName")?));
    "delete_category", true, "Delete a category, from every message too." => reply(organize::delete_category(ctx, arg(args, "name")?));
    "set_categories", true, "Add and remove categories on messages." => reply(organize::set_categories(ctx, arg(args, "ids")?, arg(args, "add")?, arg(args, "remove")?));
    "set_folder_favorite", true, "List a folder under Favourites (this device only)." => reply(organize::set_folder_favorite(ctx, arg(args, "folderId")?, arg(args, "favorite")?));
    "reorder_accounts", true, "Put the accounts in a new order." => reply(organize::reorder_accounts(ctx, arg(args, "ids")?));
    "check_draft", false, "Check recipients and size before sending." => reply(Box::pin(outbox::check_draft(ctx, arg(args, "draft")?)).await);
    "send_message", true, "Queue a message for sending (Undo Send applies)." => reply(Box::pin(outbox::send_message(ctx, arg(args, "draft")?)).await);
    "save_draft", true, "Save a draft in the account's Drafts folder." => reply(Box::pin(outbox::save_draft(ctx, arg(args, "draft")?)).await);
    "list_outbox", false, "Messages waiting to be sent, and those that failed." => reply(outbox::list_outbox(ctx));
    "outbox_send_now", true, "Send a queued message now, or retry a failed one." => reply(outbox::outbox_send_now(ctx, arg(args, "id")?));
    "outbox_cancel", true, "Cancel a queued message." => reply(outbox::outbox_cancel(ctx, arg(args, "id")?));
    "outbox_edit", true, "Take a queued or failed message back into compose." => reply(Box::pin(outbox::outbox_edit(ctx, arg(args, "id")?)).await);
    "attachments_for_draft", false, "A message's attachments, ready to attach to a draft." => reply(Box::pin(outbox::attachments_for_draft(ctx, arg(args, "messageId")?)).await);
    "get_out_of_office", false, "An account's automatic replies." => reply(Box::pin(out_of_office::get_out_of_office(ctx, arg(args, "accountId")?)).await);
    "set_out_of_office", true, "Save automatic replies and switch them on or off." => reply(Box::pin(out_of_office::set_out_of_office(ctx, arg(args, "accountId")?, arg(args, "settings")?)).await);
    "import_pst", true, "Pick an Outlook .pst file and bring in its mail, contacts, calendar and tasks." => reply(Box::pin(pst::import_pst(ctx, arg(args, "accountId")?)).await);
    "answer_read_receipt", true, "Send or decline the read receipt a message asks for." => reply(Box::pin(read_receipts::answer_read_receipt(ctx, arg(args, "messageId")?, arg(args, "send")?)).await);
    "list_due_reminders", false, "Reminders due now, soonest first." => reply(reminders::list_due_reminders(ctx));
    "dismiss_reminder", true, "Dismiss a reminder." => reply(reminders::dismiss_reminder(ctx, arg(args, "key")?));
    "snooze_reminder", true, "Show a reminder again in some minutes." => reply(reminders::snooze_reminder(ctx, arg(args, "key")?, arg(args, "minutes")?));
    "parse_search", false, "What a search box query means." => value(rules::parse_search(arg(args, "query")?));
    "list_rules", false, "Every mail rule, in order." => reply(rules::list_rules(ctx));
    "save_rule", true, "Add or change a mail rule." => reply(rules::save_rule(ctx, arg(args, "rule")?));
    "delete_rule", true, "Delete a mail rule." => reply(rules::delete_rule(ctx, arg(args, "id")?));
    "reorder_rules", true, "Put the rules in a new order." => reply(rules::reorder_rules(ctx, arg(args, "ids")?));
    "run_rules", true, "Run rules now on a folder's newest messages." => reply(rules::run_rules(ctx, arg(args, "ruleIds")?, arg(args, "folderId")?));
    "preview_rule", false, "How many of a folder's newest messages a rule would match." => reply(rules::preview_rule(ctx, arg(args, "rule")?, arg(args, "folderId")?));
    "list_image_senders", false, "Senders whose pictures always load." => reply(rules::list_image_senders(ctx));
    "trust_image_sender", true, "Always load pictures from an address or domain." => reply(rules::trust_image_sender(ctx, arg(args, "sender")?, arg(args, "wholeDomain")?));
    "untrust_image_sender", true, "Stop always loading pictures from an address or domain." => reply(rules::untrust_image_sender(ctx, arg(args, "sender")?));
    "list_keys", false, "Every S/MIME certificate and OpenPGP key known." => reply(security::list_keys(ctx));
    "import_key", true, "Pick a key or certificate file and add what is in it." => reply(Box::pin(security::import_key(ctx, arg(args, "password")?)).await);
    "export_public_key", true, "Save a certificate or public key to share." => reply(Box::pin(security::export_public_key(ctx, arg(args, "id")?)).await);
    "remove_key", true, "Remove a key or certificate." => reply(security::remove_key(ctx, arg(args, "id")?));
    "set_key_trusted", true, "Say whether a key really is its owner's." => reply(security::set_key_trusted(ctx, arg(args, "id")?, arg(args, "trusted")?));
    "set_account_key", true, "Choose the key an account signs and decrypts with." => reply(security::set_account_key(ctx, arg(args, "accountId")?, arg(args, "kind")?, arg(args, "keyId")?));
    "get_account_keys", false, "An account's chosen keys." => reply(security::get_account_keys(ctx, arg(args, "accountId")?));
    "set_preferred_kind", true, "Choose S/MIME or OpenPGP for an account." => reply(security::set_preferred_kind(ctx, arg(args, "accountId")?, arg(args, "kind")?));
    "check_encryption", false, "Which recipients' keys are known, for encrypting." => reply(Box::pin(security::check_encryption(ctx, arg(args, "accountId")?, arg(args, "addresses")?, arg(args, "kind")?)).await);
    "get_settings", false, "The app settings." => reply(settings::get_settings(ctx));
    "set_settings", true, "Save the app settings." => reply(settings::set_settings(ctx, arg(args, "settings")?));
    "get_signature", false, "The signature a new message (or a reply) starts with." => reply(signatures::get_signature(ctx, arg(args, "accountId")?, arg(args, "reply")?));
    "list_signatures", false, "An account's signatures and its defaults." => reply(signatures::list_signatures(ctx, arg(args, "accountId")?));
    "save_signature", true, "Create or change a signature." => reply(signatures::save_signature(ctx, arg(args, "accountId")?, arg(args, "id")?, arg(args, "name")?, arg(args, "html")?));
    "delete_signature", true, "Delete a signature." => reply(signatures::delete_signature(ctx, arg(args, "accountId")?, arg(args, "id")?));
    "set_signature_defaults", true, "Choose the signatures new messages and replies start with." => reply(signatures::set_signature_defaults(ctx, arg(args, "accountId")?, arg(args, "defaultNew")?, arg(args, "defaultReply")?));
    "set_signature_image", true, "Pick a picture for a signature." => reply(Box::pin(signatures::set_signature_image(ctx, arg(args, "accountId")?, arg(args, "id")?)).await);
    "set_signature_image_width", true, "Set how wide a signature's picture shows." => reply(signatures::set_signature_image_width(ctx, arg(args, "accountId")?, arg(args, "id")?, arg(args, "width")?));
    "remove_signature_image", true, "Remove a signature's picture." => reply(signatures::remove_signature_image(ctx, arg(args, "accountId")?, arg(args, "id")?));
    "list_task_lists", false, "The device's task list, then each account's." => reply(tasks::list_task_lists(ctx));
    "list_tasks", false, "The tasks in one list, or every list." => reply(tasks::list_tasks(ctx, arg(args, "listId")?, arg(args, "includeCompleted")?));
    "save_task", true, "Create or change a task." => reply(tasks::save_task(ctx, arg(args, "task")?));
    "delete_task", true, "Delete a task." => reply(tasks::delete_task(ctx, arg(args, "id")?));
    "sync_tasks", false, "Sync every account's task lists now." => reply(Box::pin(tasks::sync_tasks(ctx)).await);
    "check_for_update", false, "The newer version this install can move to, if any." => reply(Box::pin(updates::check_for_update(ctx)).await);
    "install_update", true, "Download, check and run the newest installer." => reply(Box::pin(updates::install_update(ctx)).await);
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
