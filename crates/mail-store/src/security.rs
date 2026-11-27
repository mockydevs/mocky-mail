//! Certificates and keys for S/MIME and `OpenPGP` (0.10.0): the user's own,
//! with their private part sealed by the caller (it never reaches this
//! crate in the clear), and other people's, collected from mail or
//! imported. Also each account's chosen keys and preferred kind.

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{Result, StoreError};
use crate::ids::{new_id, to_i64};

/// S/MIME or `OpenPGP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyKind {
    /// X.509 certificates and CMS (RFC 8551).
    Smime,
    /// `OpenPGP` keys (RFC 9580, RFC 3156).
    Openpgp,
}

impl KeyKind {
    /// The stored and IPC name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Smime => "smime",
            Self::Openpgp => "openpgp",
        }
    }

    /// From the stored name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "smime" => Some(Self::Smime),
            "openpgp" => Some(Self::Openpgp),
            _ => None,
        }
    }
}

/// Where a key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOrigin {
    /// Imported from a file by the user.
    Import,
    /// From an `Autocrypt:` header of received mail.
    Autocrypt,
    /// Attached to received mail.
    Attachment,
    /// The certificate of a signed S/MIME message.
    Signed,
    /// Found by a Web Key Directory look-up.
    Wkd,
}

impl KeyOrigin {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Autocrypt => "autocrypt",
            Self::Attachment => "attachment",
            Self::Signed => "signed",
            Self::Wkd => "wkd",
        }
    }
}

/// A key to save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewKey {
    /// S/MIME or `OpenPGP`.
    pub kind: KeyKind,
    /// Upper-case hex: the certificate's SHA-256, or the `OpenPGP` primary
    /// key's fingerprint.
    pub fingerprint: String,
    /// The name on it.
    pub owner: String,
    /// Its addresses (lower-cased when saved).
    pub addresses: Vec<String>,
    /// Unix seconds; `None` when it doesn't expire.
    pub expires: Option<i64>,
    /// It can make signatures.
    pub can_sign: bool,
    /// It can be encrypted to.
    pub can_encrypt: bool,
    /// The certificate (DER) or public key (binary `OpenPGP`).
    pub public_data: Vec<u8>,
    /// S/MIME: the issuer certificates that came with it (DER).
    pub chain: Vec<Vec<u8>>,
    /// The private part, sealed by the caller; `None` for other people's.
    pub secret: Option<Vec<u8>>,
    /// Where it came from.
    pub origin: KeyOrigin,
}

/// A saved key, without its private part.
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the interface's SecurityKey"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredKey {
    /// Opaque id.
    pub id: String,
    /// S/MIME or `OpenPGP`.
    pub kind: KeyKind,
    /// Upper-case hex fingerprint.
    pub fingerprint: String,
    /// The name on it.
    pub owner: String,
    /// Its addresses, lower case.
    pub addresses: Vec<String>,
    /// Unix seconds; `None` when it doesn't expire.
    pub expires: Option<i64>,
    /// It can make signatures.
    pub can_sign: bool,
    /// It can be encrypted to.
    pub can_encrypt: bool,
    /// The certificate (DER) or public key (binary `OpenPGP`).
    pub public_data: Vec<u8>,
    /// S/MIME issuer certificates (DER).
    pub chain: Vec<Vec<u8>>,
    /// Has a private part: it is the user's own.
    pub is_own: bool,
    /// The user said it really is its owner's.
    pub trusted: bool,
    /// Unix seconds it was first saved.
    pub added_at: i64,
}

/// An account's chosen keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountKeys {
    /// The S/MIME key it signs and encrypts to itself with.
    pub smime: Option<String>,
    /// The `OpenPGP` key.
    pub openpgp: Option<String>,
    /// The kind the user chose to prefer, if they did.
    pub preferred: Option<KeyKind>,
}

/// Largest key or certificate kept (1 MiB).
pub const MAX_KEY_BYTES: usize = 1024 * 1024;
/// Most addresses kept for one key.
const MAX_ADDRESSES: usize = 64;

fn encode_chain(chain: &[Vec<u8>]) -> Result<String> {
    let encoded: Vec<String> = chain.iter().map(|der| hex(der)).collect();
    Ok(serde_json::to_string(&encoded)?)
}

fn decode_chain(json: &str) -> Result<Vec<Vec<u8>>> {
    let encoded: Vec<String> = serde_json::from_str(json)?;
    encoded
        .iter()
        .map(|h| unhex(h).ok_or(StoreError::Corrupt("certificate chain")))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i.checked_add(2)?)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect()
}

const KEY_COLUMNS: &str = "k.id, k.kind, k.fingerprint, k.owner, k.expires, k.can_sign, \
     k.can_encrypt, k.public_data, k.chain_json, k.secret IS NOT NULL, k.trusted, k.added_at, \
     (SELECT coalesce(json_group_array(a.address), '[]') FROM security_key_addresses a \
      WHERE a.key_id = k.id)";

#[allow(clippy::struct_excessive_bools, reason = "one field per column")]
struct KeyRow {
    id: String,
    kind: String,
    fingerprint: String,
    owner: String,
    expires: Option<i64>,
    can_sign: bool,
    can_encrypt: bool,
    public_data: Vec<u8>,
    chain_json: String,
    is_own: bool,
    trusted: bool,
    added_at: i64,
    addresses_json: String,
}

impl KeyRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            kind: row.get(1)?,
            fingerprint: row.get(2)?,
            owner: row.get(3)?,
            expires: row.get(4)?,
            can_sign: row.get(5)?,
            can_encrypt: row.get(6)?,
            public_data: row.get(7)?,
            chain_json: row.get(8)?,
            is_own: row.get(9)?,
            trusted: row.get(10)?,
            added_at: row.get(11)?,
            addresses_json: row.get(12)?,
        })
    }

    fn into_key(self) -> Result<StoredKey> {
        let mut addresses: Vec<String> = serde_json::from_str(&self.addresses_json)?;
        addresses.sort();
        Ok(StoredKey {
            id: self.id,
            kind: KeyKind::parse(&self.kind).ok_or(StoreError::Corrupt("key kind"))?,
            fingerprint: self.fingerprint,
            owner: self.owner,
            addresses,
            expires: self.expires,
            can_sign: self.can_sign,
            can_encrypt: self.can_encrypt,
            public_data: self.public_data,
            chain: decode_chain(&self.chain_json)?,
            is_own: self.is_own,
            trusted: self.trusted,
            added_at: self.added_at,
        })
    }
}

fn clean_addresses(addresses: &[String]) -> Vec<String> {
    let mut out: Vec<String> = addresses
        .iter()
        .map(|a| a.trim().to_lowercase())
        .filter(|a| a.contains('@') && a.len() <= 320)
        .collect();
    out.sort();
    out.dedup();
    out.truncate(MAX_ADDRESSES);
    out
}

impl Store {
    /// Save a key, or update the saved one with the same kind and
    /// fingerprint, returning its id. An update keeps the trusted mark,
    /// and keeps the private part unless `key` brings one; addresses are
    /// added to those known.
    ///
    /// # Errors
    /// [`StoreError::Invalid`] for an empty fingerprint or a key over
    /// [`MAX_KEY_BYTES`]; database errors.
    pub fn save_key(&self, key: &NewKey, now: u64) -> Result<String> {
        let fingerprint = key.fingerprint.trim().to_ascii_uppercase();
        if fingerprint.is_empty() || fingerprint.len() > 128 {
            return Err(StoreError::Invalid("key fingerprint"));
        }
        let chain_len: usize = key.chain.iter().map(Vec::len).sum();
        if key.public_data.is_empty()
            || key.public_data.len().saturating_add(chain_len) > MAX_KEY_BYTES
        {
            return Err(StoreError::Invalid("key size"));
        }
        let now = to_i64(now, "time")?;
        let chain = encode_chain(&key.chain)?;
        let owner: String = key.owner.chars().take(256).collect();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM security_keys WHERE kind = ?1 AND fingerprint = ?2",
                params![key.kind.as_str(), fingerprint],
                |row| row.get(0),
            )
            .optional()?;
        let id = if let Some(id) = existing {
            tx.execute(
                "UPDATE security_keys SET owner = ?2, expires = ?3, can_sign = ?4,
                     can_encrypt = ?5, public_data = ?6,
                     chain_json = CASE WHEN ?7 = '[]' THEN chain_json ELSE ?7 END,
                     secret = coalesce(?8, secret), updated_at = ?9
                 WHERE id = ?1",
                params![
                    id,
                    owner,
                    key.expires,
                    key.can_sign,
                    key.can_encrypt,
                    key.public_data,
                    chain,
                    key.secret,
                    now
                ],
            )?;
            id
        } else {
            let id = new_id()?;
            tx.execute(
                "INSERT INTO security_keys (id, kind, fingerprint, owner, expires, can_sign,
                     can_encrypt, public_data, chain_json, secret, trusted, origin, added_at,
                     updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?12, ?12)",
                params![
                    id,
                    key.kind.as_str(),
                    fingerprint,
                    owner,
                    key.expires,
                    key.can_sign,
                    key.can_encrypt,
                    key.public_data,
                    chain,
                    key.secret,
                    key.origin.as_str(),
                    now
                ],
            )?;
            id
        };
        for address in clean_addresses(&key.addresses) {
            tx.execute(
                "INSERT OR IGNORE INTO security_key_addresses (key_id, address) VALUES (?1, ?2)",
                params![id, address],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Every saved key: the user's own first, then by name.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_keys(&self) -> Result<Vec<StoredKey>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {KEY_COLUMNS} FROM security_keys k
             ORDER BY k.secret IS NULL, lower(k.owner), k.fingerprint"
        ))?;
        let rows = stmt.query_map([], KeyRow::read)?;
        rows.map(|row| row?.into_key()).collect()
    }

    /// One key.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id.
    pub fn get_key(&self, id: &str) -> Result<StoredKey> {
        self.conn()
            .query_row(
                &format!("SELECT {KEY_COLUMNS} FROM security_keys k WHERE k.id = ?1"),
                params![id],
                KeyRow::read,
            )
            .optional()?
            .ok_or(StoreError::NotFound("key"))?
            .into_key()
    }

    /// Keys of one kind for an address (any case), newest first.
    ///
    /// # Errors
    /// Database errors.
    pub fn keys_for_address(&self, kind: KeyKind, address: &str) -> Result<Vec<StoredKey>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {KEY_COLUMNS} FROM security_keys k
             WHERE k.kind = ?1 AND k.id IN
                 (SELECT key_id FROM security_key_addresses WHERE address = ?2)
             ORDER BY k.updated_at DESC, k.added_at DESC"
        ))?;
        let rows = stmt.query_map(
            params![kind.as_str(), address.trim().to_lowercase()],
            KeyRow::read,
        )?;
        rows.map(|row| row?.into_key()).collect()
    }

    /// The sealed private part of a key, if it is the user's own.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id.
    pub fn key_secret(&self, id: &str) -> Result<Option<Vec<u8>>> {
        self.conn()
            .query_row(
                "SELECT secret FROM security_keys WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound("key"))
    }

    /// Forget a key. Accounts that chose it no longer have one of its kind.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id.
    pub fn delete_key(&self, id: &str) -> Result<()> {
        let changed = self
            .conn()
            .execute("DELETE FROM security_keys WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(StoreError::NotFound("key"));
        }
        Ok(())
    }

    /// Mark a key as really its owner's, or not.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id.
    pub fn set_key_trusted(&self, id: &str, trusted: bool) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE security_keys SET trusted = ?2 WHERE id = ?1",
            params![id, trusted],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound("key"));
        }
        Ok(())
    }

    /// The fingerprints of the keys of one kind the user trusts.
    ///
    /// # Errors
    /// Database errors.
    pub fn trusted_fingerprints(&self, kind: KeyKind) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT fingerprint FROM security_keys WHERE kind = ?1 AND trusted = 1",
        )?;
        let rows = stmt.query_map(params![kind.as_str()], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    }

    /// An account's chosen keys (none chosen: all `None`).
    ///
    /// # Errors
    /// Database errors.
    pub fn account_keys(&self, account_id: &str) -> Result<AccountKeys> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = self
            .conn()
            .query_row(
                "SELECT smime_key, openpgp_key, preferred FROM account_security
                 WHERE account_id = ?1",
                params![account_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(
            row.map_or_else(AccountKeys::default, |(smime, openpgp, preferred)| {
                AccountKeys {
                    smime,
                    openpgp,
                    preferred: preferred.as_deref().and_then(KeyKind::parse),
                }
            }),
        )
    }

    /// Choose the key an account signs and encrypts to itself with (`None`:
    /// none of that kind). The key must be the user's own and of that kind.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown account or key;
    /// [`StoreError::Invalid`] for someone else's key or another kind.
    pub fn set_account_key(
        &self,
        account_id: &str,
        kind: KeyKind,
        key_id: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM accounts WHERE id = ?1)",
            params![account_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound("account"));
        }
        if let Some(key_id) = key_id {
            let (stored_kind, own): (String, bool) = tx
                .query_row(
                    "SELECT kind, secret IS NOT NULL FROM security_keys WHERE id = ?1",
                    params![key_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .ok_or(StoreError::NotFound("key"))?;
            if stored_kind != kind.as_str() || !own {
                return Err(StoreError::Invalid("key"));
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO account_security (account_id) VALUES (?1)",
            params![account_id],
        )?;
        let sql = match kind {
            KeyKind::Smime => "UPDATE account_security SET smime_key = ?2 WHERE account_id = ?1",
            KeyKind::Openpgp => {
                "UPDATE account_security SET openpgp_key = ?2 WHERE account_id = ?1"
            }
        };
        tx.execute(sql, params![account_id, key_id])?;
        tx.commit()?;
        Ok(())
    }

    /// The kind an account prefers when a message doesn't say.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown account.
    pub fn set_preferred_kind(&self, account_id: &str, kind: KeyKind) -> Result<()> {
        let conn = self.conn();
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM accounts WHERE id = ?1)",
            params![account_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound("account"));
        }
        conn.execute(
            "INSERT INTO account_security (account_id, preferred) VALUES (?1, ?2)
             ON CONFLICT (account_id) DO UPDATE SET preferred = excluded.preferred",
            params![account_id, kind.as_str()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::test_support::add_account;

    struct Fixture {
        store: Store,
        account: String,
    }

    fn fixture() -> Fixture {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "me@x.example");
        Fixture { store, account }
    }

    fn key(kind: KeyKind, fingerprint: &str, address: &str, secret: Option<&[u8]>) -> NewKey {
        NewKey {
            kind,
            fingerprint: fingerprint.to_owned(),
            owner: "Wanjiru Kamau".to_owned(),
            addresses: vec![address.to_owned()],
            expires: Some(2_000_000_000),
            can_sign: true,
            can_encrypt: true,
            public_data: vec![1, 2, 3],
            chain: vec![vec![4, 5]],
            secret: secret.map(<[u8]>::to_vec),
            origin: KeyOrigin::Import,
        }
    }

    #[test]
    fn keys_are_saved_once_per_fingerprint_and_keep_their_trust() {
        let f = fixture();
        let id = f
            .store
            .save_key(
                &key(KeyKind::Openpgp, "ab12", "Wanjiru@Fitout.example", None),
                10,
            )
            .unwrap();
        f.store.set_key_trusted(&id, true).unwrap();
        // The same key again (from another message): same row, still trusted,
        // and the new address is added.
        let again = f
            .store
            .save_key(
                &key(KeyKind::Openpgp, "AB12", "wk@fitout.example", None),
                20,
            )
            .unwrap();
        assert_eq!(again, id);
        let stored = f.store.get_key(&id).unwrap();
        assert!(stored.trusted);
        assert!(!stored.is_own);
        assert_eq!(stored.fingerprint, "AB12");
        assert_eq!(
            stored.addresses,
            ["wanjiru@fitout.example", "wk@fitout.example"]
        );
        assert_eq!(stored.chain, vec![vec![4u8, 5]]);
        // The same fingerprint as S/MIME is another key.
        let smime = f
            .store
            .save_key(
                &key(KeyKind::Smime, "AB12", "wk@fitout.example", Some(b"sealed")),
                30,
            )
            .unwrap();
        assert_ne!(smime, id);
        assert_eq!(
            f.store
                .keys_for_address(KeyKind::Openpgp, "WK@fitout.example")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(f.store.list_keys().unwrap()[0].id, smime);
        assert_eq!(
            f.store.key_secret(&smime).unwrap().as_deref(),
            Some(&b"sealed"[..])
        );
        // A copy without the private part doesn't lose it.
        f.store
            .save_key(&key(KeyKind::Smime, "AB12", "wk@fitout.example", None), 40)
            .unwrap();
        assert!(f.store.key_secret(&smime).unwrap().is_some());
        assert_eq!(
            f.store.trusted_fingerprints(KeyKind::Openpgp).unwrap(),
            ["AB12"]
        );
        f.store.delete_key(&id).unwrap();
        assert!(matches!(f.store.get_key(&id), Err(StoreError::NotFound(_))));
        assert!(
            f.store
                .keys_for_address(KeyKind::Openpgp, "wk@fitout.example")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn accounts_choose_their_own_keys_only() {
        let f = fixture();
        let own = f
            .store
            .save_key(&key(KeyKind::Smime, "01", "me@x.example", Some(b"s")), 1)
            .unwrap();
        let theirs = f
            .store
            .save_key(&key(KeyKind::Smime, "02", "them@x.example", None), 1)
            .unwrap();
        assert_eq!(
            f.store.account_keys(&f.account).unwrap(),
            AccountKeys::default()
        );
        assert!(matches!(
            f.store
                .set_account_key(&f.account, KeyKind::Smime, Some(&theirs)),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            f.store
                .set_account_key(&f.account, KeyKind::Openpgp, Some(&own)),
            Err(StoreError::Invalid(_))
        ));
        f.store
            .set_account_key(&f.account, KeyKind::Smime, Some(&own))
            .unwrap();
        f.store
            .set_preferred_kind(&f.account, KeyKind::Openpgp)
            .unwrap();
        let chosen = f.store.account_keys(&f.account).unwrap();
        assert_eq!(chosen.smime.as_deref(), Some(own.as_str()));
        assert_eq!(chosen.preferred, Some(KeyKind::Openpgp));
        // Removing the key unsets it.
        f.store.delete_key(&own).unwrap();
        assert_eq!(f.store.account_keys(&f.account).unwrap().smime, None);
        f.store
            .set_account_key(&f.account, KeyKind::Smime, None)
            .unwrap();
        assert!(matches!(
            f.store.set_preferred_kind("nobody", KeyKind::Smime),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn oversized_and_unnamed_keys_are_refused() {
        let f = fixture();
        let mut big = key(KeyKind::Openpgp, "AA", "a@x.example", None);
        big.public_data = vec![0; MAX_KEY_BYTES + 1];
        assert!(matches!(
            f.store.save_key(&big, 1),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            f.store
                .save_key(&key(KeyKind::Openpgp, " ", "a@x.example", None), 1),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(unhex(&hex(&[0, 255, 16])), Some(vec![0, 255, 16]));
        assert_eq!(unhex("abc"), None);
    }
}
