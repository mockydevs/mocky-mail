//! Picking, saving and opening files on Android, through `FilesPlugin`
//! (Kotlin, in `gen/android`). Android's file screens return `content://`
//! links rather than paths, which the desktop code can't write to, so on
//! the phone these go through the system's own screens instead. Every file
//! passed in or returned lives in the app's cache.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tauri::Wry;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};

use crate::error::{AppError, AppResult};

static PLUGIN: OnceLock<PluginHandle<Wry>> = OnceLock::new();

/// Register the Kotlin plugin. Android only.
pub fn init() -> TauriPlugin<Wry> {
    Builder::new("files")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("ke.co.mocky.mail", "FilesPlugin")?;
            let _ = PLUGIN.set(handle);
            Ok(())
        })
        .build()
}

fn handle() -> AppResult<&'static PluginHandle<Wry>> {
    PLUGIN
        .get()
        .ok_or_else(|| AppError::internal("Files aren't available on this device."))
}

fn plugin_error(error: tauri::plugin::mobile::PluginInvokeError) -> AppError {
    use tauri::plugin::mobile::PluginInvokeError as E;
    match error {
        E::InvokeRejected(rejected) => {
            let message = rejected.message.unwrap_or_default();
            match rejected.code.as_deref() {
                Some("too_large") => AppError::new("too_large", message),
                Some("no_app") => AppError::invalid(message),
                _ => AppError::internal(message),
            }
        }
        other => AppError::internal(other.to_string()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PickArgs {
    max_bytes: u64,
}

/// A picked file, already copied into the app's cache.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Picked {
    /// Its copy in the cache.
    pub path: PathBuf,
}

#[derive(Deserialize)]
struct PickedList {
    files: Vec<Picked>,
}

/// Show the phone's file picker. An empty list when cancelled.
///
/// # Errors
/// `too_large` when a chosen file is bigger than `max_bytes`.
pub async fn pick_files(max_bytes: u64) -> AppResult<Vec<Picked>> {
    Ok(handle()?
        .run_mobile_plugin_async::<PickedList>("pickFiles", PickArgs { max_bytes })
        .await
        .map_err(plugin_error)?
        .files)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileArgs<'a> {
    path: &'a Path,
    file_name: &'a str,
    mime_type: &'a str,
}

#[derive(Deserialize)]
struct SaveResult {
    saved: bool,
    name: Option<String>,
}

/// Show the phone's "Save to" screen and copy `path` there. The name it was
/// saved under, or `None` if the user cancelled.
///
/// # Errors
/// When the chosen place can't be written to.
pub async fn save_file(path: &Path, file_name: &str, mime_type: &str) -> AppResult<Option<String>> {
    let result = handle()?
        .run_mobile_plugin_async::<SaveResult>(
            "saveFile",
            FileArgs {
                path,
                file_name,
                mime_type,
            },
        )
        .await
        .map_err(plugin_error)?;
    Ok(result
        .saved
        .then(|| result.name.unwrap_or_else(|| file_name.to_owned())))
}

/// Hand a cached file to another app (PDF reader, Word, Gallery).
///
/// # Errors
/// `invalid` when no installed app can open this type of file.
pub async fn open_file(path: &Path, file_name: &str, mime_type: &str) -> AppResult<()> {
    handle()?
        .run_mobile_plugin_async::<serde_json::Value>(
            "openFile",
            FileArgs {
                path,
                file_name,
                mime_type,
            },
        )
        .await
        .map_err(plugin_error)?;
    Ok(())
}
