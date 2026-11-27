//! Security (0.10.0): S/MIME and OpenPGP.
//!
//! - [`open`] reads a received message: decrypts it with the user's own
//!   keys (S/MIME `enveloped-data`, PGP/MIME `multipart/encrypted`, inline
//!   armoured text), checks its signature (S/MIME `multipart/signed` and
//!   opaque `signed-data`, PGP/MIME and inline cleartext), and hands back
//!   the message to show, the result, and the certificates and keys it
//!   carried for keeping. Nothing here touches the disk or the network;
//!   decrypted text only lives in what is returned.
//! - [`protect`] signs and/or encrypts a built message (sign first, then
//!   encrypt the signed message), keeping its addressing headers outside.
//! - [`wkd`] looks up OpenPGP keys on the recipient's domain.
//! - [`vault`] seals private keys with a key kept in the system keyring.

#![allow(
    clippy::doc_markdown,
    reason = "OpenPGP, S/MIME and RustCrypto are names here, not code"
)]

pub mod ber;
pub mod mime;
pub mod openpgp;
pub mod smime;
pub mod system;
pub mod vault;
pub mod wkd;

#[cfg(test)]
mod tests;

pub use mailcraft_openpgp::{KeyRing, PublicKey};
pub use pgp::composed::SignedSecretKey;
use serde::{Deserialize, Serialize};

use crate::SyncError;

/// S/MIME or OpenPGP (`SecurityKind` in `docs/ipc.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityKind {
    /// S/MIME (X.509 certificates).
    Smime,
    /// OpenPGP.
    Openpgp,
}

/// `SignatureCheck.result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignatureResult {
    /// Valid, and the signer is trusted.
    Verified,
    /// Valid, but who made it isn't proven.
    UnverifiedSigner,
    /// Changed after it was signed.
    Changed,
    /// It couldn't be checked.
    Unknown,
}

/// `SignatureCheck.signer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignerDetails {
    /// The name on the certificate or key.
    pub name: Option<String>,
    /// Its address (the one matching From, if any).
    pub address: Option<String>,
    /// Certificate SHA-256, or OpenPGP fingerprint (upper-case hex).
    pub fingerprint: Option<String>,
    /// S/MIME: who issued the certificate.
    pub issuer: Option<String>,
}

/// What checking a message's signature found (`SignatureCheck`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureCheck {
    /// S/MIME or OpenPGP.
    pub kind: SecurityKind,
    /// The result.
    pub result: SignatureResult,
    /// Who signed, as far as the signature says.
    pub signer: Option<SignerDetails>,
    /// The signer's address isn't the From address.
    pub from_mismatch: bool,
    /// Plain words about the result.
    pub detail: String,
    /// When it was signed (Unix seconds), if it says.
    pub signed_at: Option<i64>,
}

/// `MessageDetail.encryption.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EncryptionState {
    /// Decrypted, shown.
    Decrypted,
    /// Couldn't be decrypted.
    Locked,
}

/// `MessageDetail.encryption`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionInfo {
    /// S/MIME or OpenPGP.
    pub kind: SecurityKind,
    /// Decrypted or locked.
    pub state: EncryptionState,
    /// Why it is locked.
    pub detail: Option<String>,
}

/// Text from a certificate or key, safe to show.
pub(crate) fn clean_text(text: &str) -> String {
    mail_core::message::display_safe(text)
}

/// Whether two addresses (or a From value and an address) are the same,
/// ignoring case.
pub(crate) fn same_address(a: &str, b: &str) -> bool {
    let a = mailcraft_openpgp::from_address(a);
    let b = mailcraft_openpgp::from_address(b);
    !a.is_empty() && a == b
}

/// Upper-case hex.
pub(crate) fn smime_hex(bytes: &[u8]) -> String {
    smime::hex_upper(bytes)
}

/// The keys and trust a message is opened with.
pub struct OpenKeys<'a> {
    /// The user's own S/MIME keys.
    pub smime: &'a [smime::Identity],
    /// The user's own OpenPGP keys.
    pub openpgp: &'a [SignedSecretKey],
    /// Other people's OpenPGP keys (and the user's own public halves).
    pub ring: &'a KeyRing,
    /// The roots S/MIME signers are checked against.
    pub roots: &'a smime::trust::TrustRoots,
    /// S/MIME certificates kept from earlier mail (issuers, signers).
    pub known_certs: &'a [Vec<u8>],
    /// Whether the user trusts the key or certificate with this
    /// fingerprint.
    pub trusted: &'a dyn Fn(SecurityKind, &str) -> bool,
    /// Why the user's own keys can't be used now (the keyring is locked),
    /// if so.
    pub locked: Option<&'a str>,
    /// Now, Unix seconds.
    pub now: u64,
}

impl std::fmt::Debug for OpenKeys<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenKeys")
            .field("smime", &self.smime.len())
            .field("openpgp", &self.openpgp.len())
            .field("ring", &self.ring.len())
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// A received message opened.
#[derive(Debug, Default)]
pub struct Opened {
    /// The message to show instead of the one received (decrypted, or the
    /// signed part without its signature), with the received headers.
    /// `None`: show it as it is. Keep it in memory only.
    pub shown: Option<Vec<u8>>,
    /// Its signature, checked.
    pub signature: Option<SignatureCheck>,
    /// Whether it was encrypted, and decrypted.
    pub encryption: Option<EncryptionInfo>,
    /// S/MIME signers' certificates (DER) with the certificates that came
    /// along, from signatures that weren't changed: to keep for encrypting
    /// to them.
    pub certificates: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
    /// OpenPGP keys attached to it.
    pub attached_keys: Vec<PublicKey>,
    /// The key in its `Autocrypt:` header, for the From address.
    pub autocrypt: Option<PublicKey>,
}

/// How many layers (encryption, signature) are taken off at most.
const MAX_LAYERS: usize = 4;

/// The header fields of `raw` that aren't about its content, as they came.
fn outer_header(raw: &[u8]) -> Vec<u8> {
    let (header, _) = mime::split(raw);
    let mut out = Vec::new();
    for field in mime::fields(header) {
        if !mime::is_content_field(&field.name) {
            out.extend_from_slice(&field.raw);
        }
    }
    out
}

/// A whole message from outer header fields and an entity.
fn join(outer: &[u8], entity: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(outer.len().saturating_add(entity.len()).saturating_add(2));
    out.extend_from_slice(outer);
    if entity.starts_with(b"\r\n") || entity.starts_with(b"\n") {
        // An entity without header fields: its blank line is already there.
    } else if mime::split(entity).0.is_empty() {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(entity);
    out
}

fn locked(kind: SecurityKind, why: String) -> EncryptionInfo {
    EncryptionInfo {
        kind,
        state: EncryptionState::Locked,
        detail: Some(why),
    }
}

fn decrypted(kind: SecurityKind) -> EncryptionInfo {
    EncryptionInfo {
        kind,
        state: EncryptionState::Decrypted,
        detail: None,
    }
}

fn smime_check(
    verification: smime::Verification,
    from: &str,
    opened: &mut Opened,
) -> (SignatureCheck, Option<Vec<u8>>) {
    use smime::VerifyOutcome as V;
    let (result, detail) = match &verification.outcome {
        V::Verified => (
            SignatureResult::Verified,
            verification.signer.as_ref().map_or_else(
                || "Signed with a trusted certificate.".to_owned(),
                |s| format!("Signed with a certificate from {}.", s.issuer),
            ),
        ),
        V::Untrusted(why) => (SignatureResult::UnverifiedSigner, why.clone()),
        V::Changed => (
            SignatureResult::Changed,
            "The message was changed after it was signed.".to_owned(),
        ),
        V::Unknown(why) => (SignatureResult::Unknown, why.clone()),
    };
    if verification.outcome != V::Changed
        && let Some(der) = &verification.signer_der
    {
        opened
            .certificates
            .push((der.clone(), verification.chain.clone()));
    }
    let signer = verification.signer.as_ref().map(|info| SignerDetails {
        name: Some(info.owner.clone()),
        address: info
            .addresses
            .iter()
            .find(|a| same_address(a, from))
            .or_else(|| info.addresses.first())
            .cloned(),
        fingerprint: Some(info.fingerprint.clone()),
        issuer: Some(info.issuer.clone()),
    });
    let from_mismatch = verification
        .signer
        .as_ref()
        .is_some_and(|info| !info.addresses.iter().any(|a| same_address(a, from)));
    (
        SignatureCheck {
            kind: SecurityKind::Smime,
            result,
            signer,
            from_mismatch,
            detail,
            signed_at: verification.signed_at,
        },
        verification.content,
    )
}

/// The armoured block of `kind` in `text` and where it is.
fn armoured_block(text: &[u8], begin: &[u8], end: &[u8]) -> Option<(usize, usize)> {
    let start = mime::find(text, begin)?;
    let after = text.get(start..)?;
    let stop = mime::find(after, end)?.checked_add(end.len())?;
    Some((start, start.checked_add(stop)?))
}

/// Open a received message (see the module documentation).
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one loop over the layers a message can have"
)]
pub fn open(raw: &[u8], keys: &OpenKeys<'_>) -> Opened {
    let mut opened = Opened::default();
    let (header, _) = mime::split(raw);
    let fields = mime::fields(header);
    let from = mime::header(&fields, "from").unwrap_or_default().to_owned();
    let (attached, autocrypt) = openpgp::keys_in_message(raw);
    opened.attached_keys = attached;
    opened.autocrypt = autocrypt;
    let outer = outer_header(raw);
    let trusted_pgp = |fp: &str| (keys.trusted)(SecurityKind::Openpgp, fp);
    let trusted_smime = |fp: &str| (keys.trusted)(SecurityKind::Smime, fp);
    let cx = smime::TrustContext {
        roots: keys.roots,
        known: keys.known_certs,
        trusted: &trusted_smime,
        now: i64::try_from(keys.now).unwrap_or(i64::MAX),
    };

    // The entity looked at: the whole message to start with.
    let mut entity: Vec<u8> = raw.to_vec();
    let mut unwrapped = false;
    for _ in 0..MAX_LAYERS {
        let (head, body) = mime::split(&entity);
        let fields = mime::fields(head);
        let ct = mime::ContentType::parse(mime::header(&fields, "content-type"));
        let encoding = mime::header(&fields, "content-transfer-encoding").map(str::to_owned);
        let smime_type = ct.param_lower("smime-type");
        let protocol = ct.param_lower("protocol");
        let is_pkcs7 = matches!(
            ct.value.as_str(),
            "application/pkcs7-mime" | "application/x-pkcs7-mime"
        );
        let next: Vec<u8> = if is_pkcs7 && smime_type.as_deref() == Some("signed-data") {
            let p7m = mime::decode(encoding.as_deref(), body);
            let verification = smime::verify_opaque(&p7m, &cx);
            let (check, content) = smime_check(verification, &from, &mut opened);
            opened.signature = Some(check);
            match content {
                Some(content) => content,
                None => break,
            }
        } else if is_pkcs7
            && !matches!(
                smime_type.as_deref(),
                Some("certs-only" | "compressed-data")
            )
        {
            if opened.encryption.is_some() && opened.signature.is_some() {
                break;
            }
            if let Some(why) = keys.locked.filter(|_| keys.smime.is_empty()) {
                opened.encryption = Some(locked(SecurityKind::Smime, why.to_owned()));
                break;
            }
            let p7m = mime::decode(encoding.as_deref(), body);
            match smime::decrypt(&p7m, keys.smime) {
                Ok(inner) => {
                    opened.encryption = Some(decrypted(SecurityKind::Smime));
                    inner.to_vec()
                }
                Err(error) => {
                    opened.encryption = Some(locked(SecurityKind::Smime, error.to_string()));
                    break;
                }
            }
        } else if ct.value == "multipart/encrypted"
            && protocol.as_deref() == Some("application/pgp-encrypted")
        {
            if let Some(why) = keys.locked.filter(|_| keys.openpgp.is_empty()) {
                opened.encryption = Some(locked(SecurityKind::Openpgp, why.to_owned()));
                break;
            }
            let boundary = ct.param("boundary").unwrap_or_default().to_owned();
            let parts = mime::parts(body, &boundary, 2);
            let Some(part) = parts.get(1) else {
                opened.encryption = Some(locked(
                    SecurityKind::Openpgp,
                    openpgp::DecryptError::Damaged.to_string(),
                ));
                break;
            };
            let (part_head, part_body) = mime::split(part);
            let part_fields = mime::fields(part_head);
            let data = mime::decode(
                mime::header(&part_fields, "content-transfer-encoding"),
                part_body,
            );
            match openpgp::decrypt(&data, keys.openpgp, keys.ring, &trusted_pgp, &from) {
                Ok(inner) => {
                    opened.encryption = Some(decrypted(SecurityKind::Openpgp));
                    if inner.signature.is_some() {
                        opened.signature = inner.signature;
                    }
                    inner.data.to_vec()
                }
                Err(error) => {
                    opened.encryption = Some(locked(SecurityKind::Openpgp, error.to_string()));
                    break;
                }
            }
        } else if ct.value == "multipart/signed"
            && matches!(
                protocol.as_deref(),
                Some("application/pkcs7-signature" | "application/x-pkcs7-signature")
            )
        {
            let boundary = ct.param("boundary").unwrap_or_default().to_owned();
            let parts = mime::parts(body, &boundary, 3);
            let [signed, signature] = parts.as_slice() else {
                opened.signature = Some(SignatureCheck {
                    kind: SecurityKind::Smime,
                    result: SignatureResult::Unknown,
                    signer: None,
                    from_mismatch: false,
                    detail: "The signature can't be read.".to_owned(),
                    signed_at: None,
                });
                break;
            };
            let (sig_head, sig_body) = mime::split(signature);
            let sig_fields = mime::fields(sig_head);
            let p7s = mime::decode(
                mime::header(&sig_fields, "content-transfer-encoding"),
                sig_body,
            );
            let verification = smime::verify_detached(&p7s, &mime::crlf(signed), &cx);
            let (check, _) = smime_check(verification, &from, &mut opened);
            opened.signature = Some(check);
            signed.to_vec()
        } else if ct.value == "multipart/signed"
            && protocol.as_deref() == Some("application/pgp-signature")
        {
            let message = if unwrapped {
                join(&outer, &entity)
            } else {
                entity.clone()
            };
            opened.signature = openpgp::verify(&message, &from, keys.ring, &trusted_pgp, keys.now);
            let boundary = ct.param("boundary").unwrap_or_default().to_owned();
            match mime::parts(body, &boundary, 1).first() {
                Some(first) => first.to_vec(),
                None => break,
            }
        } else if ct.value == "text/plain" {
            let text = mime::decode(encoding.as_deref(), body);
            if let Some((start, end)) = armoured_block(
                &text,
                b"-----BEGIN PGP MESSAGE-----",
                b"-----END PGP MESSAGE-----",
            ) && opened.encryption.is_none()
            {
                if let Some(why) = keys.locked.filter(|_| keys.openpgp.is_empty()) {
                    opened.encryption = Some(locked(SecurityKind::Openpgp, why.to_owned()));
                    break;
                }
                let block = text.get(start..end).unwrap_or_default();
                match openpgp::decrypt(block, keys.openpgp, keys.ring, &trusted_pgp, &from) {
                    Ok(inner) => {
                        opened.encryption = Some(decrypted(SecurityKind::Openpgp));
                        if inner.signature.is_some() {
                            opened.signature = inner.signature;
                        }
                        let mut plain = text.get(..start).unwrap_or_default().to_vec();
                        plain.extend_from_slice(&inner.data);
                        plain.extend_from_slice(text.get(end..).unwrap_or_default());
                        let mut next =
                            b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n"
                                .to_vec();
                        next.extend_from_slice(&plain);
                        next
                    }
                    Err(error) => {
                        opened.encryption = Some(locked(SecurityKind::Openpgp, error.to_string()));
                        break;
                    }
                }
            } else {
                if opened.signature.is_none()
                    && mime::find(&text, b"-----BEGIN PGP SIGNED MESSAGE-----").is_some()
                {
                    let message = if unwrapped {
                        join(&outer, &entity)
                    } else {
                        entity.clone()
                    };
                    opened.signature =
                        openpgp::verify(&message, &from, keys.ring, &trusted_pgp, keys.now);
                }
                break;
            }
        } else {
            // A mailing list's wrapper around a PGP/MIME signed part.
            if opened.signature.is_none() && ct.value == "multipart/mixed" {
                let message = if unwrapped {
                    join(&outer, &entity)
                } else {
                    entity.clone()
                };
                opened.signature =
                    openpgp::verify(&message, &from, keys.ring, &trusted_pgp, keys.now);
            }
            break;
        };
        entity = next;
        unwrapped = true;
    }
    if unwrapped {
        opened.shown = Some(join(&outer, &entity));
        // Keys attached inside what was encrypted or signed.
        if let Some(shown) = &opened.shown {
            let (inside, _) = openpgp::keys_in_message(shown);
            for key in inside {
                if !opened
                    .attached_keys
                    .iter()
                    .any(|k| k.fingerprint() == key.fingerprint())
                {
                    opened.attached_keys.push(key);
                }
            }
        }
    }
    opened
}

/// The key a message is signed with.
#[derive(Debug, Clone, Copy)]
pub enum Signer<'a> {
    /// S/MIME.
    Smime(&'a smime::Identity),
    /// OpenPGP.
    Openpgp(&'a SignedSecretKey),
}

/// Who a message is encrypted to (the sender included, so Sent stays
/// readable).
#[derive(Debug, Clone, Copy)]
pub enum Recipients<'a> {
    /// S/MIME certificates (DER).
    Smime(&'a [Vec<u8>]),
    /// OpenPGP keys.
    Openpgp(&'a [PublicKey]),
}

/// What [`protect`] does to a message.
#[derive(Debug, Clone, Copy)]
pub struct Protection<'a> {
    /// Sign with this key.
    pub sign: Option<Signer<'a>>,
    /// Encrypt to these.
    pub encrypt: Option<Recipients<'a>>,
    /// Now, Unix seconds (the S/MIME signing time).
    pub now: u64,
}

/// Sign and/or encrypt a built message (RFC 5322 bytes, CR LF). Its
/// addressing fields (From, To, Subject, Date, Message-ID...) stay
/// outside; its content (Content-* fields and body) is signed, then the
/// signed entity is encrypted. The Subject is not hidden.
///
/// # Errors
/// [`SyncError::Invalid`] when signing or encrypting fails, naming a
/// recipient whose key can't be used.
pub fn protect(raw: &[u8], protection: &Protection<'_>) -> Result<Vec<u8>, SyncError> {
    let raw = mime::crlf(raw);
    let (header, body) = mime::split(&raw);
    let mut outer = Vec::new();
    let mut entity = Vec::new();
    for field in mime::fields(header) {
        if mime::is_content_field(&field.name) {
            entity.extend_from_slice(&field.raw);
        } else {
            outer.extend_from_slice(&field.raw);
        }
    }
    if entity.is_empty() {
        entity.extend_from_slice(b"Content-Type: text/plain; charset=us-ascii\r\n");
    }
    entity.extend_from_slice(b"\r\n");
    entity.extend_from_slice(body);

    if let Some(signer) = protection.sign {
        entity = signed_entity(&entity, signer, protection.now)?;
    }
    if let Some(recipients) = protection.encrypt {
        entity = encrypted_entity(&entity, recipients)?;
    }
    let mut out = outer;
    if !mime::fields(&out)
        .iter()
        .any(|f| f.name.eq_ignore_ascii_case("mime-version"))
    {
        out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    }
    out.extend_from_slice(&entity);
    Ok(out)
}

fn signed_entity(entity: &[u8], signer: Signer<'_>, now: u64) -> Result<Vec<u8>, SyncError> {
    let boundary = mime::boundary()?;
    let (protocol, micalg, signature_part) = match signer {
        Signer::Smime(identity) => {
            let der = smime::sign(entity, identity, now)?;
            let micalg = match smime::build::hash_for(&identity.key) {
                smime::Hash::Sha384 => "sha-384",
                smime::Hash::Sha512 => "sha-512",
                smime::Hash::Sha224 | smime::Hash::Sha256 => "sha-256",
            };
            (
                "application/pkcs7-signature",
                micalg,
                format!(
                    "Content-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\n\
                     Content-Transfer-Encoding: base64\r\n\
                     Content-Disposition: attachment; filename=\"smime.p7s\"\r\n\
                     Content-Description: S/MIME Cryptographic Signature\r\n\r\n{}",
                    mime::base64_lines(&der)
                ),
            )
        }
        Signer::Openpgp(key) => {
            let armored = openpgp::sign_detached(entity, key)?;
            (
                "application/pgp-signature",
                "pgp-sha256",
                format!(
                    "Content-Type: application/pgp-signature; name=\"signature.asc\"\r\n\
                     Content-Description: OpenPGP digital signature\r\n\
                     Content-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n{}",
                    mime::crlf(armored.as_bytes())
                        .iter()
                        .map(|b| char::from(*b))
                        .collect::<String>()
                ),
            )
        }
    };
    let mut out = format!(
        "Content-Type: multipart/signed; protocol=\"{protocol}\"; micalg={micalg};\r\n boundary=\"{boundary}\"\r\n\r\n\
         This is a digitally signed message in MIME format.\r\n\r\n--{boundary}\r\n"
    )
    .into_bytes();
    out.extend_from_slice(entity);
    out.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    out.extend_from_slice(signature_part.as_bytes());
    if !signature_part.ends_with("\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(out)
}

fn encrypted_entity(entity: &[u8], recipients: Recipients<'_>) -> Result<Vec<u8>, SyncError> {
    match recipients {
        Recipients::Smime(certs) => {
            let der = smime::encrypt(entity, certs)?;
            let mut out = b"Content-Type: application/pkcs7-mime; smime-type=enveloped-data;\r\n name=\"smime.p7m\"\r\n\
                Content-Transfer-Encoding: base64\r\n\
                Content-Disposition: attachment; filename=\"smime.p7m\"\r\n\
                Content-Description: S/MIME Encrypted Message\r\n\r\n"
                .to_vec();
            out.extend_from_slice(mime::base64_lines(&der).as_bytes());
            Ok(out)
        }
        Recipients::Openpgp(keys) => {
            let armored = openpgp::encrypt(entity, keys)?;
            let boundary = mime::boundary()?;
            let mut out = format!(
                "Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\";\r\n boundary=\"{boundary}\"\r\n\r\n\
                 This is an OpenPGP/MIME encrypted message (RFC 4880 and 3156).\r\n\
                 --{boundary}\r\n\
                 Content-Type: application/pgp-encrypted\r\n\
                 Content-Description: PGP/MIME version identification\r\n\r\n\
                 Version: 1\r\n\r\n\
                 --{boundary}\r\n\
                 Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\
                 Content-Description: OpenPGP encrypted message\r\n\
                 Content-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n"
            )
            .into_bytes();
            out.extend_from_slice(&mime::crlf(armored.as_bytes()));
            if !out.ends_with(b"\r\n") {
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
            Ok(out)
        }
    }
}

/// `raw` with one more header field in front (folded already, ending in
/// CR LF), such as an `Autocrypt:` header.
#[must_use]
pub fn with_header(raw: &[u8], field: &str) -> Vec<u8> {
    let mut out = field.as_bytes().to_vec();
    if !field.ends_with("\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(raw);
    out
}
