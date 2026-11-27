//! Private keys at rest: sealed with AES-256-GCM under a 256-bit key that
//! lives in the system keyring (the same one passwords are kept in), never
//! in the database. Each sealed value is bound to what it is (its
//! fingerprint) so one can't be swapped for another.

use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use base64::Engine as _;
use rand_core::{OsRng, RngCore as _};
use zeroize::Zeroizing;

use crate::SyncError;
use crate::secrets::SecretStore;

/// The keyring entry holding the sealing key.
pub const VAULT_KEY: &str = "security:vault-key";

/// Version byte at the front of a sealed value.
const VERSION: u8 = 1;
const NONCE: usize = 12;

/// The key private keys are sealed with.
pub struct Vault {
    key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Vault")
    }
}

fn locked() -> SyncError {
    SyncError::Keyring("the key that protects your private keys can't be read".to_owned())
}

impl Vault {
    /// A vault around a key (tests; the app uses [`Vault::open`]).
    #[must_use]
    pub fn from_key(key: [u8; 32]) -> Self {
        Self {
            key: Zeroizing::new(key),
        }
    }

    /// The vault whose key is in `secrets`; with `create`, a new key is
    /// made and stored when there is none yet. `Ok(None)` when there is
    /// none and `create` is false (no private key was ever kept).
    ///
    /// # Errors
    /// When the keyring can't be used, or holds a damaged key.
    pub fn open(secrets: &dyn SecretStore, create: bool) -> Result<Option<Self>, SyncError> {
        if let Some(stored) = secrets.get(VAULT_KEY)? {
            let bytes = Zeroizing::new(
                base64::engine::general_purpose::STANDARD
                    .decode(stored.trim())
                    .map_err(|_| locked())?,
            );
            let key: [u8; 32] = bytes.as_slice().try_into().map_err(|_| locked())?;
            return Ok(Some(Self::from_key(key)));
        }
        if !create {
            return Ok(None);
        }
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key));
        secrets.set(VAULT_KEY, &encoded)?;
        Ok(Some(Self::from_key(key)))
    }

    /// Seal `plain`, bound to `label`.
    ///
    /// # Errors
    /// Only if encryption fails.
    pub fn seal(&self, label: &str, plain: &[u8]) -> Result<Vec<u8>, SyncError> {
        let cipher =
            aes_gcm::Aes256Gcm::new_from_slice(self.key.as_slice()).map_err(|_| locked())?;
        let mut nonce = [0u8; NONCE];
        OsRng.fill_bytes(&mut nonce);
        let sealed = cipher
            .encrypt(
                aes_gcm::Nonce::from_slice(&nonce),
                Payload {
                    msg: plain,
                    aad: label.as_bytes(),
                },
            )
            .map_err(|_| locked())?;
        let mut out = Vec::with_capacity(sealed.len().saturating_add(NONCE).saturating_add(1));
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// Open what [`Vault::seal`] sealed with the same `label`.
    ///
    /// # Errors
    /// When it was sealed with another key or label, or is damaged.
    pub fn open_sealed(&self, label: &str, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        let (&version, rest) = sealed.split_first().ok_or_else(locked)?;
        if version != VERSION || rest.len() < NONCE {
            return Err(locked());
        }
        let (nonce, body) = rest.split_at(NONCE);
        let cipher =
            aes_gcm::Aes256Gcm::new_from_slice(self.key.as_slice()).map_err(|_| locked())?;
        cipher
            .decrypt(
                aes_gcm::Nonce::from_slice(nonce),
                Payload {
                    msg: body,
                    aad: label.as_bytes(),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| locked())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[test]
    fn seals_and_opens_with_the_keyring_key() {
        let secrets = MemoryStore::default();
        assert!(Vault::open(&secrets, false).unwrap().is_none());
        let vault = Vault::open(&secrets, true).unwrap().unwrap();
        let sealed = vault.seal("AB12", b"private").unwrap();
        assert!(!sealed.windows(7).any(|w| w == b"private"));
        // The same key comes back from the keyring.
        let again = Vault::open(&secrets, false).unwrap().unwrap();
        assert_eq!(
            again.open_sealed("AB12", &sealed).unwrap().as_slice(),
            b"private"
        );
        // Bound to its label, and to the key.
        assert!(again.open_sealed("CD34", &sealed).is_err());
        assert!(
            Vault::from_key([7; 32])
                .open_sealed("AB12", &sealed)
                .is_err()
        );
        let mut damaged = sealed.clone();
        if let Some(last) = damaged.last_mut() {
            *last ^= 1;
        }
        assert!(again.open_sealed("AB12", &damaged).is_err());
        assert!(again.open_sealed("AB12", &[]).is_err());
    }
}
