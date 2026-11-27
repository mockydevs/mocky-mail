//! Checking an S/MIME signature: CMS `SignedData` (RFC 5652) over the
//! exact signed bytes, the signer's certificate (validity when signed, key
//! usage, email protection) and its path to a root this device trusts.
//! Revocation is not checked.

use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use der::{Decode, Encode};
use x509_cert::Certificate;

use super::trust::TrustRoots;
use super::{CertInfo, Hash, SigError, Usage, info_of, issued_by, oid, subject_key_id, unix};
use crate::security::ber;

/// What checking found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// Valid, and the certificate comes from a trusted authority (or the
    /// user trusts it).
    Verified,
    /// Valid, but who signed isn't proven: why.
    Untrusted(String),
    /// What was signed was changed afterwards.
    Changed,
    /// It couldn't be checked: why.
    Unknown(String),
}

/// A signature checked.
#[derive(Debug, Clone)]
pub struct Verification {
    /// The result.
    pub outcome: VerifyOutcome,
    /// The signer's certificate, when found.
    pub signer: Option<CertInfo>,
    /// The signer's certificate (DER), to keep for encrypting to them.
    pub signer_der: Option<Vec<u8>>,
    /// The other certificates that came with it (DER).
    pub chain: Vec<Vec<u8>>,
    /// When it says it was signed (Unix seconds).
    pub signed_at: Option<i64>,
    /// An opaque signature's content: the signed MIME entity.
    pub content: Option<Vec<u8>>,
}

impl Verification {
    fn unknown(why: &str) -> Self {
        Self {
            outcome: VerifyOutcome::Unknown(why.to_owned()),
            signer: None,
            signer_der: None,
            chain: Vec::new(),
            signed_at: None,
            content: None,
        }
    }
}

/// What a signature is checked against.
pub struct TrustContext<'a> {
    /// The trusted roots.
    pub roots: &'a TrustRoots,
    /// Other certificates known here (issuers kept from earlier mail).
    pub known: &'a [Vec<u8>],
    /// Whether the user trusts the certificate with this fingerprint.
    pub trusted: &'a dyn Fn(&str) -> bool,
    /// Now, Unix seconds.
    pub now: i64,
}

impl std::fmt::Debug for TrustContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustContext")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// Most signer infos looked at.
const MAX_SIGNERS: usize = 4;
/// Longest certificate path followed.
const MAX_PATH: usize = 8;
/// A signing time this far ahead of the clock is believed (clock skew).
const FUTURE_SKEW: i64 = 300;

/// Check a detached signature (`application/pkcs7-signature`, DER or BER)
/// over `content`, the signed MIME entity in canonical form.
#[must_use]
pub fn verify_detached(signature: &[u8], content: &[u8], cx: &TrustContext<'_>) -> Verification {
    verify(signature, Some(content), cx)
}

/// Check an opaque signature (`application/pkcs7-mime;
/// smime-type=signed-data`) and take out what it signed.
#[must_use]
pub fn verify_opaque(p7m: &[u8], cx: &TrustContext<'_>) -> Verification {
    verify(p7m, None, cx)
}

/// The tag of a SET, and of a context-specific constructed `[0]`.
const SET: u8 = 0x31;
const IMPLICIT_0: u8 = 0xA0;

fn verify(input: &[u8], detached: Option<&[u8]>, cx: &TrustContext<'_>) -> Verification {
    let Some(der) = ber::to_der(input) else {
        return Verification::unknown("The signature can't be read.");
    };
    let Ok(info) = ContentInfo::from_der(&der) else {
        return Verification::unknown("The signature can't be read.");
    };
    if info.content_type != oid::SIGNED_DATA {
        return Verification::unknown("This isn't a signature.");
    }
    let Ok(signed_der) = info.content.to_der() else {
        return Verification::unknown("The signature can't be read.");
    };
    let Ok(signed_data) = SignedData::from_der(&signed_der) else {
        return Verification::unknown("The signature can't be read.");
    };
    let raw = ber::children(&signed_der).unwrap_or_default();
    // The certificates and signer infos as they came, byte for byte.
    let certs_raw: Vec<Vec<u8>> = raw
        .iter()
        .find(|e| e.first() == Some(&IMPLICIT_0))
        .and_then(|e| ber::children(e))
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.first() == Some(&0x30))
        .map(<[u8]>::to_vec)
        .collect();
    let signers_raw: Vec<&[u8]> = raw
        .iter()
        .rev()
        .find(|e| e.first() == Some(&SET))
        .and_then(|e| ber::children(e))
        .unwrap_or_default();

    let opaque_content: Option<Vec<u8>> = signed_data
        .encap_content_info
        .econtent
        .as_ref()
        .map(|any| any.value().to_vec());
    let Some(content) = detached.or(opaque_content.as_deref()) else {
        return Verification::unknown("The signed content is missing.");
    };

    let mut pool: Vec<(Certificate, Vec<u8>)> = certs_raw
        .iter()
        .filter_map(|d| Certificate::from_der(d).ok().map(|c| (c, d.clone())))
        .collect();
    let from_message = pool.len();
    pool.extend(
        cx.known
            .iter()
            .filter_map(|d| Certificate::from_der(d).ok().map(|c| (c, d.clone()))),
    );

    let mut best: Option<Verification> = None;
    for raw_signer in signers_raw.into_iter().take(MAX_SIGNERS) {
        let Ok(signer) = SignerInfo::from_der(raw_signer) else {
            continue;
        };
        let mut checked = check_signer(
            &signer,
            raw_signer,
            signed_data.encap_content_info.econtent_type,
            content,
            &pool,
            cx,
        );
        checked.chain = pool
            .iter()
            .take(from_message)
            .map(|(_, d)| d.clone())
            .filter(|d| Some(d) != checked.signer_der.as_ref())
            .collect();
        let better = best
            .as_ref()
            .is_none_or(|b| rank(&checked.outcome) > rank(&b.outcome));
        if better {
            best = Some(checked);
        }
    }
    let mut result =
        best.unwrap_or_else(|| Verification::unknown("The signature names no signer."));
    if detached.is_none() {
        result.content = opaque_content;
    }
    result
}

fn rank(outcome: &VerifyOutcome) -> u8 {
    match outcome {
        VerifyOutcome::Verified => 3,
        VerifyOutcome::Untrusted(_) => 2,
        VerifyOutcome::Changed => 1,
        VerifyOutcome::Unknown(_) => 0,
    }
}

fn matches_sid(sid: &SignerIdentifier, cert: &Certificate) -> bool {
    match sid {
        SignerIdentifier::IssuerAndSerialNumber(ias) => {
            cert.tbs_certificate.issuer == ias.issuer
                && cert.tbs_certificate.serial_number == ias.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(ski) => {
            subject_key_id(cert).is_some_and(|id| id == ski.0.as_bytes())
        }
    }
}

fn weak(name: &str) -> String {
    format!("It was made with {name}, which is no longer safe.")
}

const UNSUPPORTED: &str = "It uses an algorithm Mocky Mail can't check.";

#[allow(
    clippy::too_many_lines,
    reason = "one pass over the steps of RFC 5652 §5.6"
)]
fn check_signer(
    signer: &SignerInfo,
    raw_signer: &[u8],
    econtent_type: der::asn1::ObjectIdentifier,
    content: &[u8],
    pool: &[(Certificate, Vec<u8>)],
    cx: &TrustContext<'_>,
) -> Verification {
    let hash = match Hash::from_oid(&signer.digest_alg.oid) {
        Some(Ok(hash)) => hash,
        Some(Err(name)) => return Verification::unknown(&weak(name)),
        None => return Verification::unknown(UNSUPPORTED),
    };
    let Some((cert, cert_der)) = pool.iter().find(|(c, _)| matches_sid(&signer.sid, c)) else {
        return Verification::unknown("The signer's certificate didn't come with the message.");
    };
    let mut result = Verification {
        outcome: VerifyOutcome::Changed,
        signer: Some(info_of(cert, cert_der)),
        signer_der: Some(cert_der.clone()),
        chain: Vec::new(),
        signed_at: None,
        content: None,
    };
    let digest = hash.digest(content);
    let signed_message: Vec<u8> = if let Some(attrs) = &signer.signed_attrs {
        let mut message_digest = None;
        let mut content_type = None;
        for attr in attrs.iter() {
            let Some(value) = attr.values.iter().next() else {
                continue;
            };
            match attr.oid {
                oid::MESSAGE_DIGEST => message_digest = Some(value.value().to_vec()),
                oid::CONTENT_TYPE => {
                    content_type = value.decode_as::<der::asn1::ObjectIdentifier>().ok();
                }
                oid::SIGNING_TIME => {
                    result.signed_at = value
                        .to_der()
                        .ok()
                        .and_then(|d| x509_cert::time::Time::from_der(&d).ok())
                        .map(|t| unix(&t));
                }
                _ => {}
            }
        }
        if content_type != Some(econtent_type) {
            return result;
        }
        if message_digest.as_deref() != Some(digest.as_slice()) {
            return result;
        }
        // The signature is over the attributes as they came, tagged as a SET.
        let Some(mut raw_attrs) = ber::children(raw_signer)
            .unwrap_or_default()
            .into_iter()
            .find(|e| e.first() == Some(&IMPLICIT_0))
            .map(<[u8]>::to_vec)
        else {
            return Verification::unknown("The signature can't be read.");
        };
        if let Some(first) = raw_attrs.first_mut() {
            *first = SET;
        }
        raw_attrs
    } else {
        content.to_vec()
    };
    match super::verify_signature(
        &cert.tbs_certificate.subject_public_key_info,
        &signer.signature_algorithm,
        Some(hash),
        &signed_message,
        signer.signature.as_bytes(),
    ) {
        Ok(()) => {}
        Err(SigError::Bad) => return result,
        Err(SigError::Weak(name)) => {
            result.outcome = VerifyOutcome::Unknown(weak(name));
            return result;
        }
        Err(SigError::Unsupported) => {
            result.outcome = VerifyOutcome::Unknown(UNSUPPORTED.to_owned());
            return result;
        }
    }
    // The signature is good. Is the certificate one to sign mail with?
    let when = result
        .signed_at
        .filter(|t| *t <= cx.now.saturating_add(FUTURE_SKEW))
        .unwrap_or(cx.now);
    let validity = &cert.tbs_certificate.validity;
    if when < unix(&validity.not_before) {
        result.outcome =
            VerifyOutcome::Unknown("The signer's certificate wasn't valid yet.".to_owned());
        return result;
    }
    if when > unix(&validity.not_after) {
        result.outcome = VerifyOutcome::Unknown(
            "The signer's certificate had expired when it was signed.".to_owned(),
        );
        return result;
    }
    let usage = Usage::of(cert);
    if !usage.allows_signing() || !usage.eku_allows_email() {
        result.outcome =
            VerifyOutcome::Unknown("The signer's certificate isn't for signing email.".to_owned());
        return result;
    }
    let fingerprint = super::fingerprint(cert_der);
    if (cx.trusted)(&fingerprint) {
        result.outcome = VerifyOutcome::Verified;
        return result;
    }
    result.outcome = match path_to_root(cert, cert_der, pool, cx.roots, when) {
        Ok(()) => VerifyOutcome::Verified,
        Err(why) => VerifyOutcome::Untrusted(why),
    };
    result
}

/// Follow issuers from `cert` up to a trusted root.
fn path_to_root(
    cert: &Certificate,
    cert_der: &[u8],
    pool: &[(Certificate, Vec<u8>)],
    roots: &TrustRoots,
    when: i64,
) -> Result<(), String> {
    let untrusted = || "The certificate isn't from an authority this device trusts.".to_owned();
    let mut current: (Certificate, Vec<u8>) = (cert.clone(), cert_der.to_vec());
    for depth in 0..MAX_PATH {
        // A root this device trusts signed it.
        if roots
            .certs()
            .iter()
            .any(|root| issued_by(&current.0, &current.1, root).is_ok())
        {
            return Ok(());
        }
        // A trusted root itself (the same certificate).
        if roots.contains(&current.1) {
            return Ok(());
        }
        let issuer = pool.iter().find(|(candidate, der)| {
            *der != current.1 && issued_by(&current.0, &current.1, candidate).is_ok()
        });
        let Some((issuer, issuer_der)) = issuer else {
            return Err(untrusted());
        };
        let usage = Usage::of(issuer);
        if !usage.ca || !usage.allows_cert_signing() {
            return Err(
                "A certificate on the way to the authority isn't an authority's.".to_owned(),
            );
        }
        if usage.path_len.is_some_and(|max| usize::from(max) < depth) {
            return Err(untrusted());
        }
        let validity = &issuer.tbs_certificate.validity;
        if when < unix(&validity.not_before) || when > unix(&validity.not_after) {
            return Err("An authority's certificate on the way had expired.".to_owned());
        }
        current = (issuer.clone(), issuer_der.clone());
    }
    Err(untrusted())
}
