//! Errors from talking to mail servers.
//!
//! Messages are written for the person using the app. They never include
//! passwords or tokens.

use thiserror::Error;

/// Category of an error, matching `AppError.kind` in `docs/ipc.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Wrong username or password, or the token was refused.
    Auth,
    /// The server could not be reached.
    Connect,
    /// The secure connection failed (bad certificate, protocol).
    Tls,
    /// The settings are wrong or incomplete.
    Config,
    /// The input is not valid.
    Invalid,
    /// The message is too large.
    TooLarge,
    /// The system keyring failed.
    Keyring,
    /// Anything else the server said.
    Internal,
}

impl ErrorKind {
    /// The `AppError.kind` string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Connect => "connect",
            Self::Tls => "tls",
            Self::Config => "config",
            Self::Invalid => "invalid",
            Self::TooLarge => "too_large",
            Self::Keyring => "keyring",
            Self::Internal => "internal",
        }
    }
}

/// An error from the network layer.
#[derive(Debug, Error)]
pub enum SyncError {
    /// Could not open a connection.
    #[error("couldn't reach {host}:{port}: {reason}.")]
    Connect {
        /// Server host.
        host: String,
        /// Server port.
        port: u16,
        /// Why, in plain words, so a blocked port and a typo look different.
        reason: String,
    },
    /// The connection broke mid-conversation (Wi-Fi lost, laptop slept).
    #[error("the connection to the mail server dropped.")]
    Dropped,
    /// No answer in time.
    #[error("{host} took too long to answer.")]
    Timeout {
        /// Server host.
        host: String,
    },
    /// The connection failed part-way through handing a message over, so
    /// the server may or may not have accepted it.
    #[error("the connection to {host} dropped while the message was being sent.")]
    SendUnconfirmed {
        /// Server host.
        host: String,
    },
    /// The encrypted connection could not be set up.
    #[error("couldn't make a secure connection to {host}: {detail}")]
    Tls {
        /// Server host.
        host: String,
        /// What went wrong, from the TLS library.
        detail: String,
    },
    /// The server refused the credentials.
    #[error(
        "the server didn't accept the login. Check the email and password (some providers need an app password)."
    )]
    Auth,
    /// The server answered with an error that may pass (a dropped
    /// connection, or a refusal it marked as temporary).
    #[error("the mail server reported an error: {0}")]
    Protocol(String),
    /// The server understood the request and said no (IMAP `NO` or `BAD`).
    /// Asking again won't help: the folder is gone, the message was moved
    /// elsewhere, and so on.
    #[error("the mail server refused: {0}")]
    Refused(String),
    /// Bad settings.
    #[error("{0}")]
    Config(String),
    /// Bad input.
    #[error("{0}")]
    Invalid(String),
    /// The system keyring failed.
    #[error("couldn't use the system keyring: {0}")]
    Keyring(String),
    /// Sending failed.
    #[error("{message}")]
    Send {
        /// What the server said.
        message: String,
        /// True when retrying will not help (5xx).
        permanent: bool,
    },
}

/// Plain words for a failed TCP connect. The OS message is kept for the
/// cases we don't recognise, so support can still tell what happened.
#[must_use]
pub fn connect_reason(error: &std::io::Error) -> String {
    use std::io::ErrorKind as Io;
    match error.kind() {
        Io::ConnectionRefused => {
            "the server refused the connection. The port may be wrong, or a firewall is blocking it".to_owned()
        }
        Io::PermissionDenied => {
            "this computer blocked the connection. Antivirus or firewall software may be stopping Mocky Mail; allow it there".to_owned()
        }
        Io::NetworkUnreachable | Io::HostUnreachable | Io::NetworkDown => {
            "there is no network route to the server. Check your internet connection".to_owned()
        }
        Io::ConnectionReset | Io::ConnectionAborted => {
            "the connection was cut off straight away, often by a firewall or antivirus".to_owned()
        }
        _ => {
            let text = error.to_string();
            let lower = text.to_lowercase();
            if lower.contains("lookup") || lower.contains("no such host") || lower.contains("not known") {
                "the server name could not be found. Check the spelling".to_owned()
            } else {
                text
            }
        }
    }
}

/// Most links followed in one error chain (a guard against a cycle).
const MAX_CAUSES: usize = 16;

/// Every error in a chain, outermost first. An `io::Error` that wraps
/// another error leaves it out of `source()`, so it is unwrapped by hand:
/// that is where TLS libraries put a certificate failure.
fn causes<'a>(
    error: &'a (dyn std::error::Error + 'static),
) -> Vec<&'a (dyn std::error::Error + 'static)> {
    let mut out = Vec::new();
    let mut next = Some(error);
    while let Some(current) = next {
        if out.len() >= MAX_CAUSES {
            break;
        }
        out.push(current);
        next = match current
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
        {
            Some(inner) => Some(inner),
            None => current.source(),
        };
    }
    out
}

/// The TLS library's own words when a failure anywhere in the chain is
/// about the secure connection (an untrusted, expired or wrong-name
/// certificate, a failed handshake). `None` for anything else.
pub(crate) fn tls_detail(error: &(dyn std::error::Error + 'static)) -> Option<String> {
    let chain = causes(error);
    if let Some(tls) = chain.iter().find_map(|e| e.downcast_ref::<rustls::Error>()) {
        return Some(tls.to_string());
    }
    // A verifier outside rustls reports in its own type; its text still
    // names the certificate.
    let innermost = chain.last()?.to_string();
    innermost
        .to_lowercase()
        .contains("certificate")
        .then_some(innermost)
}

/// Plain words for why a connection failed, from the innermost cause:
/// HTTP and SMTP libraries wrap the operating system's error in layers
/// whose own text only says "error sending request".
pub(crate) fn cause_reason(error: &(dyn std::error::Error + 'static)) -> String {
    let chain = causes(error);
    if let Some(io) = chain
        .iter()
        .rev()
        .find_map(|e| e.downcast_ref::<std::io::Error>())
    {
        return connect_reason(io);
    }
    connect_reason(&std::io::Error::other(innermost_text(error)))
}

/// The innermost cause's text.
pub(crate) fn innermost_text(error: &(dyn std::error::Error + 'static)) -> String {
    causes(error)
        .last()
        .map_or_else(|| error.to_string(), ToString::to_string)
}

impl SyncError {
    /// The category for the interface.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Connect { .. }
            | Self::Timeout { .. }
            | Self::SendUnconfirmed { .. }
            | Self::Dropped => ErrorKind::Connect,
            Self::Tls { .. } => ErrorKind::Tls,
            Self::Auth => ErrorKind::Auth,
            Self::Config(_) => ErrorKind::Config,
            Self::Invalid(_) => ErrorKind::Invalid,
            Self::Keyring(_) => ErrorKind::Keyring,
            Self::Protocol(_) | Self::Refused(_) | Self::Send { .. } => ErrorKind::Internal,
        }
    }

    /// Whether the operation may succeed if tried again later.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        match self {
            Self::Connect { .. }
            | Self::Timeout { .. }
            | Self::SendUnconfirmed { .. }
            | Self::Dropped
            | Self::Protocol(_) => true,
            Self::Send { permanent, .. } => !*permanent,
            Self::Tls { .. }
            | Self::Refused(_)
            | Self::Auth
            | Self::Config(_)
            | Self::Invalid(_)
            | Self::Keyring(_) => false,
        }
    }
}

/// Whether an IMAP `NO` carries a response code that means "try later"
/// (RFC 5530: busy, locked, a rate limit or a server fault).
#[must_use]
pub fn is_temporary_refusal(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    ["UNAVAILABLE", "INUSE", "LIMIT", "SERVERBUG"]
        .iter()
        .any(|code| upper.contains(code))
}

impl From<async_imap::error::Error> for SyncError {
    fn from(error: async_imap::error::Error) -> Self {
        use async_imap::error::Error as E;
        match error {
            E::Io(_) | E::ConnectionLost => Self::Dropped,
            E::No(text) if is_temporary_refusal(&text) => Self::Protocol(text),
            E::No(text) | E::Bad(text) => Self::Refused(text),
            other => Self::Protocol(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_failures_say_why() {
        use std::io::{Error, ErrorKind as Io};
        let blocked = connect_reason(&Error::from(Io::PermissionDenied));
        assert!(blocked.contains("Antivirus or firewall"), "{blocked}");
        let refused = connect_reason(&Error::from(Io::ConnectionRefused));
        assert!(refused.contains("refused"), "{refused}");
        let unknown = connect_reason(&Error::other("No such host is known. (os error 11001)"));
        assert!(unknown.contains("could not be found"), "{unknown}");
        let other = connect_reason(&Error::other("something odd"));
        assert_eq!(other, "something odd");
        let error = SyncError::Connect {
            host: "mail.example.com".into(),
            port: 993,
            reason: blocked,
        };
        assert!(
            error
                .to_string()
                .starts_with("couldn't reach mail.example.com:993: this computer")
        );
    }

    #[test]
    fn the_real_cause_is_found_under_the_wrappers() {
        use std::io::{Error, ErrorKind as Io};
        // A certificate failure as TLS libraries report it: inside an
        // io::Error, which hides it from `source()`.
        let bad_cert = Error::new(
            Io::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::Expired),
        );
        let wrapped = Error::other(bad_cert);
        let detail = tls_detail(&wrapped).unwrap_or_default();
        assert!(detail.to_lowercase().contains("certificate"), "{detail}");
        assert!(detail.to_lowercase().contains("expired"), "{detail}");

        let refused = Error::other(Error::from(Io::ConnectionRefused));
        assert_eq!(tls_detail(&refused), None);
        assert!(cause_reason(&refused).contains("refused the connection"));
        let lookup = Error::other(Error::other("No such host is known. (os error 11001)"));
        assert!(cause_reason(&lookup).contains("could not be found"));
        assert_eq!(innermost_text(&Error::other(Error::other("deep"))), "deep");
    }

    #[test]
    fn server_refusals_are_final_unless_marked_temporary() {
        use async_imap::error::Error as E;
        let refused = SyncError::from(E::No(
            r#"code: Some(TryCreate), info: Some("Mailbox doesn't exist")"#.into(),
        ));
        assert!(matches!(refused, SyncError::Refused(_)));
        assert!(!refused.is_transient());
        assert!(!SyncError::from(E::Bad("code: None, info: None".into())).is_transient());
        for temporary in [
            r#"code: Some(Unavailable), info: Some("try later")"#,
            r#"code: Some(Other("INUSE", None)), info: None"#,
            r#"code: Some(Other("LIMIT", None)), info: None"#,
        ] {
            assert!(
                SyncError::from(E::No(temporary.into())).is_transient(),
                "{temporary}"
            );
        }
        let dropped = SyncError::from(E::ConnectionLost);
        assert!(dropped.is_transient());
        // Shown as "offline", not as a server error.
        assert_eq!(dropped.kind(), ErrorKind::Connect);
    }

    #[test]
    fn kinds_and_retry_rules() {
        assert_eq!(SyncError::Auth.kind().as_str(), "auth");
        assert!(!SyncError::Auth.is_transient());
        assert!(SyncError::Timeout { host: "h".into() }.is_transient());
        let permanent = SyncError::Send {
            message: "550".into(),
            permanent: true,
        };
        assert!(!permanent.is_transient());
        assert_eq!(permanent.kind(), ErrorKind::Internal);
    }

    #[test]
    fn auth_message_never_mentions_a_password_value() {
        assert!(!SyncError::Auth.to_string().contains("hunter2"));
    }
}
