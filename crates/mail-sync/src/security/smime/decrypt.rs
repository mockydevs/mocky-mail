//! Reading S/MIME `EnvelopedData` (AES-CBC, triple DES) and
//! `AuthEnvelopedData` (AES-GCM, RFC 5083/5084) with one of the user's own
//! RSA keys (key transport, PKCS #1 v1.5 or OAEP). Key agreement (ECDH)
//! recipients are not supported.

use aes::cipher::{BlockDecryptMut as _, KeyIvInit as _, block_padding::Pkcs7};
use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use cms::cert::IssuerAndSerialNumber;
use cms::content_info::{CmsVersion, ContentInfo};
use cms::enveloped_data::{
    EncryptedContentInfo, EnvelopedData, OriginatorInfo, RecipientIdentifier, RecipientInfo,
    RecipientInfos,
};
use der::asn1::OctetString;
use der::{Decode, Encode, Sequence};
use rand_core::OsRng;
use x509_cert::attr::Attributes;
use zeroize::Zeroizing;

use super::{Identity, PrivateKey, oid, subject_key_id};
use crate::security::ber;

/// Why an S/MIME message couldn't be decrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecryptError {
    /// It isn't encrypted to any of the user's certificates.
    NoKey,
    /// It uses something this can't decrypt: what.
    Unsupported(&'static str),
    /// It can't be read, or decrypting it failed.
    Damaged,
}

impl std::fmt::Display for DecryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoKey => f.write_str(
                "It wasn't encrypted to a certificate of yours on this device. Import the certificate (.p12) it was sent to.",
            ),
            Self::Unsupported(what) => write!(f, "It uses {what}, which Mocky Mail can't decrypt."),
            Self::Damaged => f.write_str("It is damaged, or was changed after it was encrypted."),
        }
    }
}

/// `AuthEnvelopedData` (RFC 5083).
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct AuthEnvelopedData {
    version: CmsVersion,
    #[asn1(
        context_specific = "0",
        tag_mode = "IMPLICIT",
        constructed = "true",
        optional = "true"
    )]
    originator_info: Option<OriginatorInfo>,
    recip_infos: RecipientInfos,
    auth_encrypted_content: EncryptedContentInfo,
    #[asn1(
        context_specific = "1",
        tag_mode = "IMPLICIT",
        constructed = "true",
        optional = "true"
    )]
    auth_attrs: Option<Attributes>,
    mac: OctetString,
    #[asn1(
        context_specific = "2",
        tag_mode = "IMPLICIT",
        constructed = "true",
        optional = "true"
    )]
    unauth_attrs: Option<Attributes>,
}

/// `GCMParameters` (RFC 5084).
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct GcmParameters {
    nonce: OctetString,
    #[asn1(optional = "true")]
    icv_len: Option<u8>,
}

/// Decrypt an `application/pkcs7-mime` body (DER or BER) with one of
/// `identities`: the MIME entity inside.
///
/// # Errors
/// [`DecryptError`] saying why not.
pub fn decrypt(p7m: &[u8], identities: &[Identity]) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let der = ber::to_der(p7m).ok_or(DecryptError::Damaged)?;
    let info = ContentInfo::from_der(&der).map_err(|_| DecryptError::Damaged)?;
    let content = info.content.to_der().map_err(|_| DecryptError::Damaged)?;
    match info.content_type {
        oid::ENVELOPED_DATA => {
            let enveloped = EnvelopedData::from_der(&content).map_err(|_| DecryptError::Damaged)?;
            let key = content_key(&enveloped.recip_infos, identities)?;
            decrypt_content(&enveloped.encrypted_content, &key, None)
        }
        oid::AUTH_ENVELOPED_DATA => {
            let enveloped =
                AuthEnvelopedData::from_der(&content).map_err(|_| DecryptError::Damaged)?;
            let key = content_key(&enveloped.recip_infos, identities)?;
            let aad = match &enveloped.auth_attrs {
                Some(attrs) => attrs.to_der().map_err(|_| DecryptError::Damaged)?,
                None => Vec::new(),
            };
            decrypt_content(
                &enveloped.auth_encrypted_content,
                &key,
                Some((enveloped.mac.as_bytes(), &aad)),
            )
        }
        _ => Err(DecryptError::Unsupported("a kind of S/MIME message")),
    }
}

fn is_for(rid: &RecipientIdentifier, identity: &Identity) -> bool {
    let tbs = &identity.cert.tbs_certificate;
    match rid {
        RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer,
            serial_number,
        }) => tbs.issuer == *issuer && tbs.serial_number == *serial_number,
        RecipientIdentifier::SubjectKeyIdentifier(ski) => {
            subject_key_id(&identity.cert).is_some_and(|id| id == ski.0.as_bytes())
        }
    }
}

/// The content-encryption key, unwrapped with the first of the user's keys
/// it was sent to.
fn content_key(
    infos: &RecipientInfos,
    identities: &[Identity],
) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let mut unsupported = None;
    for info in infos.0.iter() {
        match info {
            RecipientInfo::Ktri(ktri) => {
                for identity in identities.iter().filter(|i| is_for(&ktri.rid, i)) {
                    let PrivateKey::Rsa(key) = &identity.key else {
                        continue;
                    };
                    let wrapped = ktri.enc_key.as_bytes();
                    let unwrapped = if ktri.key_enc_alg.oid == oid::RSA {
                        key.decrypt_blinded(&mut OsRng, rsa::Pkcs1v15Encrypt, wrapped)
                    } else if ktri.key_enc_alg.oid == oid::RSAES_OAEP {
                        let padding = oaep(ktri.key_enc_alg.parameters.as_ref())?;
                        key.decrypt_blinded(&mut OsRng, padding, wrapped)
                    } else {
                        unsupported = Some("a key transport");
                        continue;
                    };
                    return unwrapped
                        .map(Zeroizing::new)
                        .map_err(|_| DecryptError::Damaged);
                }
            }
            RecipientInfo::Kari(_) => {
                unsupported = Some("elliptic-curve key agreement");
            }
            _ => {}
        }
    }
    Err(unsupported.map_or(DecryptError::NoKey, DecryptError::Unsupported))
}

/// RSAES-OAEP with the hash its parameters name (SHA-1 when they don't).
fn oaep(params: Option<&der::Any>) -> Result<rsa::Oaep, DecryptError> {
    let Some(params) = params else {
        return Ok(rsa::Oaep::new::<sha1::Sha1>());
    };
    let bytes = params.to_der().map_err(|_| DecryptError::Damaged)?;
    let parsed = pkcs1::RsaOaepParams::from_der(&bytes).map_err(|_| DecryptError::Damaged)?;
    let hash = parsed.hash.oid;
    Ok(if hash == oid::SHA256 {
        rsa::Oaep::new::<sha2::Sha256>()
    } else if hash == oid::SHA384 {
        rsa::Oaep::new::<sha2::Sha384>()
    } else if hash == oid::SHA512 {
        rsa::Oaep::new::<sha2::Sha512>()
    } else if hash == oid::SHA1 {
        rsa::Oaep::new::<sha1::Sha1>()
    } else {
        return Err(DecryptError::Unsupported("an OAEP hash"));
    })
}

fn decrypt_content(
    content: &EncryptedContentInfo,
    key: &[u8],
    gcm: Option<(&[u8], &[u8])>,
) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let ciphertext = content
        .encrypted_content
        .as_ref()
        .ok_or(DecryptError::Damaged)?
        .as_bytes();
    let alg = &content.content_enc_alg;
    let params = alg.parameters.as_ref().ok_or(DecryptError::Damaged)?;
    let cbc_iv = || {
        params
            .decode_as::<OctetString>()
            .map(|iv| iv.as_bytes().to_vec())
            .map_err(|_| DecryptError::Damaged)
    };
    let plain = match (alg.oid, gcm) {
        (oid::AES128_CBC, None) => cbc::Decryptor::<aes::Aes128>::new_from_slices(key, &cbc_iv()?)
            .map_err(|_| DecryptError::Damaged)?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|_| DecryptError::Damaged),
        (oid::AES192_CBC, None) => cbc::Decryptor::<aes::Aes192>::new_from_slices(key, &cbc_iv()?)
            .map_err(|_| DecryptError::Damaged)?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|_| DecryptError::Damaged),
        (oid::AES256_CBC, None) => cbc::Decryptor::<aes::Aes256>::new_from_slices(key, &cbc_iv()?)
            .map_err(|_| DecryptError::Damaged)?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|_| DecryptError::Damaged),
        (oid::DES_EDE3_CBC, None) => {
            cbc::Decryptor::<des::TdesEde3>::new_from_slices(key, &cbc_iv()?)
                .map_err(|_| DecryptError::Damaged)?
                .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
                .map_err(|_| DecryptError::Damaged)
        }
        (oid::AES128_GCM | oid::AES256_GCM, Some((mac, aad))) => {
            let params = params
                .decode_as::<GcmParameters>()
                .map_err(|_| DecryptError::Damaged)?;
            if params.nonce.as_bytes().len() != 12 || mac.len() != 16 {
                return Err(DecryptError::Unsupported("an AES-GCM nonce or tag size"));
            }
            let nonce = aes_gcm::Nonce::from_slice(params.nonce.as_bytes());
            let mut joined = ciphertext.to_vec();
            joined.extend_from_slice(mac);
            let payload = Payload { msg: &joined, aad };
            if alg.oid == oid::AES128_GCM {
                aes_gcm::Aes128Gcm::new_from_slice(key)
                    .map_err(|_| DecryptError::Damaged)?
                    .decrypt(nonce, payload)
            } else {
                aes_gcm::Aes256Gcm::new_from_slice(key)
                    .map_err(|_| DecryptError::Damaged)?
                    .decrypt(nonce, payload)
            }
            .map_err(|_| DecryptError::Damaged)
        }
        _ => return Err(DecryptError::Unsupported("an encryption algorithm")),
    };
    plain.map(Zeroizing::new)
}
