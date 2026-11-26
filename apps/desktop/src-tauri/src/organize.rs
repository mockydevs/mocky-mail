//! Organising mail beyond moving it: copies, junk, colour categories,
//! favourite folders and the order of accounts.

use std::sync::Arc;

use mail_core::categories::{
    JUNK_KEYWORD, NOT_JUNK_KEYWORD, clean_color, clean_name, keyword_for, same_name,
};
use mail_core::folders::FolderRole;
use mail_store::Category;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::events;
use crate::mail::{
    MoveTarget, check_batch, enqueue_for, move_groups, notify_changed, resolve_target,
};
use crate::state::AppState;
use crate::sync::{self, Op};

/// `copy_messages`: the originals stay; the copies are made on the server
/// (now, or when back online) and appear when the target folder syncs.
#[tauri::command]
pub fn copy_messages(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    target: MoveTarget,
) -> AppResult<()> {
    check_batch(&ids)?;
    let groups = sync::group_by_folder(&state, &ids)?;
    for ((account_id, folder_id), messages) in &groups {
        let (target_folder, create_target) = resolve_target(&state, account_id, &target)?;
        if target_folder.id == *folder_id {
            continue;
        }
        let from = state.store.get_folder(folder_id)?.path;
        let to = target_folder.path;
        enqueue_for(
            &state,
            account_id,
            messages,
            |uids| Op::Copy {
                from: from.clone(),
                uids,
                to: to.clone(),
                create_target,
            },
            |message_id| Op::CopyByMessageId {
                from: from.clone(),
                message_id,
                to: to.clone(),
                create_target,
            },
        )?;
    }
    Ok(())
}

/// `mark_junk`: to the account's Junk folder marked `$Junk`, or (`junk`
/// false) back to the Inbox marked `$NotJunk`, so the server's filter
/// learns either way.
#[tauri::command]
pub fn mark_junk(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    junk: bool,
) -> AppResult<()> {
    check_batch(&ids)?;
    let groups = sync::group_by_folder(&state, &ids)?;
    let role = if junk {
        FolderRole::Junk
    } else {
        FolderRole::Inbox
    };
    // Every account must have the folder before anything changes.
    for (account_id, _) in groups.keys() {
        if !state
            .store
            .list_folders(account_id)?
            .iter()
            .any(|f| f.role == role)
        {
            let account = state.store.get_account(account_id)?;
            return Err(AppError::invalid(if junk {
                format!(
                    "{} has no Junk folder, so mail can't be moved there.",
                    account.email
                )
            } else {
                format!("{} has no Inbox to move this mail back to.", account.email)
            }));
        }
    }
    let (add, remove) = if junk {
        (JUNK_KEYWORD, NOT_JUNK_KEYWORD)
    } else {
        (NOT_JUNK_KEYWORD, JUNK_KEYWORD)
    };
    // The mark goes on before the move, and travels with the message.
    for ((account_id, folder_id), messages) in &groups {
        let folder = state.store.get_folder(folder_id)?.path;
        enqueue_for(
            &state,
            account_id,
            messages,
            |uids| Op::SetKeywords {
                folder: folder.clone(),
                uids,
                add: vec![add.to_owned()],
                remove: vec![remove.to_owned()],
            },
            |message_id| Op::SetKeywordsByMessageId {
                folder: folder.clone(),
                message_id,
                add: vec![add.to_owned()],
                remove: vec![remove.to_owned()],
            },
        )?;
    }
    let touched = move_groups(&state, groups, &MoveTarget::Role { role })?;
    notify_changed(&app, &touched);
    Ok(())
}

/// Queue the server side of a category change on these messages: Gmail
/// labels on Gmail, keywords elsewhere. Returns the folders they are in.
fn queue_category_change(
    state: &AppState,
    ids: &[String],
    add: &[String],
    remove: &[String],
) -> AppResult<Vec<(String, String)>> {
    if ids.is_empty() || (add.is_empty() && remove.is_empty()) {
        return Ok(Vec::new());
    }
    let groups = sync::group_by_folder(state, ids)?;
    for ((account_id, folder_id), messages) in &groups {
        let gmail = state.store.get_account(account_id)?.is_gmail();
        let folder = state.store.get_folder(folder_id)?.path;
        if gmail {
            enqueue_for(
                state,
                account_id,
                messages,
                |uids| Op::SetLabels {
                    folder: folder.clone(),
                    uids,
                    add: add.to_vec(),
                    remove: remove.to_vec(),
                },
                |message_id| Op::SetLabelsByMessageId {
                    folder: folder.clone(),
                    message_id,
                    add: add.to_vec(),
                    remove: remove.to_vec(),
                },
            )?;
        } else {
            let add: Vec<String> = add.iter().map(|n| keyword_for(n)).collect();
            let remove: Vec<String> = remove.iter().map(|n| keyword_for(n)).collect();
            enqueue_for(
                state,
                account_id,
                messages,
                |uids| Op::SetKeywords {
                    folder: folder.clone(),
                    uids,
                    add: add.clone(),
                    remove: remove.clone(),
                },
                |message_id| Op::SetKeywordsByMessageId {
                    folder: folder.clone(),
                    message_id,
                    add: add.clone(),
                    remove: remove.clone(),
                },
            )?;
        }
    }
    Ok(groups.into_keys().collect())
}

fn invalid_category(error: mail_store::StoreError) -> AppError {
    match error {
        mail_store::StoreError::Invalid(_) => {
            AppError::invalid("There's already a category with that name.")
        }
        mail_store::StoreError::NotFound(_) => {
            AppError::new("not_found", "That category no longer exists.")
        }
        other => other.into(),
    }
}

/// `list_categories`.
#[tauri::command]
pub fn list_categories(state: State<'_, Arc<AppState>>) -> AppResult<Vec<Category>> {
    Ok(state.store.list_categories()?)
}

/// `save_category`: add one, change its colour, or with `previousName`
/// rename it on every message (and on the server). Returns them all.
#[tauri::command]
pub fn save_category(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    category: Category,
    previous_name: Option<String>,
) -> AppResult<Vec<Category>> {
    let clean = Category {
        name: clean_name(&category.name).map_err(AppError::invalid)?,
        color: clean_color(&category.color).map_err(AppError::invalid)?,
    };
    let old_name = match previous_name.as_deref() {
        Some(previous) => state
            .store
            .list_categories()?
            .into_iter()
            .find(|c| same_name(&c.name, previous))
            .map(|c| c.name),
        None => None,
    };
    let changed = state
        .store
        .save_category(&clean, previous_name.as_deref())
        .map_err(invalid_category)?;
    if let Some(old_name) = old_name {
        let touched = queue_category_change(
            &state,
            &changed,
            std::slice::from_ref(&clean.name),
            std::slice::from_ref(&old_name),
        )?;
        notify_changed(&app, &touched);
    }
    Ok(state.store.list_categories()?)
}

/// `delete_category`: from every message too. Returns the rest.
#[tauri::command]
pub fn delete_category(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    name: String,
) -> AppResult<Vec<Category>> {
    let Some(stored) = state
        .store
        .list_categories()?
        .into_iter()
        .find(|c| same_name(&c.name, &name))
    else {
        return Err(AppError::new(
            "not_found",
            "That category no longer exists.",
        ));
    };
    let changed = state
        .store
        .delete_category(&stored.name)
        .map_err(invalid_category)?;
    let touched = queue_category_change(&state, &changed, &[], &[stored.name])?;
    notify_changed(&app, &touched);
    Ok(state.store.list_categories()?)
}

/// `set_categories`: add and remove categories on messages. Names not yet
/// known become categories with a palette colour.
#[tauri::command]
pub fn set_categories(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    add: Vec<String>,
    remove: Vec<String>,
) -> AppResult<()> {
    check_batch(&ids)?;
    let known = state.store.list_categories()?;
    // As the category is spelled, whatever case was passed.
    let spelled = |names: &[String]| -> AppResult<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for name in names {
            let name = clean_name(name).map_err(AppError::invalid)?;
            let name = known
                .iter()
                .find(|c| same_name(&c.name, &name))
                .map_or(name, |c| c.name.clone());
            if !out.iter().any(|n| same_name(n, &name)) {
                out.push(name);
            }
        }
        Ok(out)
    };
    let add = spelled(&add)?;
    let remove = spelled(&remove)?;
    state.store.ensure_categories(&add)?;
    let changed = state.store.set_categories(&ids, &add, &remove)?;
    let touched = queue_category_change(&state, &changed, &add, &remove)?;
    notify_changed(&app, &touched);
    Ok(())
}

/// `set_folder_favorite`: list it under Favourites (this device only).
#[tauri::command]
pub fn set_folder_favorite(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    folder_id: String,
    favorite: bool,
) -> AppResult<()> {
    state.store.set_folder_favorite(&folder_id, favorite)?;
    let folder = state.store.get_folder(&folder_id)?;
    events::changed(&app, &folder.account_id, std::slice::from_ref(&folder.id));
    Ok(())
}

/// `reorder_accounts`: every account id, in the new order.
#[tauri::command]
pub fn reorder_accounts(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
) -> AppResult<()> {
    state
        .store
        .reorder_accounts(&ids)
        .map_err(|error| match error {
            mail_store::StoreError::Invalid(_) => {
                AppError::invalid("List every account exactly once to reorder them.")
            }
            other => other.into(),
        })?;
    events::accounts_changed(&app);
    Ok(())
}
