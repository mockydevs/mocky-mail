// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Adapted from MailCraft's `crates/ews/src/ntlm.rs` (MIT OR Apache-2.0;
// see THIRD_PARTY_NOTICES.md): the message building and reading are
// MailCraft's, the HTTP legs are driven by `super::Transport` here.

//! `NTLMv2` sign-in for Exchange Server, written from Microsoft's public
//! specification [MS-NLMP] (sections 2.2, 3.1.5 and 3.3.2, and the sample
//! messages of 4.2.4).
//!
//! NTLM over HTTP is three legs on **one connection**: the client sends
//! NEGOTIATE, the server answers 401 with a CHALLENGE, the client sends
//! AUTHENTICATE with the real request, and the connection stays signed in.
//!
//! - **Only `NTLMv2`.** `NTLMv1` and the LM hash are not implemented.
//!   The response key is
//!   `HMAC_MD5(MD4(UTF-16LE(password)), UTF-16LE(UPPER(user) + domain))`.
//!   The `LMv2` response is sent when the server gave no timestamp; with
//!   one, 24 zero bytes (3.1.5.1.2).
//! - **No signing or sealing**: HTTPS needs neither, so key exchange is not
//!   negotiated and the session key is the session base key.
//! - **MIC** when the server's target information has a timestamp.
//! - **Channel binding and target name**: `MsvAvChannelBindings` (sixteen
//!   zero bytes when the certificate hash isn't known) and `MsvAvTargetName`
//!   (`HTTP/host`).
//! - **No machine name, no OS version** are sent.
//!
//! The server's CHALLENGE is hostile: every offset and length is checked.
//! The password is used only to make the response key, which is all that
//! is kept, and nothing here shows it.

use std::fmt;

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use hmac::{Hmac, Mac};
use md4::{Digest, Md4};
use md5::Md5;

/// The signature that begins every NTLM message: `NTLMSSP\0`.
const SIGNATURE: &[u8; 8] = b"NTLMSSP\0";
const NEGOTIATE_MESSAGE: u32 = 1;
const CHALLENGE_MESSAGE: u32 = 2;
const AUTHENTICATE_MESSAGE: u32 = 3;

// NegotiateFlags (2.2.2.5).
const NEGOTIATE_UNICODE: u32 = 0x0000_0001;
const REQUEST_TARGET: u32 = 0x0000_0004;
const NEGOTIATE_NTLM: u32 = 0x0000_0200;
const NEGOTIATE_ALWAYS_SIGN: u32 = 0x0000_8000;
const NEGOTIATE_EXTENDED_SESSIONSECURITY: u32 = 0x0008_0000;
const NEGOTIATE_128: u32 = 0x2000_0000;
/// What the client asks for.
const OFFERED: u32 = NEGOTIATE_UNICODE
    | REQUEST_TARGET
    | NEGOTIATE_NTLM
    | NEGOTIATE_ALWAYS_SIGN
    | NEGOTIATE_EXTENDED_SESSIONSECURITY
    | NEGOTIATE_128;

// AvIds (2.2.2.1).
const AV_EOL: u16 = 0;
const AV_FLAGS: u16 = 6;
const AV_TIMESTAMP: u16 = 7;
const AV_TARGET_NAME: u16 = 9;
const AV_CHANNEL_BINDINGS: u16 = 10;
/// `MsvAvFlags` bit 0x2: the AUTHENTICATE message has a MIC.
const FLAG_MIC: u32 = 0x2;

/// Most AV pairs, and most bytes of target information, taken.
const MAX_PAIRS: usize = 64;
const MAX_TARGET_INFO: usize = 16 * 1024;
/// Most Base64 characters of a challenge taken; the longest name part.
const MAX_TOKEN: usize = 32 * 1024;
const MAX_PART: usize = 1_024;
/// Where the MIC sits in an AUTHENTICATE message.
const MIC_AT: usize = 72;
/// Seconds between 1601-01-01 (a FILETIME's epoch) and 1970-01-01.
const FILETIME_EPOCH: u64 = 11_644_473_600;

/// Why a sign-in message could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NtlmError {
    /// No user name.
    NoUserName,
    /// A name or password with a control character, or too long.
    Unusable,
    /// The server's answer is no NTLM CHALLENGE.
    BadChallenge,
    /// The server's target information is longer than any server sends.
    TooLarge,
    /// The server will not speak Unicode.
    NotUnicode,
    /// A message could not be made.
    Internal,
}

impl fmt::Display for NtlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoUserName => "the user name is empty",
            Self::Unusable => {
                "the user name, the domain or the password has a character that cannot be sent, or is too long"
            }
            Self::BadChallenge => "the server's NTLM challenge is not one",
            Self::TooLarge => "the server's NTLM challenge is longer than any server sends",
            Self::NotUnicode => "the server will not sign in over NTLM with Unicode names",
            Self::Internal => "the NTLM message could not be made",
        })
    }
}

impl std::error::Error for NtlmError {}

/// Who signs in: the user name and domain, and the key that follows from
/// the password. The password itself is not kept.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    domain: String,
    user: String,
    response_key: [u8; 16],
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials(<redacted>)")
    }
}

impl Credentials {
    /// `username` is `DOMAIN\user` or a user principal name
    /// (`user@example.org`, sent as the user name with no domain).
    ///
    /// # Errors
    /// An empty user name, or names or a password that can't be sent.
    pub fn new(username: &str, password: &str) -> Result<Self, NtlmError> {
        let (domain, user) = username.split_once('\\').unwrap_or(("", username));
        if user.is_empty() {
            return Err(NtlmError::NoUserName);
        }
        let unusable = |text: &str| text.len() > MAX_PART || text.chars().any(char::is_control);
        if unusable(domain) || unusable(user) || unusable(password) || user.contains('\\') {
            return Err(NtlmError::Unusable);
        }
        let response_key = ntowf_v2(password, user, domain)?;
        Ok(Self {
            domain: domain.to_owned(),
            user: user.to_owned(),
            response_key,
        })
    }
}

/// What is known at the time of the third leg.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    /// Eight random bytes: the client challenge. Never reused.
    pub client_challenge: [u8; 8],
    /// Seconds since 1970, for a server that sent no timestamp.
    pub unix_seconds: u64,
    /// The host the connection is to (`HTTP/host`).
    pub host: &'a str,
    /// The `tls-server-end-point` hash of the server's certificate, if known.
    pub channel_binding: Option<&'a [u8]>,
}

impl fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("bound", &self.channel_binding.is_some())
            .finish_non_exhaustive()
    }
}

/// The first leg sent, and what the third needs of it.
#[derive(Clone)]
pub struct Handshake {
    response_key: [u8; 16],
    domain: String,
    user: String,
    negotiate: Vec<u8>,
}

impl fmt::Debug for Handshake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Handshake(<redacted>)")
    }
}

/// The NTLM token in a set of `WWW-Authenticate` values (`NTLM <token>`).
#[must_use]
pub fn ntlm_token(challenges: &[String]) -> Option<&str> {
    challenges.iter().find_map(|value| {
        let (scheme, rest) = value.trim().split_once(' ')?;
        (scheme.eq_ignore_ascii_case("ntlm") && !rest.trim().is_empty()).then(|| rest.trim())
    })
}

/// Whether a set of `WWW-Authenticate` values offers this scheme.
#[must_use]
pub fn offers(challenges: &[String], scheme: &str) -> bool {
    challenges.iter().any(|value| {
        value
            .split([',', ' '])
            .next()
            .is_some_and(|first| first.trim().eq_ignore_ascii_case(scheme))
            || value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(scheme))
    })
}

impl Handshake {
    /// The first leg: the `Authorization` value `NTLM <NEGOTIATE>`.
    #[must_use]
    pub fn negotiate(credentials: &Credentials) -> (Self, String) {
        let negotiate = negotiate_message();
        let header = format!("NTLM {}", base64(&negotiate));
        let handshake = Self {
            response_key: credentials.response_key,
            domain: credentials.domain.clone(),
            user: credentials.user.clone(),
            negotiate,
        };
        (handshake, header)
    }

    /// The third leg: the `Authorization` value `NTLM <AUTHENTICATE>`, from
    /// the Base64 token of the server's challenge.
    ///
    /// # Errors
    /// When the challenge can't be used.
    pub fn authenticate(self, token: &str, context: &Context<'_>) -> Result<String, NtlmError> {
        let raw = unbase64(token).ok_or(NtlmError::BadChallenge)?;
        let challenge = Challenge::parse(&raw)?;
        if challenge.flags & NEGOTIATE_UNICODE == 0 {
            return Err(NtlmError::NotUnicode);
        }
        let timestamp = challenge.timestamp();
        let time = timestamp.unwrap_or_else(|| filetime(context.unix_seconds));
        let target_info = challenge.reply_target_info(timestamp.is_some(), context);
        let (nt, session_key) = nt_response(
            &self.response_key,
            challenge.server_nonce,
            context.client_challenge,
            time,
            &target_info,
        )?;
        let lm = if timestamp.is_some() {
            vec![0u8; 24]
        } else {
            let mut lm = hmac_md5(
                &self.response_key,
                &[&challenge.server_nonce, &context.client_challenge],
            )?
            .to_vec();
            lm.extend_from_slice(&context.client_challenge);
            lm
        };
        let domain = utf16(&self.domain);
        let user = utf16(&self.user);
        let parts = Parts {
            flags: OFFERED & challenge.flags,
            lm: &lm,
            nt: &nt,
            domain: &domain,
            user: &user,
            workstation: &[],
            session_key: &[],
        };
        let mic = timestamp.is_some();
        let mut message = authenticate_message(&parts, mic)?;
        if mic {
            let sum = hmac_md5(&session_key, &[&self.negotiate, &raw, &message])?;
            let field = message
                .get_mut(MIC_AT..MIC_AT.saturating_add(16))
                .ok_or(NtlmError::Internal)?;
            field.copy_from_slice(&sum);
        }
        Ok(format!("NTLM {}", base64(&message)))
    }
}

/// The NEGOTIATE message: the flags, no domain, workstation or version.
fn negotiate_message() -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(SIGNATURE);
    out.extend_from_slice(&NEGOTIATE_MESSAGE.to_le_bytes());
    out.extend_from_slice(&OFFERED.to_le_bytes());
    for _ in 0..2 {
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&32u32.to_le_bytes());
    }
    out
}

/// The pieces of an AUTHENTICATE message.
struct Parts<'a> {
    flags: u32,
    lm: &'a [u8],
    nt: &'a [u8],
    domain: &'a [u8],
    user: &'a [u8],
    workstation: &'a [u8],
    session_key: &'a [u8],
}

/// An AUTHENTICATE message (2.2.1.3): the fixed fields, a zero version, a
/// zero MIC when `mic`, then Domain, User, Workstation, LM, NT, session key.
fn authenticate_message(parts: &Parts<'_>, mic: bool) -> Result<Vec<u8>, NtlmError> {
    let header = if mic {
        MIC_AT.saturating_add(16)
    } else {
        MIC_AT
    };
    let payload = [
        parts.domain,
        parts.user,
        parts.workstation,
        parts.lm,
        parts.nt,
        parts.session_key,
    ];
    let mut at = header;
    let mut places = Vec::with_capacity(payload.len());
    for field in payload {
        places.push((field.len(), at));
        at = at.saturating_add(field.len());
    }
    let mut out = Vec::with_capacity(at);
    out.extend_from_slice(SIGNATURE);
    out.extend_from_slice(&AUTHENTICATE_MESSAGE.to_le_bytes());
    // The fixed fields: LM, NT, Domain, User, Workstation, session key.
    for which in [3usize, 4, 0, 1, 2, 5] {
        let (len, offset) = places.get(which).copied().ok_or(NtlmError::Internal)?;
        let len = u16::try_from(len).map_err(|_| NtlmError::Internal)?;
        let offset = u32::try_from(offset).map_err(|_| NtlmError::Internal)?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(&parts.flags.to_le_bytes());
    out.extend_from_slice(&[0u8; 8]);
    if mic {
        out.extend_from_slice(&[0u8; 16]);
    }
    for field in payload {
        out.extend_from_slice(field);
    }
    Ok(out)
}

/// A CHALLENGE message (2.2.1.2), as far as a client needs it.
struct Challenge {
    flags: u32,
    /// The server's eight random bytes.
    server_nonce: [u8; 8],
    pairs: Vec<(u16, Vec<u8>)>,
    has_target_info: bool,
}

impl Challenge {
    fn parse(raw: &[u8]) -> Result<Self, NtlmError> {
        let bad = NtlmError::BadChallenge;
        if raw.get(..8) != Some(&SIGNATURE[..]) || u32_at(raw, 8) != Some(CHALLENGE_MESSAGE) {
            return Err(bad);
        }
        let flags = u32_at(raw, 20).ok_or(bad)?;
        let server_nonce = <[u8; 8]>::try_from(raw.get(24..32).ok_or(bad)?).map_err(|_| bad)?;
        let info = match (u16_at(raw, 40), u32_at(raw, 44)) {
            (Some(len), Some(offset)) => {
                let start = usize::try_from(offset).map_err(|_| bad)?;
                let end = start.checked_add(usize::from(len)).ok_or(bad)?;
                Some(raw.get(start..end).ok_or(bad)?)
            }
            _ => None,
        };
        let (pairs, has_target_info) = match info {
            Some(info) if !info.is_empty() => (parse_pairs(info)?, true),
            _ => (Vec::new(), false),
        };
        Ok(Self {
            flags,
            server_nonce,
            pairs,
            has_target_info,
        })
    }

    /// The server's own time (`MsvAvTimestamp`), the first well-formed one.
    fn timestamp(&self) -> Option<[u8; 8]> {
        self.pairs
            .iter()
            .filter(|(id, _)| *id == AV_TIMESTAMP)
            .find_map(|(_, value)| <[u8; 8]>::try_from(value.as_slice()).ok())
    }

    /// The target information that goes back, end marker included.
    fn reply_target_info(&self, mic: bool, context: &Context<'_>) -> Vec<u8> {
        if !self.has_target_info {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut flagged = false;
        for (id, value) in &self.pairs {
            match *id {
                AV_CHANNEL_BINDINGS | AV_TARGET_NAME => {}
                AV_FLAGS if value.len() == 4 => {
                    let old = <[u8; 4]>::try_from(value.as_slice()).map_or(0, u32::from_le_bytes);
                    flagged = true;
                    let new = if mic { old | FLAG_MIC } else { old };
                    push_pair(&mut out, AV_FLAGS, &new.to_le_bytes());
                }
                _ => push_pair(&mut out, *id, value),
            }
        }
        if mic && !flagged {
            push_pair(&mut out, AV_FLAGS, &FLAG_MIC.to_le_bytes());
        }
        push_pair(
            &mut out,
            AV_CHANNEL_BINDINGS,
            &channel_bindings(context.channel_binding),
        );
        let spn = if context.host.is_empty() {
            Vec::new()
        } else {
            utf16(&format!("HTTP/{}", context.host))
        };
        push_pair(&mut out, AV_TARGET_NAME, &spn);
        push_pair(&mut out, AV_EOL, &[]);
        out
    }
}

/// The AV pairs of `info`, up to the end marker.
fn parse_pairs(info: &[u8]) -> Result<Vec<(u16, Vec<u8>)>, NtlmError> {
    if info.len() > MAX_TARGET_INFO {
        return Err(NtlmError::TooLarge);
    }
    let mut pairs = Vec::new();
    let mut at = 0usize;
    while let (Some(id), Some(len)) = (u16_at(info, at), u16_at(info, at.saturating_add(2))) {
        if id == AV_EOL {
            break;
        }
        let start = at.saturating_add(4);
        let end = start
            .checked_add(usize::from(len))
            .ok_or(NtlmError::BadChallenge)?;
        let value = info.get(start..end).ok_or(NtlmError::BadChallenge)?;
        if pairs.len() >= MAX_PAIRS {
            return Err(NtlmError::TooLarge);
        }
        pairs.push((id, value.to_vec()));
        at = end;
    }
    Ok(pairs)
}

fn push_pair(out: &mut Vec<u8>, id: u16, value: &[u8]) {
    let len = u16::try_from(value.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(value.get(..usize::from(len)).unwrap_or(&[]));
}

/// `MsvAvChannelBindings`: the MD5 of the `gss_channel_bindings_struct`
/// whose application data is `tls-server-end-point:` and the hash; sixteen
/// zero bytes without one.
fn channel_bindings(end_point_hash: Option<&[u8]>) -> [u8; 16] {
    let Some(hash) = end_point_hash else {
        return [0; 16];
    };
    let data = [b"tls-server-end-point:".as_slice(), hash].concat();
    let mut structure = vec![0u8; 16];
    structure.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes());
    structure.extend_from_slice(&data);
    Md5::digest(&structure).into()
}

/// `NTOWFv2` (3.3.2).
fn ntowf_v2(password: &str, user: &str, domain: &str) -> Result<[u8; 16], NtlmError> {
    let hash: [u8; 16] = Md4::digest(utf16(password)).into();
    // Windows upper-cases one character to one character.
    let upper: String = user
        .chars()
        .map(|c| {
            let mut mapped = c.to_uppercase();
            match (mapped.next(), mapped.next()) {
                (Some(one), None) => one,
                _ => c,
            }
        })
        .collect();
    hmac_md5(&hash, &[&utf16(&format!("{upper}{domain}"))])
}

/// The NT challenge response and the session base key (3.3.2).
fn nt_response(
    response_key: &[u8; 16],
    server_nonce: [u8; 8],
    client_challenge: [u8; 8],
    time: [u8; 8],
    target_info: &[u8],
) -> Result<(Vec<u8>, [u8; 16]), NtlmError> {
    let mut temp = vec![1u8, 1, 0, 0, 0, 0, 0, 0];
    temp.extend_from_slice(&time);
    temp.extend_from_slice(&client_challenge);
    temp.extend_from_slice(&[0; 4]);
    temp.extend_from_slice(target_info);
    temp.extend_from_slice(&[0; 4]);
    let proof = hmac_md5(response_key, &[&server_nonce, &temp])?;
    let session_base_key = hmac_md5(response_key, &[&proof])?;
    let mut response = proof.to_vec();
    response.extend_from_slice(&temp);
    Ok((response, session_base_key))
}

fn hmac_md5(key: &[u8], parts: &[&[u8]]) -> Result<[u8; 16], NtlmError> {
    let mut mac = <Hmac<Md5> as Mac>::new_from_slice(key).map_err(|_| NtlmError::Internal)?;
    for part in parts {
        mac.update(part);
    }
    Ok(mac.finalize().into_bytes().into())
}

/// A FILETIME (100 ns ticks since 1601) from seconds since 1970.
fn filetime(unix_seconds: u64) -> [u8; 8] {
    unix_seconds
        .saturating_add(FILETIME_EPOCH)
        .saturating_mul(10_000_000)
        .to_le_bytes()
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let end = at.checked_add(2)?;
    Some(u16::from_le_bytes(
        <[u8; 2]>::try_from(bytes.get(at..end)?).ok()?,
    ))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    Some(u32::from_le_bytes(
        <[u8; 4]>::try_from(bytes.get(at..end)?).ok()?,
    ))
}

const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Standard Base64, padding optional; nothing for anything else or a token
/// over [`MAX_TOKEN`].
fn unbase64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_TOKEN {
        return None;
    }
    LENIENT.decode(text).ok().filter(|bytes| !bytes.is_empty())
}

#[cfg(test)]
#[path = "ntlm_tests.rs"]
mod tests;
