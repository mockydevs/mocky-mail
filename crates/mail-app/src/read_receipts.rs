//! Read receipts others ask for: offered when a message is opened (or, with
//! Settings `readReceipts: 'always'`, sent then by themselves), answered
//! once, and sent through the Outbox as an RFC 8098 report.

use mail_core::folders::FolderRole;
use mail_core::message::{Mailbox, make_preview};
use mail_core::outbox::OutboxItem;
use mail_core::receipts::{ReceiptRequest, requested};
use mail_store::{Account, NewOutbox, StoredMessage, read_receipt_key};
use mail_sync::outgoing::build_read_receipt;
use serde::Serialize;

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::state::{AppState, now};
use crate::{events, sync};

/// `MessageDetail.readReceipt`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReceiptView {
    /// Where the receipt would go.
    requested_by: String,
    /// Sent or declined here already.
    answered: bool,
}

/// The request in a message, if the user should be asked about it: not in
/// Sent, Drafts or Junk, not from or to the account itself.
fn eligible(
    state: &AppState,
    account: &Account,
    stored: &StoredMessage,
    raw: &[u8],
) -> AppResult<Option<ReceiptRequest>> {
    let Some(request) = requested(raw) else {
        return Ok(None);
    };
    let role = state.store.get_folder(&stored.item.folder_id)?.role;
    let mine = |address: &str| address.eq_ignore_ascii_case(&account.email);
    let from_me = stored.item.from.as_ref().is_some_and(|f| mine(&f.address));
    if matches!(
        role,
        FolderRole::Sent | FolderRole::Drafts | FolderRole::Junk
    ) || from_me
        || mine(&request.address)
    {
        return Ok(None);
    }
    Ok(Some(request))
}

/// Queue the receipt in the Outbox, once: the answer is recorded first,
/// and taken back if the receipt can't be queued.
fn send(
    ctx: &Ctx,
    state: &AppState,
    account: &Account,
    stored: &StoredMessage,
    request: &ReceiptRequest,
    automatic: bool,
) -> AppResult<()> {
    let key = read_receipt_key(stored.message_id_header.as_deref(), &stored.item.id);
    if !state
        .store
        .answer_read_receipt(&account.id, &key, true, now())?
    {
        return Ok(());
    }
    if let Err(error) = queue(state, account, stored, request, automatic) {
        let _ = state.store.forget_read_receipt(&account.id, &key);
        return Err(error);
    }
    state.outbox_wake.notify_one();
    events::outbox_changed(ctx, state);
    Ok(())
}

/// Build the receipt and put it in the Outbox, to go at once.
fn queue(
    state: &AppState,
    account: &Account,
    stored: &StoredMessage,
    request: &ReceiptRequest,
    automatic: bool,
) -> AppResult<()> {
    let receipt = mail_core::receipts::Receipt {
        final_recipient: &account.email,
        original_message_id: stored.message_id_header.as_deref(),
        subject: &stored.item.subject,
        automatic,
    };
    let from = Mailbox {
        name: Some(account.display_name.clone()).filter(|n| !n.trim().is_empty()),
        address: account.email.clone(),
    };
    let built = build_read_receipt(&from, &request.address, &receipt)?;
    let created_at = now();
    state.store.insert_outbox(&NewOutbox {
        account_id: account.id.clone(),
        created_at,
        to: vec![Mailbox {
            name: None,
            address: request.address.clone(),
        }],
        subject: mail_core::receipts::subject(&stored.item.subject),
        preview: make_preview(
            &mail_core::receipts::note(&receipt),
            mail_core::message::PREVIEW_CHARS,
        ),
        raw: built.raw,
        recipients: built.recipients,
        message_id_header: Some(built.message_id),
        reply_to_message_id: None,
        sent_folder_hint: crate::outbox::folder_path(state, &account.id, FolderRole::Sent)?,
        item: OutboxItem::scheduled(created_at, 0),
    })?;
    Ok(())
}

/// For `get_message`: the read receipt the message asks for, or `None`
/// (none asked for, not one to answer, or Settings say never). With
/// `always`, an unanswered one is sent now, when the message's
/// `Return-Path` is at the same domain as the address (otherwise it is
/// offered as with `ask`, as RFC 8098 advises).
///
/// # Errors
/// Database errors.
pub fn for_message(
    ctx: &Ctx,
    state: &AppState,
    stored: &StoredMessage,
    raw: &[u8],
) -> AppResult<Option<ReadReceiptView>> {
    let mode = crate::settings::load(state)?.read_receipts;
    if mode == "never" {
        return Ok(None);
    }
    let account = state.store.get_account(&stored.item.account_id)?;
    let Some(request) = eligible(state, &account, stored, raw)? else {
        return Ok(None);
    };
    let key = read_receipt_key(stored.message_id_header.as_deref(), &stored.item.id);
    let mut answered = state
        .store
        .read_receipt_answer(&account.id, &key)?
        .is_some();
    if !answered && mode == "always" && request.return_path_matches {
        match send(ctx, state, &account, stored, &request, true) {
            Ok(()) => answered = true,
            Err(error) => eprintln!("couldn't queue a read receipt: {error}"),
        }
    }
    Ok(Some(ReadReceiptView {
        requested_by: request.address,
        answered,
    }))
}

/// `answer_read_receipt`: send (`send: true`) or decline the read receipt
/// a message asks for. Either way it is not asked about again; answering
/// twice does nothing more.
pub async fn answer_read_receipt(ctx: &Ctx, message_id: String, send: bool) -> AppResult<()> {
    let state = &ctx.state;
    let raw = sync::body_now(state, &message_id).await?;
    let stored = state.store.get_message(&message_id)?;
    let account = state.store.get_account(&stored.item.account_id)?;
    let request = eligible(state, &account, &stored, &raw)?
        .ok_or_else(|| AppError::invalid("This message doesn't ask for a read receipt."))?;
    if send {
        self::send(ctx, state, &account, &stored, &request, false)
    } else {
        let key = read_receipt_key(stored.message_id_header.as_deref(), &stored.item.id);
        state
            .store
            .answer_read_receipt(&account.id, &key, false, now())?;
        Ok(())
    }
}
