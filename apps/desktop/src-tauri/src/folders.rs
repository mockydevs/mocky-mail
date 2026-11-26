//! Folder management: create, rename and delete the user's own folders on
//! the server. Built-in folders (Inbox, Sent, Drafts, Archive, Spam,
//! Trash) can't be renamed or deleted here. These need a connection; the
//! folder list is refreshed from the server straight after.

use std::sync::Arc;

use mail_core::folders::{FolderRole, clean_folder_name, encode_folder_name};
use mail_store::{Account, Folder};
use mail_sync::imap::ImapConnection;
use tauri::{AppHandle, State};

use crate::convert::to_sync;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::{credentials, events, sync};

/// The level separator the server uses, from any folder that says.
pub(crate) fn delimiter_of(folders: &[Folder]) -> Option<String> {
    folders
        .iter()
        .find_map(|f| f.delimiter.clone())
        .filter(|d| !d.is_empty())
}

/// Where a new top-level folder goes. Servers that keep every folder under
/// Inbox (Courier, some Dovecot setups: `INBOX.Clients`) get it there too.
pub(crate) fn top_level_prefix(folders: &[Folder], delimiter: Option<&str>) -> String {
    let Some(d) = delimiter else {
        return String::new();
    };
    let under_inbox = format!("INBOX{d}");
    let others: Vec<&Folder> = folders
        .iter()
        .filter(|f| f.role != FolderRole::Inbox)
        .collect();
    if !others.is_empty()
        && others
            .iter()
            .all(|f| f.path.to_ascii_uppercase().starts_with(&under_inbox))
    {
        under_inbox
    } else {
        String::new()
    }
}

/// The server path for a folder named `name` inside `parent` (or at the top).
fn new_path(folders: &[Folder], parent: Option<&Folder>, name: &str) -> AppResult<String> {
    let delimiter = delimiter_of(folders);
    let name = clean_folder_name(name, delimiter.as_deref()).map_err(AppError::invalid)?;
    let encoded = encode_folder_name(&name);
    Ok(match parent {
        Some(parent) => {
            let d = parent.delimiter.clone().or(delimiter).ok_or_else(|| {
                AppError::invalid("This server doesn't allow folders inside folders.")
            })?;
            format!("{}{d}{encoded}", parent.path)
        }
        None => format!(
            "{}{encoded}",
            top_level_prefix(folders, delimiter.as_deref())
        ),
    })
}

fn own_folder(folder: &Folder) -> AppResult<()> {
    if folder.role == FolderRole::Other {
        Ok(())
    } else {
        Err(AppError::invalid(format!(
            "{} is a built-in folder, so it can't be renamed or deleted.",
            folder.name
        )))
    }
}

async fn connect(state: &AppState, account: &Account) -> AppResult<ImapConnection> {
    let credentials = credentials::for_account(state, account).await?;
    Ok(ImapConnection::connect(&to_sync(&account.imap), &credentials).await?)
}

/// After a change: re-read the folder list and tell the interface.
async fn finish(
    app: &AppHandle,
    state: &AppState,
    account: &Account,
    mut conn: ImapConnection,
) -> AppResult<()> {
    let result = sync::refresh_folders(state, account, &mut conn).await;
    conn.logout().await;
    result?;
    events::changed(app, &account.id, &[]);
    Ok(())
}

/// `create_folder`: a new folder, at the top or inside `parentId`.
#[tauri::command]
pub async fn create_folder(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    account_id: String,
    name: String,
    parent_id: Option<String>,
) -> AppResult<()> {
    let account = state.store.get_account(&account_id)?;
    let folders = state.store.list_folders(&account_id)?;
    let parent = match parent_id {
        Some(id) => Some(state.store.get_folder(&id)?).filter(|f| f.account_id == account_id),
        None => None,
    };
    let path = new_path(&folders, parent.as_ref(), &name)?;
    if folders.iter().any(|f| f.path.eq_ignore_ascii_case(&path)) {
        return Err(AppError::invalid(
            "There's already a folder with that name here.",
        ));
    }
    let mut conn = connect(&state, &account).await?;
    conn.create_folder(&path).await?;
    finish(&app, &state, &account, conn).await
}

/// `rename_folder`: rename one of the user's own folders (it stays where it is).
#[tauri::command]
pub async fn rename_folder(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    folder_id: String,
    name: String,
) -> AppResult<()> {
    let folder = state.store.get_folder(&folder_id)?;
    own_folder(&folder)?;
    let account = state.store.get_account(&folder.account_id)?;
    let folders = state.store.list_folders(&account.id)?;
    let delimiter = folder.delimiter.clone().or_else(|| delimiter_of(&folders));
    let name = clean_folder_name(&name, delimiter.as_deref()).map_err(AppError::invalid)?;
    let parent_path = delimiter
        .as_deref()
        .and_then(|d| {
            folder
                .path
                .rsplit_once(d)
                .map(|(parent, _)| format!("{parent}{d}"))
        })
        .unwrap_or_default();
    let path = format!("{parent_path}{}", encode_folder_name(&name));
    if path == folder.path {
        return Ok(());
    }
    if folders.iter().any(|f| f.path.eq_ignore_ascii_case(&path)) {
        return Err(AppError::invalid(
            "There's already a folder with that name here.",
        ));
    }
    let mut conn = connect(&state, &account).await?;
    conn.rename_folder(&folder.path, &path).await?;
    finish(&app, &state, &account, conn).await
}

/// `delete_folder`: delete one of the user's own folders and the mail in it.
/// A folder with subfolders must be emptied of them first.
#[tauri::command]
pub async fn delete_folder(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    folder_id: String,
) -> AppResult<()> {
    let folder = state.store.get_folder(&folder_id)?;
    own_folder(&folder)?;
    let account = state.store.get_account(&folder.account_id)?;
    let folders = state.store.list_folders(&account.id)?;
    if let Some(d) = folder.delimiter.as_deref().filter(|d| !d.is_empty()) {
        let inside = format!("{}{d}", folder.path);
        if folders.iter().any(|f| f.path.starts_with(&inside)) {
            return Err(AppError::invalid(format!(
                "{} has folders inside it. Delete or move those first.",
                folder.name
            )));
        }
    }
    let mut conn = connect(&state, &account).await?;
    conn.delete_folder(&folder.path).await?;
    finish(&app, &state, &account, conn).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str, role: FolderRole, delimiter: &str) -> Folder {
        Folder {
            id: path.to_owned(),
            account_id: "a".to_owned(),
            path: path.to_owned(),
            name: path.rsplit(delimiter).next().unwrap_or(path).to_owned(),
            role,
            delimiter: Some(delimiter.to_owned()),
            uidvalidity: None,
            highest_uid: None,
            unread: 0,
            total: 0,
            favorite: false,
        }
    }

    #[test]
    fn new_folders_follow_the_server_layout() {
        let dovecot = vec![
            folder("INBOX", FolderRole::Inbox, "/"),
            folder("Sent", FolderRole::Sent, "/"),
        ];
        assert_eq!(
            new_path(&dovecot, None, "Clients").ok(),
            Some("Clients".to_owned())
        );
        let clients = folder("Clients", FolderRole::Other, "/");
        assert_eq!(
            new_path(&dovecot, Some(&clients), "Café").ok(),
            Some("Clients/Caf&AOk-".to_owned())
        );

        let courier = vec![
            folder("INBOX", FolderRole::Inbox, "."),
            folder("INBOX.Sent", FolderRole::Sent, "."),
            folder("INBOX.Trash", FolderRole::Trash, "."),
        ];
        assert_eq!(
            new_path(&courier, None, "Clients").ok(),
            Some("INBOX.Clients".to_owned())
        );
        assert!(new_path(&courier, None, "a.b").is_err());
    }

    #[test]
    fn built_in_folders_are_protected() {
        assert!(own_folder(&folder("Sent", FolderRole::Sent, "/")).is_err());
        assert!(own_folder(&folder("Clients", FolderRole::Other, "/")).is_ok());
    }
}
