//! Making S/MIME: a detached CMS signature (SHA-256, or SHA-384 with a
//! P-384 key; RSA PKCS #1 v1.5 or ECDSA) and CMS `EnvelopedData` with
//! AES-256-CBC, its key sent to each RSA recipient with PKCS #1 v1.5
//! (RFC 8551 §2.7: AES-256-CBC is what every S/MIME reader in use
//! decrypts; AES-GCM `AuthEnvelopedData` is read, not written).

use std::time::Duration;

use aes::cipher::{BlockEncryptMut as _, KeyIvInit as _, block_padding::Pkcs7};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::enveloped_data::{
    EncryptedContentInfo, EnvelopedData, KeyTransRecipientInfo, RecipientIdentifier, RecipientInfo,
    RecipientInfos,
};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use der::asn1::{Any, Null, OctetString, SetOfVec, UtcTime};
use der::{Decode, Encode};
use rand_core::{OsRng, RngCore as _};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;
use x509_cert::attr::Attribute;
use zeroize::Zeroizing;

use super::{Hash, Identity, PrivateKey, oid};
use crate::SyncError;

fn failed(what: &str) -> SyncError {
    SyncError::Invalid(format!("Couldn't {what} the message with S/MIME."))
}

fn attribute(id: der::asn1::ObjectIdentifier, value: Any) -> Result<Attribute, SyncError> {
    Ok(Attribute {
        oid: id,
        values: SetOfVec::try_from(vec![value]).map_err(|_| failed("sign"))?,
    })
}

/// Sign `message` (the signed attributes, DER) with `key`: the signature
/// algorithm to name and the signature.
fn sign_with(
    key: &PrivateKey,
    hash: Hash,
    message: &[u8],
) -> Result<(AlgorithmIdentifierOwned, Vec<u8>), SyncError> {
    use signature::SignatureEncoding as _;
    use signature::hazmat::PrehashSigner as _;
    match key {
        PrivateKey::Rsa(key) => {
            use signature::RandomizedSigner as _;
            let signature = match hash {
                Hash::Sha384 => rsa::pkcs1v15::SigningKey::<sha2::Sha384>::new((**key).clone())
                    .try_sign_with_rng(&mut OsRng, message)
                    .map(|s| s.to_vec()),
                Hash::Sha512 => rsa::pkcs1v15::SigningKey::<sha2::Sha512>::new((**key).clone())
                    .try_sign_with_rng(&mut OsRng, message)
                    .map(|s| s.to_vec()),
                Hash::Sha224 | Hash::Sha256 => {
                    rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new((**key).clone())
                        .try_sign_with_rng(&mut OsRng, message)
                        .map(|s| s.to_vec())
                }
            }
            .map_err(|_| failed("sign"))?;
            let null = Any::encode_from(&Null).map_err(|_| failed("sign"))?;
            Ok((
                AlgorithmIdentifierOwned {
                    oid: oid::RSA,
                    parameters: Some(null),
                },
                signature,
            ))
        }
        PrivateKey::P256(key) => {
            let signer = p256::ecdsa::SigningKey::from(key);
            let signature: p256::ecdsa::Signature = signer
                .sign_prehash(&hash.digest(message))
                .map_err(|_| failed("sign"))?;
            Ok((
                AlgorithmIdentifierOwned {
                    oid: ecdsa_oid(hash),
                    parameters: None,
                },
                signature.to_der().to_vec(),
            ))
        }
        PrivateKey::P384(key) => {
            let signer = p384::ecdsa::SigningKey::from(key);
            let signature: p384::ecdsa::Signature = signer
                .sign_prehash(&hash.digest(message))
                .map_err(|_| failed("sign"))?;
            Ok((
                AlgorithmIdentifierOwned {
                    oid: ecdsa_oid(hash),
                    parameters: None,
                },
                signature.to_der().to_vec(),
            ))
        }
    }
}

const fn ecdsa_oid(hash: Hash) -> der::asn1::ObjectIdentifier {
    match hash {
        Hash::Sha224 => oid::ECDSA_SHA224,
        Hash::Sha256 => oid::ECDSA_SHA256,
        Hash::Sha384 => oid::ECDSA_SHA384,
        Hash::Sha512 => oid::ECDSA_SHA512,
    }
}

/// The hash a key signs with.
pub(crate) const fn hash_for(key: &PrivateKey) -> Hash {
    match key {
        PrivateKey::P384(_) => Hash::Sha384,
        PrivateKey::Rsa(_) | PrivateKey::P256(_) => Hash::Sha256,
    }
}

/// A detached signature (`ContentInfo` with `SignedData`, DER) over
/// `entity`, the MIME entity in canonical form, made at `now` (Unix
/// seconds). The signer's certificate and its issuers go with it.
///
/// # Errors
/// [`SyncError::Invalid`] when it can't be made.
pub fn sign(entity: &[u8], identity: &Identity, now: u64) -> Result<Vec<u8>, SyncError> {
    let err = |_| failed("sign");
    let hash = hash_for(&identity.key);
    let digest_alg = AlgorithmIdentifierOwned {
        oid: hash.oid(),
        parameters: None,
    };
    let capabilities: Vec<AlgorithmIdentifierOwned> = [oid::AES256_CBC, oid::AES128_CBC]
        .into_iter()
        .map(|id| AlgorithmIdentifierOwned {
            oid: id,
            parameters: None,
        })
        .collect();
    let signing_time =
        UtcTime::from_unix_duration(Duration::from_secs(now)).map_err(|_| failed("sign"))?;
    let digest = OctetString::new(hash.digest(entity)).map_err(err)?;
    let attrs = vec![
        attribute(
            oid::CONTENT_TYPE,
            Any::encode_from(&oid::DATA).map_err(err)?,
        )?,
        attribute(
            oid::SIGNING_TIME,
            Any::encode_from(&signing_time).map_err(err)?,
        )?,
        attribute(oid::MESSAGE_DIGEST, Any::encode_from(&digest).map_err(err)?)?,
        attribute(
            oid::SMIME_CAPABILITIES,
            Any::encode_from(&capabilities).map_err(err)?,
        )?,
    ];
    let signed_attrs: SetOfVec<Attribute> = SetOfVec::try_from(attrs).map_err(err)?;
    let to_sign = signed_attrs.to_der().map_err(err)?;
    let (signature_algorithm, signature) = sign_with(&identity.key, hash, &to_sign)?;
    let tbs = &identity.cert.tbs_certificate;
    let signer_info = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: tbs.issuer.clone(),
            serial_number: tbs.serial_number.clone(),
        }),
        digest_alg: digest_alg.clone(),
        signed_attrs: Some(signed_attrs),
        signature_algorithm,
        signature: OctetString::new(signature).map_err(err)?,
        unsigned_attrs: None,
    };
    let mut certs = vec![CertificateChoices::Certificate(identity.cert.clone())];
    for der in &identity.chain {
        if let Ok(cert) = Certificate::from_der(der)
            && !certs
                .iter()
                .any(|c| matches!(c, CertificateChoices::Certificate(c) if *c == cert))
        {
            certs.push(CertificateChoices::Certificate(cert));
        }
    }
    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![digest_alg]).map_err(err)?,
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: oid::DATA,
            econtent: None,
        },
        certificates: Some(CertificateSet(SetOfVec::try_from(certs).map_err(err)?)),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![signer_info]).map_err(err)?),
    };
    ContentInfo {
        content_type: oid::SIGNED_DATA,
        content: Any::encode_from(&signed_data).map_err(err)?,
    }
    .to_der()
    .map_err(err)
}

/// `EnvelopedData` (in a `ContentInfo`, DER) of `entity` for each of
/// `recipients` (certificates, DER; RSA keys only).
///
/// # Errors
/// [`SyncError::Invalid`] naming a certificate that can't be encrypted to,
/// or when encrypting fails.
pub fn encrypt(entity: &[u8], recipients: &[Vec<u8>]) -> Result<Vec<u8>, SyncError> {
    use pkcs8::DecodePublicKey as _;
    let err = |_| failed("encrypt");
    if recipients.is_empty() {
        return Err(failed("encrypt"));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    let mut iv = [0u8; 16];
    OsRng.fill_bytes(key.as_mut_slice());
    OsRng.fill_bytes(&mut iv);
    let ciphertext = cbc::Encryptor::<aes::Aes256>::new(key.as_ref().into(), &iv.into())
        .encrypt_padded_vec_mut::<Pkcs7>(entity);
    let mut infos = Vec::new();
    for der in recipients {
        let cert = Certificate::from_der(der).map_err(err)?;
        let tbs = &cert.tbs_certificate;
        let spki = tbs.subject_public_key_info.to_der().map_err(err)?;
        let public = rsa::RsaPublicKey::from_public_key_der(&spki).map_err(|_| {
            SyncError::Invalid(format!(
                "Mail can't be encrypted to the certificate of {}: only RSA certificates can be used.",
                super::info_of(&cert, der).owner
            ))
        })?;
        let wrapped = public
            .encrypt(&mut OsRng, rsa::Pkcs1v15Encrypt, key.as_slice())
            .map_err(|_| failed("encrypt"))?;
        infos.push(RecipientInfo::Ktri(KeyTransRecipientInfo {
            version: CmsVersion::V0,
            rid: RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: tbs.issuer.clone(),
                serial_number: tbs.serial_number.clone(),
            }),
            key_enc_alg: AlgorithmIdentifierOwned {
                oid: oid::RSA,
                parameters: Some(Any::encode_from(&Null).map_err(err)?),
            },
            enc_key: OctetString::new(wrapped).map_err(err)?,
        }));
    }
    let enveloped = EnvelopedData {
        version: CmsVersion::V0,
        originator_info: None,
        recip_infos: RecipientInfos(SetOfVec::try_from(infos).map_err(err)?),
        encrypted_content: EncryptedContentInfo {
            content_type: oid::DATA,
            content_enc_alg: AlgorithmIdentifierOwned {
                oid: oid::AES256_CBC,
                parameters: Some(
                    Any::encode_from(&OctetString::new(iv.to_vec()).map_err(err)?).map_err(err)?,
                ),
            },
            encrypted_content: Some(OctetString::new(ciphertext).map_err(err)?),
        },
        unprotected_attrs: None,
    };
    ContentInfo {
        content_type: oid::ENVELOPED_DATA,
        content: Any::encode_from(&enveloped).map_err(err)?,
    }
    .to_der()
    .map_err(err)
}
