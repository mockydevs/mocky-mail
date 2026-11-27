//! [`TauriHost`]: what `mail-app` needs from the desktop and Android app.
//! Events go to the webview, dialogs and notifications are the system's
//! (Tauri's plugins on computers, `files_android` and `google_android` on
//! the phone), and windows, the tray's start at login and updates are the
//! desktop app's.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
#[cfg(target_os = "android")]
use mail_app::host::DeviceGoogleGrant;
#[cfg(desktop)]
use mail_app::host::WindowSpec;
use mail_app::host::{FilePick, Host, SaveChoice};
use mail_app::{AppError, AppResult};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// The Tauri app as `mail-app`'s host.
#[derive(Clone)]
pub struct TauriHost {
    app: AppHandle,
}

impl TauriHost {
    /// The host for this app.
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

/// Linux notification servers read the body as simple HTML (links, images,
/// bold), and the subject comes from whoever sent the email, so `<`, `>` and
/// `&` are escaped there. Other systems show the text as written.
fn notification_body(text: &str) -> String {
    if cfg!(all(unix, not(target_os = "macos"))) {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    } else {
        text.to_owned()
    }
}

/// The application id in `packaging/msix/AppxManifest.xml.in`.
#[cfg(windows)]
const STORE_APPLICATION_ID: &str = "MockyMail";

/// When running from the Microsoft Store package, the id Windows shows
/// notifications under (`<package family>!MockyMail`). `None` for the
/// regular installer, where the notification plugin already works.
#[cfg(windows)]
fn store_app_id() -> Option<String> {
    let family = windows::ApplicationModel::Package::Current()
        .and_then(|package| package.Id())
        .and_then(|id| id.FamilyName())
        .ok()?;
    Some(format!("{family}!{STORE_APPLICATION_ID}"))
}

/// Running from a Microsoft Store (MSIX) package.
fn packaged() -> bool {
    #[cfg(windows)]
    {
        store_app_id().is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Run a blocking file dialog off the async runtime's threads.
#[cfg(not(target_os = "android"))]
async fn blocking<T: Send + 'static>(dialog: impl FnOnce() -> T + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(dialog)
        .await
        .map_err(|e| AppError::internal(e.to_string()))
}

#[async_trait]
impl Host for TauriHost {
    fn emit(&self, event: &str, payload: serde_json::Value) -> AppResult<()> {
        self.app
            .emit(event, payload)
            .map_err(|e| AppError::internal(e.to_string()))
    }

    fn notify(&self, title: &str, body: &str) {
        #[cfg(windows)]
        if let Some(app_id) = store_app_id() {
            if let Err(error) = tauri_winrt_notification::Toast::new(&app_id)
                .title(title)
                .text1(body)
                .show()
            {
                eprintln!("couldn't show notification: {error}");
            }
            return;
        }
        if let Err(error) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(notification_body(body))
            .show()
        {
            eprintln!("couldn't show notification: {error}");
        }
    }

    fn open_url(&self, url: &str) -> AppResult<()> {
        // The opener plugin works on every platform, Android included.
        self.app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| AppError::internal(e.to_string()))
    }

    #[cfg(target_os = "android")]
    async fn open_file(&self, path: &Path, file_name: &str, mime_type: &str) -> AppResult<()> {
        crate::files_android::open_file(path, file_name, mime_type).await
    }

    #[cfg(not(target_os = "android"))]
    async fn open_file(&self, path: &Path, _file_name: &str, _mime_type: &str) -> AppResult<()> {
        self.app
            .opener()
            .open_path(path.display().to_string(), None::<&str>)
            .map_err(|_| {
                AppError::invalid("No app on this computer can open this file. Save it instead.")
            })
    }

    /// Android: the phone's picker copies each file into the app's cache.
    #[cfg(target_os = "android")]
    async fn pick_files(&self, pick: FilePick<'_>) -> AppResult<Vec<PathBuf>> {
        let picked = crate::files_android::pick_files(pick.max_bytes).await?;
        let mut paths: Vec<PathBuf> = picked.into_iter().map(|p| p.path).collect();
        if !pick.many {
            paths.truncate(1);
        }
        Ok(paths)
    }

    #[cfg(not(target_os = "android"))]
    async fn pick_files(&self, pick: FilePick<'_>) -> AppResult<Vec<PathBuf>> {
        use tauri_plugin_dialog::DialogExt;
        let mut dialog = self.app.dialog().file();
        if let Some((name, extensions)) = pick.filter {
            dialog = dialog.add_filter(name, extensions);
        }
        let many = pick.many;
        let picked = blocking(move || {
            if many {
                dialog.blocking_pick_files().unwrap_or_default()
            } else {
                dialog.blocking_pick_file().into_iter().collect()
            }
        })
        .await?;
        picked
            .into_iter()
            .map(|file| {
                file.into_path()
                    .map_err(|_| AppError::invalid(pick.not_local))
            })
            .collect()
    }

    /// Android: the file is written to the cache first; the phone's "Save
    /// to" screen comes after.
    #[cfg(target_os = "android")]
    async fn save_dialog(&self, _file_name: &str) -> AppResult<SaveChoice> {
        Ok(SaveChoice::AfterWrite)
    }

    #[cfg(not(target_os = "android"))]
    async fn save_dialog(&self, file_name: &str) -> AppResult<SaveChoice> {
        use tauri_plugin_dialog::DialogExt;
        let dialog = self.app.dialog().file().set_file_name(file_name);
        let Some(path) = blocking(move || dialog.blocking_save_file()).await? else {
            return Ok(SaveChoice::Cancelled);
        };
        let path = path.into_path().map_err(|_| {
            AppError::invalid("That place can't be saved to. Pick a folder on this computer.")
        })?;
        Ok(SaveChoice::Path(path))
    }

    #[cfg(target_os = "android")]
    async fn save_from_cache(
        &self,
        path: &Path,
        file_name: &str,
        mime_type: &str,
    ) -> AppResult<Option<String>> {
        crate::files_android::save_file(path, file_name, mime_type).await
    }

    #[cfg(target_os = "android")]
    async fn print_html(&self, html: &str, title: &str) -> AppResult<()> {
        crate::files_android::print_html(html, title).await
    }

    fn app_version(&self) -> String {
        self.app.package_info().version.to_string()
    }

    fn data_dir(&self) -> AppResult<PathBuf> {
        self.app
            .path()
            .app_data_dir()
            .map_err(|e| AppError::internal(e.to_string()))
    }

    fn cache_dir(&self) -> AppResult<PathBuf> {
        self.app
            .path()
            .app_cache_dir()
            .map_err(|e| AppError::internal(e.to_string()))
    }

    fn desktop(&self) -> bool {
        cfg!(desktop)
    }

    fn packaged(&self) -> bool {
        packaged()
    }

    #[cfg(desktop)]
    fn open_window(&self, window: &WindowSpec) -> Result<(), String> {
        use tauri::{WebviewUrl, WebviewWindowBuilder};

        let failed = |e: tauri::Error| e.to_string();
        if let Some(open) = self.app.get_webview_window(&window.label) {
            let _ = open.unminimize();
            open.show().map_err(failed)?;
            return open.set_focus().map_err(failed);
        }
        let url = WebviewUrl::App(window.url.clone().into());
        crate::guarded(
            WebviewWindowBuilder::new(&self.app, window.label.clone(), url),
            &self.app,
        )
        .title(&window.title)
        .inner_size(window.size.0, window.size.1)
        .min_inner_size(window.min_size.0, window.min_size.1)
        .build()
        .map_err(failed)?;
        Ok(())
    }

    #[cfg(desktop)]
    fn start_at_login(&self) -> AppResult<bool> {
        use tauri_plugin_autostart::ManagerExt;
        self.app
            .autolaunch()
            .is_enabled()
            .map_err(|e| AppError::internal(e.to_string()))
    }

    #[cfg(desktop)]
    fn set_start_at_login(&self, on: bool) -> AppResult<()> {
        use tauri_plugin_autostart::ManagerExt;
        let launcher = self.app.autolaunch();
        let result = if on {
            launcher.enable()
        } else {
            launcher.disable()
        };
        result.map_err(|e| AppError::invalid(format!("Couldn't change start at login: {e}")))
    }

    fn exit(&self) {
        self.app.exit(0);
    }

    fn restart(&self) {
        self.app.restart();
    }

    fn device_google(&self) -> bool {
        cfg!(target_os = "android")
    }

    #[cfg(target_os = "android")]
    async fn device_google_sign_in(&self, extra: &[String]) -> AppResult<DeviceGoogleGrant> {
        crate::google_android::sign_in(extra).await
    }

    #[cfg(target_os = "android")]
    async fn device_google_token(&self, email: &str, extra: &[String]) -> AppResult<String> {
        crate::google_android::token(email, extra).await
    }

    #[cfg(target_os = "android")]
    async fn device_google_authorize(
        &self,
        email: &str,
        extra: &[String],
    ) -> AppResult<DeviceGoogleGrant> {
        crate::google_android::authorize(email, extra).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_cannot_add_markup_to_a_linux_notification() {
        let body = notification_body(
            r#"Invoice <a href="https://x.example">pay</a> & <img src="https://t.example/p">"#,
        );
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert!(!body.contains('<') && !body.contains('>'), "{body}");
            assert!(body.starts_with("Invoice &lt;a href="), "{body}");
            assert!(body.contains("&amp; &lt;img"), "{body}");
        } else {
            assert!(body.contains("<a href="));
        }
    }
}
