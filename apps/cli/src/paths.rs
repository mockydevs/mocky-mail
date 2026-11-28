//! Where the running app's automation endpoint and secret are, the same
//! way the app works them out (`mail_app::automation`): its data folder is
//! the system's per-user data folder plus the app's identifier.

use std::path::{Path, PathBuf};

/// The app's identifier (`tauri.conf.json`), the name of its data folder.
pub const IDENTIFIER: &str = "ke.co.mocky.mail";
/// The secret's file in the data folder.
pub const TOKEN_FILE: &str = "automation-token";
/// The Unix socket in the data folder.
pub const SOCKET_FILE: &str = "automation.sock";

/// The app's data folder: `%APPDATA%\ke.co.mocky.mail` on Windows,
/// `~/Library/Application Support/ke.co.mocky.mail` on macOS, and
/// `$XDG_DATA_HOME/ke.co.mocky.mail` (or `~/.local/share/…`) elsewhere.
#[must_use]
pub fn data_dir() -> Option<PathBuf> {
    data_dir_from(&|name| std::env::var_os(name).filter(|v| !v.is_empty()))
}

fn data_dir_from(env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(env("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(env("HOME")?).join("Library/Application Support")
    } else {
        match env("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env("HOME")?).join(".local/share"),
        }
    };
    Some(base.join(IDENTIFIER))
}

/// `\\.\pipe\mocky-mail-<user>`, as the app names it.
#[must_use]
pub fn pipe_name(user: &str) -> String {
    let clean: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(64)
        .collect();
    let clean = if clean.is_empty() {
        "user".into()
    } else {
        clean
    };
    format!(r"\\.\pipe\mocky-mail-{}", clean.to_ascii_lowercase())
}

/// The endpoint: `MOCKY_MAIL_ENDPOINT`, else the user's pipe on Windows or
/// the socket in the data folder elsewhere.
#[must_use]
pub fn default_endpoint(data_dir: Option<&Path>) -> Option<String> {
    if let Some(endpoint) = std::env::var("MOCKY_MAIL_ENDPOINT")
        .ok()
        .filter(|v| !v.is_empty())
    {
        return Some(endpoint);
    }
    if cfg!(windows) {
        Some(pipe_name(&std::env::var("USERNAME").unwrap_or_default()))
    } else {
        data_dir.map(|dir| dir.join(SOCKET_FILE).to_string_lossy().into_owned())
    }
}

/// The secret's file: `MOCKY_MAIL_TOKEN_FILE`, else the one in the data
/// folder.
#[must_use]
pub fn default_token_file(data_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(file) = std::env::var_os("MOCKY_MAIL_TOKEN_FILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(file));
    }
    data_dir.map(|dir| dir.join(TOKEN_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_is_the_apps() {
        let env = |name: &str| -> Option<std::ffi::OsString> {
            match name {
                "APPDATA" => Some(r"C:\Users\don\AppData\Roaming".into()),
                "HOME" => Some("/home/don".into()),
                _ => None,
            }
        };
        let dir = data_dir_from(&env).unwrap();
        assert!(dir.ends_with(IDENTIFIER));
        if cfg!(windows) {
            assert!(dir.starts_with(r"C:\Users\don\AppData\Roaming"));
        } else if cfg!(target_os = "macos") {
            assert!(dir.starts_with("/home/don/Library/Application Support"));
        } else {
            assert!(dir.starts_with("/home/don/.local/share"));
        }
    }

    #[test]
    fn pipe_names_match_the_apps() {
        assert_eq!(pipe_name("Don Juan"), r"\\.\pipe\mocky-mail-donjuan");
        assert_eq!(pipe_name(""), r"\\.\pipe\mocky-mail-user");
    }
}
