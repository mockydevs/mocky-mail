//! The `#[tauri::command]`s the interface calls (`docs/ipc.md`). Each one
//! only calls its `mail-app` function: the names, arguments, results and
//! errors are that function's, so the interface's contract is unchanged.
//! Commands that run on the main thread here (not `async`) stay that way.

use mail_app::{AppError, AppResult, Ctx};
use tauri::State;
use tauri::ipc::Response;

// ---------------------------------------------------------------- accounts

/// `suggest_servers`: settings to pre-fill for an address. Known providers
/// first, then the domain's DNS (SRV and MX records), then a guess.
#[tauri::command]
pub async fn suggest_servers(email: String) -> Option<mail_sync::ServerSuggestion> {
    mail_app::accounts::suggest_servers(email).await
}

/// `list_accounts`.
#[tauri::command]
pub fn list_accounts(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_store::Account>> {
    mail_app::accounts::list_accounts(&ctx)
}

/// `add_account`: password sign-in.
#[tauri::command]
pub async fn add_account(
    ctx: State<'_, Ctx>,
    input: mail_app::accounts::AddAccountInput,
) -> AppResult<mail_store::Account> {
    mail_app::accounts::add_account(&ctx, input).await
}

/// `add_oauth_account`: Google or Microsoft sign-in in the system browser.
#[tauri::command]
pub async fn add_oauth_account(
    ctx: State<'_, Ctx>,
    input: mail_app::accounts::AddOAuthInput,
) -> AppResult<mail_store::Account> {
    mail_app::accounts::add_oauth_account(&ctx, input).await
}

/// `add_exchange_account`: an account on an organisation's own Exchange
/// Server (Autodiscover finds the server when `ewsUrl` is null).
#[tauri::command]
pub async fn add_exchange_account(
    ctx: State<'_, Ctx>,
    input: mail_app::accounts::ExchangeAccountInput,
) -> AppResult<mail_store::Account> {
    mail_app::accounts::add_exchange_account(&ctx, input).await
}

/// `update_account`: rename, or replace the password after testing it.
#[tauri::command]
pub async fn update_account(
    ctx: State<'_, Ctx>,
    id: String,
    patch: mail_app::accounts::AccountPatch,
) -> AppResult<mail_store::Account> {
    mail_app::accounts::update_account(&ctx, id, patch).await
}

/// `reconnect_account`: sign in again to a Google or Microsoft account
/// whose saved sign-in has expired or been revoked. Without this the only
/// way back was removing the account, which also deletes its queued
/// messages and the contacts kept only on this device.
#[tauri::command]
pub async fn reconnect_account(ctx: State<'_, Ctx>, id: String) -> AppResult<mail_store::Account> {
    mail_app::accounts::reconnect_account(&ctx, id).await
}

/// `remove_account`: stop syncing, forget secrets and local mail. Mail on
/// the server is not touched.
#[tauri::command]
pub fn remove_account(ctx: State<'_, Ctx>, id: String) -> AppResult<()> {
    mail_app::accounts::remove_account(&ctx, id)
}

// ---------------------------------------------------------------- attachments

/// `preview_attachment`: the attachment's bytes, for the in-app preview.
/// Refused (`invalid`) unless the file really is a photo, PDF or plain
/// text, whatever its name says.
#[tauri::command]
pub async fn preview_attachment(
    ctx: State<'_, Ctx>,
    message_id: String,
    index: usize,
) -> AppResult<Response> {
    mail_app::attachments::preview_attachment(&ctx, message_id, index)
        .await
        .map(Response::new)
}

/// `open_attachment`: open the file in the app the device uses for its
/// type (PDF reader, Word, photo viewer).
#[tauri::command]
pub async fn open_attachment(
    ctx: State<'_, Ctx>,
    message_id: String,
    index: usize,
) -> AppResult<()> {
    mail_app::attachments::open_attachment(&ctx, message_id, index).await
}

/// `save_attachment`: ask where to save it, then write it there. The place
/// it was saved, or `None` if the user cancelled.
#[tauri::command]
pub async fn save_attachment(
    ctx: State<'_, Ctx>,
    message_id: String,
    index: usize,
) -> AppResult<Option<String>> {
    mail_app::attachments::save_attachment(&ctx, message_id, index).await
}

/// `pick_attachments`: the open-file dialog. Only files picked here may be
/// attached to a message.
#[tauri::command]
pub async fn pick_attachments(
    ctx: State<'_, Ctx>,
) -> AppResult<Vec<mail_app::mail::DraftAttachment>> {
    mail_app::attachments::pick_attachments(&ctx).await
}

// ---------------------------------------------------------------- calendar

/// `list_calendars`: every account's calendars.
#[tauri::command]
pub fn list_calendars(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_store::Calendar>> {
    mail_app::calendar::list_calendars(&ctx)
}

/// `set_calendar_visible`: show or hide a calendar (this device only).
#[tauri::command]
pub fn set_calendar_visible(
    ctx: State<'_, Ctx>,
    calendar_id: String,
    visible: bool,
) -> AppResult<Vec<mail_store::Calendar>> {
    mail_app::calendar::set_calendar_visible(&ctx, calendar_id, visible)
}

/// `list_events`: every occurrence between `from` and `to` (Unix seconds)
/// in the calendars that are shown.
#[tauri::command]
pub fn list_events(
    ctx: State<'_, Ctx>,
    from: i64,
    to: i64,
) -> AppResult<Vec<mail_app::calendar::CalendarEvent>> {
    mail_app::calendar::list_events(&ctx, from, to)
}

/// `get_event`: the event itself (a series' first date and its rule), for
/// the editor.
#[tauri::command]
pub fn get_event(ctx: State<'_, Ctx>, id: String) -> AppResult<mail_app::calendar::CalendarEvent> {
    mail_app::calendar::get_event(&ctx, id)
}

/// `save_event`: create or change an event on the server, then here.
#[tauri::command]
pub async fn save_event(
    ctx: State<'_, Ctx>,
    event: mail_app::calendar::EventSave,
) -> AppResult<mail_app::calendar::CalendarEvent> {
    mail_app::calendar::save_event(&ctx, event).await
}

/// `delete_event`: delete an event (or a whole series), or with
/// `occurrence` just that date of a series. With `notify`, people invited
/// to a meeting you organise are sent a cancellation.
#[tauri::command]
pub async fn delete_event(
    ctx: State<'_, Ctx>,
    id: String,
    occurrence: Option<i64>,
    notify: Option<bool>,
) -> AppResult<()> {
    mail_app::calendar::delete_event(&ctx, id, occurrence, notify).await
}

/// `get_free_busy`: when these people are busy, for Find a Time. Your own
/// accounts come from the calendars kept here; others from the account's
/// server (`CalDAV` scheduling, Google's free/busy, Microsoft's
/// `getSchedule`), and are `unknown` when it can't tell.
#[tauri::command]
pub async fn get_free_busy(
    ctx: State<'_, Ctx>,
    account_id: String,
    emails: Vec<String>,
    from: i64,
    to: i64,
) -> AppResult<Vec<mail_app::calendar::FreeBusy>> {
    mail_app::calendar::get_free_busy(&ctx, account_id, emails, from, to).await
}

/// `sync_calendars`: sync every account's calendars now.
#[tauri::command]
pub async fn sync_calendars(
    ctx: State<'_, Ctx>,
) -> AppResult<Vec<mail_app::calendar::CalendarStatus>> {
    mail_app::calendar::sync_calendars(&ctx).await
}

// ---------------------------------------------------------------- compose

/// `resolve_recipients`: Check Names for one recipient field.
#[tauri::command]
pub fn resolve_recipients(
    ctx: State<'_, Ctx>,
    text: String,
) -> AppResult<mail_core::compose::RecipientCheck> {
    mail_app::compose::resolve_recipients(&ctx, text)
}

/// `contact_card_attachment`: a contact as a vCard 3.0 `.vcf` file to
/// attach.
#[tauri::command]
pub async fn contact_card_attachment(
    ctx: State<'_, Ctx>,
    contact_id: String,
) -> AppResult<mail_app::mail::DraftAttachment> {
    mail_app::compose::contact_card_attachment(&ctx, contact_id).await
}

/// `message_as_attachment`: a message, whole, as an `.eml` file to attach
/// (sent as `message/rfc822`).
#[tauri::command]
pub async fn message_as_attachment(
    ctx: State<'_, Ctx>,
    message_id: String,
) -> AppResult<mail_app::mail::DraftAttachment> {
    mail_app::compose::message_as_attachment(&ctx, message_id).await
}

/// `open_compose_window`: continue writing this draft in a window of its
/// own. Phones reject it (`invalid`).
#[tauri::command]
pub async fn open_compose_window(
    ctx: State<'_, Ctx>,
    draft: mail_app::outbox::Draft,
) -> AppResult<()> {
    mail_app::compose::open_compose_window(&ctx, draft).await
}

/// `take_compose_draft`: the draft a compose window was opened with, once;
/// `None` after that or for an unknown key.
#[tauri::command]
pub fn take_compose_draft(ctx: State<'_, Ctx>, key: String) -> Option<mail_app::outbox::Draft> {
    mail_app::compose::take_compose_draft(&ctx, key)
}

// ---------------------------------------------------------------- connections

/// `connect_calendar_and_contacts`: sign in again in the browser asking for
/// calendar, contacts, tasks and automatic-reply access, then sync them.
#[tauri::command]
pub async fn connect_calendar_and_contacts(
    ctx: State<'_, Ctx>,
    account_id: String,
) -> AppResult<()> {
    mail_app::connections::connect_calendar_and_contacts(&ctx, account_id).await
}

// ---------------------------------------------------------------- contacts

/// `list_contacts`: saved contacts matching `query`, favourites first, then
/// by name.
#[tauri::command]
pub fn list_contacts(ctx: State<'_, Ctx>, query: String) -> AppResult<Vec<mail_store::Contact>> {
    mail_app::contacts::list_contacts(&ctx, query)
}

/// `get_contact`.
#[tauri::command]
pub fn get_contact(ctx: State<'_, Ctx>, id: String) -> AppResult<mail_store::Contact> {
    mail_app::contacts::get_contact(&ctx, id)
}

/// `save_contact`: create or update; synced on the next contacts sync.
#[tauri::command]
pub fn save_contact(
    ctx: State<'_, Ctx>,
    contact: mail_app::contacts::ContactInput,
) -> AppResult<mail_store::Contact> {
    mail_app::contacts::save_contact(&ctx, contact)
}

/// `delete_contact`: removed here at once, from the server on the next sync.
#[tauri::command]
pub fn delete_contact(ctx: State<'_, Ctx>, id: String) -> AppResult<()> {
    mail_app::contacts::delete_contact(&ctx, id)
}

/// `list_contact_groups`: every group, by name.
#[tauri::command]
pub fn list_contact_groups(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_store::ContactGroup>> {
    mail_app::contacts::list_contact_groups(&ctx)
}

/// `save_contact_group`: create or change a group. Groups of password and
/// Gmail accounts sync with their address book; Microsoft has no groups
/// the app can use, so those are kept on this device.
#[tauri::command]
pub fn save_contact_group(
    ctx: State<'_, Ctx>,
    group: mail_app::contacts::GroupInput,
) -> AppResult<Vec<mail_store::ContactGroup>> {
    mail_app::contacts::save_contact_group(&ctx, group)
}

/// `delete_contact_group`: the group goes (its people stay).
#[tauri::command]
pub fn delete_contact_group(
    ctx: State<'_, Ctx>,
    id: String,
) -> AppResult<Vec<mail_store::ContactGroup>> {
    mail_app::contacts::delete_contact_group(&ctx, id)
}

/// `sync_contacts`: sync every account's address book now.
#[tauri::command]
pub async fn sync_contacts(
    ctx: State<'_, Ctx>,
) -> AppResult<Vec<mail_app::contacts::AddressBookStatus>> {
    mail_app::contacts::sync_contacts(&ctx).await
}

// ---------------------------------------------------------------- files

/// `import_mail_files`: pick `.eml` and `.mbox` files and upload their
/// messages into a folder. `None` when cancelled.
#[tauri::command]
pub async fn import_mail_files(
    ctx: State<'_, Ctx>,
    folder_id: String,
) -> AppResult<Option<mail_app::files::ImportReport>> {
    // Boxed: its state is large (clippy::large_futures).
    Box::pin(mail_app::files::import_mail_files(&ctx, folder_id)).await
}

/// `import_message_file`: add an opened `.eml` file to a folder.
#[tauri::command]
pub async fn import_message_file(
    ctx: State<'_, Ctx>,
    path: String,
    folder_id: String,
) -> AppResult<()> {
    mail_app::files::import_message_file(&ctx, path, folder_id).await
}

/// `export_mail`: a folder as one `.mbox` file, or messages as `.eml`
/// files in a `.zip`. Where it was saved; `None` when cancelled.
#[tauri::command]
pub async fn export_mail(
    ctx: State<'_, Ctx>,
    target: mail_app::files::ExportTarget,
    format: String,
) -> AppResult<Option<String>> {
    mail_app::files::export_mail(&ctx, target, format).await
}

/// `import_calendar_file`: pick an `.ics` file and add its events to a
/// calendar. `None` when cancelled.
#[tauri::command]
pub async fn import_calendar_file(
    ctx: State<'_, Ctx>,
    calendar_id: String,
) -> AppResult<Option<mail_app::files::ImportReport>> {
    mail_app::files::import_calendar_file(&ctx, calendar_id).await
}

/// `export_calendar`: a calendar as one `.ics` file.
#[tauri::command]
pub async fn export_calendar(
    ctx: State<'_, Ctx>,
    calendar_id: String,
) -> AppResult<Option<String>> {
    mail_app::files::export_calendar(&ctx, calendar_id).await
}

/// `import_contacts_file`: pick a `.vcf` file and add its contacts
/// (`accountId` null: this device). `None` when cancelled.
#[tauri::command]
pub async fn import_contacts_file(
    ctx: State<'_, Ctx>,
    account_id: Option<String>,
) -> AppResult<Option<mail_app::files::ImportReport>> {
    mail_app::files::import_contacts_file(&ctx, account_id).await
}

/// `export_contacts`: contacts as one `.vcf` file (vCard 3.0): an
/// account's, this device's (null) or everyone's (`*`).
#[tauri::command]
pub async fn export_contacts(
    ctx: State<'_, Ctx>,
    account_id: Option<String>,
) -> AppResult<Option<String>> {
    mail_app::files::export_contacts(&ctx, account_id).await
}

/// `import_opened_file`: add an opened `.ics` or `.vcf` file's items.
#[tauri::command]
pub async fn import_opened_file(
    ctx: State<'_, Ctx>,
    path: String,
    target: mail_app::files::OpenedTarget,
) -> AppResult<mail_app::files::ImportReport> {
    mail_app::files::import_opened_file(&ctx, path, target).await
}

/// `read_message_file`: an opened `.eml` file, read for showing.
#[tauri::command]
pub fn read_message_file(
    ctx: State<'_, Ctx>,
    path: String,
) -> AppResult<mail_app::mail::MessageDetail> {
    mail_app::files::read_message_file(&ctx, path)
}

// ---------------------------------------------------------------- folders

/// `create_folder`: a new folder, at the top or inside `parentId`.
#[tauri::command]
pub async fn create_folder(
    ctx: State<'_, Ctx>,
    account_id: String,
    name: String,
    parent_id: Option<String>,
) -> AppResult<()> {
    mail_app::folders::create_folder(&ctx, account_id, name, parent_id).await
}

/// `rename_folder`: rename one of the user's own folders (it stays where it is).
#[tauri::command]
pub async fn rename_folder(ctx: State<'_, Ctx>, folder_id: String, name: String) -> AppResult<()> {
    mail_app::folders::rename_folder(&ctx, folder_id, name).await
}

/// `delete_folder`: delete one of the user's own folders and the mail in it.
/// A folder with subfolders must be emptied of them first.
#[tauri::command]
pub async fn delete_folder(ctx: State<'_, Ctx>, folder_id: String) -> AppResult<()> {
    mail_app::folders::delete_folder(&ctx, folder_id).await
}

// ---------------------------------------------------------------- invitations

/// `answer_invitation`: accept, maybe or decline. Saves it in your
/// calendar (a decline removes it) and, with `notify`, emails the organiser.
#[tauri::command]
pub async fn answer_invitation(
    ctx: State<'_, Ctx>,
    id: String,
    answer: mail_core::ical::Answer,
    notify: bool,
) -> AppResult<mail_app::invitations::InvitationView> {
    // Boxed: its state is large (clippy::large_futures).
    Box::pin(mail_app::invitations::answer_invitation(
        &ctx, id, answer, notify,
    ))
    .await
}

/// `remove_cancelled_event`: take a cancelled event out of your calendar.
#[tauri::command]
pub async fn remove_cancelled_event(
    ctx: State<'_, Ctx>,
    id: String,
) -> AppResult<mail_app::invitations::InvitationView> {
    // Boxed: its state is large (clippy::large_futures).
    Box::pin(mail_app::invitations::remove_cancelled_event(&ctx, id)).await
}

/// `apply_invitation_reply`: record someone's answer to your invitation
/// in your calendar copy.
#[tauri::command]
pub async fn apply_invitation_reply(
    ctx: State<'_, Ctx>,
    id: String,
) -> AppResult<mail_app::invitations::InvitationView> {
    // Boxed: its state is large (clippy::large_futures).
    Box::pin(mail_app::invitations::apply_invitation_reply(&ctx, id)).await
}

// ---------------------------------------------------------------- mail

/// `list_folders`.
#[tauri::command]
pub fn list_folders(ctx: State<'_, Ctx>, account_id: &str) -> AppResult<Vec<mail_store::Folder>> {
    mail_app::mail::list_folders(&ctx, account_id)
}

/// `list_messages`. With `threads`, one row per conversation.
#[tauri::command]
pub fn list_messages(
    ctx: State<'_, Ctx>,
    query: mail_store::ListQuery,
    threads: Option<bool>,
) -> AppResult<mail_store::MessagePage> {
    mail_app::mail::list_messages(&ctx, query, threads)
}

/// `set_focus_rule`: always put mail from this sender (or, with
/// `wholeDomain`, anyone at its domain) in Focused or Other; `None` removes
/// the rule.
#[tauri::command]
pub fn set_focus_rule(
    ctx: State<'_, Ctx>,
    sender: String,
    focus: Option<mail_core::focus::Focus>,
    whole_domain: bool,
) -> AppResult<()> {
    mail_app::mail::set_focus_rule(&ctx, sender, focus, whole_domain)
}

/// `preview_body`: the HTML recipients will see for what is typed in
/// compose (formatting marks rendered, typed HTML escaped).
#[tauri::command]
pub fn preview_body(body: String) -> AppResult<String> {
    mail_app::mail::preview_body(body)
}

/// `get_thread`: every message in a message's conversation, oldest first.
#[tauri::command]
pub fn get_thread(ctx: State<'_, Ctx>, id: String) -> AppResult<Vec<mail_store::MessageListItem>> {
    mail_app::mail::get_thread(&ctx, id)
}

/// `get_message`: the body is downloaded if needed, cleaned and returned.
/// Opening a message never marks it read; the interface calls `set_flags`.
/// With Settings `readReceipts: 'always'`, a read receipt it asks for is
/// queued the first time.
#[tauri::command]
pub async fn get_message(
    ctx: State<'_, Ctx>,
    id: String,
    allow_remote: bool,
) -> AppResult<mail_app::mail::MessageDetail> {
    mail_app::mail::get_message(&ctx, id, allow_remote).await
}

/// `set_flags`: read, flagged, pinned, importance and follow-up date.
/// Changes show at once; read and flagged reach the server when online,
/// the rest stay on this device. The arguments are read whole, since
/// Tauri can't tell a `flagDue` of null from one left out.
#[tauri::command]
pub fn set_flags(ctx: State<'_, Ctx>, request: tauri::ipc::Request<'_>) -> AppResult<()> {
    let tauri::ipc::InvokeBody::Json(body) = request.body() else {
        return Err(AppError::invalid("set_flags expects JSON arguments."));
    };
    mail_app::mail::set_flags(&ctx, body)
}

/// `suggest_recipients`: people to offer while typing an address.
///
/// # Errors
/// Database errors.
#[tauri::command]
pub fn suggest_recipients(
    ctx: State<'_, Ctx>,
    query: String,
) -> AppResult<Vec<mail_core::message::Mailbox>> {
    mail_app::mail::suggest_recipients(&ctx, query)
}

/// `move_messages`: archive, trash, junk or any folder.
#[tauri::command]
pub fn move_messages(
    ctx: State<'_, Ctx>,
    ids: Vec<String>,
    target: mail_app::mail::MoveTarget,
) -> AppResult<()> {
    mail_app::mail::move_messages(&ctx, ids, target)
}

/// `delete_forever`: only from Trash or Junk, so a slip can't destroy mail.
#[tauri::command]
pub fn delete_forever(ctx: State<'_, Ctx>, ids: Vec<String>) -> AppResult<()> {
    mail_app::mail::delete_forever(&ctx, ids)
}

/// `sync_now`.
#[tauri::command]
pub fn sync_now(ctx: State<'_, Ctx>, account_id: Option<String>) {
    mail_app::mail::sync_now(&ctx, account_id);
}

/// `open_external`: web and mail links open in the user's own apps.
#[tauri::command]
pub fn open_external(ctx: State<'_, Ctx>, url: String) -> AppResult<()> {
    mail_app::mail::open_external(&ctx, url)
}

// ---------------------------------------------------------------- mailto

/// `take_mailto`: the link the app was started with, once.
#[tauri::command]
pub fn take_mailto() -> Option<mail_core::mailto::Mailto> {
    mail_app::mailto::take_mailto()
}

/// `parse_mailto`: read a link handed over another way.
#[tauri::command]
pub fn parse_mailto(url: String) -> Option<mail_core::mailto::Mailto> {
    mail_app::mailto::parse_mailto(url)
}

// ---------------------------------------------------------------- message_tools

/// `save_message`: save the whole message as an `.eml` file, which mail
/// apps and Outlook open. Where it was saved, or `None` if cancelled.
#[tauri::command]
pub async fn save_message(ctx: State<'_, Ctx>, message_id: String) -> AppResult<Option<String>> {
    mail_app::message_tools::save_message(&ctx, message_id).await
}

/// `message_source`: the message as received (headers and body), as text.
/// Very large messages are cut, with a note.
#[tauri::command]
pub async fn message_source(ctx: State<'_, Ctx>, message_id: String) -> AppResult<String> {
    mail_app::message_tools::message_source(&ctx, message_id).await
}

/// `print_html`: Android only: print a page (the system print screen,
/// which can also save a PDF). Desktop prints from the page itself.
#[tauri::command]
pub async fn print_html(ctx: State<'_, Ctx>, html: String, title: String) -> AppResult<()> {
    mail_app::message_tools::print_html(&ctx, html, title).await
}

// ---------------------------------------------------------------- message_window

/// `open_message_window`: the message in its own window, or the one
/// already showing it brought to the front. Phones reject it (`invalid`).
#[tauri::command]
pub async fn open_message_window(ctx: State<'_, Ctx>, id: String) -> AppResult<()> {
    mail_app::message_window::open_message_window(&ctx, id).await
}

// ---------------------------------------------------------------- opened

/// `take_opened_files`: the files the app was started with (or shared to
/// it before the page was ready), once each.
#[tauri::command]
pub fn take_opened_files() -> Vec<mail_app::opened::OpenedFile> {
    mail_app::opened::take_opened_files()
}

// ---------------------------------------------------------------- organize

/// `copy_messages`: the originals stay; the copies are made on the server
/// (now, or when back online) and appear when the target folder syncs.
#[tauri::command]
pub fn copy_messages(
    ctx: State<'_, Ctx>,
    ids: Vec<String>,
    target: mail_app::mail::MoveTarget,
) -> AppResult<()> {
    mail_app::organize::copy_messages(&ctx, ids, target)
}

/// `mark_junk`: to the account's Junk folder marked `$Junk`, or (`junk`
/// false) back to the Inbox marked `$NotJunk`, so the server's filter
/// learns either way.
#[tauri::command]
pub fn mark_junk(ctx: State<'_, Ctx>, ids: Vec<String>, junk: bool) -> AppResult<()> {
    mail_app::organize::mark_junk(&ctx, ids, junk)
}

/// `list_categories`.
#[tauri::command]
pub fn list_categories(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_store::Category>> {
    mail_app::organize::list_categories(&ctx)
}

/// `save_category`: add one, change its colour, or with `previousName`
/// rename it on every message (and on the server). Returns them all.
#[tauri::command]
pub fn save_category(
    ctx: State<'_, Ctx>,
    category: mail_store::Category,
    previous_name: Option<String>,
) -> AppResult<Vec<mail_store::Category>> {
    mail_app::organize::save_category(&ctx, category, previous_name)
}

/// `delete_category`: from every message too. Returns the rest.
#[tauri::command]
pub fn delete_category(ctx: State<'_, Ctx>, name: String) -> AppResult<Vec<mail_store::Category>> {
    mail_app::organize::delete_category(&ctx, name)
}

/// `set_categories`: add and remove categories on messages. Names not yet
/// known become categories with a palette colour.
#[tauri::command]
pub fn set_categories(
    ctx: State<'_, Ctx>,
    ids: Vec<String>,
    add: Vec<String>,
    remove: Vec<String>,
) -> AppResult<()> {
    mail_app::organize::set_categories(&ctx, ids, add, remove)
}

/// `set_folder_favorite`: list it under Favourites (this device only).
#[tauri::command]
pub fn set_folder_favorite(
    ctx: State<'_, Ctx>,
    folder_id: String,
    favorite: bool,
) -> AppResult<()> {
    mail_app::organize::set_folder_favorite(&ctx, folder_id, favorite)
}

/// `reorder_accounts`: every account id, in the new order.
#[tauri::command]
pub fn reorder_accounts(ctx: State<'_, Ctx>, ids: Vec<String>) -> AppResult<()> {
    mail_app::organize::reorder_accounts(&ctx, ids)
}

// ---------------------------------------------------------------- outbox

/// `check_draft`: validates recipients and size before sending.
#[tauri::command]
pub async fn check_draft(
    ctx: State<'_, Ctx>,
    draft: mail_app::outbox::Draft,
) -> AppResult<mail_app::outbox::DraftCheck> {
    mail_app::outbox::check_draft(&ctx, draft).await
}

/// `send_message`: queue it; the worker sends it.
#[tauri::command]
pub async fn send_message(
    ctx: State<'_, Ctx>,
    draft: mail_app::outbox::Draft,
) -> AppResult<mail_app::outbox::OutboxDto> {
    mail_app::outbox::send_message(&ctx, draft).await
}

/// `save_draft`: stores it in the account's Drafts folder on the server,
/// replacing the earlier saved copy, so there is only ever one.
#[tauri::command]
pub async fn save_draft(
    ctx: State<'_, Ctx>,
    draft: mail_app::outbox::Draft,
) -> AppResult<mail_app::outbox::DraftSaved> {
    mail_app::outbox::save_draft(&ctx, draft).await
}

/// `list_outbox`.
#[tauri::command]
pub fn list_outbox(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_app::outbox::OutboxDto>> {
    mail_app::outbox::list_outbox(&ctx)
}

/// `outbox_send_now`: skip the retry wait, or retry a failed message.
#[tauri::command]
pub fn outbox_send_now(ctx: State<'_, Ctx>, id: String) -> AppResult<()> {
    mail_app::outbox::outbox_send_now(&ctx, id)
}

/// `outbox_cancel`.
#[tauri::command]
pub fn outbox_cancel(ctx: State<'_, Ctx>, id: String) -> AppResult<()> {
    mail_app::outbox::outbox_cancel(&ctx, id)
}

/// `outbox_edit`: take a queued or failed message back into compose.
/// Attachments are restored as files in the app's cache folder.
#[tauri::command]
pub async fn outbox_edit(ctx: State<'_, Ctx>, id: String) -> AppResult<mail_app::outbox::Draft> {
    mail_app::outbox::outbox_edit(&ctx, id).await
}

/// `attachments_for_draft`: a message's attachments, ready to attach.
#[tauri::command]
pub async fn attachments_for_draft(
    ctx: State<'_, Ctx>,
    message_id: String,
) -> AppResult<Vec<mail_app::mail::DraftAttachment>> {
    mail_app::outbox::attachments_for_draft(&ctx, message_id).await
}

// ---------------------------------------------------------------- out_of_office

/// `get_out_of_office`: an account's automatic replies.
#[tauri::command]
pub async fn get_out_of_office(
    ctx: State<'_, Ctx>,
    account_id: String,
) -> AppResult<mail_app::out_of_office::OutOfOffice> {
    mail_app::out_of_office::get_out_of_office(&ctx, account_id).await
}

/// `set_out_of_office`: save and switch automatic replies on or off.
#[tauri::command]
pub async fn set_out_of_office(
    ctx: State<'_, Ctx>,
    account_id: String,
    settings: mail_core::vacation::Vacation,
) -> AppResult<mail_app::out_of_office::OutOfOffice> {
    mail_app::out_of_office::set_out_of_office(&ctx, account_id, settings).await
}

// ---------------------------------------------------------------- pst

/// `import_pst`: pick an Outlook `.pst` file and bring its mail, contacts
/// and calendar into an account. `None` when cancelled.
#[tauri::command]
pub async fn import_pst(
    ctx: State<'_, Ctx>,
    account_id: String,
) -> AppResult<Option<mail_app::files::ImportReport>> {
    // Boxed: its state is large (clippy::large_futures).
    Box::pin(mail_app::pst::import_pst(&ctx, account_id)).await
}

// ---------------------------------------------------------------- read_receipts

/// `answer_read_receipt`: send (`send: true`) or decline the read receipt
/// a message asks for. Either way it is not asked about again; answering
/// twice does nothing more.
#[tauri::command]
pub async fn answer_read_receipt(
    ctx: State<'_, Ctx>,
    message_id: String,
    send: bool,
) -> AppResult<()> {
    mail_app::read_receipts::answer_read_receipt(&ctx, message_id, send).await
}

// ---------------------------------------------------------------- reminders

/// `list_due_reminders`: what is due now, soonest first.
#[tauri::command]
pub fn list_due_reminders(
    ctx: State<'_, Ctx>,
) -> AppResult<Vec<mail_core::reminders::DueReminder>> {
    mail_app::reminders::list_due_reminders(&ctx)
}

/// `dismiss_reminder`: never list it again.
#[tauri::command]
pub fn dismiss_reminder(
    ctx: State<'_, Ctx>,
    key: String,
) -> AppResult<Vec<mail_core::reminders::DueReminder>> {
    mail_app::reminders::dismiss_reminder(&ctx, key)
}

/// `snooze_reminder`: list it again in `minutes`.
#[tauri::command]
pub fn snooze_reminder(
    ctx: State<'_, Ctx>,
    key: String,
    minutes: u32,
) -> AppResult<Vec<mail_core::reminders::DueReminder>> {
    mail_app::reminders::snooze_reminder(&ctx, key, minutes)
}

// ---------------------------------------------------------------- rules

/// `parse_search`: what a search box query means, for chips and help.
#[tauri::command]
pub fn parse_search(query: String) -> mail_core::search::ParsedSearch {
    mail_app::rules::parse_search(query)
}

/// `list_rules`.
#[tauri::command]
pub fn list_rules(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_core::rules::Rule>> {
    mail_app::rules::list_rules(&ctx)
}

/// `save_rule`: add (id null) or change a rule. Writing it to the mail
/// server happens in the background (`mail://rules-changed` when done).
#[tauri::command]
pub fn save_rule(
    ctx: State<'_, Ctx>,
    rule: mail_core::rules::RuleInput,
) -> AppResult<Vec<mail_core::rules::Rule>> {
    mail_app::rules::save_rule(&ctx, rule)
}

/// `delete_rule`.
#[tauri::command]
pub fn delete_rule(ctx: State<'_, Ctx>, id: String) -> AppResult<Vec<mail_core::rules::Rule>> {
    mail_app::rules::delete_rule(&ctx, id)
}

/// `reorder_rules`: every rule id, in the new order.
#[tauri::command]
pub fn reorder_rules(
    ctx: State<'_, Ctx>,
    ids: Vec<String>,
) -> AppResult<Vec<mail_core::rules::Rule>> {
    mail_app::rules::reorder_rules(&ctx, ids)
}

/// `run_rules`: run rules now on a folder's newest [`MAX_RUN`](mail_app::rules::MAX_RUN) messages.
/// `rule_ids` `None`: every enabled rule of the folder's account.
#[tauri::command]
pub fn run_rules(
    ctx: State<'_, Ctx>,
    rule_ids: Option<Vec<String>>,
    folder_id: String,
) -> AppResult<mail_app::rules::RuleRunResult> {
    mail_app::rules::run_rules(&ctx, rule_ids, folder_id)
}

/// `preview_rule`: how many of a folder's newest [`MAX_RUN`](mail_app::rules::MAX_RUN) messages an
/// unsaved rule's conditions and exceptions match (its actions, name and
/// account are not needed yet).
#[tauri::command]
pub fn preview_rule(
    ctx: State<'_, Ctx>,
    rule: mail_core::rules::RuleInput,
    folder_id: String,
) -> AppResult<u32> {
    mail_app::rules::preview_rule(&ctx, rule, folder_id)
}

/// `list_image_senders`.
#[tauri::command]
pub fn list_image_senders(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_store::ImageSender>> {
    mail_app::rules::list_image_senders(&ctx)
}

/// `trust_image_sender`: always load pictures from this address, or with
/// `wholeDomain` from anyone at its domain (and its subdomains).
#[tauri::command]
pub fn trust_image_sender(
    ctx: State<'_, Ctx>,
    sender: String,
    whole_domain: bool,
) -> AppResult<Vec<mail_store::ImageSender>> {
    mail_app::rules::trust_image_sender(&ctx, sender, whole_domain)
}

/// `untrust_image_sender`: as listed (address or domain).
#[tauri::command]
pub fn untrust_image_sender(
    ctx: State<'_, Ctx>,
    sender: String,
) -> AppResult<Vec<mail_store::ImageSender>> {
    mail_app::rules::untrust_image_sender(&ctx, sender)
}

// ---------------------------------------------------------------- security

/// `list_keys`.
#[tauri::command]
pub fn list_keys(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_app::security::SecurityKeyDto>> {
    mail_app::security::list_keys(&ctx)
}

/// `import_key`: pick a key or certificate file and add what is in it.
/// `password` opens a .p12/.pfx or a protected `OpenPGP` secret key; it is
/// used once and not kept. `None` when cancelled.
#[tauri::command]
pub async fn import_key(
    ctx: State<'_, Ctx>,
    password: Option<String>,
) -> AppResult<Option<Vec<mail_app::security::SecurityKeyDto>>> {
    mail_app::security::import_key(&ctx, password).await
}

/// `export_public_key`: save a certificate (.cer) or public key (.asc) to
/// share. The place saved to, or `None` when cancelled.
#[tauri::command]
pub async fn export_public_key(ctx: State<'_, Ctx>, id: String) -> AppResult<Option<String>> {
    mail_app::security::export_public_key(&ctx, id).await
}

/// `remove_key`.
#[tauri::command]
pub fn remove_key(
    ctx: State<'_, Ctx>,
    id: String,
) -> AppResult<Vec<mail_app::security::SecurityKeyDto>> {
    mail_app::security::remove_key(&ctx, id)
}

/// `set_key_trusted`: say a key or certificate really is its owner's (a key
/// from a system store is first kept in Mocky Mail's).
#[tauri::command]
pub fn set_key_trusted(
    ctx: State<'_, Ctx>,
    id: String,
    trusted: bool,
) -> AppResult<Vec<mail_app::security::SecurityKeyDto>> {
    mail_app::security::set_key_trusted(&ctx, id, trusted)
}

/// `set_account_key`.
#[tauri::command]
pub fn set_account_key(
    ctx: State<'_, Ctx>,
    account_id: String,
    kind: mail_sync::security::SecurityKind,
    key_id: Option<String>,
) -> AppResult<()> {
    mail_app::security::set_account_key(&ctx, account_id, kind, key_id)
}

/// `get_account_keys`.
#[tauri::command]
pub fn get_account_keys(
    ctx: State<'_, Ctx>,
    account_id: String,
) -> AppResult<mail_app::security::AccountKeysDto> {
    mail_app::security::get_account_keys(&ctx, account_id)
}

/// `set_preferred_kind`.
#[tauri::command]
pub fn set_preferred_kind(
    ctx: State<'_, Ctx>,
    account_id: String,
    kind: mail_sync::security::SecurityKind,
) -> AppResult<()> {
    mail_app::security::set_preferred_kind(&ctx, account_id, kind)
}

/// `check_encryption`: which recipients' keys are known, looking `OpenPGP`
/// keys up by WKD (and keeping what is found, untrusted).
#[tauri::command]
pub async fn check_encryption(
    ctx: State<'_, Ctx>,
    account_id: String,
    addresses: Vec<String>,
    kind: mail_sync::security::SecurityKind,
) -> AppResult<mail_app::security::EncryptionCheck> {
    mail_app::security::check_encryption(&ctx, account_id, addresses, kind).await
}

// ---------------------------------------------------------------- settings

/// `get_settings` command.
#[tauri::command]
pub fn get_settings(ctx: State<'_, Ctx>) -> AppResult<mail_app::settings::Settings> {
    mail_app::settings::get_settings(&ctx)
}

/// `set_settings` command. Start at login is applied to the system.
#[tauri::command]
pub fn set_settings(
    ctx: State<'_, Ctx>,
    settings: mail_app::settings::Settings,
) -> AppResult<mail_app::settings::Settings> {
    mail_app::settings::set_settings(&ctx, settings)
}

// ---------------------------------------------------------------- signatures

/// `get_signature`: the signature a new message (or, with `reply`, a reply
/// or forward) starts with.
#[tauri::command]
pub fn get_signature(
    ctx: State<'_, Ctx>,
    account_id: String,
    reply: Option<bool>,
) -> AppResult<Option<mail_core::signature::Signature>> {
    mail_app::signatures::get_signature(&ctx, account_id, reply)
}

/// `list_signatures`: all of the account's signatures and its defaults.
#[tauri::command]
pub fn list_signatures(
    ctx: State<'_, Ctx>,
    account_id: String,
) -> AppResult<mail_app::signatures::SignatureSet> {
    mail_app::signatures::list_signatures(&ctx, account_id)
}

/// `save_signature`: create (`id` null) or change a signature's name and
/// words. The words are cleaned like incoming mail; the picture is kept.
/// The first signature becomes the default for new messages and replies.
#[tauri::command]
pub fn save_signature(
    ctx: State<'_, Ctx>,
    account_id: String,
    id: Option<String>,
    name: String,
    html: String,
) -> AppResult<mail_app::signatures::SignatureSet> {
    mail_app::signatures::save_signature(&ctx, account_id, id, name, html)
}

/// `delete_signature`: remove one; defaults that pointed at it become none.
#[tauri::command]
pub fn delete_signature(
    ctx: State<'_, Ctx>,
    account_id: String,
    id: String,
) -> AppResult<mail_app::signatures::SignatureSet> {
    mail_app::signatures::delete_signature(&ctx, account_id, id)
}

/// `set_signature_defaults`: which signature new messages, and which
/// replies and forwards, start with (null: none).
#[tauri::command]
pub fn set_signature_defaults(
    ctx: State<'_, Ctx>,
    account_id: String,
    default_new: Option<String>,
    default_reply: Option<String>,
) -> AppResult<mail_app::signatures::SignatureSet> {
    mail_app::signatures::set_signature_defaults(&ctx, account_id, default_new, default_reply)
}

/// `set_signature_image`: choose a picture (PNG, JPG or GIF, up to 1 MB)
/// for a signature. `None` if the user cancelled.
#[tauri::command]
pub async fn set_signature_image(
    ctx: State<'_, Ctx>,
    account_id: String,
    id: String,
) -> AppResult<Option<mail_app::signatures::SignatureSet>> {
    mail_app::signatures::set_signature_image(&ctx, account_id, id).await
}

/// `set_signature_image_width`: how wide a signature's picture shows, px.
#[tauri::command]
pub fn set_signature_image_width(
    ctx: State<'_, Ctx>,
    account_id: String,
    id: String,
    width: u32,
) -> AppResult<mail_app::signatures::SignatureSet> {
    mail_app::signatures::set_signature_image_width(&ctx, account_id, id, width)
}

/// `remove_signature_image`: drop a signature's picture, keep its words.
#[tauri::command]
pub fn remove_signature_image(
    ctx: State<'_, Ctx>,
    account_id: String,
    id: String,
) -> AppResult<mail_app::signatures::SignatureSet> {
    mail_app::signatures::remove_signature_image(&ctx, account_id, id)
}

// ---------------------------------------------------------------- tasks

/// `list_task_lists`: the device's list, then each account's.
#[tauri::command]
pub fn list_task_lists(ctx: State<'_, Ctx>) -> AppResult<Vec<mail_store::TaskList>> {
    mail_app::tasks::list_task_lists(&ctx)
}

/// `list_tasks`: the tasks in one list (`null`: every list).
#[tauri::command]
pub fn list_tasks(
    ctx: State<'_, Ctx>,
    list_id: Option<String>,
    include_completed: bool,
) -> AppResult<Vec<mail_store::Task>> {
    mail_app::tasks::list_tasks(&ctx, list_id, include_completed)
}

/// `save_task`: create or change a task; sent to its list's server on the
/// next sync.
#[tauri::command]
pub fn save_task(
    ctx: State<'_, Ctx>,
    task: mail_app::tasks::TaskInput,
) -> AppResult<mail_store::Task> {
    mail_app::tasks::save_task(&ctx, task)
}

/// `delete_task`: gone here at once, from the server on the next sync.
#[tauri::command]
pub fn delete_task(ctx: State<'_, Ctx>, id: String) -> AppResult<()> {
    mail_app::tasks::delete_task(&ctx, id)
}

/// `sync_tasks`: sync every account's task lists now.
#[tauri::command]
pub async fn sync_tasks(ctx: State<'_, Ctx>) -> AppResult<()> {
    mail_app::tasks::sync_tasks(&ctx).await
}

// ---------------------------------------------------------------- updates

/// `check_for_update`: the newer version this install can move to, if any.
#[tauri::command]
pub async fn check_for_update(
    ctx: State<'_, Ctx>,
) -> AppResult<Option<mail_app::updates::UpdateInfo>> {
    mail_app::updates::check_for_update(&ctx).await
}

/// `install_update`: download the newest installer, check it, run it.
/// On Windows the installer takes over and the app closes; on Linux apt
/// installs it (asking for the password) and the app restarts.
#[tauri::command]
pub async fn install_update(ctx: State<'_, Ctx>) -> AppResult<()> {
    mail_app::updates::install_update(&ctx).await
}
