//! Web Key Directory (draft-koch-openpgp-webkey-service): finding a
//! recipient's OpenPGP key on their domain's web server, the advanced
//! method (`openpgpkey.` subdomain) first, then the direct one. Over
//! HTTPS only, with the system's certificate checks; whatever is found is
//! checked and kept untrusted.

use std::time::Duration;

use sha1::Digest as _;

use crate::SyncError;

/// How long one look-up may take.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest answer read.
const MAX_ANSWER: usize = 256 * 1024;

/// z-base-32 (RFC 6189 §5.1.6).
fn zbase32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits = bits.saturating_add(8);
        while bits >= 5 {
            bits = bits.saturating_sub(5);
            let index = usize::try_from((buffer >> bits) & 0x1F).unwrap_or(0);
            out.push(char::from(*ALPHABET.get(index).unwrap_or(&b'y')));
        }
        buffer &= (1u32 << bits).wrapping_sub(1);
    }
    if bits > 0 {
        let index = usize::try_from((buffer << 5u32.saturating_sub(bits)) & 0x1F).unwrap_or(0);
        out.push(char::from(*ALPHABET.get(index).unwrap_or(&b'y')));
    }
    out
}

fn percent_encode(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The two WKD addresses for an email address: advanced, then direct.
/// `None` for something that isn't an address on a plain domain name.
#[must_use]
pub fn urls(address: &str) -> Option<[String; 2]> {
    let (local, domain) = address.trim().rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let plain = !domain.is_empty()
        && domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if local.is_empty() || !plain {
        return None;
    }
    // The hash is of the local part in lower case (ASCII), as the draft says.
    let hash = zbase32(&sha1::Sha1::digest(local.to_ascii_lowercase().as_bytes()));
    let l = percent_encode(local);
    Some([
        format!("https://openpgpkey.{domain}/.well-known/openpgpkey/{domain}/hu/{hash}?l={l}"),
        format!("https://{domain}/.well-known/openpgpkey/hu/{hash}?l={l}"),
    ])
}

/// Look `address` up: the keys found for it (binary OpenPGP), checked and
/// holding the address. Empty when there are none.
///
/// # Errors
/// Only when no HTTP client can be made; a domain without a directory is
/// an empty answer.
pub async fn lookup(address: &str) -> Result<Vec<mailcraft_openpgp::PublicKey>, SyncError> {
    let Some(urls) = urls(address) else {
        return Ok(Vec::new());
    };
    let http = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .https_only(true)
        .user_agent("Mocky Mail")
        .build()
        .map_err(|e| SyncError::Config(e.to_string()))?;
    for url in urls {
        let Ok(response) = http.get(&url).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        if response
            .content_length()
            .is_some_and(|len| len > u64::try_from(MAX_ANSWER).unwrap_or(u64::MAX))
        {
            continue;
        }
        let Ok(body) = response.bytes().await else {
            continue;
        };
        if body.len() > MAX_ANSWER {
            continue;
        }
        let keys: Vec<_> = mailcraft_openpgp::PublicKey::parse(&body)
            .unwrap_or_default()
            .into_iter()
            .filter(|key| key.has_address(address))
            .collect();
        if !keys.is_empty() {
            return Ok(keys);
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_both_addresses() {
        // The draft's own example: Joe.Doe@Example.ORG.
        let [advanced, direct] = urls("Joe.Doe@Example.ORG").unwrap_or_default();
        assert_eq!(
            advanced,
            "https://openpgpkey.example.org/.well-known/openpgpkey/example.org/hu/iy9q119eutrkn8s1mk4r39qejnbu3n5q?l=Joe.Doe"
        );
        assert_eq!(
            direct,
            "https://example.org/.well-known/openpgpkey/hu/iy9q119eutrkn8s1mk4r39qejnbu3n5q?l=Joe.Doe"
        );
    }

    #[test]
    fn refuses_what_is_not_an_address_on_a_domain() {
        for bad in [
            "nobody",
            "a@",
            "@x.org",
            "a@localhost",
            "a@x..org",
            "a@-x.org",
            "a@x/y.org",
        ] {
            assert!(urls(bad).is_none(), "{bad}");
        }
        let [advanced, _] = urls("a+b c@x.org").unwrap_or_default();
        assert!(advanced.ends_with("?l=a%2Bb%20c"));
    }

    #[test]
    fn zbase32_matches_known_values() {
        assert_eq!(zbase32(&[]), "");
        assert_eq!(zbase32(&[0xF0]), "6y");
        assert_eq!(zbase32(&[0xD4, 0x7A, 0x04]), "4t7ye");
    }
}
