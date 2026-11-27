//! S/MIME (RFC 8551): certificates, checking and making signatures,
//! encrypting and decrypting, and reading `.p12` files. CMS and X.509 are
//! read with RustCrypto's DER types after BER is made DER; the algorithms
//! are RSA (PKCS #1 v1.5 signatures, v1.5 and OAEP key transport) and
//! ECDSA on P-256 and P-384, with SHA-2. MD5 and SHA-1 signatures and RSA
//! keys under 2048 bits are refused.

pub(crate) mod build;
mod decrypt;
pub mod pkcs12;
pub mod trust;
pub mod verify;

pub use build::{encrypt, sign};
pub use decrypt::{DecryptError, decrypt};
pub use verify::{TrustContext, Verification, VerifyOutcome, verify_detached, verify_opaque};

use der::asn1::ObjectIdentifier;
use der::{Decode, Encode};
use sha2::Digest as _;
use spki::{AlgorithmIdentifierOwned, SubjectPublicKeyInfoOwned};
use x509_cert::Certificate;
use x509_cert::ext::pkix::{
    BasicConstraints, ExtendedKeyUsage, KeyUsage, KeyUsages, SubjectAltName, SubjectKeyIdentifier,
    name::GeneralName,
};
use x509_cert::name::Name;
use zeroize::Zeroizing;

use crate::SyncError;

pub(crate) mod oid {
    use der::asn1::ObjectIdentifier as O;
    pub const DATA: O = O::new_unwrap("1.2.840.113549.1.7.1");
    pub const SIGNED_DATA: O = O::new_unwrap("1.2.840.113549.1.7.2");
    pub const ENVELOPED_DATA: O = O::new_unwrap("1.2.840.113549.1.7.3");
    pub const ENCRYPTED_DATA: O = O::new_unwrap("1.2.840.113549.1.7.6");
    pub const AUTH_ENVELOPED_DATA: O = O::new_unwrap("1.2.840.113549.1.9.16.1.23");
    pub const CONTENT_TYPE: O = O::new_unwrap("1.2.840.113549.1.9.3");
    pub const MESSAGE_DIGEST: O = O::new_unwrap("1.2.840.113549.1.9.4");
    pub const SIGNING_TIME: O = O::new_unwrap("1.2.840.113549.1.9.5");
    pub const SMIME_CAPABILITIES: O = O::new_unwrap("1.2.840.113549.1.9.15");
    pub const EMAIL_ADDRESS: O = O::new_unwrap("1.2.840.113549.1.9.1");
    pub const COMMON_NAME: O = O::new_unwrap("2.5.4.3");
    pub const ORGANIZATION: O = O::new_unwrap("2.5.4.10");

    pub const RSA: O = O::new_unwrap("1.2.840.113549.1.1.1");
    pub const MD5_RSA: O = O::new_unwrap("1.2.840.113549.1.1.4");
    pub const SHA1_RSA: O = O::new_unwrap("1.2.840.113549.1.1.5");
    pub const RSAES_OAEP: O = O::new_unwrap("1.2.840.113549.1.1.7");
    pub const RSASSA_PSS: O = O::new_unwrap("1.2.840.113549.1.1.10");
    pub const SHA256_RSA: O = O::new_unwrap("1.2.840.113549.1.1.11");
    pub const SHA384_RSA: O = O::new_unwrap("1.2.840.113549.1.1.12");
    pub const SHA512_RSA: O = O::new_unwrap("1.2.840.113549.1.1.13");
    pub const SHA224_RSA: O = O::new_unwrap("1.2.840.113549.1.1.14");
    pub const EC_PUBLIC_KEY: O = O::new_unwrap("1.2.840.10045.2.1");
    pub const ECDSA_SHA1: O = O::new_unwrap("1.2.840.10045.4.1");
    pub const ECDSA_SHA224: O = O::new_unwrap("1.2.840.10045.4.3.1");
    pub const ECDSA_SHA256: O = O::new_unwrap("1.2.840.10045.4.3.2");
    pub const ECDSA_SHA384: O = O::new_unwrap("1.2.840.10045.4.3.3");
    pub const ECDSA_SHA512: O = O::new_unwrap("1.2.840.10045.4.3.4");
    pub const P256: O = O::new_unwrap("1.2.840.10045.3.1.7");
    pub const P384: O = O::new_unwrap("1.3.132.0.34");

    pub const MD5: O = O::new_unwrap("1.2.840.113549.2.5");
    pub const SHA1: O = O::new_unwrap("1.3.14.3.2.26");
    pub const SHA224: O = O::new_unwrap("2.16.840.1.101.3.4.2.4");
    pub const SHA256: O = O::new_unwrap("2.16.840.1.101.3.4.2.1");
    pub const SHA384: O = O::new_unwrap("2.16.840.1.101.3.4.2.2");
    pub const SHA512: O = O::new_unwrap("2.16.840.1.101.3.4.2.3");

    pub const AES128_CBC: O = O::new_unwrap("2.16.840.1.101.3.4.1.2");
    pub const AES192_CBC: O = O::new_unwrap("2.16.840.1.101.3.4.1.22");
    pub const AES256_CBC: O = O::new_unwrap("2.16.840.1.101.3.4.1.42");
    pub const AES128_GCM: O = O::new_unwrap("2.16.840.1.101.3.4.1.6");
    pub const AES256_GCM: O = O::new_unwrap("2.16.840.1.101.3.4.1.46");
    pub const DES_EDE3_CBC: O = O::new_unwrap("1.2.840.113549.3.7");

    pub const KEY_USAGE: O = O::new_unwrap("2.5.29.15");
    pub const SUBJECT_ALT_NAME: O = O::new_unwrap("2.5.29.17");
    pub const BASIC_CONSTRAINTS: O = O::new_unwrap("2.5.29.19");
    pub const EXT_KEY_USAGE: O = O::new_unwrap("2.5.29.37");
    pub const SUBJECT_KEY_ID: O = O::new_unwrap("2.5.29.14");
    pub const EMAIL_PROTECTION: O = O::new_unwrap("1.3.6.1.5.5.7.3.4");
    pub const ANY_EXTENDED_KEY_USAGE: O = O::new_unwrap("2.5.29.37.0");
}

/// A hash a signature may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hash {
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    /// From a digest algorithm's OID: `Err` names a weak one.
    pub(crate) fn from_oid(oid: &ObjectIdentifier) -> Option<Result<Self, &'static str>> {
        Some(Ok(match *oid {
            oid::SHA224 => Self::Sha224,
            oid::SHA256 => Self::Sha256,
            oid::SHA384 => Self::Sha384,
            oid::SHA512 => Self::Sha512,
            oid::SHA1 => return Some(Err("SHA-1")),
            oid::MD5 => return Some(Err("MD5")),
            _ => return None,
        }))
    }

    pub(crate) const fn oid(self) -> ObjectIdentifier {
        match self {
            Self::Sha224 => oid::SHA224,
            Self::Sha256 => oid::SHA256,
            Self::Sha384 => oid::SHA384,
            Self::Sha512 => oid::SHA512,
        }
    }

    pub(crate) fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha224 => sha2::Sha224::digest(data).to_vec(),
            Self::Sha256 => sha2::Sha256::digest(data).to_vec(),
            Self::Sha384 => sha2::Sha384::digest(data).to_vec(),
            Self::Sha512 => sha2::Sha512::digest(data).to_vec(),
        }
    }
}

/// Upper-case hex.
pub(crate) fn hex_upper(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(out, "{byte:02X}");
    }
    out
}

/// A certificate's SHA-256 fingerprint, upper-case hex: how it is known
/// in the key store.
#[must_use]
pub fn fingerprint(der: &[u8]) -> String {
    hex_upper(&sha2::Sha256::digest(der))
}

/// A private key S/MIME can use.
pub enum PrivateKey {
    /// RSA, 2048 bits or more.
    Rsa(Box<rsa::RsaPrivateKey>),
    /// ECDSA on P-256.
    P256(p256::SecretKey),
    /// ECDSA on P-384.
    P384(p384::SecretKey),
}

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Rsa(_) => "PrivateKey::Rsa",
            Self::P256(_) => "PrivateKey::P256",
            Self::P384(_) => "PrivateKey::P384",
        })
    }
}

impl PrivateKey {
    /// From a PKCS #8 `PrivateKeyInfo` (DER).
    ///
    /// # Errors
    /// [`SyncError::Invalid`] for a key of another kind or one that can't
    /// be read.
    pub fn from_pkcs8(der: &[u8]) -> Result<Self, SyncError> {
        use pkcs8::DecodePrivateKey as _;
        let info = pkcs8::PrivateKeyInfo::from_der(der).map_err(|_| unreadable_key())?;
        let algorithm = info.algorithm.oid;
        if algorithm == oid::RSA {
            let key = rsa::RsaPrivateKey::from_pkcs8_der(der).map_err(|_| unreadable_key())?;
            return Ok(Self::Rsa(Box::new(key)));
        }
        if algorithm == oid::EC_PUBLIC_KEY {
            let curve = info
                .algorithm
                .parameters_oid()
                .map_err(|_| unreadable_key())?;
            return match curve {
                oid::P256 => Ok(Self::P256(
                    p256::SecretKey::from_pkcs8_der(der).map_err(|_| unreadable_key())?,
                )),
                oid::P384 => Ok(Self::P384(
                    p384::SecretKey::from_pkcs8_der(der).map_err(|_| unreadable_key())?,
                )),
                _ => Err(SyncError::Invalid(
                    "This key uses an elliptic curve Mocky Mail doesn't support (P-256 and P-384 only)."
                        .to_owned(),
                )),
            };
        }
        Err(SyncError::Invalid(
            "This key is of a kind Mocky Mail doesn't support (RSA and ECDSA only).".to_owned(),
        ))
    }

    /// The PKCS #8 `PrivateKeyInfo` (DER), for sealing.
    ///
    /// # Errors
    /// When the key can't be encoded.
    pub fn to_pkcs8(&self) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        use pkcs8::EncodePrivateKey as _;
        let doc = match self {
            Self::Rsa(key) => key.to_pkcs8_der(),
            Self::P256(key) => key.to_pkcs8_der(),
            Self::P384(key) => key.to_pkcs8_der(),
        }
        .map_err(|_| unreadable_key())?;
        Ok(Zeroizing::new(doc.as_bytes().to_vec()))
    }

    /// Whether this is the private half of the key in `spki`.
    #[must_use]
    pub fn matches(&self, spki: &SubjectPublicKeyInfoOwned) -> bool {
        use pkcs8::DecodePublicKey as _;
        let Ok(der) = spki.to_der() else { return false };
        match self {
            Self::Rsa(key) => rsa::RsaPublicKey::from_public_key_der(&der)
                .is_ok_and(|public| public == key.to_public_key()),
            Self::P256(key) => {
                p256::PublicKey::from_public_key_der(&der).is_ok_and(|p| p == key.public_key())
            }
            Self::P384(key) => {
                p384::PublicKey::from_public_key_der(&der).is_ok_and(|p| p == key.public_key())
            }
        }
    }
}

fn unreadable_key() -> SyncError {
    SyncError::Invalid("The private key in this file can't be read.".to_owned())
}

/// The user's own S/MIME key: certificate, the issuers that came with it,
/// and the private key.
#[derive(Debug)]
pub struct Identity {
    /// The certificate (DER).
    pub cert_der: Vec<u8>,
    /// The certificate, read.
    pub cert: Certificate,
    /// Issuer certificates sent along with signatures (DER).
    pub chain: Vec<Vec<u8>>,
    /// The private key.
    pub key: PrivateKey,
}

impl Identity {
    /// From a certificate, its issuers and the PKCS #8 private key.
    ///
    /// # Errors
    /// [`SyncError::Invalid`] when they can't be read or don't belong
    /// together.
    pub fn new(cert_der: Vec<u8>, chain: Vec<Vec<u8>>, pkcs8: &[u8]) -> Result<Self, SyncError> {
        let cert = Certificate::from_der(&cert_der).map_err(|_| unreadable_cert())?;
        let key = PrivateKey::from_pkcs8(pkcs8)?;
        if !key.matches(&cert.tbs_certificate.subject_public_key_info) {
            return Err(SyncError::Invalid(
                "The private key doesn't belong to the certificate.".to_owned(),
            ));
        }
        Ok(Self {
            cert_der,
            cert,
            chain,
            key,
        })
    }
}

fn unreadable_cert() -> SyncError {
    SyncError::Invalid("This certificate can't be read.".to_owned())
}

/// What a certificate says, for the key list and for choosing recipients.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each is a separate fact about the certificate"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    /// SHA-256 of the certificate, upper-case hex.
    pub fingerprint: String,
    /// The subject's common name, else its address, else the whole subject.
    pub owner: String,
    /// Email addresses: the subject alternative names and the subject's
    /// `emailAddress`, lower case.
    pub addresses: Vec<String>,
    /// Who issued it (common name or organisation).
    pub issuer: String,
    /// Unix seconds.
    pub not_before: i64,
    /// Unix seconds.
    pub not_after: i64,
    /// It may sign mail: key usage allows signatures and the extended key
    /// usage (if any) names email protection.
    pub can_sign: bool,
    /// Mail can be encrypted to it: an RSA key whose key usage allows key
    /// encipherment, for email protection.
    pub can_encrypt: bool,
    /// A certification authority's certificate.
    pub is_ca: bool,
}

/// Read a certificate (DER) into [`CertInfo`].
///
/// # Errors
/// [`SyncError::Invalid`] when it isn't an X.509 certificate.
pub fn cert_info(der: &[u8]) -> Result<CertInfo, SyncError> {
    let cert = Certificate::from_der(der).map_err(|_| unreadable_cert())?;
    Ok(info_of(&cert, der))
}

pub(crate) fn info_of(cert: &Certificate, der: &[u8]) -> CertInfo {
    let tbs = &cert.tbs_certificate;
    let addresses = cert_addresses(cert);
    let owner = name_part(&tbs.subject, oid::COMMON_NAME)
        .or_else(|| addresses.first().cloned())
        .unwrap_or_else(|| tbs.subject.to_string());
    let issuer = name_part(&tbs.issuer, oid::COMMON_NAME)
        .or_else(|| name_part(&tbs.issuer, oid::ORGANIZATION))
        .unwrap_or_else(|| tbs.issuer.to_string());
    let usage = Usage::of(cert);
    let email = usage.eku_allows_email();
    let rsa = tbs.subject_public_key_info.algorithm.oid == oid::RSA;
    CertInfo {
        fingerprint: fingerprint(der),
        owner,
        addresses,
        issuer,
        not_before: unix(&tbs.validity.not_before),
        not_after: unix(&tbs.validity.not_after),
        can_sign: email && usage.allows_signing() && !usage.ca,
        can_encrypt: email && rsa && usage.allows_key_encipherment() && !usage.ca,
        is_ca: usage.ca,
    }
}

pub(crate) fn unix(time: &x509_cert::time::Time) -> i64 {
    i64::try_from(time.to_unix_duration().as_secs()).unwrap_or(i64::MAX)
}

/// The text of the first attribute `kind` in a name.
pub(crate) fn name_part(name: &Name, kind: ObjectIdentifier) -> Option<String> {
    name.0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find(|atv| atv.oid == kind)
        .and_then(|atv| any_text(&atv.value))
        .map(|text| crate::security::clean_text(&text))
        .filter(|text| !text.is_empty())
}

/// A directory string's text.
fn any_text(value: &der::Any) -> Option<String> {
    use der::{Tag, Tagged as _};
    let bytes = value.value();
    match value.tag() {
        Tag::Utf8String | Tag::PrintableString | Tag::Ia5String | Tag::TeletexString => {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
        Tag::BmpString => {
            let units: Vec<u16> = bytes
                .chunks(2)
                .map(|pair| match pair {
                    [hi, lo] => u16::from_be_bytes([*hi, *lo]),
                    _ => 0xFFFD,
                })
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

/// A certificate's email addresses, lower case.
pub(crate) fn cert_addresses(cert: &Certificate) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(san) = extension::<SubjectAltName>(cert, oid::SUBJECT_ALT_NAME) {
        for name in san.0 {
            if let GeneralName::Rfc822Name(address) = name {
                out.push(address.to_string().trim().to_lowercase());
            }
        }
    }
    for rdn in &cert.tbs_certificate.subject.0 {
        for atv in rdn.0.iter() {
            if atv.oid == oid::EMAIL_ADDRESS
                && let Some(text) = any_text(&atv.value)
            {
                out.push(text.trim().to_lowercase());
            }
        }
    }
    out.retain(|a| a.contains('@'));
    out.sort();
    out.dedup();
    out
}

/// A certificate extension, read.
pub(crate) fn extension<'a, T: Decode<'a>>(
    cert: &'a Certificate,
    id: ObjectIdentifier,
) -> Option<T> {
    cert.tbs_certificate
        .extensions
        .as_ref()?
        .iter()
        .find(|ext| ext.extn_id == id)
        .and_then(|ext| T::from_der(ext.extn_value.as_bytes()).ok())
}

/// A certificate's key usage, extended key usage and CA flag.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each is a separate fact about the certificate"
)]
pub(crate) struct Usage {
    bits: Option<KeyUsage>,
    eku: Option<Vec<ObjectIdentifier>>,
    pub(crate) ca: bool,
    pub(crate) path_len: Option<u8>,
}

impl Usage {
    pub(crate) fn of(cert: &Certificate) -> Self {
        let constraints = extension::<BasicConstraints>(cert, oid::BASIC_CONSTRAINTS);
        Self {
            bits: extension::<KeyUsage>(cert, oid::KEY_USAGE),
            eku: extension::<ExtendedKeyUsage>(cert, oid::EXT_KEY_USAGE).map(|e| e.0),
            ca: constraints.as_ref().is_some_and(|c| c.ca),
            path_len: constraints.and_then(|c| c.path_len_constraint),
        }
    }

    fn has(&self, usage: KeyUsages) -> bool {
        self.bits.as_ref().is_none_or(|ku| ku.0.contains(usage))
    }

    pub(crate) fn allows_signing(&self) -> bool {
        self.has(KeyUsages::DigitalSignature) || self.has(KeyUsages::NonRepudiation)
    }

    pub(crate) fn allows_key_encipherment(&self) -> bool {
        self.has(KeyUsages::KeyEncipherment)
    }

    pub(crate) fn allows_cert_signing(&self) -> bool {
        self.has(KeyUsages::KeyCertSign)
    }

    /// No extended key usage, or one that names email protection (or any).
    pub(crate) fn eku_allows_email(&self) -> bool {
        self.eku.as_ref().is_none_or(|uses| {
            uses.iter()
                .any(|u| *u == oid::EMAIL_PROTECTION || *u == oid::ANY_EXTENDED_KEY_USAGE)
        })
    }
}

/// A certificate's subject key identifier.
pub(crate) fn subject_key_id(cert: &Certificate) -> Option<Vec<u8>> {
    extension::<SubjectKeyIdentifier>(cert, oid::SUBJECT_KEY_ID)
        .map(|ski| ski.0.as_bytes().to_vec())
}

/// Why a signature or key can't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SigError {
    /// It doesn't match: what was signed was changed (or another key).
    Bad,
    /// An algorithm no longer safe: its name.
    Weak(&'static str),
    /// An algorithm this doesn't do.
    Unsupported,
}

/// The hash a signature algorithm names, or `None` when it names none
/// (`rsaEncryption`, which takes the digest algorithm's).
fn hash_of_signature(alg: &ObjectIdentifier) -> Result<Option<Hash>, SigError> {
    Ok(Some(match *alg {
        oid::SHA224_RSA | oid::ECDSA_SHA224 => Hash::Sha224,
        oid::SHA256_RSA | oid::ECDSA_SHA256 => Hash::Sha256,
        oid::SHA384_RSA | oid::ECDSA_SHA384 => Hash::Sha384,
        oid::SHA512_RSA | oid::ECDSA_SHA512 => Hash::Sha512,
        oid::SHA1_RSA | oid::ECDSA_SHA1 => return Err(SigError::Weak("SHA-1")),
        oid::MD5_RSA => return Err(SigError::Weak("MD5")),
        oid::RSA | oid::EC_PUBLIC_KEY => return Ok(None),
        _ => return Err(SigError::Unsupported),
    }))
}

/// RSA keys smaller than this are refused.
pub(crate) const MIN_RSA_BITS: usize = 2048;

/// Check `signature` over `message` by the key in `spki`, with the
/// signature algorithm `alg` (and the digest algorithm `digest`, when the
/// signature algorithm doesn't name one).
pub(crate) fn verify_signature(
    spki: &SubjectPublicKeyInfoOwned,
    alg: &AlgorithmIdentifierOwned,
    digest: Option<Hash>,
    message: &[u8],
    signature: &[u8],
) -> Result<(), SigError> {
    use pkcs8::DecodePublicKey as _;
    use rsa::traits::PublicKeyParts as _;
    use signature::Verifier as _;
    use signature::hazmat::PrehashVerifier as _;
    if alg.oid == oid::RSASSA_PSS {
        return Err(SigError::Unsupported);
    }
    let hash = hash_of_signature(&alg.oid)?
        .or(digest)
        .ok_or(SigError::Unsupported)?;
    let der = spki.to_der().map_err(|_| SigError::Unsupported)?;
    match spki.algorithm.oid {
        oid::RSA => {
            let key = rsa::RsaPublicKey::from_public_key_der(&der).map_err(|_| SigError::Bad)?;
            if key.n().bits() < MIN_RSA_BITS {
                return Err(SigError::Weak("RSA under 2048 bits"));
            }
            let sig = rsa::pkcs1v15::Signature::try_from(signature).map_err(|_| SigError::Bad)?;
            let result = match hash {
                Hash::Sha224 => {
                    rsa::pkcs1v15::VerifyingKey::<sha2::Sha224>::new(key).verify(message, &sig)
                }
                Hash::Sha256 => {
                    rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(key).verify(message, &sig)
                }
                Hash::Sha384 => {
                    rsa::pkcs1v15::VerifyingKey::<sha2::Sha384>::new(key).verify(message, &sig)
                }
                Hash::Sha512 => {
                    rsa::pkcs1v15::VerifyingKey::<sha2::Sha512>::new(key).verify(message, &sig)
                }
            };
            result.map_err(|_| SigError::Bad)
        }
        oid::EC_PUBLIC_KEY => {
            let curve = spki
                .algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.decode_as::<ObjectIdentifier>().ok())
                .ok_or(SigError::Unsupported)?;
            let prehash = hash.digest(message);
            match curve {
                oid::P256 => {
                    let key = p256::ecdsa::VerifyingKey::from_public_key_der(&der)
                        .map_err(|_| SigError::Bad)?;
                    let sig =
                        p256::ecdsa::Signature::from_der(signature).map_err(|_| SigError::Bad)?;
                    key.verify_prehash(&prehash, &sig)
                        .map_err(|_| SigError::Bad)
                }
                oid::P384 => {
                    let key = p384::ecdsa::VerifyingKey::from_public_key_der(&der)
                        .map_err(|_| SigError::Bad)?;
                    let sig =
                        p384::ecdsa::Signature::from_der(signature).map_err(|_| SigError::Bad)?;
                    key.verify_prehash(&prehash, &sig)
                        .map_err(|_| SigError::Bad)
                }
                _ => Err(SigError::Unsupported),
            }
        }
        _ => Err(SigError::Unsupported),
    }
}

/// The DER of a certificate's `tbsCertificate`, as it came.
pub(crate) fn tbs_bytes(cert_der: &[u8]) -> Option<&[u8]> {
    crate::security::ber::children(cert_der)?.first().copied()
}

/// Whether `issuer`'s key signed `cert` (given as DER).
pub(crate) fn issued_by(
    cert: &Certificate,
    cert_der: &[u8],
    issuer: &Certificate,
) -> Result<(), SigError> {
    if cert.tbs_certificate.issuer != issuer.tbs_certificate.subject {
        return Err(SigError::Bad);
    }
    let tbs = tbs_bytes(cert_der).ok_or(SigError::Bad)?;
    verify_signature(
        &issuer.tbs_certificate.subject_public_key_info,
        &cert.signature_algorithm,
        None,
        tbs,
        cert.signature.raw_bytes(),
    )
}

/// Read certificates from PEM or DER.
///
/// # Errors
/// [`SyncError::Invalid`] when there are none.
pub fn read_certificates(bytes: &[u8]) -> Result<Vec<Vec<u8>>, SyncError> {
    if crate::security::mime::find(bytes, b"-----BEGIN").is_some() {
        let certs = Certificate::load_pem_chain(bytes).map_err(|_| unreadable_cert())?;
        let ders: Vec<Vec<u8>> = certs.iter().filter_map(|c| c.to_der().ok()).collect();
        return if ders.is_empty() {
            Err(unreadable_cert())
        } else {
            Ok(ders)
        };
    }
    let der = crate::security::ber::to_der(bytes).ok_or_else(unreadable_cert)?;
    Certificate::from_der(&der).map_err(|_| unreadable_cert())?;
    Ok(vec![der])
}

/// A certificate as PEM text, to share.
///
/// # Errors
/// When it isn't a certificate.
pub fn to_pem(der: &[u8]) -> Result<String, SyncError> {
    use der::EncodePem as _;
    Certificate::from_der(der)
        .map_err(|_| unreadable_cert())?
        .to_pem(der::pem::LineEnding::CRLF)
        .map_err(|_| unreadable_cert())
}
