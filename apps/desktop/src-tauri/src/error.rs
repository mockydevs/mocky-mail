//! The error shape the interface receives (`AppError` in `docs/ipc.md`).
//! Messages are written for people and never contain secrets.

use serde::Serialize;

/// An error returned from any command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppError {
    /// Category, one of the `AppError.kind` strings.
    pub kind: &'static str,
    /// Text safe to show the user.
    pub message: String,
}

/// Result type for commands.
pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    /// Build an error.
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// The input was not valid.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    /// Something unexpected.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<mail_sync::SyncError> for AppError {
    fn from(error: mail_sync::SyncError) -> Self {
        Self::new(error.kind().as_str(), error.to_string())
    }
}

impl From<mail_store::StoreError> for AppError {
    fn from(error: mail_store::StoreError) -> Self {
        match error {
            mail_store::StoreError::NotFound(what) => {
                Self::new("not_found", format!("That {what} no longer exists."))
            }
            other => {
                eprintln!("database error: {other}");
                Self::new("storage", "Couldn't read or write the local mail database.")
            }
        }
    }
}

impl From<mail_core::message::ParseError> for AppError {
    fn from(error: mail_core::message::ParseError) -> Self {
        Self::invalid(format!("This message couldn't be read: {error}."))
    }
}

impl From<mail_core::compose::AddressListError> for AppError {
    fn from(error: mail_core::compose::AddressListError) -> Self {
        Self::invalid(error.to_string())
    }
}

impl From<mail_core::limits::TooLarge> for AppError {
    fn from(error: mail_core::limits::TooLarge) -> Self {
        Self::new("too_large", error.to_string())
    }
}

impl From<mail_auth::AuthError> for AppError {
    fn from(error: mail_auth::AuthError) -> Self {
        use mail_auth::AuthError as E;
        let kind = match error {
            E::NotConfigured(_) => "config",
            E::Denied | E::Reauthorize => "auth",
            E::Timeout | E::Network => "connect",
            E::Local | E::Protocol => "internal",
        };
        Self::new(kind, error.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::internal(format!("A file operation failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_kinds() {
        assert_eq!(AppError::from(mail_sync::SyncError::Auth).kind, "auth");
        assert_eq!(
            AppError::from(mail_store::StoreError::NotFound("message")).kind,
            "not_found"
        );
        assert_eq!(
            AppError::from(mail_auth::AuthError::Timeout).kind,
            "connect"
        );
        assert_eq!(
            AppError::from(mail_auth::AuthError::NotConfigured(
                mail_auth::Provider::Google
            ))
            .kind,
            "config"
        );
    }

    #[test]
    fn serializes_for_the_interface() {
        let json = serde_json::to_string(&AppError::invalid("Bad")).unwrap_or_default();
        assert_eq!(json, r#"{"kind":"invalid","message":"Bad"}"#);
    }
}
