//! The root certificates S/MIME signers are checked against: the system's
//! own. Windows: the current user's Trusted Root store, only roots Windows
//! allows for secure email. macOS and Linux: the system's trusted roots
//! (keychain trust settings, or the distribution's bundle). Android: the
//! system and user-added certificate folders. Only roots are trusted;
//! intermediates come from the message (or earlier mail).

use der::Decode;
use x509_cert::Certificate;

/// Trusted root certificates.
#[derive(Debug, Clone, Default)]
pub struct TrustRoots {
    certs: Vec<Certificate>,
    ders: Vec<Vec<u8>>,
}

impl TrustRoots {
    /// Roots from certificates (DER); unreadable ones are left out.
    #[must_use]
    pub fn from_der(certs: impl IntoIterator<Item = Vec<u8>>) -> Self {
        let mut out = Self::default();
        for der in certs {
            if let Ok(cert) = Certificate::from_der(&der) {
                out.certs.push(cert);
                out.ders.push(der);
            }
        }
        out
    }

    /// The system's roots for email. Never fails: a store that can't be
    /// read gives no roots (and signatures are then not "verified").
    #[must_use]
    pub fn system() -> Self {
        Self::from_der(system_roots())
    }

    /// The roots.
    #[must_use]
    pub fn certs(&self) -> &[Certificate] {
        &self.certs
    }

    /// Whether this certificate (DER) is one of the roots.
    #[must_use]
    pub fn contains(&self, der: &[u8]) -> bool {
        self.ders.iter().any(|d| d == der)
    }

    /// How many there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.certs.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.certs.is_empty()
    }
}

#[cfg(windows)]
fn system_roots() -> Vec<Vec<u8>> {
    use schannel::cert_context::ValidUses;
    let Ok(store) = schannel::cert_store::CertStore::open_current_user("ROOT") else {
        return Vec::new();
    };
    store
        .certs()
        .filter(|cert| match cert.valid_uses() {
            Ok(ValidUses::All) => true,
            Ok(ValidUses::Oids(oids)) => oids.iter().any(|o| o == "1.3.6.1.5.5.7.3.4"),
            Err(_) => false,
        })
        .map(|cert| cert.to_der().to_vec())
        .collect()
}

#[cfg(target_os = "android")]
fn system_roots() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for dir in [
        "/apex/com.android.conscrypt/cacerts",
        "/system/etc/security/cacerts",
        "/data/misc/user/0/cacerts-added",
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten().take(2_000) {
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let pem = pem_certificates(&bytes);
            if !pem.is_empty() {
                out.extend(pem);
            } else if Certificate::from_der(&bytes).is_ok() {
                out.push(bytes);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The certificates (DER) in a file of PEM blocks, ignoring any text around
/// them: Android's certificate files follow the PEM block with `openssl x509
/// -text` output, which a strict PEM chain reader refuses.
#[cfg(any(test, target_os = "android"))]
fn pem_certificates(bytes: &[u8]) -> Vec<Vec<u8>> {
    use der::{DecodePem as _, Encode as _};
    const BEGIN: &[u8] = b"-----BEGIN CERTIFICATE-----";
    const END: &[u8] = b"-----END CERTIFICATE-----";
    let find = |haystack: &[u8], needle: &[u8]| {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    };
    let mut out = Vec::new();
    let mut rest = bytes;
    while let Some(start) = find(rest, BEGIN) {
        let Some(block) = rest.get(start..) else {
            break;
        };
        let Some(end) = find(block, END).and_then(|end| end.checked_add(END.len())) else {
            break;
        };
        let (Some(pem), Some(after)) = (block.get(..end), block.get(end..)) else {
            break;
        };
        if let Ok(cert) = Certificate::from_pem(pem)
            && let Ok(der) = cert.to_der()
        {
            out.push(der);
        }
        rest = after;
    }
    out
}

#[cfg(not(any(windows, target_os = "android")))]
fn system_roots() -> Vec<Vec<u8>> {
    rustls_native_certs::load_native_certs()
        .certs
        .into_iter()
        .map(|cert| cert.as_ref().to_vec())
        .collect()
}

#[cfg(test)]
mod tests {
    use der::{Decode as _, EncodePem as _, pem::LineEnding};
    use x509_cert::Certificate;

    use super::pem_certificates;

    const CA: &[u8] = include_bytes!("../fixtures/ca.cer");

    #[test]
    fn pem_files_with_text_after_the_certificate_are_read() {
        let pem = Certificate::from_der(CA)
            .unwrap()
            .to_pem(LineEnding::LF)
            .unwrap();
        // As Android keeps them: the PEM block, then the text dump.
        let android = format!(
            "{pem}Certificate:\n    Data:\n        Version: 3 (0x2)\nSHA1 Fingerprint=00:11\n"
        );
        assert_eq!(pem_certificates(android.as_bytes()), vec![CA.to_vec()]);
        // Two blocks with text between, and a broken one, which is left out.
        let two = format!(
            "intro\n{pem}between\n{pem}-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n"
        );
        assert_eq!(
            pem_certificates(two.as_bytes()),
            vec![CA.to_vec(), CA.to_vec()]
        );
        assert!(pem_certificates(CA).is_empty());
    }
}
