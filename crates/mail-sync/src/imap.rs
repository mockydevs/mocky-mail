//! IMAP: reading folders and messages, changing flags, moving and
//! appending.
//!
//! Message bodies are always fetched with `BODY.PEEK[]`, so looking at a
//! message never marks it read on the server; only an explicit
//! [`ImapConnection::store_flags`] does.

use std::collections::HashSet;
use std::time::Duration;

use async_imap::types::{Fetch, Flag, NameAttribute};
use async_imap::{Client, Session};
use futures::TryStreamExt;
use mail_core::message::{MessageSummary, parse_summary};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::client::TlsStream;
use tokio_util::either::Either;

use crate::{Credentials, Security, ServerConfig, SyncError, tls};

/// The byte stream under an IMAP session.
pub type ImapStream = Either<TlsStream<TcpStream>, TcpStream>;

/// How long any single network step may take.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(45);
/// How long to wait in IDLE before re-issuing it and re-checking the inbox.
/// RFC 2177 allows up to 29 minutes, but home routers and office firewalls
/// often drop a quiet connection after 5 to 15; re-issuing every few minutes
/// keeps it open and catches mail even if the server's push was lost.
pub const IDLE_TIMEOUT: Duration = Duration::from_mins(4);

/// TCP keepalive: probe a quiet connection after this long, so a link that
/// died (Wi-Fi change, sleep, a router dropping it) is noticed in about a
/// minute and a half instead of hanging until the next timeout.
const KEEPALIVE_IDLE: Duration = Duration::from_mins(1);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Ask the operating system to probe a quiet connection. Best effort: a
/// platform without these options still works, just notices a dead link later.
fn keep_alive(tcp: &TcpStream) {
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL);
    let _ = socket2::SockRef::from(tcp).set_tcp_keepalive(&keepalive);
}

/// Arrival dates further than this in the future of the server's arrival
/// time come from a sender whose clock is wrong.
const FUTURE_DATE_SLACK: i64 = 24 * 60 * 60;
/// The shortest Message-ID looked up on the server (`a@b.c` is five
/// characters). Shorter text would match unrelated messages.
pub const MIN_MESSAGE_ID_LEN: usize = 5;

/// The date a message is listed under: its Date header, unless that is
/// missing or in the future of when the server received it (a sender with a
/// wrong clock), in which case the server's arrival time. Otherwise a new
/// message could sort far from the top and look as if it never arrived.
#[must_use]
pub fn listed_date(header: Option<i64>, arrived: Option<i64>) -> Option<i64> {
    match (header, arrived) {
        (Some(date), Some(arrived)) if date > arrived.saturating_add(FUTURE_DATE_SLACK) => {
            Some(arrived)
        }
        (Some(date), _) => Some(date),
        (None, arrived) => arrived,
    }
}

/// What is shown for a message whose headers can't be read, so it still
/// appears (and can be opened) instead of silently never arriving.
#[must_use]
pub fn unreadable_summary(arrived: Option<i64>) -> MessageSummary {
    MessageSummary {
        message_id: None,
        subject: "(This message's details could not be read)".to_owned(),
        from: None,
        reply_to: Vec::new(),
        to: Vec::new(),
        cc: Vec::new(),
        date: arrived,
        preview: String::new(),
        has_html: false,
        attachments: Vec::new(),
        in_reply_to: None,
        references: Vec::new(),
        bulk: false,
        importance: mail_core::importance::Importance::Normal,
    }
}
/// Messages larger than this are not downloaded in the background.
pub const MAX_BACKGROUND_BODY_BYTES: u32 = 2 * 1024 * 1024;

/// The slowest link we plan for when a whole message crosses the network
/// (about 0.5 Mbit/s, a weak mobile signal).
const SLOW_LINK_BYTES_PER_SEC: u64 = 64 * 1024;

/// Time allowed for a step that moves `bytes` of mail: the usual limit plus
/// enough for a slow link, so a large email can still open or be filed.
#[must_use]
pub fn transfer_timeout(bytes: u64) -> Duration {
    NETWORK_TIMEOUT.saturating_add(Duration::from_secs(bytes / SLOW_LINK_BYTES_PER_SEC))
}

async fn within<T>(host: &str, future: impl Future<Output = T>) -> Result<T, SyncError> {
    timeout(NETWORK_TIMEOUT, future)
        .await
        .map_err(|_| SyncError::Timeout {
            host: host.to_owned(),
        })
}

async fn within_for<T>(
    host: &str,
    limit: Duration,
    future: impl Future<Output = T>,
) -> Result<T, SyncError> {
    timeout(limit, future)
        .await
        .map_err(|_| SyncError::Timeout {
            host: host.to_owned(),
        })
}

/// Why a wait in IDLE ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleEnd {
    /// The server reported a change in the folder.
    Changed,
    /// Nothing happened within `IDLE_TIMEOUT`.
    Quiet,
    /// The caller's stop future completed.
    Stopped,
}

/// How an IDLE wait ended: a stop the caller asked for wins over whatever
/// the server said meanwhile, since the caller then syncs anyway.
fn idle_end(stopped: bool, response: &async_imap::extensions::idle::IdleResponse) -> IdleEnd {
    use async_imap::extensions::idle::IdleResponse;
    if stopped {
        return IdleEnd::Stopped;
    }
    match response {
        IdleResponse::NewData(_) => IdleEnd::Changed,
        IdleResponse::Timeout | IdleResponse::ManualInterrupt => IdleEnd::Quiet,
    }
}

/// A folder as the server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolder {
    /// Server name (modified UTF-7), used in commands.
    pub path: String,
    /// Hierarchy separator.
    pub delimiter: Option<String>,
    /// LIST attributes such as `\Sent`.
    pub attributes: Vec<String>,
    /// False for container-only folders (`\Noselect`).
    pub selectable: bool,
}

/// Counters from SELECT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderStatus {
    /// Changes when the server renumbers the folder.
    pub uid_validity: u32,
    /// The UID the next new message will get.
    pub uid_next: Option<u32>,
    /// Number of messages.
    pub exists: u32,
    /// The folder keeps keywords of the app's own (PERMANENTFLAGS has
    /// `\*`), so categories and junk marks can be stored on the server.
    pub accepts_keywords: bool,
}

/// Keywords and (on Gmail) labels of a message: where categories are kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tags {
    /// IMAP keywords (flags without a backslash), as the server sent them.
    pub keywords: Vec<String>,
    /// Gmail labels (`X-GM-LABELS`), modified UTF-7 as sent; `None` on
    /// other servers.
    pub labels: Option<Vec<String>>,
}

impl Tags {
    fn from_fetch(fetch: &Fetch) -> Self {
        Self {
            keywords: fetch
                .flags()
                .filter_map(|flag| match flag {
                    Flag::Custom(keyword) if !keyword.starts_with('\\') => {
                        Some(keyword.into_owned())
                    }
                    _ => None,
                })
                .collect(),
            labels: fetch
                .gmail_labels()
                .map(|labels| labels.iter().map(|label| unescape_quoted(label)).collect()),
        }
    }
}

/// A label as Gmail meant it. The parser hands over a quoted label's
/// contents without undoing its escapes (`"\\Inbox"`, `"a \"b\""`); a
/// backslash before anything else is part of an unquoted flag (`\Inbox`).
fn unescape_quoted(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
            && matches!(next, '\\' | '"')
        {
            out.push(next);
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// An IMAP keyword safe to send: a non-empty atom (RFC 3501 `flag-keyword`).
fn check_keyword(keyword: &str) -> Result<(), SyncError> {
    let ok = !keyword.is_empty()
        && keyword.bytes().all(|b| {
            b.is_ascii_graphic()
                && !matches!(b, b'(' | b')' | b'{' | b'%' | b'*' | b'"' | b'\\' | b']')
        });
    if ok {
        Ok(())
    } else {
        Err(SyncError::Invalid(format!(
            "\"{keyword}\" can't be an IMAP keyword"
        )))
    }
}

/// `+FLAGS.SILENT (a b)` style commands for adding and removing keywords.
fn keyword_commands(add: &[String], remove: &[String]) -> Result<Vec<String>, SyncError> {
    let mut commands = Vec::new();
    for (sign, list) in [('+', add), ('-', remove)] {
        if list.is_empty() {
            continue;
        }
        for keyword in list {
            check_keyword(keyword)?;
        }
        commands.push(format!("{sign}FLAGS.SILENT ({})", list.join(" ")));
    }
    Ok(commands)
}

/// `+X-GM-LABELS.SILENT ("a" "b")` style commands for Gmail labels, which
/// are given already encoded (modified UTF-7).
fn label_commands(add: &[String], remove: &[String]) -> Result<Vec<String>, SyncError> {
    let mut commands = Vec::new();
    for (sign, list) in [('+', add), ('-', remove)] {
        if list.is_empty() {
            continue;
        }
        let quoted = list
            .iter()
            .map(|label| quote_search(label))
            .collect::<Result<Vec<_>, _>>()?;
        commands.push(format!("{sign}X-GM-LABELS.SILENT ({})", quoted.join(" ")));
    }
    Ok(commands)
}

/// Standard flags we track.
#[allow(
    clippy::struct_excessive_bools,
    reason = "one field per IMAP system flag"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    /// `\Seen`.
    pub seen: bool,
    /// `\Flagged`.
    pub flagged: bool,
    /// `\Answered`.
    pub answered: bool,
    /// `\Draft`.
    pub draft: bool,
    /// `\Deleted`.
    pub deleted: bool,
}

impl Flags {
    fn from_fetch(fetch: &Fetch) -> Self {
        let mut flags = Self::default();
        for flag in fetch.flags() {
            match flag {
                Flag::Seen => flags.seen = true,
                Flag::Flagged => flags.flagged = true,
                Flag::Answered => flags.answered = true,
                Flag::Draft => flags.draft = true,
                Flag::Deleted => flags.deleted = true,
                _ => {}
            }
        }
        flags
    }
}

/// A message's headers and state as fetched during sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMessage {
    /// UID within the folder.
    pub uid: u32,
    /// Flags.
    pub flags: Flags,
    /// Size of the full message in bytes.
    pub size: u32,
    /// Parsed headers (preview is empty until the body is fetched).
    pub summary: MessageSummary,
    /// Keywords and Gmail labels.
    pub tags: Tags,
}

/// A flag change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagChange {
    /// Mark read or unread.
    Seen(bool),
    /// Flag or unflag.
    Flagged(bool),
    /// Mark answered.
    Answered(bool),
}

impl FlagChange {
    fn command(self) -> &'static str {
        match self {
            Self::Seen(true) => "+FLAGS.SILENT (\\Seen)",
            Self::Seen(false) => "-FLAGS.SILENT (\\Seen)",
            Self::Flagged(true) => "+FLAGS.SILENT (\\Flagged)",
            Self::Flagged(false) => "-FLAGS.SILENT (\\Flagged)",
            Self::Answered(true) => "+FLAGS.SILENT (\\Answered)",
            Self::Answered(false) => "-FLAGS.SILENT (\\Answered)",
        }
    }
}

/// Turn UIDs into a compact IMAP sequence set such as `1:3,7,9:10`.
/// Returns `None` for an empty list, because an empty set is a syntax error.
#[must_use]
pub fn uid_set(uids: &[u32]) -> Option<String> {
    let mut sorted: Vec<u32> = uids.iter().copied().filter(|&u| u > 0).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut iter = sorted.into_iter();
    let first = iter.next()?;
    let mut parts = Vec::new();
    let (mut start, mut end) = (first, first);
    for uid in iter {
        if end.checked_add(1) == Some(uid) {
            end = uid;
        } else {
            parts.push(range(start, end));
            (start, end) = (uid, uid);
        }
    }
    parts.push(range(start, end));
    Some(parts.join(","))
}

/// The messages marked `\Deleted` that are not ours to remove: every UID in
/// `marked` that is not in `ours`, in order.
fn others_marked_deleted(marked: &HashSet<u32>, ours: &[u32]) -> Vec<u32> {
    let mut others: Vec<u32> = marked
        .iter()
        .copied()
        .filter(|uid| !ours.contains(uid))
        .collect();
    others.sort_unstable();
    others
}

fn range(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}:{end}")
    }
}

/// Quote a string for an IMAP SEARCH argument. Refuses line breaks, which
/// could otherwise inject a second command.
fn quote_search(value: &str) -> Result<String, SyncError> {
    if value.contains(['\r', '\n']) {
        return Err(SyncError::Invalid(
            "search text can't contain line breaks".to_owned(),
        ));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

struct XOAuth2<'a> {
    username: &'a str,
    token: &'a str,
    sent: bool,
}

impl async_imap::Authenticator for XOAuth2<'_> {
    type Response = String;
    fn process(&mut self, challenge: &[u8]) -> Self::Response {
        if self.sent {
            // A second challenge means the server refused the token and sent
            // its reason (JSON: status and scope, no secrets). RFC 7628 and
            // Gmail expect an empty reply, after which the server answers NO.
            // Sending the token again would leave both sides waiting.
            eprintln!(
                "XOAUTH2 refused by the server: {}",
                String::from_utf8_lossy(challenge)
            );
            return String::new();
        }
        self.sent = true;
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.username, self.token
        )
    }
}

fn name_attribute(attribute: &NameAttribute<'_>) -> String {
    match attribute {
        NameAttribute::NoInferiors => "\\Noinferiors".to_owned(),
        NameAttribute::NoSelect => "\\Noselect".to_owned(),
        NameAttribute::Marked => "\\Marked".to_owned(),
        NameAttribute::Unmarked => "\\Unmarked".to_owned(),
        NameAttribute::All => "\\All".to_owned(),
        NameAttribute::Archive => "\\Archive".to_owned(),
        NameAttribute::Drafts => "\\Drafts".to_owned(),
        NameAttribute::Flagged => "\\Flagged".to_owned(),
        NameAttribute::Junk => "\\Junk".to_owned(),
        NameAttribute::Sent => "\\Sent".to_owned(),
        NameAttribute::Trash => "\\Trash".to_owned(),
        NameAttribute::Extension(other) => other.to_string(),
        _ => String::new(),
    }
}

/// A logged-in IMAP connection.
pub struct ImapConnection {
    session: Session<ImapStream>,
    host: String,
    capabilities: HashSet<String>,
    selected: Option<String>,
    /// Whether the selected folder keeps the app's own keywords.
    selected_accepts_keywords: bool,
}

impl std::fmt::Debug for ImapConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapConnection")
            .field("host", &self.host)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

async fn open_stream(server: &ServerConfig) -> Result<ImapStream, SyncError> {
    server.validate()?;
    let host = server.host.as_str();
    let tcp = within(host, TcpStream::connect((host, server.port)))
        .await?
        .map_err(|e| SyncError::Connect {
            host: host.to_owned(),
            port: server.port,
            reason: crate::error::connect_reason(&e),
        })?;
    keep_alive(&tcp);
    match server.security {
        Security::Tls => Ok(Either::Left(within(host, tls::wrap(tcp, host)).await??)),
        Security::Starttls => {
            let mut client = Client::new(tcp);
            within(host, client.read_response())
                .await?
                .map_err(|_| SyncError::Connect {
                    host: host.to_owned(),
                    port: server.port,
                    reason: "the server did not send a greeting".to_owned(),
                })?;
            within(host, client.run_command_and_check_ok("STARTTLS", None))
                .await?
                .map_err(|_| SyncError::Tls {
                    host: host.to_owned(),
                    detail: "the server refused STARTTLS".to_owned(),
                })?;
            let tcp = client.into_inner();
            Ok(Either::Left(within(host, tls::wrap(tcp, host)).await??))
        }
        #[cfg(feature = "insecure-plaintext")]
        Security::Plain => Ok(Either::Right(tcp)),
    }
}

impl ImapConnection {
    /// Connect and log in.
    ///
    /// # Errors
    /// [`SyncError::Auth`] for refused credentials; connect, TLS or timeout
    /// errors otherwise.
    pub async fn connect(
        server: &ServerConfig,
        credentials: &Credentials,
    ) -> Result<Self, SyncError> {
        let host = server.host.clone();
        let client = Client::new(open_stream(server).await?);
        let login = async {
            match credentials {
                Credentials::Password { username, password } => {
                    client.login(username, password).await.map_err(|(e, _)| e)
                }
                Credentials::OAuth2 {
                    username,
                    access_token,
                } => client
                    .authenticate(
                        "XOAUTH2",
                        XOAuth2 {
                            username,
                            token: access_token,
                            sent: false,
                        },
                    )
                    .await
                    .map_err(|(e, _)| e),
            }
        };
        let mut session = within(&host, login).await?.map_err(|error| match error {
            // "Try later" (server busy) is not a wrong password.
            async_imap::error::Error::No(text) if crate::error::is_temporary_refusal(&text) => {
                SyncError::Protocol(text)
            }
            async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => SyncError::Auth,
            other => SyncError::from(other),
        })?;
        let capabilities = within(&host, session.capabilities())
            .await??
            .iter()
            .map(|c| format!("{c:?}").to_ascii_uppercase())
            .collect();
        Ok(Self {
            session,
            host,
            capabilities,
            selected: None,
            selected_accepts_keywords: false,
        })
    }

    fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c.contains(capability))
    }

    /// Whether the server supports IDLE push.
    #[must_use]
    pub fn supports_idle(&self) -> bool {
        self.has("IDLE")
    }

    /// List every folder.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn list_folders(&mut self) -> Result<Vec<RemoteFolder>, SyncError> {
        let host = self.host.clone();
        let names: Vec<_> = within(&host, async {
            self.session
                .list(Some(""), Some("*"))
                .await?
                .try_collect::<Vec<_>>()
                .await
        })
        .await??;
        Ok(names
            .iter()
            .map(|name| {
                let attributes: Vec<String> = name
                    .attributes()
                    .iter()
                    .map(name_attribute)
                    .filter(|a| !a.is_empty())
                    .collect();
                RemoteFolder {
                    path: name.name().to_owned(),
                    delimiter: name.delimiter().map(str::to_owned),
                    selectable: !attributes
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case("\\Noselect")),
                    attributes,
                }
            })
            .collect())
    }

    /// Select a folder for reading and writing.
    ///
    /// # Errors
    /// Network or server errors, or a server that reports no UIDVALIDITY.
    pub async fn select(&mut self, path: &str) -> Result<FolderStatus, SyncError> {
        let host = self.host.clone();
        // A SELECT that fails leaves no folder selected on the server (RFC
        // 3501 section 6.3.1), so the old one is forgotten first. Otherwise
        // the next command for it would skip SELECT and be refused.
        self.selected = None;
        let mailbox = within(&host, self.session.select(path)).await??;
        self.selected = Some(path.to_owned());
        let accepts_keywords = mailbox.permanent_flags.contains(&Flag::MayCreate);
        self.selected_accepts_keywords = accepts_keywords;
        Ok(FolderStatus {
            uid_validity: mailbox.uid_validity.ok_or_else(|| {
                SyncError::Protocol("the server didn't report UIDVALIDITY".to_owned())
            })?,
            uid_next: mailbox.uid_next,
            exists: mailbox.exists,
            accepts_keywords,
        })
    }

    /// Whether this is Gmail (its IMAP extensions, labels among them).
    #[must_use]
    pub fn is_gmail(&self) -> bool {
        self.has("X-GM-EXT-1")
    }

    async fn ensure_selected(&mut self, path: &str) -> Result<(), SyncError> {
        if self.selected.as_deref() != Some(path) {
            self.select(path).await?;
        }
        Ok(())
    }

    async fn fetch(&mut self, path: &str, set: &str, query: &str) -> Result<Vec<Fetch>, SyncError> {
        self.fetch_within(path, set, query, NETWORK_TIMEOUT).await
    }

    async fn fetch_within(
        &mut self,
        path: &str,
        set: &str,
        query: &str,
        limit: Duration,
    ) -> Result<Vec<Fetch>, SyncError> {
        self.ensure_selected(path).await?;
        let host = self.host.clone();
        within_for(&host, limit, async {
            self.session
                .uid_fetch(set, query)
                .await?
                .try_collect::<Vec<_>>()
                .await
        })
        .await?
        .map_err(SyncError::from)
    }

    /// Headers, flags and sizes for the given UIDs (or a range like `500:*`).
    ///
    /// # Errors
    /// Network or server errors. Unparseable messages are skipped.
    pub async fn fetch_headers(
        &mut self,
        path: &str,
        set: &str,
    ) -> Result<Vec<RemoteMessage>, SyncError> {
        let query = if self.is_gmail() {
            "(UID FLAGS RFC822.SIZE INTERNALDATE X-GM-LABELS BODY.PEEK[HEADER])"
        } else {
            "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER])"
        };
        let fetches = self.fetch(path, set, query).await?;
        Ok(fetches
            .iter()
            .filter_map(|fetch| {
                let uid = fetch.uid?;
                let arrived = fetch.internal_date().map(|date| date.timestamp());
                // A reply without the header is the server reporting a flag
                // change on some other message while this command ran. It
                // must not replace that message's details with a placeholder.
                let header = fetch.header()?;
                // Never drop a message: one we can't parse is still listed.
                let mut summary =
                    parse_summary(header).unwrap_or_else(|_| unreadable_summary(arrived));
                summary.date = listed_date(summary.date, arrived);
                Some(RemoteMessage {
                    uid,
                    flags: Flags::from_fetch(fetch),
                    size: fetch.size.unwrap_or(0),
                    summary,
                    tags: Tags::from_fetch(fetch),
                })
            })
            .collect())
    }

    /// Whether each message is bulk or automated mail (Focused Inbox), from
    /// its headers. Used once for mail synced before the app recorded it.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn fetch_bulk_marks(
        &mut self,
        path: &str,
        set: &str,
    ) -> Result<Vec<(u32, bool)>, SyncError> {
        let fetches = self.fetch(path, set, "(UID BODY.PEEK[HEADER])").await?;
        Ok(fetches
            .iter()
            .filter_map(|f| {
                let bulk = f
                    .header()
                    .and_then(|h| parse_summary(h).ok())
                    .is_some_and(|s| s.bulk);
                Some((f.uid?, bulk))
            })
            .collect())
    }

    /// Current flags for the given UIDs.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn fetch_flags(
        &mut self,
        path: &str,
        set: &str,
    ) -> Result<Vec<(u32, Flags)>, SyncError> {
        let fetches = self.fetch(path, set, "(UID FLAGS)").await?;
        Ok(fetches
            .iter()
            .filter_map(|f| Some((f.uid?, Flags::from_fetch(f))))
            .collect())
    }

    /// Current flags with keywords and (on Gmail) labels, for the given UIDs.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn fetch_flags_and_tags(
        &mut self,
        path: &str,
        set: &str,
    ) -> Result<Vec<(u32, Flags, Tags)>, SyncError> {
        let query = if self.is_gmail() {
            "(UID FLAGS X-GM-LABELS)"
        } else {
            "(UID FLAGS)"
        };
        let fetches = self.fetch(path, set, query).await?;
        Ok(fetches
            .iter()
            .filter_map(|f| Some((f.uid?, Flags::from_fetch(f), Tags::from_fetch(f))))
            .collect())
    }

    /// UIDs of the messages that arrived on or after `since` (an IMAP date,
    /// see `mail_core::sync_window::imap_since`).
    ///
    /// # Errors
    /// Network or server errors, or a malformed date.
    pub async fn uids_since(&mut self, path: &str, since: &str) -> Result<HashSet<u32>, SyncError> {
        if since.is_empty() || !since.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(SyncError::Invalid(format!(
                "\"{since}\" is not an IMAP date"
            )));
        }
        self.ensure_selected(path).await?;
        let host = self.host.clone();
        within(&host, self.session.uid_search(format!("SINCE {since}")))
            .await?
            .map_err(SyncError::from)
    }

    /// Add and remove keywords (`$Junk`, category keywords). Returns
    /// `false`, changing nothing, when the folder doesn't keep keywords of
    /// the app's own.
    ///
    /// # Errors
    /// A keyword that isn't an IMAP atom; network or server errors.
    pub async fn store_keywords(
        &mut self,
        path: &str,
        uids: &[u32],
        add: &[String],
        remove: &[String],
    ) -> Result<bool, SyncError> {
        let commands = keyword_commands(add, remove)?;
        let Some(set) = uid_set(uids) else {
            return Ok(true);
        };
        self.ensure_selected(path).await?;
        if !self.selected_accepts_keywords {
            return Ok(false);
        }
        for command in commands {
            self.uid_store(&set, &command).await?;
        }
        Ok(true)
    }

    /// Add and remove Gmail labels (encoded in modified UTF-7). Gmail
    /// creates a label the first time it is used.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn store_labels(
        &mut self,
        path: &str,
        uids: &[u32],
        add: &[String],
        remove: &[String],
    ) -> Result<(), SyncError> {
        let commands = label_commands(add, remove)?;
        let Some(set) = uid_set(uids) else {
            return Ok(());
        };
        self.ensure_selected(path).await?;
        for command in commands {
            self.uid_store(&set, &command).await?;
        }
        Ok(())
    }

    async fn uid_store(&mut self, set: &str, command: &str) -> Result<(), SyncError> {
        let host = self.host.clone();
        within(&host, async {
            self.session
                .uid_store(set, command)
                .await?
                .try_collect::<Vec<_>>()
                .await
        })
        .await??;
        Ok(())
    }

    /// Copy messages to another folder; the originals stay.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn copy_messages(
        &mut self,
        from: &str,
        uids: &[u32],
        to: &str,
    ) -> Result<(), SyncError> {
        let Some(set) = uid_set(uids) else {
            return Ok(());
        };
        self.ensure_selected(from).await?;
        let host = self.host.clone();
        within(&host, self.session.uid_copy(&set, to)).await??;
        Ok(())
    }

    /// The full raw message, without marking it read. `size` is its size as
    /// last reported by the server; it sets how long the download may take.
    ///
    /// # Errors
    /// Network or server errors. `Ok(None)` when the message is gone.
    pub async fn fetch_raw(
        &mut self,
        path: &str,
        uid: u32,
        size: u64,
    ) -> Result<Option<Vec<u8>>, SyncError> {
        let fetches = self
            .fetch_within(
                path,
                &uid.to_string(),
                "(UID BODY.PEEK[])",
                transfer_timeout(size),
            )
            .await?;
        // A flags-only reply for the same message may come first; the
        // body is in whichever reply carries one.
        Ok(fetches
            .iter()
            .filter(|f| f.uid == Some(uid))
            .find_map(|f| f.body().map(<[u8]>::to_vec)))
    }

    /// Every UID in the folder (to notice messages deleted elsewhere).
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn all_uids(&mut self, path: &str) -> Result<HashSet<u32>, SyncError> {
        self.ensure_selected(path).await?;
        let host = self.host.clone();
        within(&host, self.session.uid_search("ALL"))
            .await?
            .map_err(SyncError::from)
    }

    /// UIDs of messages in the folder with this Message-ID (usually one).
    /// See [`MIN_MESSAGE_ID_LEN`] for IDs that are refused.
    ///
    /// # Errors
    /// Invalid IDs, network or server errors.
    pub async fn uids_with_message_id(
        &mut self,
        path: &str,
        message_id: &str,
    ) -> Result<Vec<u32>, SyncError> {
        // The server matches any Message-ID that contains the text, and an
        // empty text matches every message. Callers move and delete what
        // this finds, so an ID too short to be one is refused.
        if message_id.trim().len() < MIN_MESSAGE_ID_LEN {
            return Err(SyncError::Invalid(
                "this message has no usable Message-ID to look it up by.".to_owned(),
            ));
        }
        let query = format!("HEADER Message-ID {}", quote_search(message_id)?);
        self.ensure_selected(path).await?;
        let host = self.host.clone();
        let mut uids: Vec<u32> = within(&host, self.session.uid_search(query))
            .await??
            .into_iter()
            .collect();
        uids.sort_unstable();
        Ok(uids)
    }

    /// Whether a message with this Message-ID is in the folder (checks Sent
    /// before resending after a crash).
    ///
    /// # Errors
    /// Invalid IDs, network or server errors.
    pub async fn contains_message_id(
        &mut self,
        path: &str,
        message_id: &str,
    ) -> Result<bool, SyncError> {
        Ok(!self
            .uids_with_message_id(path, message_id)
            .await?
            .is_empty())
    }

    /// Add or remove a flag on messages.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn store_flags(
        &mut self,
        path: &str,
        uids: &[u32],
        change: FlagChange,
    ) -> Result<(), SyncError> {
        let Some(set) = uid_set(uids) else {
            return Ok(());
        };
        self.ensure_selected(path).await?;
        let host = self.host.clone();
        within(&host, async {
            self.session
                .uid_store(&set, change.command())
                .await?
                .try_collect::<Vec<_>>()
                .await
        })
        .await??;
        Ok(())
    }

    /// Add or remove `\Deleted` on messages in the selected folder.
    async fn mark_deleted(&mut self, set: &str, deleted: bool) -> Result<(), SyncError> {
        let host = self.host.clone();
        let change = if deleted {
            "+FLAGS.SILENT (\\Deleted)"
        } else {
            "-FLAGS.SILENT (\\Deleted)"
        };
        within(&host, async {
            self.session
                .uid_store(set, change)
                .await?
                .try_collect::<Vec<_>>()
                .await
        })
        .await??;
        Ok(())
    }

    /// Remove for good the messages in `uids`, which are already marked
    /// `\Deleted`, from the selected folder.
    async fn expunge_uids(&mut self, uids: &[u32], set: &str) -> Result<(), SyncError> {
        let host = self.host.clone();
        if self.has("UIDPLUS") {
            within(&host, async {
                self.session
                    .uid_expunge(set)
                    .await?
                    .try_collect::<Vec<_>>()
                    .await
            })
            .await??;
            return Ok(());
        }
        // Without UIDPLUS there is only a plain EXPUNGE, which removes every
        // message marked \Deleted: also ones another mail app marked and
        // its user may still want back. Their mark is lifted for the moment
        // of the expunge and put back after.
        let marked = within(&host, self.session.uid_search("DELETED")).await??;
        let others = others_marked_deleted(&marked, uids);
        let spared = uid_set(&others);
        if let Some(spared) = &spared {
            self.mark_deleted(spared, false).await?;
        }
        let expunged = within(&host, async {
            self.session.expunge().await?.try_collect::<Vec<_>>().await
        })
        .await;
        // Put the marks back even if the expunge failed.
        if let Some(spared) = &spared {
            self.mark_deleted(spared, true).await?;
        }
        expunged??;
        Ok(())
    }

    /// Move messages to another folder (MOVE, or COPY + delete).
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn move_messages(
        &mut self,
        from: &str,
        uids: &[u32],
        to: &str,
    ) -> Result<(), SyncError> {
        let Some(set) = uid_set(uids) else {
            return Ok(());
        };
        self.ensure_selected(from).await?;
        let host = self.host.clone();
        if self.has("MOVE") {
            within(&host, self.session.uid_mv(&set, to)).await??;
            return Ok(());
        }
        within(&host, self.session.uid_copy(&set, to)).await??;
        self.mark_deleted(&set, true).await?;
        self.expunge_uids(uids, &set).await
    }

    /// Permanently delete messages.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn delete_forever(&mut self, path: &str, uids: &[u32]) -> Result<(), SyncError> {
        let Some(set) = uid_set(uids) else {
            return Ok(());
        };
        self.ensure_selected(path).await?;
        self.mark_deleted(&set, true).await?;
        self.expunge_uids(uids, &set).await
    }

    /// Create a folder (for example Archive or Sent when the account has none).
    ///
    /// # Errors
    /// Network or server errors, including "already exists".
    pub async fn create_folder(&mut self, path: &str) -> Result<(), SyncError> {
        let host = self.host.clone();
        within(&host, self.session.create(path)).await??;
        Ok(())
    }

    /// Rename a folder (its subfolders move with it).
    ///
    /// # Errors
    /// Network or server errors, including a name already in use.
    pub async fn rename_folder(&mut self, from: &str, to: &str) -> Result<(), SyncError> {
        let host = self.host.clone();
        // The selected folder may be this one or one inside it; its old
        // name must not be taken as still selected.
        self.selected = None;
        within(&host, self.session.rename(from, to)).await??;
        Ok(())
    }

    /// Delete a folder and the messages in it.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn delete_folder(&mut self, path: &str) -> Result<(), SyncError> {
        let host = self.host.clone();
        // A folder made later under the same name is a different folder,
        // so a deleted one must not be taken as still selected.
        self.selected = None;
        within(&host, self.session.delete(path)).await??;
        Ok(())
    }

    /// Store a message in a folder (a copy in Sent, or a draft).
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn append(&mut self, path: &str, raw: &[u8], flags: &str) -> Result<(), SyncError> {
        let host = self.host.clone();
        let limit = transfer_timeout(u64::try_from(raw.len()).unwrap_or(u64::MAX));
        within_for(
            &host,
            limit,
            self.session.append(path, Some(flags), None, raw),
        )
        .await??;
        // APPEND to the selected folder changes its contents; force a re-select.
        self.selected = None;
        Ok(())
    }

    /// Store a message brought in from a file: with its flags (such as
    /// `(\Seen)`) and, when known, its arrival time (`INTERNALDATE`, an IMAP
    /// `date-time` in quotes such as `mail_core::mbox::imap_date` writes).
    ///
    /// # Errors
    /// Network or server errors (a message the server refuses among them).
    pub async fn append_dated(
        &mut self,
        path: &str,
        raw: &[u8],
        flags: &str,
        internal_date: Option<&str>,
    ) -> Result<(), SyncError> {
        if flags.contains(['\r', '\n'])
            || internal_date.is_some_and(|d| d.contains(['\r', '\n']) || !d.starts_with('"'))
        {
            return Err(SyncError::Invalid(
                "a message's flags or date can't be sent like that".to_owned(),
            ));
        }
        let host = self.host.clone();
        let limit = transfer_timeout(u64::try_from(raw.len()).unwrap_or(u64::MAX));
        within_for(
            &host,
            limit,
            self.session.append(path, Some(flags), internal_date, raw),
        )
        .await??;
        self.selected = None;
        Ok(())
    }

    /// The Message-IDs of every message in a folder (without angle
    /// brackets, lower case), for leaving out what is already there.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn message_ids(&mut self, path: &str) -> Result<HashSet<String>, SyncError> {
        let status = self.select(path).await?;
        if status.exists == 0 {
            return Ok(HashSet::new());
        }
        let limit = transfer_timeout(u64::from(status.exists).saturating_mul(1024));
        let fetches = self
            .fetch_within(path, "1:*", "(UID ENVELOPE)", limit)
            .await?;
        Ok(fetches
            .iter()
            .filter_map(|f| {
                let id = f.envelope()?.message_id.as_ref()?;
                let id = String::from_utf8_lossy(id);
                let key = id
                    .trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .trim()
                    .to_ascii_lowercase();
                (!key.is_empty()).then_some(key)
            })
            .collect())
    }

    /// Check that the connection still works (NOOP), for one that sat quiet
    /// for a while before being used again.
    ///
    /// # Errors
    /// Network or server errors; the connection should then be dropped.
    pub async fn noop(&mut self) -> Result<(), SyncError> {
        let host = self.host.clone();
        within(&host, self.session.noop()).await??;
        Ok(())
    }

    /// Wait for the server to report a change in `path` (IDLE), up to
    /// `IDLE_TIMEOUT`. Returns the connection and whether anything changed.
    ///
    /// # Errors
    /// Network or server errors; the connection should then be dropped.
    pub async fn idle(self, path: &str) -> Result<(Self, bool), SyncError> {
        let (conn, end) = self.idle_until(path, std::future::pending()).await?;
        Ok((conn, end == IdleEnd::Changed))
    }

    /// Like [`Self::idle`], but also ends IDLE as soon as `stop` completes
    /// (the app has changes to send, say). IDLE is then ended cleanly with
    /// DONE and the same logged-in connection comes back, ready for more
    /// commands. `stop` is only polled while idling.
    ///
    /// # Errors
    /// Network or server errors; the connection should then be dropped.
    pub async fn idle_until(
        mut self,
        path: &str,
        stop: impl Future<Output = ()>,
    ) -> Result<(Self, IdleEnd), SyncError> {
        self.ensure_selected(path).await?;
        let Self {
            session,
            host,
            capabilities,
            selected,
            selected_accepts_keywords,
        } = self;
        let mut handle = session.idle();
        within(&host, handle.init()).await??;
        let end = {
            let (wait, interrupt) = handle.wait_with_timeout(IDLE_TIMEOUT);
            let mut interrupt = Some(interrupt);
            let mut stopped = false;
            tokio::pin!(wait);
            tokio::pin!(stop);
            loop {
                tokio::select! {
                    response = &mut wait => break idle_end(stopped, &response?),
                    () = &mut stop, if !stopped => {
                        // Dropping the stop source makes the wait return at
                        // once (it checks the stop before reading), so this
                        // loop never waits on the network after a stop.
                        stopped = true;
                        drop(interrupt.take());
                    }
                }
            }
        };
        let session = within(&host, handle.done()).await??;
        Ok((
            Self {
                session,
                host,
                capabilities,
                selected,
                selected_accepts_keywords,
            },
            end,
        ))
    }

    /// Log out politely. Errors are ignored: the connection is closing anyway.
    pub async fn logout(mut self) {
        let host = self.host.clone();
        let _ = within(&host, self.session.logout()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{IDLE_TIMEOUT, IdleEnd, idle_end, listed_date, unreadable_summary};
    use async_imap::extensions::idle::IdleResponse;

    #[test]
    fn a_stopped_idle_says_so_whatever_the_server_sent() {
        // A quiet IDLE that ran its course.
        assert_eq!(idle_end(false, &IdleResponse::Timeout), IdleEnd::Quiet);
        // The app asked to stop: it syncs next, so this is not "nothing new".
        assert_eq!(
            idle_end(true, &IdleResponse::ManualInterrupt),
            IdleEnd::Stopped
        );
        assert_eq!(idle_end(true, &IdleResponse::Timeout), IdleEnd::Stopped);
    }

    #[test]
    fn a_message_is_listed_by_its_date_unless_the_sender_clock_is_wrong() {
        let arrived = 1_800_000_000;
        // Normal: the Date header, even if a little before arrival.
        assert_eq!(
            listed_date(Some(arrived - 120), Some(arrived)),
            Some(arrived - 120)
        );
        // Missing Date: when the server received it.
        assert_eq!(listed_date(None, Some(arrived)), Some(arrived));
        // A sender's clock days ahead would pin it to the top for days.
        assert_eq!(
            listed_date(Some(arrived + 3 * 86_400), Some(arrived)),
            Some(arrived)
        );
        // Old mail moved in later keeps its real date.
        assert_eq!(
            listed_date(Some(arrived - 400 * 86_400), Some(arrived)),
            Some(arrived - 400 * 86_400)
        );
        assert_eq!(listed_date(None, None), None);
    }

    #[test]
    fn a_message_that_cannot_be_read_is_still_listed() {
        let summary = unreadable_summary(Some(1_800_000_000));
        assert!(!summary.subject.is_empty());
        assert_eq!(summary.date, Some(1_800_000_000));
    }

    #[test]
    fn idle_is_renewed_before_routers_drop_a_quiet_connection() {
        assert!(IDLE_TIMEOUT <= std::time::Duration::from_mins(5));
    }

    #[test]
    fn a_refused_oauth_token_gets_an_empty_reply_not_a_hang() {
        use async_imap::Authenticator as _;
        let mut auth = XOAuth2 {
            username: "a@gmail.com",
            token: "t",
            sent: false,
        };
        assert_eq!(
            auth.process(b""),
            "user=a@gmail.com\x01auth=Bearer t\x01\x01"
        );
        assert_eq!(auth.process(br#"{"status":"400"}"#), "");
    }

    use super::*;

    #[test]
    fn uid_sets_are_compact_and_sorted() {
        assert_eq!(uid_set(&[]), None);
        assert_eq!(uid_set(&[0]), None);
        assert_eq!(uid_set(&[5]).as_deref(), Some("5"));
        assert_eq!(
            uid_set(&[3, 1, 2, 7, 9, 10, 10]).as_deref(),
            Some("1:3,7,9:10")
        );
        assert_eq!(
            uid_set(&[u32::MAX, u32::MAX - 1]).as_deref(),
            Some("4294967294:4294967295")
        );
    }

    #[test]
    fn a_plain_expunge_spares_what_other_apps_marked_deleted() {
        let marked: HashSet<u32> = [4, 9, 2, 7].into_iter().collect();
        assert_eq!(others_marked_deleted(&marked, &[7, 2]), [4, 9]);
        assert!(others_marked_deleted(&marked, &[2, 4, 7, 9]).is_empty());
        assert!(others_marked_deleted(&HashSet::new(), &[1]).is_empty());
    }

    #[test]
    fn large_transfers_get_time_for_a_slow_link() {
        assert_eq!(transfer_timeout(0), NETWORK_TIMEOUT);
        assert_eq!(transfer_timeout(10 * 1024), NETWORK_TIMEOUT);
        // 25 MB at 0.5 Mbit/s takes about 400 s; the old flat 45 s cut it off.
        let big = transfer_timeout(25 * 1024 * 1024);
        assert_eq!(big, NETWORK_TIMEOUT + Duration::from_secs(400));
        assert_eq!(
            transfer_timeout(u64::MAX),
            NETWORK_TIMEOUT + Duration::from_secs(u64::MAX / SLOW_LINK_BYTES_PER_SEC)
        );
    }

    #[test]
    fn search_values_are_quoted_and_line_breaks_refused() {
        assert_eq!(quote_search(r#"a"b\c@x"#).unwrap(), r#""a\"b\\c@x""#);
        assert!(quote_search("x\r\nA1 DELETE INBOX").is_err());
    }

    #[test]
    fn quoted_labels_lose_their_escapes() {
        assert_eq!(unescape_quoted(r"\\Inbox"), r"\Inbox");
        assert_eq!(unescape_quoted(r"\Important"), r"\Important");
        assert_eq!(unescape_quoted(r#"Old \"one\""#), r#"Old "one""#);
        assert_eq!(unescape_quoted(r"a\\\\b"), r"a\\b");
        assert_eq!(unescape_quoted("Clients"), "Clients");
    }

    #[test]
    fn keyword_and_label_commands_are_silent_and_safe() {
        let s = |list: &[&str]| -> Vec<String> { list.iter().map(|k| (*k).to_owned()).collect() };
        assert_eq!(
            keyword_commands(&s(&["$Junk", "$Cat_Red"]), &s(&["$NotJunk"])).unwrap(),
            ["+FLAGS.SILENT ($Junk $Cat_Red)", "-FLAGS.SILENT ($NotJunk)"]
        );
        assert!(keyword_commands(&[], &[]).unwrap().is_empty());
        for bad in ["", "a b", "x)", "\\Seen", "a\r\nA1 LOGOUT", "caf\u{e9}"] {
            assert!(keyword_commands(&s(&[bad]), &[]).is_err(), "{bad:?}");
        }
        assert_eq!(
            label_commands(&s(&["Clients", "Entw&APw-rfe"]), &s(&["a\"b"])).unwrap(),
            [
                "+X-GM-LABELS.SILENT (\"Clients\" \"Entw&APw-rfe\")",
                "-X-GM-LABELS.SILENT (\"a\\\"b\")"
            ]
        );
        assert!(label_commands(&s(&["x\r\ny"]), &[]).is_err());
    }

    #[test]
    fn flag_commands_are_silent() {
        assert_eq!(FlagChange::Seen(true).command(), "+FLAGS.SILENT (\\Seen)");
        assert_eq!(
            FlagChange::Flagged(false).command(),
            "-FLAGS.SILENT (\\Flagged)"
        );
    }
}
