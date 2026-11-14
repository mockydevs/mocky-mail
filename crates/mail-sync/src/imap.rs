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
/// How long to wait in IDLE before re-issuing it (RFC 2177 says under 29 min).
pub const IDLE_TIMEOUT: Duration = Duration::from_mins(25);
/// Messages larger than this are not downloaded in the background.
pub const MAX_BACKGROUND_BODY_BYTES: u32 = 2 * 1024 * 1024;

async fn within<T>(host: &str, future: impl Future<Output = T>) -> Result<T, SyncError> {
    timeout(NETWORK_TIMEOUT, future)
        .await
        .map_err(|_| SyncError::Timeout {
            host: host.to_owned(),
        })
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
}

impl async_imap::Authenticator for XOAuth2<'_> {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
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
        .map_err(|_| SyncError::Connect {
            host: host.to_owned(),
            port: server.port,
        })?;
    match server.security {
        Security::Tls => Ok(Either::Left(within(host, tls::wrap(tcp, host)).await??)),
        Security::Starttls => {
            let mut client = Client::new(tcp);
            within(host, client.read_response())
                .await?
                .map_err(|_| SyncError::Connect {
                    host: host.to_owned(),
                    port: server.port,
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
                        },
                    )
                    .await
                    .map_err(|(e, _)| e),
            }
        };
        let mut session = within(&host, login).await?.map_err(|error| match error {
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
        let mailbox = within(&host, self.session.select(path)).await??;
        self.selected = Some(path.to_owned());
        Ok(FolderStatus {
            uid_validity: mailbox.uid_validity.ok_or_else(|| {
                SyncError::Protocol("the server didn't report UIDVALIDITY".to_owned())
            })?,
            uid_next: mailbox.uid_next,
            exists: mailbox.exists,
        })
    }

    async fn ensure_selected(&mut self, path: &str) -> Result<(), SyncError> {
        if self.selected.as_deref() != Some(path) {
            self.select(path).await?;
        }
        Ok(())
    }

    async fn fetch(&mut self, path: &str, set: &str, query: &str) -> Result<Vec<Fetch>, SyncError> {
        self.ensure_selected(path).await?;
        let host = self.host.clone();
        within(&host, async {
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
        let fetches = self
            .fetch(path, set, "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])")
            .await?;
        Ok(fetches
            .iter()
            .filter_map(|fetch| {
                let uid = fetch.uid?;
                let summary = parse_summary(fetch.header()?).ok()?;
                Some(RemoteMessage {
                    uid,
                    flags: Flags::from_fetch(fetch),
                    size: fetch.size.unwrap_or(0),
                    summary,
                })
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

    /// The full raw message, without marking it read.
    ///
    /// # Errors
    /// Network or server errors. `Ok(None)` when the message is gone.
    pub async fn fetch_raw(&mut self, path: &str, uid: u32) -> Result<Option<Vec<u8>>, SyncError> {
        let fetches = self
            .fetch(path, &uid.to_string(), "(UID BODY.PEEK[])")
            .await?;
        Ok(fetches
            .iter()
            .find(|f| f.uid == Some(uid))
            .and_then(|f| f.body().map(<[u8]>::to_vec)))
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
    ///
    /// # Errors
    /// Invalid IDs, network or server errors.
    pub async fn uids_with_message_id(
        &mut self,
        path: &str,
        message_id: &str,
    ) -> Result<Vec<u32>, SyncError> {
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

    async fn expunge_uids(&mut self, set: &str) -> Result<(), SyncError> {
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
        } else {
            within(&host, async {
                self.session.expunge().await?.try_collect::<Vec<_>>().await
            })
            .await??;
        }
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
        within(&host, async {
            self.session
                .uid_store(&set, "+FLAGS.SILENT (\\Deleted)")
                .await?
                .try_collect::<Vec<_>>()
                .await
        })
        .await??;
        self.expunge_uids(&set).await
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
        let host = self.host.clone();
        within(&host, async {
            self.session
                .uid_store(&set, "+FLAGS.SILENT (\\Deleted)")
                .await?
                .try_collect::<Vec<_>>()
                .await
        })
        .await??;
        self.expunge_uids(&set).await
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

    /// Store a message in a folder (a copy in Sent, or a draft).
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn append(&mut self, path: &str, raw: &[u8], flags: &str) -> Result<(), SyncError> {
        let host = self.host.clone();
        within(&host, self.session.append(path, Some(flags), None, raw)).await??;
        // APPEND to the selected folder changes its contents; force a re-select.
        self.selected = None;
        Ok(())
    }

    /// Wait for the server to report a change in `path` (IDLE), up to
    /// `IDLE_TIMEOUT`. Returns the connection and whether anything changed.
    ///
    /// # Errors
    /// Network or server errors; the connection should then be dropped.
    pub async fn idle(mut self, path: &str) -> Result<(Self, bool), SyncError> {
        self.ensure_selected(path).await?;
        let Self {
            session,
            host,
            capabilities,
            selected,
        } = self;
        let mut handle = session.idle();
        within(&host, handle.init()).await??;
        let (wait, _stop) = handle.wait_with_timeout(IDLE_TIMEOUT);
        let changed = matches!(
            wait.await?,
            async_imap::extensions::idle::IdleResponse::NewData(_)
        );
        let session = within(&host, handle.done()).await??;
        Ok((
            Self {
                session,
                host,
                capabilities,
                selected,
            },
            changed,
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
    fn search_values_are_quoted_and_line_breaks_refused() {
        assert_eq!(quote_search(r#"a"b\c@x"#).unwrap(), r#""a\"b\\c@x""#);
        assert!(quote_search("x\r\nA1 DELETE INBOX").is_err());
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
