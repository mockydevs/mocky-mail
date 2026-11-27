//! Security (0.10.0): S/MIME and OpenPGP keys, opening signed and
//! encrypted mail, and signing and encrypting what is sent.
//!
//! Keys live in the database (`mail-store`), their private parts sealed
//! with a key held in the system keyring (`mail_sync::security::vault`).
//! Decrypted messages are kept in memory only, a few at a time, for the
//! reading pane and its attachments; never in the database or the search
//! index.

#![allow(
    clippy::doc_markdown,
    reason = "OpenPGP and GnuPG are names here, not code"
)]

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use mail_core::message::{AttachmentInfo, Mailbox, safe_file_name};
use mail_store::{Account, KeyKind, KeyOrigin, NewKey, StoredKey, StoredMessage};
use mail_sync::security::smime::trust::TrustRoots;
use mail_sync::security::smime::{self, Identity};
use mail_sync::security::vault::Vault;
use mail_sync::security::{
    self as sec, KeyRing, OpenKeys, Opened, Protection, PublicKey, Recipients, SecurityKind,
    SignedSecretKey, Signer, mime, openpgp, wkd,
};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::{AppState, lock, now};

/// Decrypted messages kept for the reading pane and its attachments.
const OPENED_KEPT: usize = 16;
/// Largest key or certificate file imported.
const MAX_KEY_FILE: u64 = 1024 * 1024;
/// Most recipients looked up in Web Key Directories at once.
const MAX_LOOKUPS: usize = 20;

/// What security keeps in memory.
#[derive(Default)]
pub struct SecurityState {
    roots: OnceLock<TrustRoots>,
    opened: Mutex<VecDeque<(String, Arc<Vec<u8>>)>>,
}

impl SecurityState {
    /// The system's trusted roots, read once.
    fn roots(&self) -> &TrustRoots {
        self.roots.get_or_init(TrustRoots::system)
    }

    fn remember(&self, id: &str, shown: &[u8]) {
        let mut opened = lock(&self.opened);
        opened.retain(|(kept, _)| kept != id);
        opened.push_back((id.to_owned(), Arc::new(shown.to_vec())));
        while opened.len() > OPENED_KEPT {
            opened.pop_front();
        }
    }

    fn recall(&self, id: &str) -> Option<Arc<Vec<u8>>> {
        lock(&self.opened)
            .iter()
            .find(|(kept, _)| kept == id)
            .map(|(_, shown)| Arc::clone(shown))
    }

    /// Forget every decrypted message (keys changed).
    fn forget_all(&self) {
        lock(&self.opened).clear();
    }
}

const fn to_store(kind: SecurityKind) -> KeyKind {
    match kind {
        SecurityKind::Smime => KeyKind::Smime,
        SecurityKind::Openpgp => KeyKind::Openpgp,
    }
}

const fn from_store(kind: KeyKind) -> SecurityKind {
    match kind {
        KeyKind::Smime => SecurityKind::Smime,
        KeyKind::Openpgp => SecurityKind::Openpgp,
    }
}

/// What a private part is sealed to: its kind and fingerprint.
fn label(kind: KeyKind, fingerprint: &str) -> String {
    format!("{}:{fingerprint}", kind.as_str())
}

fn now_i64() -> i64 {
    i64::try_from(now()).unwrap_or(i64::MAX)
}

const LOCKED: &str = "Your private keys can't be unlocked: the system keyring isn't available. Unlock it (or sign in again) and reopen the message.";

/// The user's own keys, unsealed, and everything to check with.
struct Loaded {
    smime_ids: Vec<String>,
    smime: Vec<Identity>,
    openpgp_ids: Vec<String>,
    openpgp: Vec<SignedSecretKey>,
    openpgp_public: Vec<PublicKey>,
    ring: KeyRing,
    known: Vec<Vec<u8>>,
    trusted: HashSet<String>,
    locked: Option<&'static str>,
}

impl Loaded {
    fn is_trusted(&self, kind: SecurityKind, fingerprint: &str) -> bool {
        self.trusted
            .contains(&label(to_store(kind), &fingerprint.to_ascii_uppercase()))
    }
}

fn load(state: &AppState) -> AppResult<Loaded> {
    let keys = state.store.list_keys()?;
    let mut loaded = Loaded {
        smime_ids: Vec::new(),
        smime: Vec::new(),
        openpgp_ids: Vec::new(),
        openpgp: Vec::new(),
        openpgp_public: Vec::new(),
        ring: KeyRing::new(),
        known: Vec::new(),
        trusted: HashSet::new(),
        locked: None,
    };
    let any_own = keys.iter().any(|k| k.is_own);
    let vault = if any_own {
        let vault = Vault::open(&*state.secrets, false).ok().flatten();
        if vault.is_none() {
            loaded.locked = Some(LOCKED);
        }
        vault
    } else {
        None
    };
    for key in &keys {
        if key.trusted {
            loaded.trusted.insert(label(key.kind, &key.fingerprint));
        }
        match key.kind {
            KeyKind::Smime => {
                loaded.known.push(key.public_data.clone());
                loaded.known.extend(key.chain.iter().cloned());
            }
            KeyKind::Openpgp => {
                if let Some(public) = openpgp::public_from_bytes(&key.public_data) {
                    loaded.ring.insert(public);
                }
            }
        }
        if !key.is_own {
            continue;
        }
        let Some(vault) = &vault else { continue };
        let Some(sealed) = state.store.key_secret(&key.id)? else {
            continue;
        };
        let Ok(plain) = vault.open_sealed(&label(key.kind, &key.fingerprint), &sealed) else {
            loaded.locked = Some(LOCKED);
            continue;
        };
        match key.kind {
            KeyKind::Smime => {
                match Identity::new(key.public_data.clone(), key.chain.clone(), &plain) {
                    Ok(identity) => {
                        loaded.smime_ids.push(key.id.clone());
                        loaded.smime.push(identity);
                    }
                    Err(error) => eprintln!("a saved S/MIME key can't be used: {error}"),
                }
            }
            KeyKind::Openpgp => {
                let public = openpgp::public_from_bytes(&key.public_data);
                match (openpgp::secret_from_bytes(&plain), public) {
                    (Ok(secret), Some(public)) => {
                        loaded.openpgp_ids.push(key.id.clone());
                        loaded.openpgp.push(secret);
                        loaded.openpgp_public.push(public);
                    }
                    _ => eprintln!("a saved OpenPGP key can't be used"),
                }
            }
        }
    }
    for key in gpg_keys() {
        loaded.ring.insert(key);
    }
    Ok(loaded)
}

/// GnuPG's public keys (computers only).
#[cfg(desktop)]
fn gpg_keys() -> Vec<PublicKey> {
    sec::system::gnupg_public_keys()
}

#[cfg(not(desktop))]
fn gpg_keys() -> Vec<PublicKey> {
    Vec::new()
}

fn header_has(raw: &[u8], name: &[u8]) -> bool {
    let (header, _) = mime::split(raw);
    header.split(|b| *b == b'\n').any(|line| {
        line.len() >= name.len()
            && line
                .get(..name.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(name))
    })
}

/// Open a received message: decrypt it and check its signature when it is
/// signed or encrypted, and keep the keys and certificates it carries.
///
/// # Errors
/// Database errors.
pub fn open_message(state: &AppState, stored: &StoredMessage, raw: &[u8]) -> AppResult<Opened> {
    let secure =
        stored.item.signed || stored.item.encrypted || mime::find(raw, b"-----BEGIN PGP").is_some();
    let opened = if secure {
        open_with_keys(state, raw)?
    } else if header_has(raw, b"autocrypt:") || mime::find(raw, b"application/pgp-keys").is_some() {
        let (attached_keys, autocrypt) = openpgp::keys_in_message(raw);
        Opened {
            attached_keys,
            autocrypt,
            ..Opened::default()
        }
    } else {
        Opened::default()
    };
    keep_found(state, &opened);
    if let (Some(shown), Some(encryption)) = (&opened.shown, &opened.encryption)
        && encryption.state == sec::EncryptionState::Decrypted
    {
        state.security.remember(&stored.item.id, shown);
    }
    Ok(opened)
}

/// Open `raw` with every key here.
fn open_with_keys(state: &AppState, raw: &[u8]) -> AppResult<Opened> {
    let loaded = load(state)?;
    let trusted = |kind: SecurityKind, fp: &str| loaded.is_trusted(kind, fp);
    Ok(sec::open(
        raw,
        &OpenKeys {
            smime: &loaded.smime,
            openpgp: &loaded.openpgp,
            ring: &loaded.ring,
            roots: state.security.roots(),
            known_certs: &loaded.known,
            trusted: &trusted,
            locked: loaded.locked,
            now: now(),
        },
    ))
}

/// A message this device signed or encrypted (queued in the Outbox),
/// opened with the sender's own keys to be edited: the message inside, and
/// the kinds it was signed and encrypted with.
///
/// # Errors
/// `invalid` when it can't be decrypted here any more.
pub fn open_own_copy(
    state: &AppState,
    raw: &[u8],
) -> AppResult<(Vec<u8>, Option<SecurityKind>, Option<SecurityKind>)> {
    let flags = mail_core::message::parse_summary(raw).ok();
    if !flags.is_some_and(|f| f.signed || f.encrypted) {
        return Ok((raw.to_vec(), None, None));
    }
    let opened = open_with_keys(state, raw)?;
    if let Some(encryption) = &opened.encryption
        && encryption.state == sec::EncryptionState::Locked
    {
        return Err(AppError::invalid(format!(
            "This message can't be opened for editing. {}",
            encryption.detail.as_deref().unwrap_or_default()
        )));
    }
    Ok((
        opened.shown.unwrap_or_else(|| raw.to_vec()),
        opened.signature.map(|s| s.kind),
        opened.encryption.map(|e| e.kind),
    ))
}

/// The message as the reading pane shows it (decrypted, signature taken
/// off), for its attachments. The received bytes when it is neither
/// signed nor encrypted.
///
/// # Errors
/// Database errors.
pub fn shown_bytes(state: &AppState, id: &str, raw: Vec<u8>) -> AppResult<Vec<u8>> {
    if let Some(shown) = state.security.recall(id) {
        return Ok(shown.to_vec());
    }
    let stored = state.store.get_message(id)?;
    if !(stored.item.signed
        || stored.item.encrypted
        || mime::find(&raw, b"-----BEGIN PGP").is_some())
    {
        return Ok(raw);
    }
    Ok(open_message(state, &stored, &raw)?.shown.unwrap_or(raw))
}

fn smime_key(
    der: &[u8],
    chain: Vec<Vec<u8>>,
    secret: Option<Vec<u8>>,
    origin: KeyOrigin,
) -> AppResult<NewKey> {
    let info = smime::cert_info(der)?;
    Ok(NewKey {
        kind: KeyKind::Smime,
        fingerprint: info.fingerprint,
        owner: info.owner,
        addresses: info.addresses,
        expires: Some(info.not_after),
        can_sign: info.can_sign,
        can_encrypt: info.can_encrypt,
        public_data: der.to_vec(),
        chain,
        secret,
        origin,
    })
}

fn openpgp_key(
    key: &PublicKey,
    secret: Option<Vec<u8>>,
    origin: KeyOrigin,
    can_sign: Option<bool>,
) -> NewKey {
    let info = openpgp::key_info(key, can_sign);
    NewKey {
        kind: KeyKind::Openpgp,
        fingerprint: info.fingerprint,
        owner: info.owner,
        addresses: info.addresses,
        expires: info.expires,
        can_sign: info.can_sign,
        can_encrypt: info.can_encrypt,
        public_data: key.to_bytes(),
        chain: Vec::new(),
        secret,
        origin,
    }
}

/// Keep what a message carried: S/MIME signers' certificates and OpenPGP
/// keys (attached, and Autocrypt's). A failure is only logged: the message
/// still opens.
fn keep_found(state: &AppState, opened: &Opened) {
    let now = now();
    for (cert, chain) in &opened.certificates {
        match smime_key(cert, chain.clone(), None, KeyOrigin::Signed) {
            Ok(key) => {
                if let Err(error) = state.store.save_key(&key, now) {
                    eprintln!("couldn't keep a signer's certificate: {error}");
                }
            }
            Err(error) => eprintln!("couldn't read a signer's certificate: {error}"),
        }
    }
    let found = opened
        .attached_keys
        .iter()
        .map(|k| (k, KeyOrigin::Attachment))
        .chain(opened.autocrypt.iter().map(|k| (k, KeyOrigin::Autocrypt)));
    for (key, origin) in found {
        if let Err(error) = state
            .store
            .save_key(&openpgp_key(key, None, origin, None), now)
        {
            eprintln!("couldn't keep an OpenPGP key: {error}");
        }
    }
}

/// Keep the keys a downloaded message carries (its Autocrypt header and
/// attached keys), without opening it. Called when bodies are cached.
pub fn collect_keys(state: &AppState, raw: &[u8]) {
    if !(header_has(raw, b"autocrypt:") || mime::find(raw, b"application/pgp-keys").is_some()) {
        return;
    }
    let (attached_keys, autocrypt) = openpgp::keys_in_message(raw);
    keep_found(
        state,
        &Opened {
            attached_keys,
            autocrypt,
            ..Opened::default()
        },
    );
}

// ---- Sending ----

/// What a draft asks for (`Draft.sign`, `encrypt`, `securityKind`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Choice {
    /// Sign; `None`: as Settings `signByDefault` says.
    pub sign: Option<bool>,
    /// Encrypt; `None`: as Settings `encryptWhenPossible` says.
    pub encrypt: Option<bool>,
    /// S/MIME or OpenPGP; `None`: the account's preferred kind.
    pub kind: Option<SecurityKind>,
}

/// The account's own key of each kind: the one chosen for it, else one
/// for its address.
struct OwnKeys<'a> {
    smime: Option<&'a Identity>,
    openpgp: Option<(&'a SignedSecretKey, &'a PublicKey)>,
}

fn own_keys<'a>(state: &AppState, account: &Account, loaded: &'a Loaded) -> AppResult<OwnKeys<'a>> {
    let chosen = state.store.account_keys(&account.id)?;
    let email = account.email.trim().to_lowercase();
    let smime_index = chosen
        .smime
        .as_deref()
        .and_then(|id| loaded.smime_ids.iter().position(|kid| kid == id))
        .or_else(|| {
            loaded.smime.iter().position(|i| {
                smime::cert_info(&i.cert_der).is_ok_and(|info| info.addresses.contains(&email))
            })
        });
    let openpgp_index = chosen
        .openpgp
        .as_deref()
        .and_then(|id| loaded.openpgp_ids.iter().position(|kid| kid == id))
        .or_else(|| {
            loaded
                .openpgp_public
                .iter()
                .position(|public| public.has_address(&email))
        });
    let smime = smime_index.and_then(|i| loaded.smime.get(i));
    let openpgp =
        openpgp_index.and_then(|i| Some((loaded.openpgp.get(i)?, loaded.openpgp_public.get(i)?)));
    Ok(OwnKeys { smime, openpgp })
}

/// The kind an account uses when a message doesn't say.
fn preferred_kind(
    state: &AppState,
    account_id: &str,
    own: Option<&OwnKeys<'_>>,
) -> AppResult<SecurityKind> {
    let chosen = state.store.account_keys(account_id)?;
    if let Some(kind) = chosen.preferred {
        return Ok(from_store(kind));
    }
    let has_smime = chosen.smime.is_some() || own.is_some_and(|o| o.smime.is_some());
    let has_openpgp = chosen.openpgp.is_some() || own.is_some_and(|o| o.openpgp.is_some());
    Ok(if has_openpgp && !has_smime {
        SecurityKind::Openpgp
    } else {
        SecurityKind::Smime
    })
}

fn kind_name(kind: SecurityKind) -> &'static str {
    match kind {
        SecurityKind::Smime => "S/MIME certificate",
        SecurityKind::Openpgp => "OpenPGP key",
    }
}

/// S/MIME certificates mail can be encrypted to for an address.
fn smime_recipient(state: &AppState, address: &str) -> AppResult<Option<Vec<u8>>> {
    let now = now_i64();
    Ok(state
        .store
        .keys_for_address(KeyKind::Smime, address)?
        .into_iter()
        .find(|k| k.can_encrypt && k.expires.is_none_or(|end| end > now))
        .map(|k| k.public_data))
}

/// An OpenPGP key mail can be encrypted to for an address: known here, or
/// (with `lookup`, when online) found by WKD and kept.
async fn openpgp_recipient(
    state: &AppState,
    ring: &KeyRing,
    address: &str,
    lookup: bool,
) -> AppResult<Option<PublicKey>> {
    let now = now();
    if let Some(key) = ring
        .for_address(address)
        .find(|k| openpgp::can_encrypt_to(k, now))
    {
        return Ok(Some(key.clone()));
    }
    if !lookup || crate::settings::working_offline(state) {
        return Ok(None);
    }
    let found = wkd::lookup(address).await.unwrap_or_default();
    let mut usable = None;
    for key in found {
        if let Err(error) = state
            .store
            .save_key(&openpgp_key(&key, None, KeyOrigin::Wkd, None), now)
        {
            eprintln!("couldn't keep a key found by WKD: {error}");
        }
        if usable.is_none() && openpgp::can_encrypt_to(&key, now) {
            usable = Some(key);
        }
    }
    Ok(usable)
}

fn addresses_of(lists: &[&[Mailbox]]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for mailbox in lists.iter().flat_map(|l| l.iter()) {
        let address = mailbox.address.trim().to_lowercase();
        if !address.is_empty() && !out.contains(&address) {
            out.push(address);
        }
    }
    out
}

/// Sign and/or encrypt a built message as the draft asks (and Settings
/// say), and add the account's `Autocrypt:` header. For a draft being
/// saved, `for_draft`: never signed, and encrypted to the sender only.
///
/// # Errors
/// `invalid` when the account has no key to sign or encrypt with, or a
/// recipient's key isn't known (naming them).
#[allow(
    clippy::too_many_lines,
    reason = "one pass: what to do, the keys for it, then doing it"
)]
pub async fn protect_outgoing(
    state: &AppState,
    account: &Account,
    choice: Choice,
    recipients: (&[Mailbox], &[Mailbox], &[Mailbox]),
    raw: Vec<u8>,
    for_draft: bool,
) -> AppResult<Vec<u8>> {
    let wants_anything = choice.sign.unwrap_or(false) || choice.encrypt.unwrap_or(false);
    let settings = crate::settings::load(state)?;
    if !wants_anything
        && !settings.sign_by_default
        && !settings.encrypt_when_possible
        && state.store.account_keys(&account.id)?.openpgp.is_none()
        && !state.store.list_keys()?.iter().any(|k| k.is_own)
    {
        return Ok(raw);
    }
    let loaded = load(state)?;
    let own = own_keys(state, account, &loaded)?;
    let kind = match choice.kind {
        Some(kind) => kind,
        None => preferred_kind(state, &account.id, Some(&own))?,
    };
    let has_own = match kind {
        SecurityKind::Smime => own.smime.is_some(),
        SecurityKind::Openpgp => own.openpgp.is_some(),
    };
    let sign = !for_draft && choice.sign.unwrap_or(settings.sign_by_default && has_own);
    let (to, cc, bcc) = recipients;
    let addresses = if for_draft {
        Vec::new()
    } else {
        addresses_of(&[to, cc, bcc])
    };
    let mut encrypt = match choice.encrypt {
        Some(encrypt) => encrypt,
        None => !for_draft && settings.encrypt_when_possible && has_own,
    };
    let explicit_encrypt = choice.encrypt == Some(true);
    if (sign || encrypt) && !has_own {
        if loaded.locked.is_some() {
            return Err(AppError::new("keyring", LOCKED));
        }
        return Err(AppError::invalid(format!(
            "This account has no {} to {} with. Add one in Settings › Security.",
            kind_name(kind),
            if sign { "sign" } else { "encrypt" }
        )));
    }
    let mut smime_to: Vec<Vec<u8>> = Vec::new();
    let mut openpgp_to: Vec<PublicKey> = Vec::new();
    if encrypt {
        let mut missing = Vec::new();
        for address in &addresses {
            match kind {
                SecurityKind::Smime => match smime_recipient(state, address)? {
                    Some(cert) => smime_to.push(cert),
                    None => missing.push(address.clone()),
                },
                SecurityKind::Openpgp => {
                    match openpgp_recipient(state, &loaded.ring, address, explicit_encrypt).await? {
                        Some(key) => openpgp_to.push(key),
                        None => missing.push(address.clone()),
                    }
                }
            }
        }
        if !missing.is_empty() {
            if explicit_encrypt {
                return Err(AppError::invalid(format!(
                    "The message can't be encrypted: there is no {} for {}.",
                    kind_name(kind),
                    missing.join(", ")
                )));
            }
            // Only "encrypt when possible": it isn't, so it goes as it is.
            encrypt = false;
        }
        // To yourself as well, so the copy in Sent can be read.
        if let Some(identity) = own.smime {
            smime_to.push(identity.cert_der.clone());
        }
        if let Some((_, public)) = own.openpgp {
            openpgp_to.push(public.clone());
        }
    }
    let signer = match (sign, kind) {
        (false, _) => None,
        (true, SecurityKind::Smime) => own.smime.map(Signer::Smime),
        (true, SecurityKind::Openpgp) => own.openpgp.map(|(secret, _)| Signer::Openpgp(secret)),
    };
    let recipients = match (encrypt, kind) {
        (false, _) => None,
        (true, SecurityKind::Smime) => Some(Recipients::Smime(&smime_to)),
        (true, SecurityKind::Openpgp) => Some(Recipients::Openpgp(&openpgp_to)),
    };
    let mut out = if signer.is_some() || recipients.is_some() {
        sec::protect(
            &raw,
            &Protection {
                sign: signer,
                encrypt: recipients,
                now: now(),
            },
        )?
    } else {
        raw
    };
    if !for_draft
        && let Some((_, public)) = own.openpgp
        && let Some(header) = openpgp::autocrypt_header(&account.email, public)
    {
        out = sec::with_header(&out, &header);
    }
    Ok(out)
}

// ---- Commands ----

/// `SecurityKey` in `docs/ipc.md`.
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the interface's SecurityKey"
)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityKeyDto {
    id: String,
    kind: SecurityKind,
    owner: String,
    addresses: Vec<String>,
    fingerprint: String,
    expires: Option<i64>,
    can_sign: bool,
    can_encrypt: bool,
    is_own: bool,
    source: &'static str,
    trusted: bool,
}

fn dto(key: &StoredKey) -> SecurityKeyDto {
    SecurityKeyDto {
        id: key.id.clone(),
        kind: from_store(key.kind),
        owner: key.owner.clone(),
        addresses: key.addresses.clone(),
        fingerprint: key.fingerprint.clone(),
        expires: key.expires,
        can_sign: key.can_sign && key.is_own,
        can_encrypt: key.can_encrypt,
        is_own: key.is_own,
        source: "mocky",
        trusted: key.trusted,
    }
}

/// A key in a system store, not in Mocky Mail's.
enum SystemKey {
    Windows(Vec<u8>),
    Gpg(Box<PublicKey>),
}

const WINDOWS_PREFIX: &str = "windows:";
const GPG_PREFIX: &str = "gpg:";

fn system_keys(stored: &[StoredKey]) -> Vec<(SecurityKeyDto, SystemKey)> {
    let known: HashSet<String> = stored
        .iter()
        .map(|k| label(k.kind, &k.fingerprint))
        .collect();
    let mut out = Vec::new();
    for cert in sec::system::windows_personal_certificates() {
        let Ok(info) = smime::cert_info(&cert.der) else {
            continue;
        };
        if known.contains(&label(KeyKind::Smime, &info.fingerprint)) {
            continue;
        }
        out.push((
            SecurityKeyDto {
                id: format!("{WINDOWS_PREFIX}{}", info.fingerprint),
                kind: SecurityKind::Smime,
                owner: info.owner.clone(),
                addresses: info.addresses.clone(),
                fingerprint: info.fingerprint.clone(),
                expires: Some(info.not_after),
                // Its private key stays in Windows, which Mocky Mail doesn't use.
                can_sign: false,
                can_encrypt: info.can_encrypt,
                is_own: cert.has_private_key,
                source: "windows",
                trusted: false,
            },
            SystemKey::Windows(cert.der),
        ));
    }
    for key in gpg_keys() {
        let info = openpgp::key_info(&key, None);
        if known.contains(&label(KeyKind::Openpgp, &info.fingerprint)) {
            continue;
        }
        out.push((
            SecurityKeyDto {
                id: format!("{GPG_PREFIX}{}", info.fingerprint),
                kind: SecurityKind::Openpgp,
                owner: info.owner,
                addresses: info.addresses,
                fingerprint: info.fingerprint,
                expires: info.expires,
                can_sign: false,
                can_encrypt: info.can_encrypt,
                is_own: false,
                source: "gpg",
                trusted: false,
            },
            SystemKey::Gpg(Box::new(key)),
        ));
    }
    out
}

fn all_keys(state: &AppState) -> AppResult<Vec<SecurityKeyDto>> {
    let stored = state.store.list_keys()?;
    let mut out: Vec<SecurityKeyDto> = stored.iter().map(dto).collect();
    out.extend(system_keys(&stored).into_iter().map(|(dto, _)| dto));
    Ok(out)
}

/// `list_keys`.
#[tauri::command]
pub fn list_keys(state: State<'_, Arc<AppState>>) -> AppResult<Vec<SecurityKeyDto>> {
    all_keys(&state)
}

/// Keep a key from a system store in Mocky Mail's (to trust it).
fn copy_system_key(state: &AppState, id: &str) -> AppResult<Option<String>> {
    if !(id.starts_with(WINDOWS_PREFIX) || id.starts_with(GPG_PREFIX)) {
        return Ok(None);
    }
    let stored = state.store.list_keys()?;
    let Some((_, key)) = system_keys(&stored)
        .into_iter()
        .find(|(dto, _)| dto.id == id)
    else {
        return Err(AppError::new("not_found", "That key no longer exists."));
    };
    let new = match key {
        SystemKey::Windows(der) => smime_key(&der, Vec::new(), None, KeyOrigin::Import)?,
        SystemKey::Gpg(key) => openpgp_key(&key, None, KeyOrigin::Import, None),
    };
    Ok(Some(state.store.save_key(&new, now())?))
}

/// Make an own key the account's when it has none of that kind, for each
/// account whose address the key has.
fn adopt_for_accounts(
    state: &AppState,
    kind: KeyKind,
    key_id: &str,
    addresses: &[String],
) -> AppResult<()> {
    for account in state.store.list_accounts()? {
        let email = account.email.trim().to_lowercase();
        if !addresses.contains(&email) {
            continue;
        }
        let chosen = state.store.account_keys(&account.id)?;
        let has = match kind {
            KeyKind::Smime => chosen.smime.is_some(),
            KeyKind::Openpgp => chosen.openpgp.is_some(),
        };
        if !has {
            state
                .store
                .set_account_key(&account.id, kind, Some(key_id))?;
        }
    }
    Ok(())
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Add the keys in one picked file.
fn import_bytes(
    state: &AppState,
    file_name: &str,
    bytes: &[u8],
    password: Option<&str>,
) -> AppResult<()> {
    let now = now();
    let ext = extension(file_name);
    let seal = |kind: KeyKind, fingerprint: &str, plain: &[u8]| -> AppResult<Vec<u8>> {
        let vault =
            Vault::open(&*state.secrets, true)?.ok_or_else(|| AppError::new("keyring", LOCKED))?;
        Ok(vault.seal(&label(kind, &fingerprint.to_ascii_uppercase()), plain)?)
    };
    let looks_p12 = matches!(ext.as_str(), "p12" | "pfx");
    let looks_pgp = matches!(ext.as_str(), "asc" | "gpg" | "pgp" | "key")
        || mime::find(bytes, b"-----BEGIN PGP").is_some();
    if looks_p12 {
        let contents = smime::pkcs12::read(bytes, password.unwrap_or_default())?;
        let info = smime::cert_info(&contents.cert)?;
        let sealed = seal(KeyKind::Smime, &info.fingerprint, &contents.key)?;
        let key = smime_key(
            &contents.cert,
            contents.chain,
            Some(sealed),
            KeyOrigin::Import,
        )?;
        let id = state.store.save_key(&key, now)?;
        adopt_for_accounts(state, KeyKind::Smime, &id, &key.addresses)?;
        return Ok(());
    }
    if looks_pgp {
        if openpgp::holds_secret(bytes) {
            let own = openpgp::read_secret_key(bytes, password)?;
            let info = openpgp::key_info(&own.public, Some(openpgp::can_sign(&own.secret)));
            let sealed = seal(KeyKind::Openpgp, &info.fingerprint, &own.secret_bytes()?)?;
            let key = openpgp_key(
                &own.public,
                Some(sealed),
                KeyOrigin::Import,
                Some(openpgp::can_sign(&own.secret)),
            );
            let id = state.store.save_key(&key, now)?;
            adopt_for_accounts(state, KeyKind::Openpgp, &id, &key.addresses)?;
        } else {
            for key in openpgp::read_public_keys(bytes)? {
                state
                    .store
                    .save_key(&openpgp_key(&key, None, KeyOrigin::Import, None), now)?;
            }
        }
        return Ok(());
    }
    // .cer, .crt, .pem, .der: certificates (someone else's, or an authority's).
    for der in smime::read_certificates(bytes)? {
        state
            .store
            .save_key(&smime_key(&der, Vec::new(), None, KeyOrigin::Import)?, now)?;
    }
    Ok(())
}

/// `import_key`: pick a key or certificate file and add what is in it.
/// `password` opens a .p12/.pfx or a protected OpenPGP secret key; it is
/// used once and not kept. `None` when cancelled.
#[tauri::command]
pub async fn import_key(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    password: Option<String>,
) -> AppResult<Option<Vec<SecurityKeyDto>>> {
    let Some(path) = pick_key_file(&app).await? else {
        return Ok(None);
    };
    let meta = tokio::fs::metadata(&path).await?;
    if !meta.is_file() || meta.len() > MAX_KEY_FILE {
        return Err(AppError::invalid(
            "That isn't a key or certificate file (they are small: under 1 MB).",
        ));
    }
    let bytes = tokio::fs::read(&path).await?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    import_bytes(&state, &file_name, &bytes, password.as_deref())?;
    state.security.forget_all();
    Ok(Some(all_keys(&state)?))
}

#[cfg(target_os = "android")]
async fn pick_key_file(_app: &AppHandle) -> AppResult<Option<std::path::PathBuf>> {
    Ok(crate::files_android::pick_files(MAX_KEY_FILE)
        .await?
        .into_iter()
        .next()
        .map(|p| p.path))
}

#[cfg(not(target_os = "android"))]
async fn pick_key_file(app: &AppHandle) -> AppResult<Option<std::path::PathBuf>> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file().add_filter(
        "Keys and certificates",
        &[
            "p12", "pfx", "cer", "crt", "pem", "der", "asc", "gpg", "pgp", "key",
        ],
    );
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    picked
        .map(|file| {
            file.into_path()
                .map_err(|_| AppError::invalid("Pick a file on this device."))
        })
        .transpose()
}

/// `export_public_key`: save a certificate (.cer) or public key (.asc) to
/// share. The place saved to, or `None` when cancelled.
#[tauri::command]
pub async fn export_public_key(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<Option<String>> {
    let stored = state.store.list_keys()?;
    let (kind, owner, bytes) = if let Some(key) = stored.iter().find(|k| k.id == id) {
        let bytes = match key.kind {
            KeyKind::Smime => key.public_data.clone(),
            KeyKind::Openpgp => {
                let public = openpgp::public_from_bytes(&key.public_data)
                    .ok_or_else(|| AppError::invalid("This key can't be exported."))?;
                openpgp::armored_public(&public)?.into_bytes()
            }
        };
        (key.kind, key.owner.clone(), bytes)
    } else {
        match system_keys(&stored)
            .into_iter()
            .find(|(dto, _)| dto.id == id)
        {
            Some((dto, SystemKey::Windows(der))) => (KeyKind::Smime, dto.owner, der),
            Some((dto, SystemKey::Gpg(key))) => (
                KeyKind::Openpgp,
                dto.owner,
                openpgp::armored_public(&key)?.into_bytes(),
            ),
            None => return Err(AppError::new("not_found", "That key no longer exists.")),
        }
    };
    let (extension, mime_type) = match kind {
        KeyKind::Smime => ("cer", "application/pkix-cert"),
        KeyKind::Openpgp => ("asc", "application/pgp-keys"),
    };
    let info = AttachmentInfo {
        file_name: safe_file_name(&format!("{owner}.{extension}")),
        mime_type: mime_type.to_owned(),
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    };
    crate::attachments::save(
        &app,
        &state,
        &format!("key-{}", safe_file_name(&id)),
        &info,
        bytes,
    )
    .await
}

/// `remove_key`.
#[tauri::command]
pub fn remove_key(state: State<'_, Arc<AppState>>, id: String) -> AppResult<Vec<SecurityKeyDto>> {
    if id.starts_with(WINDOWS_PREFIX) {
        return Err(AppError::invalid(
            "This certificate is in Windows' certificate store. Remove it there (certmgr.msc).",
        ));
    }
    if id.starts_with(GPG_PREFIX) {
        return Err(AppError::invalid(
            "This key is in GnuPG's keyring. Remove it there (gpg --delete-key).",
        ));
    }
    state.store.delete_key(&id)?;
    state.security.forget_all();
    all_keys(&state)
}

/// `set_key_trusted`: say a key or certificate really is its owner's (a key
/// from a system store is first kept in Mocky Mail's).
#[tauri::command]
pub fn set_key_trusted(
    state: State<'_, Arc<AppState>>,
    id: String,
    trusted: bool,
) -> AppResult<Vec<SecurityKeyDto>> {
    let id = copy_system_key(&state, &id)?.unwrap_or(id);
    state.store.set_key_trusted(&id, trusted)?;
    state.security.forget_all();
    all_keys(&state)
}

/// `set_account_key`.
#[tauri::command]
pub fn set_account_key(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    kind: SecurityKind,
    key_id: Option<String>,
) -> AppResult<()> {
    if key_id
        .as_deref()
        .is_some_and(|id| id.starts_with(WINDOWS_PREFIX) || id.starts_with(GPG_PREFIX))
    {
        return Err(AppError::invalid(
            "Mocky Mail can only sign with keys in its own store. Export this one with its private key (.p12 or .asc) and import it.",
        ));
    }
    match state
        .store
        .set_account_key(&account_id, to_store(kind), key_id.as_deref())
    {
        Err(mail_store::StoreError::Invalid(_)) => Err(AppError::invalid(format!(
            "Choose one of your own {}s (with its private key).",
            kind_name(kind)
        ))),
        other => Ok(other?),
    }
}

/// `get_account_keys` result.
#[derive(Debug, Serialize)]
pub struct AccountKeysDto {
    smime: Option<String>,
    openpgp: Option<String>,
    preferred: SecurityKind,
}

/// `get_account_keys`.
#[tauri::command]
pub fn get_account_keys(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> AppResult<AccountKeysDto> {
    let chosen = state.store.account_keys(&account_id)?;
    Ok(AccountKeysDto {
        preferred: preferred_kind(&state, &account_id, None)?,
        smime: chosen.smime,
        openpgp: chosen.openpgp,
    })
}

/// `set_preferred_kind`.
#[tauri::command]
pub fn set_preferred_kind(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    kind: SecurityKind,
) -> AppResult<()> {
    Ok(state
        .store
        .set_preferred_kind(&account_id, to_store(kind))?)
}

/// One recipient of `EncryptionCheck`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipientKey {
    address: String,
    can_encrypt: bool,
}

/// `EncryptionCheck` in `docs/ipc.md`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionCheck {
    kind: SecurityKind,
    recipients: Vec<RecipientKey>,
    own_key: bool,
}

/// `check_encryption`: which recipients' keys are known, looking OpenPGP
/// keys up by WKD (and keeping what is found, untrusted).
#[tauri::command]
pub async fn check_encryption(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    addresses: Vec<String>,
    kind: SecurityKind,
) -> AppResult<EncryptionCheck> {
    if addresses.len() > 500 {
        return Err(AppError::invalid("Too many recipients to check at once."));
    }
    let account = state.store.get_account(&account_id)?;
    let loaded = load(&state)?;
    let own = own_keys(&state, &account, &loaded)?;
    let own_key = match kind {
        SecurityKind::Smime => own.smime.is_some(),
        SecurityKind::Openpgp => own.openpgp.is_some(),
    };
    let mut recipients = Vec::new();
    let mut lookups = 0usize;
    for address in addresses {
        let clean = address.trim().to_lowercase();
        let can_encrypt = match kind {
            SecurityKind::Smime => smime_recipient(&state, &clean)?.is_some(),
            SecurityKind::Openpgp => {
                let lookup = lookups < MAX_LOOKUPS;
                if lookup && loaded.ring.for_address(&clean).next().is_none() {
                    lookups = lookups.saturating_add(1);
                }
                openpgp_recipient(&state, &loaded.ring, &clean, lookup)
                    .await?
                    .is_some()
            }
        };
        recipients.push(RecipientKey {
            address,
            can_encrypt,
        });
    }
    Ok(EncryptionCheck {
        kind,
        recipients,
        own_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypted_messages_are_kept_a_few_at_a_time() {
        let state = SecurityState::default();
        for n in 0..20 {
            state.remember(&format!("m{n}"), b"x");
        }
        assert!(state.recall("m0").is_none());
        assert!(state.recall("m19").is_some());
        state.forget_all();
        assert!(state.recall("m19").is_none());
    }

    #[test]
    fn autocrypt_is_found_in_the_header_only() {
        assert!(header_has(
            b"From: a\r\nAutoCrypt: addr=a\r\n\r\nbody",
            b"autocrypt:"
        ));
        assert!(!header_has(
            b"From: a\r\n\r\nAutocrypt: in the body",
            b"autocrypt:"
        ));
    }
}
