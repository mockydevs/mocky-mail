//! Tests of the security module: interop fixtures made with OpenSSL 3.1
//! and GnuPG 2.5 (in `fixtures/`, never re-encoded), and certificates and
//! keys made at test time.
//!
//! Fixtures: `ca.cer` (a test root), `leaf.pem`/`leaf.pk8` (Amina's RSA
//! certificate and key), `leaf.p12` (AES-256, SHA-256 MAC) and
//! `leaf-3des.p12` (the older triple DES), password `kenya-2026`;
//! `signed.eml` (detached, SHA-256), `sha1.eml`, `opaque.eml` and
//! `opaque-ber.eml` (streamed BER), `enveloped.eml`, `enveloped-ber.eml`
//! (AES-256-CBC) and `gcm.eml` (AES-256-GCM `AuthEnvelopedData`), all to
//! Amina. `baraka-*.asc`: a GnuPG Ed25519/Cv25519 key, passphrase
//! `jambo-2026`; `pgp-*.eml`: PGP/MIME signed, inline clear-signed,
//! PGP/MIME signed-and-encrypted (GnuPG's OCB packet), inline encrypted.

use der::{Decode, Encode};
use mailcraft_openpgp::KeyRing;

use super::smime::trust::TrustRoots;
use super::*;

const CA: &[u8] = include_bytes!("fixtures/ca.cer");
const LEAF_PEM: &[u8] = include_bytes!("fixtures/leaf.pem");
const LEAF_KEY: &[u8] = include_bytes!("fixtures/leaf.pk8");
const LEAF_P12: &[u8] = include_bytes!("fixtures/leaf.p12");
const LEAF_3DES_P12: &[u8] = include_bytes!("fixtures/leaf-3des.p12");
const SIGNED: &[u8] = include_bytes!("fixtures/signed.eml");
const SHA1: &[u8] = include_bytes!("fixtures/sha1.eml");
const OPAQUE: &[u8] = include_bytes!("fixtures/opaque.eml");
const OPAQUE_BER: &[u8] = include_bytes!("fixtures/opaque-ber.eml");
const ENVELOPED: &[u8] = include_bytes!("fixtures/enveloped.eml");
const ENVELOPED_BER: &[u8] = include_bytes!("fixtures/enveloped-ber.eml");
const GCM: &[u8] = include_bytes!("fixtures/gcm.eml");
const BARAKA_PUB: &[u8] = include_bytes!("fixtures/baraka-pub.asc");
const BARAKA_SEC: &[u8] = include_bytes!("fixtures/baraka-sec.asc");
const PGP_SIGNED: &[u8] = include_bytes!("fixtures/pgp-signed.eml");
const PGP_INLINE_SIGNED: &[u8] = include_bytes!("fixtures/pgp-inline-signed.eml");
const PGP_ENCRYPTED: &[u8] = include_bytes!("fixtures/pgp-encrypted.eml");
const PGP_INLINE_ENCRYPTED: &[u8] = include_bytes!("fixtures/pgp-inline-encrypted.eml");

/// 2026-11-27, when the fixtures were made (they are valid for 100 years).
const NOW: u64 = 1_795_800_000;

fn leaf_der() -> Vec<u8> {
    smime::read_certificates(LEAF_PEM).unwrap().remove(0)
}

fn amina() -> smime::Identity {
    smime::Identity::new(leaf_der(), vec![CA.to_vec()], LEAF_KEY).unwrap()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    mime::find(haystack, needle.as_bytes()).is_some()
}

struct Keys {
    smime: Vec<smime::Identity>,
    openpgp: Vec<pgp::composed::SignedSecretKey>,
    ring: KeyRing,
    roots: TrustRoots,
    trusted: Vec<String>,
}

impl Keys {
    fn none() -> Self {
        Self {
            smime: Vec::new(),
            openpgp: Vec::new(),
            ring: KeyRing::new(),
            roots: TrustRoots::default(),
            trusted: Vec::new(),
        }
    }

    fn open(&self, raw: &[u8]) -> Opened {
        let trusted = |_: SecurityKind, fp: &str| self.trusted.iter().any(|t| t == fp);
        open(
            raw,
            &OpenKeys {
                smime: &self.smime,
                openpgp: &self.openpgp,
                ring: &self.ring,
                roots: &self.roots,
                known_certs: &[],
                trusted: &trusted,
                locked: None,
                now: NOW,
            },
        )
    }
}

fn signature(opened: &Opened) -> &SignatureCheck {
    opened.signature.as_ref().unwrap()
}

// ---- S/MIME ----

#[test]
fn openssl_signed_mail_is_verified_against_its_root() {
    let mut keys = Keys::none();
    keys.roots = TrustRoots::from_der([CA.to_vec()]);
    let opened = keys.open(SIGNED);
    let check = signature(&opened);
    assert_eq!(check.kind, SecurityKind::Smime);
    assert_eq!(check.result, SignatureResult::Verified, "{}", check.detail);
    assert!(!check.from_mismatch);
    let signer = check.signer.as_ref().unwrap();
    assert_eq!(signer.address.as_deref(), Some("amina@example.org"));
    assert_eq!(signer.name.as_deref(), Some("Amina Otieno"));
    assert_eq!(signer.issuer.as_deref(), Some("Mocky Test Root CA"));
    assert!(check.signed_at.is_some());
    assert!(check.detail.contains("Mocky Test Root CA"));
    // Shown without the signature part, with the received headers.
    let shown = opened.shown.as_deref().unwrap();
    assert!(contains(shown, "Subject: Signed quote"));
    assert!(contains(shown, "Habari Amina"));
    assert!(!contains(shown, "smime.p7s"));
    // The signer's certificate is kept for encrypting to her.
    assert_eq!(opened.certificates.len(), 1);
    assert_eq!(opened.certificates[0].0, leaf_der());
    assert!(opened.encryption.is_none());
}

#[test]
fn a_certificate_from_an_unknown_authority_is_not_verified() {
    let opened = Keys::none().open(SIGNED);
    let check = signature(&opened);
    assert_eq!(check.result, SignatureResult::UnverifiedSigner);
    assert!(check.detail.contains("authority"));
    // Unless the user said this certificate is Amina's.
    let mut keys = Keys::none();
    keys.trusted = vec![smime::fingerprint(&leaf_der())];
    assert_eq!(
        signature(&keys.open(SIGNED)).result,
        SignatureResult::Verified
    );
}

#[test]
fn a_changed_signed_message_is_caught() {
    let mut keys = Keys::none();
    keys.roots = TrustRoots::from_der([CA.to_vec()]);
    let text = String::from_utf8_lossy(SIGNED).replace("Habari Amina", "Habari Kamau");
    let opened = keys.open(text.as_bytes());
    assert_eq!(signature(&opened).result, SignatureResult::Changed);
    assert!(opened.certificates.is_empty());
}

#[test]
fn sha1_signatures_are_not_trusted() {
    let mut keys = Keys::none();
    keys.roots = TrustRoots::from_der([CA.to_vec()]);
    let check = keys.open(SHA1).signature.unwrap();
    assert_eq!(check.result, SignatureResult::Unknown);
    assert!(check.detail.contains("SHA-1"), "{}", check.detail);
}

#[test]
fn opaque_signatures_der_and_streamed_ber() {
    let mut keys = Keys::none();
    keys.roots = TrustRoots::from_der([CA.to_vec()]);
    for raw in [OPAQUE, OPAQUE_BER] {
        let opened = keys.open(raw);
        assert_eq!(signature(&opened).result, SignatureResult::Verified);
        assert!(contains(opened.shown.as_deref().unwrap(), "Habari Amina"));
    }
}

#[test]
fn openssl_encrypted_mail_is_decrypted_cbc_ber_and_gcm() {
    let mut keys = Keys::none();
    keys.smime = vec![amina()];
    for raw in [ENVELOPED, ENVELOPED_BER, GCM] {
        let opened = keys.open(raw);
        let encryption = opened.encryption.unwrap();
        assert_eq!(encryption.kind, SecurityKind::Smime);
        assert_eq!(encryption.state, EncryptionState::Decrypted);
        let shown = opened.shown.unwrap();
        assert!(contains(&shown, "The secret plan."));
        assert!(contains(&shown, "From: boss@example.com"));
        // What is shown parses as an ordinary message.
        let body = mail_core::message::parse_body(&shown).unwrap();
        assert_eq!(
            body.text.as_deref().map(str::trim),
            Some("The secret plan.")
        );
        assert!(body.attachments.is_empty());
    }
}

#[test]
fn encrypted_mail_without_the_key_is_locked_with_why() {
    let opened = Keys::none().open(ENVELOPED);
    let encryption = opened.encryption.unwrap();
    assert_eq!(encryption.state, EncryptionState::Locked);
    assert!(encryption.detail.unwrap().contains("certificate"));
    assert!(opened.shown.is_none());
    // The keyring can't be opened: say so.
    let trusted = |_: SecurityKind, _: &str| false;
    let ring = KeyRing::new();
    let roots = TrustRoots::default();
    let opened = open(
        ENVELOPED,
        &OpenKeys {
            smime: &[],
            openpgp: &[],
            ring: &ring,
            roots: &roots,
            known_certs: &[],
            trusted: &trusted,
            locked: Some("The system keyring is locked."),
            now: NOW,
        },
    );
    assert_eq!(
        opened.encryption.unwrap().detail.as_deref(),
        Some("The system keyring is locked.")
    );
}

#[test]
fn p12_files_are_read_with_their_password() {
    for file in [LEAF_P12, LEAF_3DES_P12] {
        let contents = smime::pkcs12::read(file, "kenya-2026").unwrap();
        assert_eq!(contents.cert, leaf_der());
        assert_eq!(contents.chain, vec![CA.to_vec()]);
        let identity = smime::Identity::new(contents.cert, contents.chain, &contents.key).unwrap();
        assert!(matches!(identity.key, smime::PrivateKey::Rsa(_)));
        let wrong = smime::pkcs12::read(file, "nairobi").unwrap_err();
        assert!(wrong.to_string().contains("isn't right"), "{wrong}");
    }
    assert!(smime::pkcs12::read(b"not a p12", "x").is_err());
}

#[test]
fn certificates_are_read_from_pem_and_der() {
    let pem = smime::read_certificates(LEAF_PEM).unwrap();
    assert_eq!(pem.len(), 1);
    assert_eq!(smime::read_certificates(CA).unwrap(), vec![CA.to_vec()]);
    let info = smime::cert_info(&pem[0]).unwrap();
    assert_eq!(info.owner, "Amina Otieno");
    assert_eq!(info.addresses, ["amina@example.org"]);
    assert_eq!(info.issuer, "Mocky Test Root CA");
    assert!(info.can_sign && info.can_encrypt && !info.is_ca);
    let ca = smime::cert_info(CA).unwrap();
    assert!(ca.is_ca && !ca.can_sign && !ca.can_encrypt);
    // Back to PEM, to share.
    let shared = smime::to_pem(&pem[0]).unwrap();
    assert_eq!(smime::read_certificates(shared.as_bytes()).unwrap(), pem);
    assert!(smime::read_certificates(b"nothing").is_err());
}

const BUILT: &[u8] = b"From: Amina Otieno <amina@example.org>\r\nTo: boss@example.com\r\nSubject: Quote\r\nDate: Fri, 27 Nov 2026 10:00:00 +0300\r\nMessage-ID: <q1@example.org>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n--mix\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nThe quote, attached.  \r\n--mix\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"quote.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--mix--\r\n";

#[test]
fn smime_sign_then_encrypt_round_trip() {
    let me = amina();
    let recipients = vec![leaf_der()];
    let sent = protect(
        BUILT,
        &Protection {
            sign: Some(Signer::Smime(&me)),
            encrypt: Some(Recipients::Smime(&recipients)),
            now: NOW,
        },
    )
    .unwrap();
    // The addressing stays outside; the content doesn't.
    assert!(contains(&sent, "Subject: Quote"));
    assert!(contains(&sent, "smime-type=enveloped-data"));
    assert!(!contains(&sent, "The quote, attached."));
    let summary = mail_core::message::parse_summary(&sent).unwrap();
    assert!(summary.encrypted && !summary.signed);

    let mut keys = Keys::none();
    keys.smime = vec![amina()];
    keys.roots = TrustRoots::from_der([CA.to_vec()]);
    let opened = keys.open(&sent);
    assert_eq!(
        opened.encryption.as_ref().unwrap().state,
        EncryptionState::Decrypted
    );
    assert_eq!(signature(&opened).result, SignatureResult::Verified);
    let shown = opened.shown.unwrap();
    let body = mail_core::message::parse_body(&shown).unwrap();
    assert!(body.text.unwrap().contains("The quote, attached."));
    assert_eq!(body.attachments.len(), 1);
    assert_eq!(body.attachments[0].file_name, "quote.pdf");
}

#[test]
fn smime_signed_only_keeps_the_trailing_spaces_it_signed() {
    let me = amina();
    let sent = protect(
        BUILT,
        &Protection {
            sign: Some(Signer::Smime(&me)),
            encrypt: None,
            now: NOW,
        },
    )
    .unwrap();
    let summary = mail_core::message::parse_summary(&sent).unwrap();
    assert!(summary.signed && !summary.encrypted);
    assert!(contains(&sent, "micalg=sha-256"));
    let mut keys = Keys::none();
    keys.roots = TrustRoots::from_der([CA.to_vec()]);
    assert_eq!(
        signature(&keys.open(&sent)).result,
        SignatureResult::Verified
    );
}

/// A CA and a signer made now, with ECDSA P-256.
fn made_now() -> (Vec<u8>, smime::Identity) {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
        KeyPair, KeyUsagePurpose, SanType,
    };
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Made Now CA");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let leaf_key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, "Wanjiru Kamau");
    params.subject_alt_names = vec![SanType::Rfc822Name(
        "wanjiru@fitout.example".try_into().unwrap(),
    )];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::EmailProtection];
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let leaf = params.signed_by(&leaf_key, &issuer).unwrap();
    let identity =
        smime::Identity::new(leaf.der().to_vec(), Vec::new(), &leaf_key.serialize_der()).unwrap();
    (ca.der().to_vec(), identity)
}

const NOTE: &[u8] = b"From: wanjiru@fitout.example\r\nTo: you@example.com\r\nSubject: Note\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nKaribu =E2=80=94 see you at 10.\r\n";

#[test]
fn certificates_made_now_good_changed_untrusted_and_weak() {
    let (ca, me) = made_now();
    assert!(matches!(me.key, smime::PrivateKey::P256(_)));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let sent = protect(
        NOTE,
        &Protection {
            sign: Some(Signer::Smime(&me)),
            encrypt: None,
            now,
        },
    )
    .unwrap();
    let mut keys = Keys::none();
    keys.roots = TrustRoots::from_der([ca.clone()]);
    let check = keys.open(&sent).signature.unwrap();
    assert_eq!(check.result, SignatureResult::Verified, "{}", check.detail);
    assert_eq!(
        check.signer.unwrap().address.as_deref(),
        Some("wanjiru@fitout.example")
    );
    // Changed.
    let changed = String::from_utf8_lossy(&sent).replace("see you at 10", "see you at 11");
    assert_eq!(
        keys.open(changed.as_bytes()).signature.unwrap().result,
        SignatureResult::Changed
    );
    // A root nobody trusts.
    assert_eq!(
        Keys::none().open(&sent).signature.unwrap().result,
        SignatureResult::UnverifiedSigner
    );
    // The same signature claiming a SHA-1 digest is refused before it is checked.
    let (header, body) = mime::split(&sent);
    let fields = mime::fields(header);
    let ct = mime::ContentType::parse(mime::header(&fields, "content-type"));
    let parts = mime::parts(body, ct.param("boundary").unwrap(), 2);
    let (_, sig_body) = mime::split(parts[1]);
    let der = mime::base64_lenient(sig_body);
    let info = cms::content_info::ContentInfo::from_der(&der).unwrap();
    let mut signed = info
        .content
        .decode_as::<cms::signed_data::SignedData>()
        .unwrap();
    let mut signers: Vec<cms::signed_data::SignerInfo> =
        signed.signer_infos.0.iter().cloned().collect();
    signers[0].digest_alg.oid = smime::oid::SHA1;
    signed.signer_infos = cms::signed_data::SignerInfos(signers.try_into().unwrap());
    let weak = cms::content_info::ContentInfo {
        content_type: info.content_type,
        content: der::Any::encode_from(&signed).unwrap(),
    }
    .to_der()
    .unwrap();
    let cx = smime::TrustContext {
        roots: &keys.roots,
        known: &[],
        trusted: &|_| false,
        now: i64::try_from(now).unwrap(),
    };
    let signed_part = mime::crlf(parts[0]);
    let checked = smime::verify_detached(&weak, &signed_part, &cx);
    assert!(
        matches!(&checked.outcome, smime::VerifyOutcome::Unknown(why) if why.contains("SHA-1")),
        "{:?}",
        checked.outcome
    );
    // And the untouched one checks the same way.
    let good = smime::verify_detached(&der, &signed_part, &cx);
    assert_eq!(good.outcome, smime::VerifyOutcome::Verified);
}

#[test]
fn encrypting_to_a_non_rsa_certificate_names_it() {
    let (_, me) = made_now();
    let error = smime::encrypt(b"x", std::slice::from_ref(&me.cert_der)).unwrap_err();
    assert!(error.to_string().contains("Wanjiru Kamau"), "{error}");
    assert!(smime::encrypt(b"x", &[]).is_err());
}

// ---- OpenPGP ----

fn baraka_public() -> mailcraft_openpgp::PublicKey {
    openpgp::read_public_keys(BARAKA_PUB).unwrap().remove(0)
}

fn baraka_keys(trusted: bool) -> Keys {
    let mut keys = Keys::none();
    let public = baraka_public();
    if trusted {
        keys.trusted = vec![public.fingerprint().to_hex()];
    }
    keys.ring.insert(public);
    keys
}

#[test]
fn gnupg_keys_are_imported_public_and_secret() {
    let public = baraka_public();
    let info = openpgp::key_info(&public, None);
    assert_eq!(info.fingerprint, "6C681E262957FF61CCE3FBAD084C42827E3D6D93");
    assert_eq!(info.owner, "Baraka Mwangi");
    assert_eq!(info.addresses, ["baraka@example.net"]);
    assert!(info.can_sign && info.can_encrypt && !info.revoked);
    assert_eq!(info.expires, None);
    // Binary, as a .gpg file holds it.
    let binary = public.to_bytes();
    assert_eq!(
        openpgp::read_public_keys(&binary).unwrap()[0].fingerprint(),
        public.fingerprint()
    );
    // The secret key, with its passphrase, which isn't kept.
    assert!(openpgp::holds_secret(BARAKA_SEC));
    assert!(!openpgp::holds_secret(BARAKA_PUB));
    let own = openpgp::read_secret_key(BARAKA_SEC, Some("jambo-2026")).unwrap();
    assert_eq!(own.public.fingerprint(), public.fingerprint());
    assert!(openpgp::can_sign(&own.secret));
    let saved = own.secret_bytes().unwrap();
    let back = openpgp::secret_from_bytes(&saved).unwrap();
    assert!(openpgp::can_sign(&back));
    let wrong = openpgp::read_secret_key(BARAKA_SEC, Some("habari")).unwrap_err();
    assert!(wrong.to_string().contains("isn't right"), "{wrong}");
    let missing = openpgp::read_secret_key(BARAKA_SEC, None).unwrap_err();
    assert!(missing.to_string().contains("passphrase"), "{missing}");
    assert!(openpgp::read_public_keys(b"no key here").is_err());
    // To share.
    let armored = openpgp::armored_public(&public).unwrap();
    assert!(armored.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
}

#[test]
fn gnupg_pgp_mime_signatures() {
    let check = baraka_keys(true).open(PGP_SIGNED).signature.unwrap();
    assert_eq!(check.kind, SecurityKind::Openpgp);
    assert_eq!(check.result, SignatureResult::Verified, "{}", check.detail);
    assert!(!check.from_mismatch);
    let signer = check.signer.unwrap();
    assert_eq!(signer.name.as_deref(), Some("Baraka Mwangi"));
    assert_eq!(signer.address.as_deref(), Some("baraka@example.net"));
    // A key that isn't marked as his.
    let opened = baraka_keys(false).open(PGP_SIGNED);
    assert_eq!(signature(&opened).result, SignatureResult::UnverifiedSigner);
    assert!(!contains(opened.shown.as_deref().unwrap(), "signature.asc"));
    // No key.
    assert_eq!(
        Keys::none().open(PGP_SIGNED).signature.unwrap().result,
        SignatureResult::Unknown
    );
    // Changed.
    let changed = String::from_utf8_lossy(PGP_SIGNED).replace("Habari!", "Habari?");
    assert_eq!(
        baraka_keys(true)
            .open(changed.as_bytes())
            .signature
            .unwrap()
            .result,
        SignatureResult::Changed
    );
    // From someone else.
    let other = String::from_utf8_lossy(PGP_SIGNED).replace(
        "From: Baraka Mwangi <baraka@example.net>",
        "From: imposter@example.net",
    );
    assert!(
        baraka_keys(true)
            .open(other.as_bytes())
            .signature
            .unwrap()
            .from_mismatch
    );
}

#[test]
fn gnupg_inline_cleartext_signature() {
    let check = baraka_keys(true).open(PGP_INLINE_SIGNED).signature.unwrap();
    assert_eq!(check.result, SignatureResult::Verified, "{}", check.detail);
    let changed = String::from_utf8_lossy(PGP_INLINE_SIGNED).replace("Jambo!", "Jambo?");
    assert_eq!(
        baraka_keys(true)
            .open(changed.as_bytes())
            .signature
            .unwrap()
            .result,
        SignatureResult::Changed
    );
}

#[test]
fn gnupg_encrypted_mail_is_decrypted_and_its_inner_signature_checked() {
    let own = openpgp::read_secret_key(BARAKA_SEC, Some("jambo-2026")).unwrap();
    let mut keys = baraka_keys(true);
    keys.openpgp = vec![own.secret];
    let opened = keys.open(PGP_ENCRYPTED);
    let encryption = opened.encryption.as_ref().unwrap();
    assert_eq!(encryption.kind, SecurityKind::Openpgp);
    assert_eq!(encryption.state, EncryptionState::Decrypted);
    assert_eq!(signature(&opened).result, SignatureResult::Verified);
    let shown = opened.shown.unwrap();
    assert!(contains(&shown, "The encrypted plan, signed inside."));
    // Inline armoured.
    let opened = keys.open(PGP_INLINE_ENCRYPTED);
    assert_eq!(opened.encryption.unwrap().state, EncryptionState::Decrypted);
    let body = mail_core::message::parse_body(&opened.shown.unwrap()).unwrap();
    assert!(body.text.unwrap().contains("Inline secret."));
    // Without the secret key.
    let locked = baraka_keys(true).open(PGP_ENCRYPTED).encryption.unwrap();
    assert_eq!(locked.state, EncryptionState::Locked);
    assert!(locked.detail.unwrap().contains("key"));
}

/// An OpenPGP key made now (Ed25519 and X25519, as Thunderbird makes).
fn pgp_key(user: &str) -> pgp::composed::SignedSecretKey {
    use pgp::composed::{EncryptionCaps, KeyType, SecretKeyParamsBuilder, SubkeyParamsBuilder};
    use pgp::crypto::ecc_curve::ECCCurve;
    let mut params = SecretKeyParamsBuilder::default();
    params
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(true)
        .primary_user_id(user.into())
        .subkeys(vec![
            SubkeyParamsBuilder::default()
                .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
                .can_encrypt(EncryptionCaps::All)
                .build()
                .unwrap(),
        ]);
    params.build().unwrap().generate(rand_core::OsRng).unwrap()
}

fn public_of(secret: &pgp::composed::SignedSecretKey) -> mailcraft_openpgp::PublicKey {
    use pgp::ser::Serialize as _;
    let bytes = secret.to_public_key().to_bytes().unwrap();
    openpgp::read_public_keys(&bytes).unwrap().remove(0)
}

#[test]
fn pgp_mime_sign_and_encrypt_round_trip() {
    let me = pgp_key("Wanjiru Kamau <wanjiru@fitout.example>");
    let them = pgp_key("Baraka Mwangi <baraka@example.net>");
    let me_public = public_of(&me);
    let them_public = public_of(&them);
    let to = vec![them_public.clone(), me_public.clone()];
    let raw = String::from_utf8_lossy(BUILT).replace(
        "From: Amina Otieno <amina@example.org>",
        "From: Wanjiru Kamau <wanjiru@fitout.example>",
    );
    let sent = protect(
        raw.as_bytes(),
        &Protection {
            sign: Some(Signer::Openpgp(&me)),
            encrypt: Some(Recipients::Openpgp(&to)),
            now: NOW,
        },
    )
    .unwrap();
    assert!(contains(&sent, "multipart/encrypted"));
    assert!(!contains(&sent, "The quote, attached."));
    // The recipient reads it, and trusts the sender's key.
    let mut keys = Keys::none();
    keys.openpgp = vec![them];
    keys.trusted = vec![me_public.fingerprint().to_hex()];
    keys.ring.insert(me_public.clone());
    let opened = keys.open(&sent);
    assert_eq!(
        opened.encryption.as_ref().unwrap().state,
        EncryptionState::Decrypted
    );
    let check = signature(&opened);
    assert_eq!(check.result, SignatureResult::Verified, "{}", check.detail);
    let body = mail_core::message::parse_body(opened.shown.as_deref().unwrap()).unwrap();
    assert!(body.text.unwrap().contains("The quote, attached."));
    assert_eq!(body.attachments.len(), 1);
    // So does the sender (Sent stays readable).
    let mut mine = Keys::none();
    mine.openpgp = vec![me];
    assert_eq!(
        mine.open(&sent).encryption.unwrap().state,
        EncryptionState::Decrypted
    );
}

#[test]
fn pgp_mime_signed_only_round_trip() {
    let me = pgp_key("Wanjiru Kamau <wanjiru@fitout.example>");
    let sent = protect(
        NOTE,
        &Protection {
            sign: Some(Signer::Openpgp(&me)),
            encrypt: None,
            now: NOW,
        },
    )
    .unwrap();
    assert!(contains(&sent, "micalg=pgp-sha256"));
    let mut keys = Keys::none();
    keys.ring.insert(public_of(&me));
    let opened = keys.open(&sent);
    assert_eq!(signature(&opened).result, SignatureResult::UnverifiedSigner);
    let body = mail_core::message::parse_body(opened.shown.as_deref().unwrap()).unwrap();
    assert!(body.text.unwrap().contains("see you at 10"));
}

#[test]
fn autocrypt_headers_are_written_and_read_back() {
    let me = pgp_key("Wanjiru Kamau <wanjiru@fitout.example>");
    let public = public_of(&me);
    let header = openpgp::autocrypt_header("Wanjiru@fitout.example", &public).unwrap();
    assert!(header.starts_with("Autocrypt: addr=wanjiru@fitout.example; keydata="));
    assert!(header.lines().all(|l| l.len() <= 78));
    let raw = with_header(NOTE, &header);
    let (_, autocrypt) = openpgp::keys_in_message(&raw);
    assert_eq!(autocrypt.unwrap().fingerprint(), public.fingerprint());
    // It is the sender's own: opening the message hands it over for keeping.
    let opened = Keys::none().open(&raw);
    assert!(opened.autocrypt.is_some());
    // One with an address other than From's counts for nothing.
    let other = openpgp::autocrypt_header("someone@else.example", &public).unwrap();
    let (_, none) = openpgp::keys_in_message(&with_header(NOTE, &other));
    assert!(none.is_none());
    // Unfolded, the value reads with the Level 1 parser.
    let value: String = header
        .trim_start_matches("Autocrypt:")
        .split("\r\n")
        .map(str::trim)
        .collect();
    let parsed = mailcraft_openpgp::parse_autocrypt(&value).unwrap();
    assert_eq!(parsed.address, "wanjiru@fitout.example");
}

#[test]
fn attached_keys_are_found() {
    let raw = format!(
        "From: baraka@example.net\r\nTo: you@example.com\r\nSubject: My key\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"k\"\r\n\r\n--k\r\nContent-Type: text/plain\r\n\r\nHere it is.\r\n--k\r\nContent-Type: application/pgp-keys; name=\"key.asc\"\r\n\r\n{}\r\n--k--\r\n",
        String::from_utf8_lossy(BARAKA_PUB).replace('\n', "\r\n")
    );
    let opened = Keys::none().open(raw.as_bytes());
    assert_eq!(opened.attached_keys.len(), 1);
    assert!(opened.signature.is_none() && opened.encryption.is_none());
}

#[test]
fn plain_mail_is_left_alone() {
    let opened = Keys::none().open(NOTE);
    assert!(opened.shown.is_none());
    assert!(opened.signature.is_none());
    assert!(opened.encryption.is_none());
}

#[test]
fn results_serialize_for_the_interface() {
    let check = SignatureCheck {
        kind: SecurityKind::Smime,
        result: SignatureResult::UnverifiedSigner,
        signer: None,
        from_mismatch: true,
        detail: "x".to_owned(),
        signed_at: None,
    };
    let json = serde_json::to_value(&check).unwrap();
    assert_eq!(json["result"], "unverifiedSigner");
    assert_eq!(json["kind"], "smime");
    assert_eq!(json["fromMismatch"], true);
    let info = EncryptionInfo {
        kind: SecurityKind::Openpgp,
        state: EncryptionState::Locked,
        detail: None,
    };
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["state"], "locked");
    assert_eq!(json["kind"], "openpgp");
}
