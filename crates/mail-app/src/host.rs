//! What the application needs from the program it runs in: the desktop and
//! Android app (Tauri), and later the command line, the MCP server and the
//! browser version's server. Everything platform-specific goes through
//! here, so the logic in this crate is the same everywhere.
//!
//! Every method has a default: either "nothing to do" or an `invalid`
//! error saying it isn't available here, so a host implements only what it
//! can do.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{AppError, AppResult};

/// The error for something this host can't do.
#[must_use]
pub fn not_available(what: &str) -> AppError {
    AppError::invalid(format!("{what} is not available here."))
}

/// A file picker: which files it offers and how many may be picked.
#[derive(Debug, Clone, Copy)]
pub struct FilePick<'a> {
    /// The kind of file shown (`"Mail files"`) and its extensions; `None`
    /// offers any file.
    pub filter: Option<(&'a str, &'a [&'a str])>,
    /// Several files may be picked.
    pub many: bool,
    /// Largest file accepted where picking copies the file first (Android
    /// copies it into the cache; `too_large` past this).
    pub max_bytes: u64,
    /// The `invalid` message when a picked file isn't a file on this device
    /// (a desktop dialog can return a link instead).
    pub not_local: &'a str,
}

/// Where a file being saved goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveChoice {
    /// The user cancelled.
    Cancelled,
    /// Write the file here (a desktop save dialog).
    Path(PathBuf),
    /// Write it to the cache, then [`Host::save_from_cache`] shows the
    /// system's own "Save to" screen (Android).
    AfterWrite,
}

/// A window of its own showing part of the app (computers): a message, or
/// a message being written.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    /// Unique label (`message-…`, `compose-…`). An open window with this
    /// label is brought to the front instead.
    pub label: String,
    /// The app page it loads, with its query (`index.html?message=…`).
    pub url: String,
    /// The title.
    pub title: String,
    /// Width and height.
    pub size: (f64, f64),
    /// Smallest width and height.
    pub min_size: (f64, f64),
}

/// Google sign-in with the device's own accounts (Android).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceGoogleGrant {
    /// The Google account chosen on the device.
    pub email: String,
    /// Bearer token for IMAP and SMTP (XOAUTH2).
    pub access_token: String,
    /// The permissions this token carries, as Google listed them.
    #[serde(default)]
    pub granted_scopes: Vec<String>,
}

/// Google issues access tokens for about an hour; count a little less.
pub const DEVICE_GOOGLE_TOKEN_SECS: u64 = 50 * 60;

/// The program the app runs in.
#[async_trait]
pub trait Host: Send + Sync {
    /// Push an event to the interface (`docs/ipc.md`, Events).
    ///
    /// # Errors
    /// When it couldn't be sent; the caller logs it.
    fn emit(&self, event: &str, payload: serde_json::Value) -> AppResult<()>;

    /// Show a system notification (new mail, reminders). Failures are
    /// logged, not returned.
    fn notify(&self, title: &str, body: &str) {
        let _ = (title, body);
    }

    /// Open a web or mail link in the user's own apps.
    ///
    /// # Errors
    /// `internal` with the system's reason when it couldn't.
    fn open_url(&self, url: &str) -> AppResult<()> {
        let _ = url;
        Err(not_available("Opening links"))
    }

    /// Open a file in the app the device uses for its type.
    ///
    /// # Errors
    /// `invalid` when no app can open it.
    async fn open_file(&self, path: &Path, file_name: &str, mime_type: &str) -> AppResult<()> {
        let _ = (path, file_name, mime_type);
        Err(not_available("Opening files"))
    }

    /// Show a file picker. Empty when cancelled.
    ///
    /// # Errors
    /// `too_large`, or `invalid` (`pick.not_local`).
    async fn pick_files(&self, pick: FilePick<'_>) -> AppResult<Vec<PathBuf>> {
        let _ = pick;
        Err(not_available("Picking files"))
    }

    /// Ask where to save a file called `file_name`.
    ///
    /// # Errors
    /// `invalid` when the place picked can't be written to.
    async fn save_dialog(&self, file_name: &str) -> AppResult<SaveChoice> {
        let _ = file_name;
        Err(not_available("Saving files"))
    }

    /// After [`SaveChoice::AfterWrite`]: show the system's "Save to"
    /// screen for the file written at `path`. The name it was saved under,
    /// or `None` when cancelled.
    ///
    /// # Errors
    /// When the chosen place can't be written to.
    async fn save_from_cache(
        &self,
        path: &Path,
        file_name: &str,
        mime_type: &str,
    ) -> AppResult<Option<String>> {
        let _ = (path, file_name, mime_type);
        Err(not_available("Saving files"))
    }

    /// Print a page with the system's print screen (Android; computers
    /// print from the page itself).
    ///
    /// # Errors
    /// `invalid` where printing is done some other way.
    async fn print_html(&self, html: &str, title: &str) -> AppResult<()> {
        let _ = (html, title);
        Err(AppError::invalid(
            "Printing is done from the window on this device.",
        ))
    }

    /// This program's version (`major.minor.patch`), which updates compare
    /// against.
    fn app_version(&self) -> String;

    /// Where the app keeps its database.
    ///
    /// # Errors
    /// When the system has no such folder.
    fn data_dir(&self) -> AppResult<PathBuf> {
        Err(not_available("An app data folder"))
    }

    /// Where the app keeps temporary files.
    ///
    /// # Errors
    /// When the system has no such folder.
    fn cache_dir(&self) -> AppResult<PathBuf> {
        Err(not_available("An app cache folder"))
    }

    /// A computer: windows of their own, the tray, start at login and the
    /// system's `GnuPG` keys. `false` on phones.
    fn desktop(&self) -> bool {
        false
    }

    /// Installed from the Microsoft Store (MSIX), which updates it and
    /// starts it at login itself.
    fn packaged(&self) -> bool {
        false
    }

    /// Open (or bring to the front) a window of its own. Only called when
    /// [`Host::desktop`] is true.
    ///
    /// # Errors
    /// The system's reason, as text.
    fn open_window(&self, window: &WindowSpec) -> Result<(), String> {
        let _ = window;
        Err("windows are not available here".to_owned())
    }

    /// Whether the app starts at login, as the system has it.
    ///
    /// # Errors
    /// When the system can't say.
    fn start_at_login(&self) -> AppResult<bool> {
        Err(not_available("Start at login"))
    }

    /// Turn start at login on or off.
    ///
    /// # Errors
    /// `invalid` when the system refused.
    fn set_start_at_login(&self, on: bool) -> AppResult<()> {
        let _ = on;
        Err(not_available("Start at login"))
    }

    /// Close the program (an installer is taking over).
    fn exit(&self) {}

    /// Start the program again (an update was installed).
    fn restart(&self) {}

    /// Google accounts can be signed in with the device's own accounts
    /// (Android), instead of in the browser.
    fn device_google(&self) -> bool {
        false
    }

    /// Show the device's account chooser, then Google's consent sheet.
    /// `extra` are permissions beyond mail to ask for as well.
    ///
    /// # Errors
    /// `auth` when cancelled or declined; `connect` for other failures.
    async fn device_google_sign_in(&self, extra: &[String]) -> AppResult<DeviceGoogleGrant> {
        let _ = extra;
        Err(not_available("Google sign-in with this device's accounts"))
    }

    /// A fresh access token for an account signed in on the device, with
    /// no screen shown.
    ///
    /// # Errors
    /// `auth` when access was withdrawn.
    async fn device_google_token(&self, email: &str, extra: &[String]) -> AppResult<String> {
        let _ = (email, extra);
        Err(not_available("Google sign-in with this device's accounts"))
    }

    /// Ask for more permissions for an account on the device, showing
    /// Google's consent sheet.
    ///
    /// # Errors
    /// `auth` when cancelled or declined; `config` when not set up.
    async fn device_google_authorize(
        &self,
        email: &str,
        extra: &[String],
    ) -> AppResult<DeviceGoogleGrant> {
        let _ = (email, extra);
        Err(not_available("Google sign-in with this device's accounts"))
    }
}

/// A host that does nothing: events go nowhere and everything
/// platform-specific is unavailable. For tests and tools without an
/// interface.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHost;

impl Host for NoHost {
    fn emit(&self, _event: &str, _payload: serde_json::Value) -> AppResult<()> {
        Ok(())
    }

    fn app_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }
}
