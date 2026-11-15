//! New-version check for the desktop app.
//!
//! Each release publishes `latest.json` next to the installers on the
//! download site. The app reads it at start-up and every few hours; when a
//! newer version exists the interface offers to update. Updating downloads the
//! installer for this kind of install, checks its SHA-256 against the
//! manifest, and runs it (Windows: the installer; Linux .deb: apt through the
//! system's password prompt). Installs that update themselves (Google Play,
//! the Microsoft Store) are left to their store.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

/// Where releases publish the manifest. A build can point elsewhere (a test
/// server) with `MOCKY_MAIL_UPDATE_URL` at compile time.
const MANIFEST_URL: &str = match option_env!("MOCKY_MAIL_UPDATE_URL") {
    Some(url) => url,
    None => "https://mailer.co.ke/downloads/latest.json",
};

/// Largest installer accepted.
const MAX_INSTALLER_BYTES: u64 = 300 * 1024 * 1024;
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(15);

/// The published manifest (`latest.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// The newest version, `major.minor.patch`.
    pub version: String,
    /// What's new, shown in the prompt.
    #[serde(default)]
    pub notes: String,
    /// Installers by kind: `windows-nsis`, `linux-deb`.
    #[serde(default)]
    pub installers: std::collections::HashMap<String, Installer>,
}

/// One downloadable installer.
#[derive(Debug, Clone, Deserialize)]
pub struct Installer {
    /// HTTPS download address.
    pub url: String,
    /// SHA-256 of the file, hex.
    pub sha256: String,
}

/// How this copy of the app gets updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "each platform builds only the kinds it can be")]
enum InstallKind {
    WindowsInstaller,
    LinuxDeb,
    /// Something we can't update in place (a binary copied by hand, an
    /// AppImage): point to the download page.
    Manual,
    /// Updated by its store.
    Store,
}

impl InstallKind {
    const fn installer_key(self) -> Option<&'static str> {
        match self {
            Self::WindowsInstaller => Some("windows-nsis"),
            Self::LinuxDeb => Some("linux-deb"),
            Self::Manual | Self::Store => None,
        }
    }
}

fn install_kind() -> InstallKind {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        InstallKind::Store
    }
    #[cfg(windows)]
    {
        if crate::events::running_from_store() {
            InstallKind::Store
        } else {
            InstallKind::WindowsInstaller
        }
    }
    #[cfg(target_os = "linux")]
    {
        // The .deb puts the app under /usr; anything else was copied by hand.
        let deb = std::env::current_exe().is_ok_and(|exe| exe.starts_with("/usr/"));
        if deb {
            InstallKind::LinuxDeb
        } else {
            InstallKind::Manual
        }
    }
    #[cfg(target_os = "macos")]
    {
        InstallKind::Manual
    }
}

/// `major.minor.patch` as numbers; anything after a `-` or `+` is ignored.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let core = version
        .trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
        parts.next().flatten().unwrap_or(0),
    );
    Some(version)
}

/// Whether `latest` is newer than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// What the interface shows when an update exists.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: String,
    /// `install` (the app can download and run the installer), or `download`
    /// (open the download page).
    how: &'static str,
    /// The download page, for `download`.
    page: String,
}

fn download_page() -> String {
    MANIFEST_URL
        .rsplit_once("/downloads/")
        .map_or_else(|| MANIFEST_URL.to_owned(), |(site, _)| format!("{site}/"))
}

fn update_for(manifest: &Manifest, current: &str, kind: InstallKind) -> Option<UpdateInfo> {
    if kind == InstallKind::Store || !is_newer(&manifest.version, current) {
        return None;
    }
    let installable = kind
        .installer_key()
        .is_some_and(|key| manifest.installers.contains_key(key));
    Some(UpdateInfo {
        version: manifest.version.trim().trim_start_matches('v').to_owned(),
        notes: manifest.notes.trim().to_owned(),
        how: if installable { "install" } else { "download" },
        page: download_page(),
    })
}

fn client(timeout: Duration) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .https_only(true)
        .user_agent(concat!("MockyMail/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::internal(format!("Couldn't check for updates: {e}")))
}

async fn fetch_manifest() -> AppResult<Manifest> {
    let unavailable =
        |e: reqwest::Error| AppError::new("connect", format!("Couldn't check for updates: {e}"));
    client(MANIFEST_TIMEOUT)?
        .get(MANIFEST_URL)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(unavailable)?
        .json::<Manifest>()
        .await
        .map_err(unavailable)
}

/// `check_for_update`: the newer version this install can move to, if any.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> AppResult<Option<UpdateInfo>> {
    let kind = install_kind();
    if kind == InstallKind::Store {
        return Ok(None);
    }
    let manifest = fetch_manifest().await?;
    Ok(update_for(
        &manifest,
        &app.package_info().version.to_string(),
        kind,
    ))
}

/// Hex SHA-256 of `data`.
fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The downloaded installer must be exactly the published one.
fn verify(data: &[u8], expected: &str) -> AppResult<()> {
    if sha256_hex(data).eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(AppError::invalid(
            "The downloaded update didn't match the published checksum, so it wasn't installed. Try again later.",
        ))
    }
}

async fn download(installer: &Installer) -> AppResult<Vec<u8>> {
    if !installer.url.starts_with("https://") {
        return Err(AppError::invalid(
            "The update address isn't secure, so it wasn't downloaded.",
        ));
    }
    let failed =
        |e: reqwest::Error| AppError::new("connect", format!("Couldn't download the update: {e}"));
    let response = client(DOWNLOAD_TIMEOUT)?
        .get(&installer.url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(failed)?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_INSTALLER_BYTES)
    {
        return Err(AppError::invalid(
            "The update is larger than expected, so it wasn't downloaded.",
        ));
    }
    let data = response.bytes().await.map_err(failed)?;
    if u64::try_from(data.len()).unwrap_or(u64::MAX) > MAX_INSTALLER_BYTES {
        return Err(AppError::invalid(
            "The update is larger than expected, so it wasn't downloaded.",
        ));
    }
    verify(&data, &installer.sha256)?;
    Ok(data.to_vec())
}

fn file_name(url: &str) -> String {
    let name = url.rsplit('/').next().unwrap_or("mocky-mail-update");
    mail_core::message::safe_file_name(name)
}

/// `install_update`: download the newest installer, check it, run it.
/// On Windows the installer takes over and the app closes; on Linux apt
/// installs it (asking for the password) and the app restarts.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> AppResult<()> {
    let kind = install_kind();
    let manifest = fetch_manifest().await?;
    let current = app.package_info().version.to_string();
    if !is_newer(&manifest.version, &current) {
        return Err(AppError::invalid("You already have the newest version."));
    }
    let installer = kind
        .installer_key()
        .and_then(|key| manifest.installers.get(key))
        .ok_or_else(|| {
            AppError::invalid("This copy can't update itself; download the new version instead.")
        })?;
    let data = download(installer).await?;
    let dir: PathBuf = app
        .path()
        .app_cache_dir()
        .map_err(|e| AppError::internal(e.to_string()))?
        .join("updates");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(file_name(&installer.url));
    tokio::fs::write(&path, &data).await?;
    run_installer(&app, kind, &path).await
}

async fn run_installer(
    app: &AppHandle,
    kind: InstallKind,
    path: &std::path::Path,
) -> AppResult<()> {
    match kind {
        InstallKind::WindowsInstaller => {
            // The installer replaces the app's files, so the app must not be running.
            std::process::Command::new(path)
                .spawn()
                .map_err(|e| AppError::internal(format!("Couldn't start the update: {e}")))?;
            app.exit(0);
            Ok(())
        }
        InstallKind::LinuxDeb => {
            let status = tokio::process::Command::new("pkexec")
                .args(["apt-get", "install", "-y", "--allow-downgrades"])
                .arg(path)
                .status()
                .await
                .map_err(|_| {
                    AppError::invalid(
                        "Couldn't ask for your password to install the update. Install it from the download page instead.",
                    )
                })?;
            if !status.success() {
                return Err(AppError::invalid(
                    "The update wasn't installed (the password prompt was closed, or apt failed). Try again, or install it from the download page.",
                ));
            }
            app.restart();
        }
        InstallKind::Manual | InstallKind::Store => Err(AppError::invalid(
            "This copy can't update itself; download the new version instead.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, installers: &[&str]) -> Manifest {
        Manifest {
            version: version.to_owned(),
            notes: " Faster sync. ".to_owned(),
            installers: installers
                .iter()
                .map(|key| {
                    (
                        (*key).to_owned(),
                        Installer {
                            url: format!("https://mailer.co.ke/downloads/{key}"),
                            sha256: "00".to_owned(),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("0.2.0", "0.2.1"));
        assert!(!is_newer("garbage", "0.2.1"));
        assert!(is_newer("0.3", "0.2.9"));
    }

    #[test]
    fn offers_the_installer_for_this_kind_of_install() {
        let m = manifest("0.3.0", &["windows-nsis", "linux-deb"]);
        let deb = update_for(&m, "0.2.1", InstallKind::LinuxDeb).unwrap();
        assert_eq!(deb.how, "install");
        assert_eq!(deb.version, "0.3.0");
        assert_eq!(deb.notes, "Faster sync.");
        assert_eq!(
            update_for(&m, "0.2.1", InstallKind::WindowsInstaller)
                .unwrap()
                .how,
            "install"
        );
        // A copy we can't update in place is pointed at the download page.
        let manual = update_for(&m, "0.2.1", InstallKind::Manual).unwrap();
        assert_eq!(manual.how, "download");
        assert_eq!(manual.page, "https://mailer.co.ke/");
        // No installer of our kind published yet: download page too.
        let only_windows = manifest("0.3.0", &["windows-nsis"]);
        assert_eq!(
            update_for(&only_windows, "0.2.1", InstallKind::LinuxDeb)
                .unwrap()
                .how,
            "download"
        );
    }

    #[test]
    fn nothing_when_current_or_updated_by_a_store() {
        let m = manifest("0.3.0", &["windows-nsis"]);
        assert_eq!(update_for(&m, "0.3.0", InstallKind::WindowsInstaller), None);
        assert_eq!(update_for(&m, "0.2.1", InstallKind::Store), None);
    }

    #[test]
    fn only_the_published_installer_is_run() {
        let data = b"installer bytes";
        let good = sha256_hex(data);
        assert!(verify(data, &good).is_ok());
        assert!(verify(data, &good.to_uppercase()).is_ok());
        assert!(verify(b"tampered bytes", &good).is_err());
    }

    #[test]
    fn the_manifest_parses() {
        let json = r#"{"version":"0.3.0","notes":"x","installers":{"linux-deb":{"url":"https://mailer.co.ke/downloads/a.deb","sha256":"ab"}}}"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert!(m.installers.contains_key("linux-deb"));
    }
}
