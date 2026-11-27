//! `ManageSieve` client (RFC 5804): reading and storing the server-side
//! mail rules (Sieve scripts) that run as mail arrives, used for
//! out-of-office replies and for the rules kept on the server. Port
//! 4190, upgraded with `STARTTLS` (which is required: the password is
//! never sent in the clear), `PLAIN` login.

use std::time::Duration;

use base64::Engine as _;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufStream};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use crate::SyncError;

/// The `ManageSieve` port.
pub const PORT: u16 = 4190;
/// How long a reply, or the whole of connecting and logging in, may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// How long opening the connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
/// Largest reply read (scripts are small).
const MAX_REPLY_BYTES: usize = 1024 * 1024;

/// A script on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInfo {
    /// Its name.
    pub name: String,
    /// The one that runs.
    pub active: bool,
}

/// A logged-in `ManageSieve` connection.
pub struct SieveClient {
    stream: BufStream<TlsStream<TcpStream>>,
    host: String,
    /// `SIEVE` capability: the extensions scripts may use.
    pub extensions: Vec<String>,
}

impl std::fmt::Debug for SieveClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SieveClient")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// One server reply: the lines before the status, and the status.
#[derive(Debug, Default)]
struct Reply {
    /// Data lines, with literals joined in as strings.
    items: Vec<String>,
    ok: bool,
    /// The status line's text (after `OK`/`NO`/`BYE`).
    text: String,
}

fn protocol(detail: impl Into<String>) -> SyncError {
    SyncError::Protocol(detail.into())
}

async fn within<T>(
    host: &str,
    f: impl Future<Output = Result<T, SyncError>>,
) -> Result<T, SyncError> {
    tokio::time::timeout(TIMEOUT, f)
        .await
        .map_err(|_| SyncError::Timeout {
            host: host.to_owned(),
        })?
}

/// Read one reply. Literals (`{12}` then 12 bytes) are read whole, so
/// script text that looks like a status line can't end it early.
async fn read_reply<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufStream<S>,
) -> Result<Reply, SyncError> {
    let mut reply = Reply::default();
    let mut total = 0usize;
    loop {
        let mut line = String::new();
        let n = stream
            .read_line(&mut line)
            .await
            .map_err(|e| protocol(e.to_string()))?;
        if n == 0 {
            return Err(SyncError::Dropped);
        }
        total = total.saturating_add(n);
        if total > MAX_REPLY_BYTES {
            return Err(protocol("the filter server's reply was too large"));
        }
        let line = line.trim_end_matches(['\r', '\n']).to_owned();
        if let Some((ok, rest)) = status_line(&line) {
            reply.ok = ok;
            // The human-readable text may come as a literal (RFC 5804
            // section 1.3), as Dovecot sends a script's errors. Left
            // unread, its bytes would be taken for the next reply.
            let Some((before, size)) = literal_tail(rest) else {
                rest.clone_into(&mut reply.text);
                return Ok(reply);
            };
            let text = read_literal(stream, size).await?;
            // The line the literal sat in still has its line break.
            let mut end = String::new();
            stream
                .read_line(&mut end)
                .await
                .map_err(|e| protocol(e.to_string()))?;
            format!("{before} {}", text.trim())
                .trim()
                .clone_into(&mut reply.text);
            return Ok(reply);
        }
        if let Some(size) = literal_size(&line) {
            let text = read_literal(stream, size).await?;
            total = total.saturating_add(size);
            reply.items.push(text);
            continue;
        }
        if !line.is_empty() {
            reply.items.push(line);
        }
    }
}

/// Read a literal's bytes as text.
async fn read_literal<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufStream<S>,
    size: usize,
) -> Result<String, SyncError> {
    if size > MAX_REPLY_BYTES {
        return Err(protocol("the filter server's reply was too large"));
    }
    let mut data = vec![0u8; size];
    stream
        .read_exact(&mut data)
        .await
        .map_err(|e| protocol(e.to_string()))?;
    Ok(String::from_utf8_lossy(&data).into_owned())
}

/// Whether a line is the status that ends a reply: `OK`, `NO` or `BYE`,
/// alone or followed by a space. Gives success and the text after the word.
fn status_line(line: &str) -> Option<(bool, &str)> {
    [("OK", true), ("NO", false), ("BYE", false)]
        .into_iter()
        .find_map(|(word, ok)| {
            let head = line.get(..word.len())?;
            let rest = line.get(word.len()..)?;
            (head.eq_ignore_ascii_case(word) && (rest.is_empty() || rest.starts_with(' ')))
                .then(|| (ok, rest.trim()))
        })
}

/// A status line's text that ends in a literal marker: what comes before
/// the marker (such as a `(QUOTA)` code) and the literal's size.
fn literal_tail(rest: &str) -> Option<(&str, usize)> {
    let size = literal_size(rest)?;
    let before = rest.get(..rest.rfind('{')?)?.trim_end();
    // A marker counts only as a word of its own; `"see {3}` is just text.
    (before.is_empty() || before.ends_with(')')).then_some((before, size))
}

/// `{123}` or `{123+}` at the end of a line.
fn literal_size(line: &str) -> Option<usize> {
    let open = line.rfind('{')?;
    let inner = line.get(open.checked_add(1)?..)?.strip_suffix('}')?;
    inner.trim_end_matches('+').parse().ok()
}

/// A quoted string for a command.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The first quoted string in a line, unescaped.
fn first_quoted(line: &str) -> Option<String> {
    let mut chars = line.chars();
    chars.by_ref().find(|c| *c == '"')?;
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

fn capabilities(items: &[String]) -> Vec<String> {
    items
        .iter()
        .find(|l| first_quoted(l).is_some_and(|k| k.eq_ignore_ascii_case("SIEVE")))
        .and_then(|l| {
            let rest = l.get(l.find('"')?.checked_add(1)?..)?;
            let rest = rest.get(rest.find('"')?.checked_add(1)?..)?;
            first_quoted(rest)
        })
        .map(|s| s.split_whitespace().map(str::to_ascii_lowercase).collect())
        .unwrap_or_default()
}

fn has_starttls(items: &[String]) -> bool {
    items
        .iter()
        .any(|l| first_quoted(l).is_some_and(|k| k.eq_ignore_ascii_case("STARTTLS")))
}

impl SieveClient {
    /// Connect to `host` on port 4190, require `STARTTLS`, and log in.
    ///
    /// # Errors
    /// [`SyncError::Auth`] for a refused password, [`SyncError::Config`]
    /// when the server won't encrypt, connection and protocol errors.
    pub async fn connect(host: &str, username: &str, password: &str) -> Result<Self, SyncError> {
        within(host, Self::connect_inner(host, username, password)).await
    }

    async fn connect_inner(host: &str, username: &str, password: &str) -> Result<Self, SyncError> {
        // Most servers that don't offer filters have the port firewalled, so
        // the connection just hangs. That is "not offered" (a connect
        // error), found quickly, not a server that is slow to answer.
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, PORT)))
            .await
            .map_err(|_| SyncError::Connect {
                host: host.to_owned(),
                port: PORT,
                reason: "the server did not answer on the filter port".to_owned(),
            })?
            .map_err(|e| SyncError::Connect {
                host: host.to_owned(),
                port: PORT,
                reason: crate::error::connect_reason(&e),
            })?;
        let mut plain = BufStream::new(tcp);
        let greeting = read_reply(&mut plain).await?;
        if !greeting.ok || !has_starttls(&greeting.items) {
            return Err(SyncError::Config(
                "the mail server's filter service won't encrypt the connection, so Mocky Mail won't send your password to it"
                    .to_owned(),
            ));
        }
        plain
            .write_all(b"STARTTLS\r\n")
            .await
            .map_err(|e| protocol(e.to_string()))?;
        plain.flush().await.map_err(|e| protocol(e.to_string()))?;
        if !read_reply(&mut plain).await?.ok {
            return Err(SyncError::Config(
                "the filter service refused to encrypt".to_owned(),
            ));
        }
        let tls = crate::tls::wrap(plain.into_inner(), host).await?;
        let mut stream = BufStream::new(tls);
        // Capabilities again, now over TLS.
        let caps = read_reply(&mut stream).await?;
        let mut client = Self {
            stream,
            host: host.to_owned(),
            extensions: capabilities(&caps.items),
        };
        let token =
            base64::engine::general_purpose::STANDARD.encode(format!("\0{username}\0{password}"));
        let reply = client
            .command(&format!("AUTHENTICATE \"PLAIN\" \"{token}\""))
            .await?;
        if !reply.ok {
            return Err(SyncError::Auth);
        }
        Ok(client)
    }

    async fn command(&mut self, line: &str) -> Result<Reply, SyncError> {
        self.stream
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .map_err(|e| protocol(e.to_string()))?;
        self.stream
            .flush()
            .await
            .map_err(|e| protocol(e.to_string()))?;
        let host = self.host.clone();
        within(&host, read_reply(&mut self.stream)).await
    }

    fn refused(reply: &Reply, what: &str) -> SyncError {
        SyncError::Refused(format!(
            "the filter server refused to {what}: {}",
            reply.text
        ))
    }

    /// The scripts on the server.
    ///
    /// # Errors
    /// Connection and protocol errors.
    pub async fn list(&mut self) -> Result<Vec<ScriptInfo>, SyncError> {
        let reply = self.command("LISTSCRIPTS").await?;
        if !reply.ok {
            return Err(Self::refused(&reply, "list filters"));
        }
        Ok(reply
            .items
            .iter()
            .filter_map(|l| {
                let name = first_quoted(l)?;
                Some(ScriptInfo {
                    active: l.to_ascii_uppercase().trim_end().ends_with("ACTIVE"),
                    name,
                })
            })
            .collect())
    }

    /// A script's text.
    ///
    /// # Errors
    /// Connection and protocol errors; [`SyncError::Refused`] when it
    /// doesn't exist.
    pub async fn get(&mut self, name: &str) -> Result<String, SyncError> {
        let reply = self.command(&format!("GETSCRIPT {}", quoted(name))).await?;
        if !reply.ok {
            return Err(Self::refused(&reply, "read a filter"));
        }
        Ok(reply.items.into_iter().next().unwrap_or_default())
    }

    /// Store a script (replacing one of the same name). The server checks
    /// it first; a script it can't run is refused with its reason.
    ///
    /// # Errors
    /// [`SyncError::Refused`] with the server's reason.
    pub async fn put(&mut self, name: &str, script: &str) -> Result<(), SyncError> {
        let line = format!(
            "PUTSCRIPT {} {{{}+}}\r\n{script}",
            quoted(name),
            script.len()
        );
        let reply = self.command(&line).await?;
        if reply.ok {
            Ok(())
        } else {
            Err(Self::refused(&reply, "save the filter"))
        }
    }

    /// Make a script the one that runs; `""` turns all of them off.
    ///
    /// # Errors
    /// [`SyncError::Refused`] with the server's reason.
    pub async fn set_active(&mut self, name: &str) -> Result<(), SyncError> {
        let reply = self.command(&format!("SETACTIVE {}", quoted(name))).await?;
        if reply.ok {
            Ok(())
        } else {
            Err(Self::refused(&reply, "switch filters"))
        }
    }

    /// Say goodbye (errors ignored: the work is done).
    pub async fn logout(mut self) {
        let _ = self.command("LOGOUT").await;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn reply_of(text: &str) -> Reply {
        let (mut a, b) = tokio::io::duplex(4096);
        a.write_all(text.as_bytes()).await.unwrap();
        drop(a);
        read_reply(&mut BufStream::new(b)).await.unwrap()
    }

    #[tokio::test]
    async fn reads_capabilities_lists_and_literals() {
        let greeting = reply_of(
            "\"IMPLEMENTATION\" \"Dovecot Pigeonhole\"\r\n\"SIEVE\" \"fileinto reject envelope vacation include date relational\"\r\n\"STARTTLS\"\r\nOK \"Dovecot ready.\"\r\n",
        )
        .await;
        assert!(greeting.ok);
        assert!(has_starttls(&greeting.items));
        assert!(capabilities(&greeting.items).contains(&"vacation".to_owned()));

        let list = reply_of("\"sogo\" ACTIVE\r\n\"old \\\"one\\\"\"\r\nOK\r\n").await;
        assert_eq!(list.items.len(), 2);
        assert_eq!(first_quoted(&list.items[1]).as_deref(), Some("old \"one\""));

        // A literal containing a line that looks like a status line.
        let script = "require \"vacation\";\r\nOK this is not the end\r\n";
        let got = reply_of(&format!(
            "{{{}}}\r\n{script}\r\nOK \"Getscript completed.\"\r\n",
            script.len()
        ))
        .await;
        assert!(got.ok);
        assert_eq!(got.items[0], script);

        let no = reply_of("NO (QUOTA) \"Too big\"\r\n").await;
        assert!(!no.ok);
        assert!(no.text.contains("Too big"));
    }

    #[tokio::test]
    async fn a_status_line_may_end_in_a_literal() {
        // Two replies in a row: the literal of the first must not be left
        // behind for the second.
        let reason = "line 3: unknown command 'vacaton'";
        let wire = format!(
            "NO {{{}}}\r\n{reason}\r\n\"sogo\" ACTIVE\r\nOK\r\n",
            reason.len()
        );
        let (mut a, b) = tokio::io::duplex(4096);
        a.write_all(wire.as_bytes()).await.unwrap();
        drop(a);
        let mut stream = BufStream::new(b);
        let no = read_reply(&mut stream).await.unwrap();
        assert!(!no.ok);
        assert_eq!(no.text, reason);
        let next = read_reply(&mut stream).await.unwrap();
        assert!(next.ok);
        assert_eq!(next.items, ["\"sogo\" ACTIVE"]);

        let coded = reply_of("NO (QUOTA/MAXSIZE) {7+}\r\nToo big\r\n").await;
        assert_eq!(coded.text, "(QUOTA/MAXSIZE) Too big");
    }

    #[test]
    fn status_lines_and_their_literals() {
        assert_eq!(status_line("OK"), Some((true, "")));
        assert_eq!(status_line("ok \"Done.\""), Some((true, "\"Done.\"")));
        assert_eq!(status_line("NO {81}"), Some((false, "{81}")));
        assert_eq!(status_line("BYE (TRYLATER)"), Some((false, "(TRYLATER)")));
        assert_eq!(status_line("NOT a status"), None);
        assert_eq!(status_line("\"OK\""), None);
        assert_eq!(status_line(""), None);

        assert_eq!(literal_tail("{81}"), Some(("", 81)));
        assert_eq!(literal_tail("(QUOTA) {7+}"), Some(("(QUOTA)", 7)));
        assert_eq!(literal_tail("\"Too big\""), None);
        assert_eq!(literal_tail("\"see {3}"), None);
        assert_eq!(literal_tail(""), None);
    }

    #[test]
    fn quoting_and_literal_sizes() {
        assert_eq!(quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(literal_size("{42}"), Some(42));
        assert_eq!(literal_size("{7+}"), Some(7));
        assert_eq!(literal_size("\"name\""), None);
    }
}
