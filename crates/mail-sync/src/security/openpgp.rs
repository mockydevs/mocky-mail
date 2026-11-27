//! OpenPGP (RFC 9580, PGP/MIME RFC 3156): keys read and checked, PGP/MIME
//! and inline signatures checked (`mailcraft-openpgp`, which caps every
//! size and catches rPGP's panics), and with rPGP itself: detached
//! signatures made, messages encrypted to keys and decrypted with the
//! user's own, and Autocrypt headers written.

use mailcraft_openpgp::{CantCheck, KeyRing, Outcome, PublicKey};
use pgp::composed::{
    ArmorOptions, Deserializable as _, DetachedSignature, Message, MessageBuilder, SignedPublicKey,
    SignedSecretKey,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::SignatureType;
use pgp::ser::Serialize as _;
use pgp::types::{KeyDetails as _, Password};
use rand_core::OsRng;
use zeroize::Zeroizing;

use super::{
    SecurityKind, SignatureCheck, SignatureResult, SignerDetails, clean_text, same_address,
};
use crate::SyncError;

/// Largest OpenPGP message decrypted (the size of the largest mail).
pub const MAX_ENCRYPTED: usize = mail_core::message::MAX_MESSAGE_BYTES;

/// What a key is, for the key list.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each is a separate fact about the key"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// The primary key's fingerprint, upper-case hex.
    pub fingerprint: String,
    /// The name of its first User ID (or its address).
    pub owner: String,
    /// Its addresses, lower case.
    pub addresses: Vec<String>,
    /// Unix seconds; `None` when it doesn't expire.
    pub expires: Option<i64>,
    /// It has a key that may sign.
    pub can_sign: bool,
    /// It has a key mail can be encrypted to.
    pub can_encrypt: bool,
    /// Its owner revoked it.
    pub revoked: bool,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The name in a User ID such as `Wanjiru Kamau <wk@example.com>`.
fn user_name(id: &str) -> Option<String> {
    let name = id.split('<').next().unwrap_or_default();
    let name = clean_text(name.trim().trim_matches('"'));
    (!name.is_empty() && !name.contains('@')).then_some(name)
}

/// The first User ID's name.
fn owner_of(key: &SignedPublicKey) -> Option<String> {
    key.details
        .users
        .iter()
        .find_map(|user| user.id.as_str().and_then(user_name))
}

/// A key from bytes rPGP can use (a key `mailcraft-openpgp` checked).
fn rpgp_public(key: &PublicKey) -> Option<SignedPublicKey> {
    SignedPublicKey::from_bytes(key.to_bytes().as_slice()).ok()
}

/// Whether a binding signature is current at `now`.
fn not_expired(created: u32, sig: &pgp::packet::Signature, now: u64) -> bool {
    sig.key_expiration_time()
        .is_none_or(|valid| u64::from(created).saturating_add(u64::from(valid.as_secs())) > now)
}

/// The newest binding signature of a subkey, and whether it was revoked.
fn binding(signatures: &[pgp::packet::Signature]) -> (Option<&pgp::packet::Signature>, bool) {
    let revoked = signatures
        .iter()
        .any(|s| s.typ() == Some(SignatureType::SubkeyRevocation));
    let newest = signatures
        .iter()
        .filter(|s| s.typ() == Some(SignatureType::SubkeyBinding))
        .max_by_key(|s| s.created().map(pgp::types::Timestamp::as_secs));
    (newest, revoked)
}

/// The primary key's newest self-signature on a User ID (its key flags).
fn primary_self_signature(key: &SignedPublicKey) -> Option<&pgp::packet::Signature> {
    key.details
        .users
        .iter()
        .flat_map(|u| u.signatures.iter())
        .chain(key.details.direct_signatures.iter())
        .max_by_key(|s| s.created().map(pgp::types::Timestamp::as_secs))
}

/// What to encrypt to: the newest subkey for encryption that is valid at
/// `now`, else the primary key if it may encrypt.
enum Recipient<'a> {
    Subkey(&'a pgp::composed::SignedPublicSubKey),
    Primary(&'a pgp::packet::PublicKey),
}

fn encryption_key(key: &SignedPublicKey, now: u64) -> Option<Recipient<'_>> {
    let sub = key
        .public_subkeys
        .iter()
        .filter(|sub| sub.key.algorithm().can_encrypt())
        .filter_map(|sub| {
            let (sig, revoked) = binding(&sub.signatures);
            let sig = sig?;
            let flags = sig.key_flags();
            (!revoked
                && (flags.encrypt_comms() || flags.encrypt_storage())
                && not_expired(sub.key.created_at().as_secs(), sig, now))
            .then_some(sub)
        })
        .max_by_key(|sub| sub.key.created_at().as_secs());
    if let Some(sub) = sub {
        return Some(Recipient::Subkey(sub));
    }
    let primary_ok = key.primary_key.algorithm().can_encrypt()
        && primary_self_signature(key).is_none_or(|s| {
            let flags = s.key_flags();
            flags.encrypt_comms() || flags.encrypt_storage()
        });
    primary_ok.then_some(Recipient::Primary(&key.primary_key))
}

/// Whether mail can be encrypted to this key now.
#[must_use]
pub fn can_encrypt_to(key: &PublicKey, now: u64) -> bool {
    !key.revoked()
        && key.expires().is_none_or(|end| end > now)
        && rpgp_public(key).is_some_and(|k| encryption_key(&k, now).is_some())
}

/// What a checked public key is.
#[must_use]
pub fn key_info(key: &PublicKey, can_sign: Option<bool>) -> KeyInfo {
    let now = now_secs();
    let rpgp = rpgp_public(key);
    let owner = rpgp
        .as_ref()
        .and_then(owner_of)
        .or_else(|| key.addresses().first().cloned())
        .unwrap_or_else(|| key.fingerprint().grouped());
    let signs = can_sign.unwrap_or_else(|| {
        rpgp.as_ref().is_some_and(|k| {
            let primary = k.primary_key.algorithm().can_sign()
                && primary_self_signature(k).is_none_or(|s| s.key_flags().sign());
            primary
                || k.public_subkeys.iter().any(|sub| {
                    let (sig, revoked) = binding(&sub.signatures);
                    !revoked && sig.is_some_and(|s| s.key_flags().sign())
                })
        })
    });
    KeyInfo {
        fingerprint: key.fingerprint().to_hex(),
        owner,
        addresses: key.addresses().to_vec(),
        expires: key.expires().and_then(|e| i64::try_from(e).ok()),
        can_sign: signs && !key.revoked(),
        can_encrypt: can_encrypt_to(key, now),
        revoked: key.revoked(),
    }
}

/// Public keys in a file or attachment (armoured or binary), each checked.
///
/// # Errors
/// [`SyncError::Invalid`] when none can be read.
pub fn read_public_keys(bytes: &[u8]) -> Result<Vec<PublicKey>, SyncError> {
    PublicKey::parse(bytes).map_err(|_| {
        SyncError::Invalid("This file has no OpenPGP key that can be read.".to_owned())
    })
}

/// Whether the bytes hold a secret key.
#[must_use]
pub fn holds_secret(bytes: &[u8]) -> bool {
    mailcraft_openpgp::secret::holds_secret(bytes)
}

/// The user's own key, read from a file.
pub struct OwnKey {
    /// The key, its secret parts no longer protected by the passphrase
    /// (to be sealed before it is stored).
    pub secret: SignedSecretKey,
    /// Its public half, checked.
    pub public: PublicKey,
}

impl std::fmt::Debug for OwnKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnKey")
            .field("fingerprint", &self.public.fingerprint().to_hex())
            .finish_non_exhaustive()
    }
}

impl OwnKey {
    /// The secret key as binary packets, for sealing.
    ///
    /// # Errors
    /// When it can't be written.
    pub fn secret_bytes(&self) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        self.secret
            .to_bytes()
            .map(Zeroizing::new)
            .map_err(|_| SyncError::Invalid("The secret key can't be saved.".to_owned()))
    }
}

/// Read a secret key (armoured or binary), unlocking it with `passphrase`
/// for good: the passphrase isn't kept, so neither is its protection (the
/// key is sealed with the device's key instead).
///
/// # Errors
/// [`SyncError::Invalid`] for a wrong passphrase or a key that can't be
/// read.
pub fn read_secret_key(bytes: &[u8], passphrase: Option<&str>) -> Result<OwnKey, SyncError> {
    let unreadable = || SyncError::Invalid("This secret key can't be read.".to_owned());
    let info = mailcraft_openpgp::secret::read(bytes).map_err(|e| match e {
        mailcraft_openpgp::Error::SeveralKeys => SyncError::Invalid(
            "This file holds more than one key. Export one secret key at a time.".to_owned(),
        ),
        _ => unreadable(),
    })?;
    let mut secret = guard(|| {
        SignedSecretKey::from_armor_single(bytes)
            .map(|(key, _)| key)
            .or_else(|_| SignedSecretKey::from_bytes(bytes))
            .ok()
    })
    .flatten()
    .ok_or_else(unreadable)?;
    let password = Password::from(passphrase.unwrap_or_default());
    let wrong = || {
        SyncError::Invalid(if passphrase.is_some_and(|p| !p.is_empty()) {
            "The passphrase for this key isn't right.".to_owned()
        } else {
            "This key is protected by a passphrase. Type it in and import it again.".to_owned()
        })
    };
    secret
        .primary_key
        .remove_password(&password)
        .map_err(|_| wrong())?;
    for sub in &mut secret.secret_subkeys {
        sub.key.remove_password(&password).map_err(|_| wrong())?;
    }
    Ok(OwnKey {
        secret,
        public: info.public,
    })
}

/// A sealed-and-opened secret key back from its binary packets.
///
/// # Errors
/// When it can't be read.
pub fn secret_from_bytes(bytes: &[u8]) -> Result<SignedSecretKey, SyncError> {
    guard(|| SignedSecretKey::from_bytes(bytes).ok())
        .flatten()
        .ok_or_else(|| SyncError::Invalid("A saved secret key can't be read.".to_owned()))
}

/// A checked public key back from its binary packets.
#[must_use]
pub fn public_from_bytes(bytes: &[u8]) -> Option<PublicKey> {
    PublicKey::parse(bytes).ok()?.into_iter().next()
}

/// A public key armoured, to share (`.asc`).
///
/// # Errors
/// When it can't be written.
pub fn armored_public(key: &PublicKey) -> Result<String, SyncError> {
    rpgp_public(key)
        .and_then(|k| k.to_armored_string(ArmorOptions::default()).ok())
        .ok_or_else(|| SyncError::Invalid("This key can't be exported.".to_owned()))
}

/// Run rPGP on hostile input without letting a panic out.
fn guard<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// The key of `secret` that signs: the primary key if it may, else the
/// newest subkey for signing.
fn signer(secret: &SignedSecretKey) -> Option<&dyn pgp::types::SigningKey> {
    let public = secret.to_public_key();
    let primary_ok = secret.primary_key.algorithm().can_sign()
        && primary_self_signature(&public).is_none_or(|s| s.key_flags().sign());
    if primary_ok {
        return Some(&secret.primary_key);
    }
    secret
        .secret_subkeys
        .iter()
        .filter(|sub| {
            let (sig, revoked) = binding(&sub.signatures);
            !revoked && sig.is_some_and(|s| s.key_flags().sign()) && sub.key.algorithm().can_sign()
        })
        .max_by_key(|sub| sub.key.created_at().as_secs())
        .map(|sub| &sub.key as &dyn pgp::types::SigningKey)
}

/// Whether the user's key can sign.
#[must_use]
pub fn can_sign(secret: &SignedSecretKey) -> bool {
    signer(secret).is_some()
}

/// A detached signature (armoured) over `data`, the signed MIME entity in
/// canonical form, with SHA-256.
///
/// # Errors
/// [`SyncError::Invalid`] when the key can't sign.
pub fn sign_detached(data: &[u8], secret: &SignedSecretKey) -> Result<String, SyncError> {
    let failed = || SyncError::Invalid("Couldn't sign the message with OpenPGP.".to_owned());
    let key = signer(secret)
        .ok_or_else(|| SyncError::Invalid("Your OpenPGP key can't make signatures.".to_owned()))?;
    let signature = DetachedSignature::sign_binary_data(
        OsRng,
        &Box::new(key),
        &Password::empty(),
        HashAlgorithm::Sha256,
        data,
    )
    .map_err(|_| failed())?;
    signature
        .to_armored_string(ArmorOptions::default())
        .map_err(|_| failed())
}

/// `data` encrypted (armoured, SEIPD v1 with AES-256) to each of
/// `recipients`.
///
/// # Errors
/// [`SyncError::Invalid`] naming a key that can't be encrypted to.
pub fn encrypt(data: &[u8], recipients: &[PublicKey]) -> Result<String, SyncError> {
    let failed = || SyncError::Invalid("Couldn't encrypt the message with OpenPGP.".to_owned());
    let now = now_secs();
    let keys: Vec<SignedPublicKey> = recipients.iter().filter_map(rpgp_public).collect();
    if keys.is_empty() || keys.len() != recipients.len() {
        return Err(failed());
    }
    let mut builder = MessageBuilder::from_bytes("", data.to_vec())
        .seipd_v1(OsRng, SymmetricKeyAlgorithm::AES256);
    for (key, public) in keys.iter().zip(recipients) {
        let added = match encryption_key(key, now) {
            Some(Recipient::Subkey(sub)) => builder.encrypt_to_key(OsRng, sub),
            Some(Recipient::Primary(primary)) => builder.encrypt_to_key(OsRng, primary),
            None => {
                return Err(SyncError::Invalid(format!(
                    "The OpenPGP key of {} can't be encrypted to.",
                    public
                        .addresses()
                        .first()
                        .map_or("a recipient", String::as_str)
                )));
            }
        };
        added.map_err(|_| failed())?;
    }
    builder
        .to_armored_string(OsRng, ArmorOptions::default())
        .map_err(|_| failed())
}

/// Why an OpenPGP message couldn't be decrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecryptError {
    /// It isn't encrypted to any of the user's keys here.
    NoKey,
    /// It can't be read, or was changed (its integrity check failed).
    Damaged,
}

impl std::fmt::Display for DecryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoKey => {
                "It wasn't encrypted to an OpenPGP key of yours on this device. Import your secret key."
            }
            Self::Damaged => "It is damaged, or was changed after it was encrypted.",
        })
    }
}

/// A message decrypted.
pub struct Decrypted {
    /// What was inside.
    pub data: Zeroizing<Vec<u8>>,
    /// A signature made inside the encryption (RFC 3156 §6.2), checked.
    pub signature: Option<SignatureCheck>,
}

impl std::fmt::Debug for Decrypted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decrypted")
            .field("len", &self.data.len())
            .field("signature", &self.signature)
            .finish()
    }
}

/// Decrypt an OpenPGP message (armoured or binary) with one of `keys`,
/// checking a signature inside it against `ring`.
///
/// # Errors
/// [`DecryptError`] saying why not.
pub fn decrypt(
    message: &[u8],
    keys: &[SignedSecretKey],
    ring: &KeyRing,
    trusted: &dyn Fn(&str) -> bool,
    from: &str,
) -> Result<Decrypted, DecryptError> {
    if message.len() > MAX_ENCRYPTED {
        return Err(DecryptError::Damaged);
    }
    if keys.is_empty() {
        return Err(DecryptError::NoKey);
    }
    guard(|| decrypt_inner(message, keys, ring, trusted, from))
        .unwrap_or(Err(DecryptError::Damaged))
}

fn decrypt_inner(
    message: &[u8],
    keys: &[SignedSecretKey],
    public_ring: &KeyRing,
    trusted: &dyn Fn(&str) -> bool,
    from: &str,
) -> Result<Decrypted, DecryptError> {
    let armored = super::mime::find(message, b"-----BEGIN PGP MESSAGE-----").is_some();
    let parsed = if armored {
        Message::from_armor(message).map(|(m, _)| m)
    } else {
        Message::from_bytes(message)
    }
    .map_err(|_| DecryptError::Damaged)?;
    if !parsed.is_encrypted() {
        return Err(DecryptError::Damaged);
    }
    let empty = Password::empty();
    let passwords: Vec<&Password> = keys.iter().map(|_| &empty).collect();
    // GnuPG 2.4 and later encrypt with its own AEAD packet (OCB) to keys that
    // say they can read it, as keys it makes do.
    let ring = pgp::composed::TheRing {
        secret_keys: keys.iter().collect(),
        key_passwords: passwords,
        decrypt_options: pgp::composed::DecryptionOptions::new().enable_gnupg_aead(),
        ..Default::default()
    };
    let (mut decrypted, _) = parsed.decrypt_the_ring(ring, true).map_err(|e| match e {
        pgp::errors::Error::MissingKey => DecryptError::NoKey,
        _ => DecryptError::Damaged,
    })?;
    if decrypted.is_compressed() {
        decrypted = decrypted.decompress().map_err(|_| DecryptError::Damaged)?;
    }
    // Reading to the end checks the integrity (MDC) too.
    let data = Zeroizing::new(decrypted.as_data_vec().map_err(|_| DecryptError::Damaged)?);
    let signature = embedded_signature(&decrypted, public_ring, trusted, from);
    Ok(Decrypted { data, signature })
}

/// A signature inside a decrypted message, checked.
#[allow(clippy::too_many_lines, reason = "one pass over the signatures")]
fn embedded_signature(
    message: &Message<'_>,
    ring: &KeyRing,
    trusted: &dyn Fn(&str) -> bool,
    from: &str,
) -> Option<SignatureCheck> {
    let Message::Signed { reader, .. } = message else {
        return None;
    };
    let count = reader.num_signatures().min(8);
    let mut best: Option<SignatureCheck> = None;
    for index in 0..count {
        let Some(signature) = reader.signature(index) else {
            continue;
        };
        let fingerprints: Vec<Vec<u8>> = signature
            .issuer_fingerprint()
            .into_iter()
            .map(|f| f.as_bytes().to_vec())
            .collect();
        let ids: Vec<Vec<u8>> = signature
            .issuer_key_id()
            .into_iter()
            .map(|id| id.as_ref().to_vec())
            .collect();
        let signed_at = signature.created().map(|t| i64::from(t.as_secs()));
        let candidates = ring.keys().iter().filter(|key| {
            rpgp_public(key).is_some_and(|k| {
                let mut parts: Vec<(Vec<u8>, Vec<u8>)> = vec![(
                    k.primary_key.fingerprint().as_bytes().to_vec(),
                    k.primary_key.legacy_key_id().as_ref().to_vec(),
                )];
                parts.extend(k.public_subkeys.iter().map(|s| {
                    (
                        s.key.fingerprint().as_bytes().to_vec(),
                        s.key.legacy_key_id().as_ref().to_vec(),
                    )
                }));
                parts
                    .iter()
                    .any(|(fp, id)| fingerprints.contains(fp) || ids.contains(id))
            })
        });
        let mut check = None;
        for key in candidates.take(4) {
            let Some(rpgp) = rpgp_public(key) else {
                continue;
            };
            let valid = message.verify_nested_explicit(index, &rpgp).is_ok()
                || rpgp
                    .public_subkeys
                    .iter()
                    .any(|sub| message.verify_nested_explicit(index, sub).is_ok());
            let fingerprint = key.fingerprint().to_hex();
            let signer = SignerDetails {
                name: owner_of(&rpgp),
                address: key.addresses().first().cloned(),
                fingerprint: Some(fingerprint.clone()),
                issuer: None,
            };
            let from_mismatch = !key.addresses().iter().any(|a| same_address(a, from));
            check = Some(if valid {
                let is_trusted = trusted(&fingerprint);
                SignatureCheck {
                    kind: SecurityKind::Openpgp,
                    result: if is_trusted {
                        SignatureResult::Verified
                    } else {
                        SignatureResult::UnverifiedSigner
                    },
                    signer: Some(signer),
                    from_mismatch,
                    detail: if is_trusted {
                        "Signed with a key you trust.".to_owned()
                    } else {
                        UNVERIFIED_KEY.to_owned()
                    },
                    signed_at,
                }
            } else {
                SignatureCheck {
                    kind: SecurityKind::Openpgp,
                    result: SignatureResult::Changed,
                    signer: Some(signer),
                    from_mismatch,
                    detail: CHANGED.to_owned(),
                    signed_at,
                }
            });
            if valid {
                break;
            }
        }
        let check = check.unwrap_or_else(|| SignatureCheck {
            kind: SecurityKind::Openpgp,
            result: SignatureResult::Unknown,
            signer: fingerprints.first().map(|fp| SignerDetails {
                name: None,
                address: None,
                fingerprint: Some(super::smime_hex(fp)),
                issuer: None,
            }),
            from_mismatch: false,
            detail: NO_KEY.to_owned(),
            signed_at,
        });
        if best
            .as_ref()
            .is_none_or(|b| rank(check.result) > rank(b.result))
        {
            best = Some(check);
        }
    }
    best
}

const UNVERIFIED_KEY: &str =
    "The signature is valid, but you haven't said this key is really the sender's.";
const CHANGED: &str = "The message was changed after it was signed.";
const NO_KEY: &str = "The signer's key isn't on this device, so the signature can't be checked.";

fn rank(result: SignatureResult) -> u8 {
    match result {
        SignatureResult::Verified => 3,
        SignatureResult::UnverifiedSigner => 2,
        SignatureResult::Changed => 1,
        SignatureResult::Unknown => 0,
    }
}

/// Check the OpenPGP signature of a message (PGP/MIME, or an inline
/// cleartext-signed text): `None` when it has none.
#[must_use]
pub fn verify(
    message: &[u8],
    from: &str,
    ring: &KeyRing,
    trusted: &dyn Fn(&str) -> bool,
    now: u64,
) -> Option<SignatureCheck> {
    let checked =
        mailcraft_openpgp::verify(message, from, ring, now, |fp, _| trusted(&fp.to_hex()))?;
    let partly = !checked.coverage.whole;
    let details = |signer: &mailcraft_openpgp::Signer| {
        let key = ring.get(&signer.fingerprint);
        SignerDetails {
            name: key.and_then(rpgp_public).as_ref().and_then(owner_of),
            address: signer
                .addresses
                .iter()
                .find(|a| same_address(a, from))
                .or_else(|| signer.addresses.first())
                .cloned(),
            fingerprint: Some(signer.fingerprint.to_hex()),
            issuer: None,
        }
    };
    let mismatch = |signer: &mailcraft_openpgp::Signer| {
        !signer.addresses.iter().any(|a| same_address(a, from))
    };
    let signed_at = |signer: &mailcraft_openpgp::Signer| i64::try_from(signer.signed).ok();
    Some(match checked.outcome {
        Outcome::Verified(signer) => SignatureCheck {
            kind: SecurityKind::Openpgp,
            result: SignatureResult::Verified,
            from_mismatch: mismatch(&signer),
            signed_at: signed_at(&signer),
            signer: Some(details(&signer)),
            detail: "Signed with a key you trust.".to_owned(),
        },
        Outcome::Valid { signer, why } => {
            let trusted_key = !why.key_unverified;
            let (result, detail) = if partly || why.partly_signed {
                (
                    SignatureResult::UnverifiedSigner,
                    "Only part of this message is signed.",
                )
            } else if trusted_key {
                (SignatureResult::Verified, "Signed with a key you trust.")
            } else {
                (SignatureResult::UnverifiedSigner, UNVERIFIED_KEY)
            };
            SignatureCheck {
                kind: SecurityKind::Openpgp,
                result,
                from_mismatch: mismatch(&signer),
                signed_at: signed_at(&signer),
                signer: Some(details(&signer)),
                detail: detail.to_owned(),
            }
        }
        Outcome::Changed(signer) => SignatureCheck {
            kind: SecurityKind::Openpgp,
            result: SignatureResult::Changed,
            from_mismatch: mismatch(&signer),
            signed_at: signed_at(&signer),
            signer: Some(details(&signer)),
            detail: CHANGED.to_owned(),
        },
        Outcome::CantCheck { why, issuer } => SignatureCheck {
            kind: SecurityKind::Openpgp,
            result: SignatureResult::Unknown,
            from_mismatch: false,
            signed_at: None,
            signer: issuer
                .fingerprint
                .map(|fp| fp.to_hex())
                .or_else(|| issuer.key_id.map(|id| id.to_hex()))
                .map(|fingerprint| SignerDetails {
                    name: None,
                    address: None,
                    fingerprint: Some(fingerprint),
                    issuer: None,
                }),
            detail: cant_check(&why),
        },
    })
}

fn cant_check(why: &CantCheck) -> String {
    match why {
        CantCheck::NoPublicKey | CantCheck::NoMatchingKey => NO_KEY.to_owned(),
        CantCheck::WeakAlgorithm(name) => {
            format!("It was made with {name}, which is no longer safe.")
        }
        CantCheck::UnknownAlgorithm => "It uses an algorithm Mocky Mail can't check.".to_owned(),
        CantCheck::KeyRevoked => "The signer's key was revoked.".to_owned(),
        CantCheck::KeyExpired => "The signer's key had expired when it was signed.".to_owned(),
        CantCheck::NotASigningKey => "The key that matches may not make signatures.".to_owned(),
        CantCheck::SignatureExpired => "The signature has expired.".to_owned(),
        CantCheck::TooLarge => "The message is too large to check.".to_owned(),
        CantCheck::Unreadable => "The signature can't be read.".to_owned(),
    }
}

/// The keys a message carries (attached `application/pgp-keys` parts, and
/// the sender's `Autocrypt:` header), checked, for keeping.
#[must_use]
pub fn keys_in_message(message: &[u8]) -> (Vec<PublicKey>, Option<PublicKey>) {
    let found = mailcraft_openpgp::keys_in_message(message);
    (found.attached, found.autocrypt.map(|a| a.key))
}

/// An `Autocrypt:` header (Autocrypt Level 1 §2.1) for `address` with
/// `key`: the primary key, one User ID for the address and the encryption
/// subkey, folded. `None` when the key can't be cut down that way.
#[must_use]
pub fn autocrypt_header(address: &str, key: &PublicKey) -> Option<String> {
    let full = rpgp_public(key)?;
    let now = now_secs();
    let user = full
        .details
        .users
        .iter()
        .find(|u| {
            u.id.as_str()
                .is_some_and(|id| same_address(&mailcraft_openpgp::from_address(id), address))
        })
        .or_else(|| full.details.users.first())?
        .clone();
    let subkeys: Vec<_> = match encryption_key(&full, now) {
        Some(Recipient::Subkey(sub)) => vec![sub.clone()],
        _ => Vec::new(),
    };
    let details = pgp::composed::SignedKeyDetails::new(
        full.details.revocation_signatures.clone(),
        full.details.direct_signatures.clone(),
        vec![user],
        Vec::new(),
    );
    let minimal = SignedPublicKey::new(full.primary_key.clone(), details, subkeys);
    let bytes = minimal.to_bytes().ok()?;
    let data = super::mime::base64_lines(&bytes);
    let mut header = format!(
        "Autocrypt: addr={}; keydata=",
        address.trim().to_lowercase()
    );
    for line in data.lines() {
        header.push_str("\r\n ");
        header.push_str(line);
    }
    header.push_str("\r\n");
    Some(header)
}
