//! Keys and certificates in the system's own stores, read so they can be
//! listed (and other people's used to check and encrypt): the Windows
//! "Personal" certificate store, and GnuPG's public keyring. Mocky Mail
//! signs and decrypts only with keys in its own store; a private key held
//! by Windows or gpg-agent has to be exported (.p12, .asc) and imported.

use std::path::PathBuf;

use mailcraft_openpgp::PublicKey;

/// A certificate in a system store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCert {
    /// The certificate (DER).
    pub der: Vec<u8>,
    /// The store has its private key.
    pub has_private_key: bool,
}

/// Most certificates or keys read from one store.
const MAX_ITEMS: usize = 500;
/// Largest keyring file read.
const MAX_KEYRING: u64 = 64 * 1024 * 1024;

/// The certificates in the current user's Personal ("MY") store that are
/// for email (they name an address). Empty elsewhere than Windows.
#[cfg(windows)]
#[must_use]
pub fn windows_personal_certificates() -> Vec<SystemCert> {
    let Ok(store) = schannel::cert_store::CertStore::open_current_user("MY") else {
        return Vec::new();
    };
    store
        .certs()
        .take(MAX_ITEMS)
        .filter_map(|cert| {
            let der = cert.to_der().to_vec();
            let info = super::smime::cert_info(&der).ok()?;
            if info.addresses.is_empty() || info.is_ca {
                return None;
            }
            // Asked silently: a smart card is never prompted for here.
            let has_private_key = cert
                .private_key()
                .silent(true)
                .compare_key(true)
                .acquire()
                .is_ok();
            Some(SystemCert {
                der,
                has_private_key,
            })
        })
        .collect()
}

/// The certificates in the current user's Personal store (Windows only).
#[cfg(not(windows))]
#[must_use]
pub fn windows_personal_certificates() -> Vec<SystemCert> {
    Vec::new()
}

/// GnuPG's home folder: `GNUPGHOME`, else `%APPDATA%\gnupg` on Windows and
/// `~/.gnupg` elsewhere. `None` on Android.
#[must_use]
pub fn gnupg_home() -> Option<PathBuf> {
    if cfg!(target_os = "android") {
        return None;
    }
    if let Some(home) = std::env::var_os("GNUPGHOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("gnupg"));
    }
    std::env::var_os("HOME").map(|dir| PathBuf::from(dir).join(".gnupg"))
}

/// The public keys in GnuPG's keyring (`pubring.kbx`, or the older
/// `pubring.gpg`), checked. Empty when there is none.
#[must_use]
pub fn gnupg_public_keys() -> Vec<PublicKey> {
    let Some(home) = gnupg_home() else {
        return Vec::new();
    };
    for name in ["pubring.kbx", "pubring.gpg"] {
        let path = home.join(name);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_KEYRING {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            return read_keyring(&bytes);
        }
    }
    Vec::new()
}

fn be32(bytes: &[u8], at: usize) -> Option<usize> {
    let four: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
    usize::try_from(u32::from_be_bytes(four)).ok()
}

/// The keys in a GnuPG keyring: a keybox (`pubring.kbx`: blobs, each with
/// one OpenPGP key block) or plain packets (`pubring.gpg`).
#[must_use]
pub fn read_keyring(bytes: &[u8]) -> Vec<PublicKey> {
    // A keybox starts with its header blob: type 1, then "KBXf" at 8.
    let is_keybox = bytes.get(4) == Some(&1) && bytes.get(8..12) == Some(b"KBXf".as_slice());
    if !is_keybox {
        return PublicKey::parse(bytes).unwrap_or_default();
    }
    let mut out = Vec::new();
    let mut at = 0usize;
    while out.len() < MAX_ITEMS {
        let Some(len) = be32(bytes, at) else { break };
        let Some(end) = at.checked_add(len) else {
            break;
        };
        let Some(blob) = bytes.get(at..end) else {
            break;
        };
        if len < 16 {
            break;
        }
        // Type 2: an OpenPGP key block at the offset and length the blob says.
        if blob.get(4) == Some(&2)
            && let (Some(offset), Some(size)) = (be32(blob, 8), be32(blob, 12))
            && let Some(block) = offset
                .checked_add(size)
                .and_then(|stop| blob.get(offset..stop))
        {
            out.extend(PublicKey::parse(block).unwrap_or_default());
        }
        at = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_keybox_blobs_and_plain_keyrings() {
        let key = mailcraft_openpgp::PublicKey::parse(include_bytes!("fixtures/baraka-pub.asc"))
            .unwrap_or_default()
            .remove(0);
        let block = key.to_bytes();
        // A header blob, then one OpenPGP blob holding the key block.
        let mut kbx = vec![0, 0, 0, 32, 1, 1, 0, 0];
        kbx.extend_from_slice(b"KBXf");
        kbx.resize(32, 0);
        let len = u32::try_from(16 + block.len()).unwrap_or_default();
        kbx.extend_from_slice(&len.to_be_bytes());
        kbx.extend_from_slice(&[2, 1, 0, 0]);
        kbx.extend_from_slice(&16u32.to_be_bytes());
        kbx.extend_from_slice(&u32::try_from(block.len()).unwrap_or_default().to_be_bytes());
        kbx.extend_from_slice(&block);
        let keys = read_keyring(&kbx);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].fingerprint(), key.fingerprint());
        assert_eq!(read_keyring(&block).len(), 1);
        assert!(read_keyring(b"nothing").is_empty());
        // A damaged length stops reading, it doesn't run past the end.
        let mut bad = kbx.clone();
        bad[32] = 0x7F;
        assert!(read_keyring(&bad).is_empty());
    }
}
