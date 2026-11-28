// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Adapted from MailCraft's `crates/ews/src/ntlm_tests.rs` (see
// THIRD_PARTY_NOTICES.md).

//! `NTLMv2`: the sample values and messages of MS-NLMP 4.2.4, a full sign-in
//! checked by a verifier that holds the password, and hostile CHALLENGEs.
#![allow(clippy::arithmetic_side_effects)]

use super::*;

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

/// The `CHALLENGE_MESSAGE` of 4.2.4.3.
fn spec_challenge() -> Vec<u8> {
    hex("4e544c4d535350000200000 00c000c00 38000000 33828ae2 0123456789abcdef
         0000000000000000 24002400 44000000 060070170000000f
         53006500720076006500720002000c004400 6f006d00610069006e00 01000c0053006500720076006500720000000000")
}

/// The `AUTHENTICATE_MESSAGE` of 4.2.4.3.
fn spec_authenticate() -> Vec<u8> {
    hex("4e544c4d5353500003000000 18001800 6c000000 54005400 84000000 0c000c00 48000000 08000800 54000000
         10001000 5c000000 10001000 d8000000 358288e2 0501280a0000000f
         44006f006d00610069006e00 5500730065007200 43004f004d0050005500540045005200
         86c35097ac9cec102554764a57cccc19aaaaaaaaaaaaaaaa
         68cd0ab851e51c96aabc927bebef6a1c
         0101000000000000 0000000000000000 aaaaaaaaaaaaaaaa 00000000
         02000c0044006f006d00610069006e00 01000c00530065007200760065007200 00000000 00000000
         c5dad2544fc9799094ce1ce90bc9d03e")
}

fn spec_password_key() -> [u8; 16] {
    ntowf_v2("Password", "User", "Domain").unwrap()
}

#[test]
fn md4_matches_rfc_1320() {
    for (input, digest) in [
        ("", "31d6cfe0d16ae931b73c59d7e0c089c0"),
        ("a", "bde52cb31de33e46245e05fbdbd6fb24"),
        ("abc", "a448017aaf21d8525fc10ae87aa6729d"),
        ("message digest", "d9130a8164549fe818874806e1c7014b"),
    ] {
        assert_eq!(
            Md4::digest(input.as_bytes()).to_vec(),
            hex(digest),
            "{input}"
        );
    }
}

#[test]
fn ntowf_v2_matches_the_sample() {
    // 4.2.4.1.1: NTOWFv2("Password", "User", "Domain").
    assert_eq!(
        spec_password_key().to_vec(),
        hex("0c868a403bfd7a93a3001ef22ef02e3f")
    );
}

#[test]
fn the_challenge_of_the_sample_is_read() {
    let raw = spec_challenge();
    let challenge = Challenge::parse(&raw).unwrap();
    assert_eq!(challenge.flags, 0xe28a_8233);
    assert_eq!(challenge.server_nonce.to_vec(), hex("0123456789abcdef"));
    assert!(challenge.has_target_info);
    assert_eq!(challenge.pairs.len(), 2);
    assert_eq!(challenge.timestamp(), None);
}

#[test]
fn nt_response_and_session_key_match_the_sample() {
    let raw = spec_challenge();
    let target_info = &raw[0x44..0x44 + 0x24];
    let challenge = Challenge::parse(&raw).unwrap();
    let (response, session_key) = nt_response(
        &spec_password_key(),
        challenge.server_nonce,
        [0xaa; 8],
        [0; 8],
        target_info,
    )
    .unwrap();
    // 4.2.4.2.2: NTProofStr, and 4.2.4.1.2's session base key.
    assert_eq!(
        response[..16].to_vec(),
        hex("68cd0ab851e51c96aabc927bebef6a1c")
    );
    assert_eq!(
        session_key.to_vec(),
        hex("8de40ccadbc14a82f15cb0ad0de95ca3")
    );
}

#[test]
fn lmv2_response_matches_the_sample() {
    let lm = hmac_md5(
        &spec_password_key(),
        &[&hex("0123456789abcdef"), &[0xaa; 8]],
    )
    .unwrap();
    assert_eq!(lm.to_vec(), hex("86c35097ac9cec102554764a57cccc19"));
}

/// RC4, for this test only (the client doesn't negotiate key exchange).
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + usize::from(s[i]) + usize::from(key[i % key.len()])) % 256;
        s.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    data.iter()
        .map(|byte| {
            i = (i + 1) % 256;
            j = (j + usize::from(s[i])) % 256;
            s.swap(i, j);
            byte ^ s[(usize::from(s[i]) + usize::from(s[j])) % 256]
        })
        .collect()
}

#[test]
fn the_authenticate_message_of_the_sample_is_made_byte_for_byte() {
    let raw = spec_challenge();
    let target_info = &raw[0x44..0x44 + 0x24];
    let challenge = Challenge::parse(&raw).unwrap();
    let (nt, session_base_key) = nt_response(
        &spec_password_key(),
        challenge.server_nonce,
        [0xaa; 8],
        [0; 8],
        target_info,
    )
    .unwrap();
    let mut lm = hmac_md5(&spec_password_key(), &[&challenge.server_nonce, &[0xaa; 8]])
        .unwrap()
        .to_vec();
    lm.extend_from_slice(&[0xaa; 8]);
    let encrypted = rc4(&session_base_key, &[0x55; 16]);
    assert_eq!(encrypted, hex("c5dad2544fc9799094ce1ce90bc9d03e"));
    let parts = Parts {
        flags: 0xe288_8235,
        lm: &lm,
        nt: &nt,
        domain: &utf16("Domain"),
        user: &utf16("User"),
        workstation: &utf16("COMPUTER"),
        session_key: &encrypted,
    };
    let mine = authenticate_message(&parts, false).unwrap();
    let theirs = spec_authenticate();
    // The sample carries a version, which this client leaves zero.
    assert_eq!(mine.len(), theirs.len());
    assert_eq!(mine[..0x40], theirs[..0x40]);
    assert_eq!(mine[0x48..], theirs[0x48..]);
    assert_eq!(mine[0x40..0x48], [0u8; 8]);
}

#[test]
fn the_negotiate_message_offers_no_signing_and_names_no_machine() {
    let credentials = Credentials::new("Domain\\User", "Password").unwrap();
    let (_, first) = Handshake::negotiate(&credentials);
    let negotiate = unbase64(&first[5..]).unwrap();
    assert_eq!(negotiate.len(), 32);
    assert_eq!(&negotiate[..12], b"NTLMSSP\0\x01\0\0\0");
    let flags = u32_at(&negotiate, 12).unwrap();
    for wanted in [
        NEGOTIATE_UNICODE,
        NEGOTIATE_NTLM,
        NEGOTIATE_EXTENDED_SESSIONSECURITY,
        NEGOTIATE_128,
    ] {
        assert!(flags & wanted != 0);
    }
    assert_eq!(flags & 0x0000_0030, 0, "sign or seal");
    assert_eq!(flags & 0x4000_0000, 0, "key exchange");
    assert_eq!(flags & 0x0200_3000, 0, "version, workstation, domain");
}

/// A CHALLENGE with `pairs` as its target information.
fn challenge_with(flags: u32, pairs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut info = Vec::new();
    for (id, value) in pairs {
        info.extend_from_slice(&id.to_le_bytes());
        info.extend_from_slice(&u16::try_from(value.len()).unwrap().to_le_bytes());
        info.extend_from_slice(value);
    }
    info.extend_from_slice(&[0, 0, 0, 0]);
    let mut out = Vec::new();
    out.extend_from_slice(b"NTLMSSP\0");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 56, 0, 0, 0]);
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&u16::try_from(info.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&u16::try_from(info.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&56u32.to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&info);
    out
}

/// What a server that holds the password checks.
struct Verified {
    pairs: Vec<(u16, Vec<u8>)>,
    mic_checked: bool,
    lm_is_zero: bool,
}

fn server_verifies(
    password: &str,
    user: &str,
    domain: &str,
    negotiate: &[u8],
    challenge: &[u8],
    message: &[u8],
) -> Verified {
    let key = ntowf_v2(password, user, domain).unwrap();
    let field = |at: usize| {
        let len = usize::from(u16_at(message, at).unwrap());
        let offset = usize::try_from(u32_at(message, at + 4).unwrap()).unwrap();
        &message[offset..offset + len]
    };
    let nt = field(20);
    let (proof, temp) = nt.split_at(16);
    let server_challenge = &challenge[24..32];
    assert_eq!(
        hmac_md5(&key, &[server_challenge, temp]).unwrap().to_vec(),
        proof
    );
    let session_key = hmac_md5(&key, &[proof]).unwrap();
    let info = &temp[28..temp.len() - 4];
    let pairs = parse_pairs(info).unwrap();
    let claims_mic = pairs.iter().any(|(id, v)| {
        *id == AV_FLAGS && u32::from_le_bytes(v[..4].try_into().unwrap()) & FLAG_MIC != 0
    });
    let mut mic_checked = false;
    if claims_mic {
        let mut zeroed = message.to_vec();
        let mic = zeroed[MIC_AT..MIC_AT + 16].to_vec();
        zeroed[MIC_AT..MIC_AT + 16].fill(0);
        assert_eq!(
            hmac_md5(&session_key, &[negotiate, challenge, &zeroed])
                .unwrap()
                .to_vec(),
            mic
        );
        mic_checked = true;
    }
    Verified {
        pairs,
        mic_checked,
        lm_is_zero: field(12).iter().all(|b| *b == 0),
    }
}

fn run(
    password: &str,
    username: &str,
    challenge: &[u8],
    binding: Option<&[u8]>,
) -> (Vec<u8>, Vec<u8>) {
    let credentials = Credentials::new(username, password).unwrap();
    let (handshake, first) = Handshake::negotiate(&credentials);
    let negotiate = unbase64(&first[5..]).unwrap();
    let context = Context {
        client_challenge: [7; 8],
        unix_seconds: 1_800_000_000,
        host: "mail.example.org",
        channel_binding: binding,
    };
    let answer = handshake
        .authenticate(&base64(challenge), &context)
        .unwrap();
    (negotiate, unbase64(&answer[5..]).unwrap())
}

#[test]
fn a_server_timestamp_brings_the_mic_the_binding_the_target_name_and_a_zero_lm() {
    let challenge = challenge_with(
        0xe28a_8233,
        &[
            (2, utf16("EXAMPLE")),
            (1, utf16("MAIL")),
            (AV_FLAGS, 1u32.to_le_bytes().to_vec()),
            (AV_TIMESTAMP, vec![1, 2, 3, 4, 5, 6, 7, 8]),
        ],
    );
    let (negotiate, message) = run("S3cret!", "EXAMPLE\\ada", &challenge, Some(&[9; 32]));
    let verified = server_verifies(
        "S3cret!", "ada", "EXAMPLE", &negotiate, &challenge, &message,
    );
    assert!(verified.mic_checked);
    assert!(verified.lm_is_zero);
    let flags = verified.pairs.iter().find(|p| p.0 == AV_FLAGS).unwrap();
    assert_eq!(flags.1, 3u32.to_le_bytes().to_vec());
    assert_eq!(
        verified
            .pairs
            .iter()
            .find(|p| p.0 == AV_TARGET_NAME)
            .unwrap()
            .1,
        utf16("HTTP/mail.example.org")
    );
}

#[test]
fn a_server_without_a_timestamp_gets_lmv2_no_mic_and_a_zero_binding() {
    let challenge = challenge_with(0xe28a_8233, &[(2, utf16("EXAMPLE")), (1, utf16("MAIL"))]);
    let (negotiate, message) = run("S3cret!", "ada@example.org", &challenge, None);
    let verified = server_verifies(
        "S3cret!",
        "ada@example.org",
        "",
        &negotiate,
        &challenge,
        &message,
    );
    assert!(!verified.mic_checked);
    assert!(!verified.lm_is_zero);
    assert_eq!(
        verified
            .pairs
            .iter()
            .find(|p| p.0 == AV_CHANNEL_BINDINGS)
            .unwrap()
            .1,
        vec![0u8; 16]
    );
}

#[test]
fn the_channel_binding_hash_is_pinned_by_an_independent_vector() {
    assert_eq!(
        channel_bindings(Some(&[9; 32])).to_vec(),
        hex("b477d84bb5e1a11ab950338f6386aea0")
    );
    assert_eq!(channel_bindings(None), [0; 16]);
}

#[test]
fn the_time_is_a_filetime() {
    assert_eq!(u64::from_le_bytes(filetime(0)), 116_444_736_000_000_000);
    assert_eq!(u64::from_le_bytes(filetime(u64::MAX)), u64::MAX);
}

#[test]
fn a_server_that_does_not_speak_unicode_is_refused() {
    let challenge = challenge_with(0x0000_0002, &[(1, utf16("MAIL"))]);
    let credentials = Credentials::new("u", "p").unwrap();
    let (handshake, _) = Handshake::negotiate(&credentials);
    let context = Context {
        client_challenge: [0; 8],
        unix_seconds: 0,
        host: "h",
        channel_binding: None,
    };
    assert_eq!(
        handshake
            .authenticate(&base64(&challenge), &context)
            .unwrap_err(),
        NtlmError::NotUnicode
    );
}

#[test]
fn user_names_are_split_and_checked() {
    assert!(Credentials::new("DOMAIN\\ada", "pw").is_ok());
    assert!(Credentials::new("ada@example.org", "pw").is_ok());
    assert_eq!(
        Credentials::new("", "pw").unwrap_err(),
        NtlmError::NoUserName
    );
    assert_eq!(
        Credentials::new("DOMAIN\\", "pw").unwrap_err(),
        NtlmError::NoUserName
    );
    assert_eq!(
        Credentials::new("a\\b\\c", "pw").unwrap_err(),
        NtlmError::Unusable
    );
    assert_eq!(
        Credentials::new("ada", "p\r\nw").unwrap_err(),
        NtlmError::Unusable
    );
}

#[test]
fn nothing_secret_is_shown() {
    let credentials = Credentials::new("EXAMPLE\\ada", "hunter2-hunter2").unwrap();
    let (handshake, _) = Handshake::negotiate(&credentials);
    let shown = format!("{credentials:?} {handshake:?}");
    for secret in ["hunter2", "ada", "EXAMPLE"] {
        assert!(!shown.contains(secret), "{shown}");
    }
}

#[test]
fn hostile_challenges_are_refused() {
    let many: Vec<(u16, Vec<u8>)> = (0..=MAX_PAIRS).map(|_| (1u16, vec![0u8; 2])).collect();
    assert_eq!(
        Challenge::parse(&challenge_with(0xe28a_8233, &many)).err(),
        Some(NtlmError::TooLarge)
    );
    let big = vec![(1u16, vec![0u8; 60_000])];
    assert_eq!(
        Challenge::parse(&challenge_with(0xe28a_8233, &big)).err(),
        Some(NtlmError::TooLarge)
    );
    let mut far = challenge_with(0xe28a_8233, &[(1, vec![0u8; 8])]);
    far[44..48].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Challenge::parse(&far).is_err());
    assert!(Challenge::parse(&[]).is_err());
    assert!(Challenge::parse(b"NTLMSSP\0\x01\0\0\0").is_err());
    let mut raw = challenge_with(0xe28a_8233, &[(1, utf16("MAIL"))]);
    raw[58..60].copy_from_slice(&200u16.to_le_bytes());
    assert_eq!(Challenge::parse(&raw).err(), Some(NtlmError::BadChallenge));
    // Noise never panics.
    for len in 0..120usize {
        let noise: Vec<u8> = (0..len)
            .map(|n| u8::try_from((n * 37 + 11) % 256).unwrap())
            .collect();
        let _ = Challenge::parse(&noise);
        let mut shaped = challenge_with(0xffff_ffff, &[]);
        shaped.truncate(40);
        shaped.extend_from_slice(&noise);
        let _ = Challenge::parse(&shaped);
    }
}

#[test]
fn the_token_is_found_among_the_offers() {
    let offers_list = vec![
        "Negotiate".to_owned(),
        "NTLM TlRMTVNTUAACAAAA".to_owned(),
        "Basic realm=\"mail.example.org\"".to_owned(),
    ];
    assert_eq!(ntlm_token(&offers_list), Some("TlRMTVNTUAACAAAA"));
    assert!(offers(&offers_list, "basic"));
    assert!(offers(&["Negotiate, NTLM".to_owned()], "ntlm"));
    assert!(!offers(&["Negotiate".to_owned()], "ntlm"));
    assert_eq!(ntlm_token(&["NTLM".to_owned()]), None);
    assert_eq!(unbase64("A"), None);
    assert_eq!(unbase64("ab!d"), None);
}
