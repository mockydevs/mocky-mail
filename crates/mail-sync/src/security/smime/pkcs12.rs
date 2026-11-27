//! Reading a `.p12` / `.pfx` file (PKCS #12, RFC 7292): its MAC checked
//! with the password, its bags decrypted (PBES2 with AES, or the older
//! PKCS #12 schemes with triple DES or RC2 that Windows and OpenSSL 1.x
//! write), and the private key matched with its certificate.

use cbc::cipher::{BlockDecryptMut as _, InnerIvInit as _, KeyIvInit as _, block_padding::Pkcs7};
use cms::encrypted_data::EncryptedData;
use der::asn1::OctetString;
use der::{Decode, Encode};
use hmac::Mac as _;
use pkcs12::kdf::{Pkcs12KeyType, derive_key_utf8};
use pkcs12::pbe_params::{EncryptedPrivateKeyInfo, Pkcs12PbeParams};
use pkcs12::pfx::Pfx;
use pkcs12::safe_bag::SafeBag;
use pkcs12::{
    PKCS_12_CERT_BAG_OID, PKCS_12_KEY_BAG_OID, PKCS_12_PBE_WITH_SHAAND2_KEY_TRIPLE_DES_CBC,
    PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC, PKCS_12_PBE_WITH_SHAAND128_BIT_RC2_CBC,
    PKCS_12_PBEWITH_SHAAND40_BIT_RC2_CBC, PKCS_12_PKCS8_KEY_BAG_OID, PKCS_12_SAFE_CONTENTS_BAG_OID,
    PKCS_12_X509_CERT_OID,
};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;
use zeroize::Zeroizing;

use super::{PrivateKey, oid};
use crate::SyncError;
use crate::security::ber;

/// What a `.p12` file holds.
#[derive(Debug)]
pub struct Contents {
    /// The private key (PKCS #8 DER).
    pub key: Zeroizing<Vec<u8>>,
    /// Its certificate (DER).
    pub cert: Vec<u8>,
    /// The other certificates in the file: its issuers (DER).
    pub chain: Vec<Vec<u8>>,
}

const PBES2: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.113549.1.5.13");
/// Most bags read, and how deep bags of bags go.
const MAX_BAGS: usize = 64;
const MAX_NESTING: usize = 4;
/// Most iterations a password derivation may ask for.
const MAX_ITERATIONS: i32 = 10_000_000;

fn wrong_password() -> SyncError {
    SyncError::Invalid("The password for this file isn't right.".to_owned())
}

fn unreadable() -> SyncError {
    SyncError::Invalid("This .p12/.pfx file can't be read.".to_owned())
}

fn unsupported() -> SyncError {
    SyncError::Invalid(
        "This .p12/.pfx file is protected in a way Mocky Mail can't open. Export it again with AES encryption.".to_owned(),
    )
}

/// Read a PKCS #12 file with `password`.
///
/// # Errors
/// [`SyncError::Invalid`] for a wrong password, a damaged file, one
/// protected with a scheme not supported, or one without a key and its
/// certificate.
pub fn read(bytes: &[u8], password: &str) -> Result<Contents, SyncError> {
    let der = ber::to_der(bytes).ok_or_else(unreadable)?;
    let pfx = Pfx::from_der(&der).map_err(|_| unreadable())?;
    if pfx.auth_safe.content_type != oid::DATA {
        return Err(unsupported());
    }
    let auth_safe = pfx
        .auth_safe
        .content
        .decode_as::<OctetString>()
        .map_err(|_| unreadable())?;
    let auth_safe = auth_safe.as_bytes();
    if let Some(mac) = &pfx.mac_data {
        check_mac(mac, auth_safe, password)?;
    }
    let contents: Vec<cms::content_info::ContentInfo> =
        Vec::from_der(auth_safe).map_err(|_| unreadable())?;
    let mut keys: Vec<Zeroizing<Vec<u8>>> = Vec::new();
    let mut certs: Vec<Vec<u8>> = Vec::new();
    for info in contents.iter().take(MAX_BAGS) {
        let safe_contents: Zeroizing<Vec<u8>> = match info.content_type {
            oid::DATA => Zeroizing::new(
                info.content
                    .decode_as::<OctetString>()
                    .map_err(|_| unreadable())?
                    .as_bytes()
                    .to_vec(),
            ),
            oid::ENCRYPTED_DATA => {
                let encrypted = info
                    .content
                    .decode_as::<EncryptedData>()
                    .map_err(|_| unreadable())?;
                let ciphertext = encrypted
                    .enc_content_info
                    .encrypted_content
                    .as_ref()
                    .ok_or_else(unreadable)?;
                decrypt_with_password(
                    &encrypted.enc_content_info.content_enc_alg,
                    ciphertext.as_bytes(),
                    password,
                )?
            }
            _ => continue,
        };
        read_bags(&safe_contents, password, 0, &mut keys, &mut certs)?;
    }
    // The key whose certificate is here; the other certificates are its issuers.
    for key_der in keys {
        let key = PrivateKey::from_pkcs8(&key_der)?;
        let found = certs.iter().position(|der| {
            Certificate::from_der(der)
                .is_ok_and(|cert| key.matches(&cert.tbs_certificate.subject_public_key_info))
        });
        if let Some(index) = found {
            let cert = certs.remove(index);
            return Ok(Contents {
                key: key.to_pkcs8()?,
                cert,
                chain: certs,
            });
        }
    }
    Err(SyncError::Invalid(
        "This file has no private key with its certificate. Use a .cer file to add someone else's certificate.".to_owned(),
    ))
}

fn check_mac(
    mac: &pkcs12::mac_data::MacData,
    data: &[u8],
    password: &str,
) -> Result<(), SyncError> {
    if mac.iterations < 1 || mac.iterations > MAX_ITERATIONS {
        return Err(unreadable());
    }
    let salt = mac.mac_salt.as_bytes();
    let expected = mac.mac.digest.as_bytes();
    let alg = mac.mac.algorithm.oid;
    macro_rules! verify {
        ($digest:ty, $len:expr) => {{
            let key = derive_key_utf8::<$digest>(
                password,
                salt,
                Pkcs12KeyType::Mac,
                mac.iterations,
                $len,
            )
            .map_err(|_| unreadable())?;
            let mut hmac = hmac::Hmac::<$digest>::new_from_slice(&key).map_err(|_| unreadable())?;
            hmac.update(data);
            hmac.verify_slice(expected).map_err(|_| wrong_password())
        }};
    }
    if alg == oid::SHA1 {
        verify!(sha1::Sha1, 20)
    } else if alg == oid::SHA256 {
        verify!(sha2::Sha256, 32)
    } else if alg == oid::SHA384 {
        verify!(sha2::Sha384, 48)
    } else if alg == oid::SHA512 {
        verify!(sha2::Sha512, 64)
    } else {
        Err(unsupported())
    }
}

/// The value inside a bag's `[0] EXPLICIT` wrapper.
fn explicit_value(bag_value: &[u8]) -> Option<&[u8]> {
    let element = ber::element(bag_value)?;
    (element.tag == 0xA0).then_some(element.content)
}

fn read_bags(
    safe_contents: &[u8],
    password: &str,
    depth: usize,
    keys: &mut Vec<Zeroizing<Vec<u8>>>,
    certs: &mut Vec<Vec<u8>>,
) -> Result<(), SyncError> {
    if depth > MAX_NESTING {
        return Err(unreadable());
    }
    let bags: Vec<SafeBag> = Vec::from_der(safe_contents).map_err(|_| unreadable())?;
    for bag in bags.iter().take(MAX_BAGS) {
        let Some(value) = explicit_value(&bag.bag_value) else {
            continue;
        };
        match bag.bag_id {
            PKCS_12_KEY_BAG_OID => keys.push(Zeroizing::new(value.to_vec())),
            PKCS_12_PKCS8_KEY_BAG_OID => {
                let shrouded =
                    EncryptedPrivateKeyInfo::from_der(value).map_err(|_| unreadable())?;
                keys.push(decrypt_with_password(
                    &shrouded.encryption_algorithm,
                    shrouded.encrypted_data.as_bytes(),
                    password,
                )?);
            }
            PKCS_12_CERT_BAG_OID => {
                let bag = pkcs12::cert_type::CertBag::from_der(value).map_err(|_| unreadable())?;
                if bag.cert_id == PKCS_12_X509_CERT_OID {
                    certs.push(bag.cert_value.as_bytes().to_vec());
                }
            }
            PKCS_12_SAFE_CONTENTS_BAG_OID => {
                read_bags(value, password, depth.saturating_add(1), keys, certs)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn decrypt_with_password(
    alg: &AlgorithmIdentifierOwned,
    ciphertext: &[u8],
    password: &str,
) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    if alg.oid == PBES2 {
        let der = alg.to_der().map_err(|_| unreadable())?;
        let alg_ref = spki::AlgorithmIdentifierRef::from_der(&der).map_err(|_| unreadable())?;
        let scheme = pkcs5::EncryptionScheme::try_from(alg_ref).map_err(|_| unsupported())?;
        return scheme
            .decrypt(password.as_bytes(), ciphertext)
            .map(Zeroizing::new)
            .map_err(|_| wrong_password());
    }
    let params = alg
        .parameters
        .as_ref()
        .ok_or_else(unreadable)?
        .decode_as::<Pkcs12PbeParams>()
        .map_err(|_| unreadable())?;
    if params.iterations < 1 || params.iterations > MAX_ITERATIONS {
        return Err(unreadable());
    }
    let salt = params.salt.as_bytes();
    let derive = |kind: Pkcs12KeyType, len: usize| {
        derive_key_utf8::<sha1::Sha1>(password, salt, kind, params.iterations, len)
            .map(Zeroizing::new)
            .map_err(|_| unreadable())
    };
    let plain = match alg.oid {
        PKCS_12_PBE_WITH_SHAAND3_KEY_TRIPLE_DES_CBC => {
            let key = derive(Pkcs12KeyType::EncryptionKey, 24)?;
            let iv = derive(Pkcs12KeyType::Iv, 8)?;
            cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
                .map_err(|_| unreadable())?
                .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        }
        PKCS_12_PBE_WITH_SHAAND2_KEY_TRIPLE_DES_CBC => {
            let key = derive(Pkcs12KeyType::EncryptionKey, 16)?;
            let iv = derive(Pkcs12KeyType::Iv, 8)?;
            cbc::Decryptor::<des::TdesEde2>::new_from_slices(&key, &iv)
                .map_err(|_| unreadable())?
                .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        }
        PKCS_12_PBEWITH_SHAAND40_BIT_RC2_CBC | PKCS_12_PBE_WITH_SHAAND128_BIT_RC2_CBC => {
            let bits: usize = if alg.oid == PKCS_12_PBEWITH_SHAAND40_BIT_RC2_CBC {
                40
            } else {
                128
            };
            let key = derive(Pkcs12KeyType::EncryptionKey, bits / 8)?;
            let iv = derive(Pkcs12KeyType::Iv, 8)?;
            let cipher = rc2::Rc2::new_with_eff_key_len(&key, bits);
            cbc::Decryptor::<rc2::Rc2>::inner_iv_slice_init(cipher, &iv)
                .map_err(|_| unreadable())?
                .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        }
        _ => return Err(unsupported()),
    };
    plain.map(Zeroizing::new).map_err(|_| wrong_password())
}
