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
    #[error("couldn't reach {host}:{port}. Check the server name and your connection.")]
    Connect {
        /// Server host.
        host: String,
        /// Server port.
        port: u16,
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

impl SyncError {
    /// The category for the interface.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Connect { .. } | Self::Timeout { .. } | Self::Dropped => ErrorKind::Connect,
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
            Self::Connect { .. } | Self::Timeout { .. } | Self::Dropped | Self::Protocol(_) => true,
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
