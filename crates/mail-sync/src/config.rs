//! Server settings and credentials.

use serde::{Deserialize, Serialize};

/// How the connection is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// TLS from the first byte (IMAP 993, SMTP 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (SMTP 587, IMAP 143).
    /// The upgrade is mandatory: if the server refuses it, we disconnect.
    Starttls,
    /// No encryption. Only exists for tests against a local test server.
    #[cfg(feature = "insecure-plaintext")]
    Plain,
}

/// Where a server is and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host name.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Encryption mode.
    pub security: Security,
}

impl ServerConfig {
    /// Reject obviously broken settings before connecting.
    ///
    /// # Errors
    /// A readable message when the host is empty or not a host name.
    pub fn validate(&self) -> Result<(), crate::SyncError> {
        let host_ok = !self.host.is_empty()
            && self.host.len() <= 253
            && self
                .host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
            && !self.host.starts_with(['.', '-']);
        if !host_ok {
            return Err(crate::SyncError::Config(format!(
                "\"{}\" is not a valid server name.",
                self.host
            )));
        }
        if self.port == 0 {
            return Err(crate::SyncError::Config(
                "The port must be between 1 and 65535.".to_owned(),
            ));
        }
        Ok(())
    }
}

/// How an account signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// Username and password (or app password).
    Password,
    /// Google OAuth.
    Google,
    /// Microsoft OAuth.
    Microsoft,
}

/// Login details for one connection.
#[derive(Clone, PartialEq, Eq)]
pub enum Credentials {
    /// Username and password.
    Password {
        /// Login name, usually the email address.
        username: String,
        /// The password.
        password: String,
    },
    /// OAuth 2 access token, sent with SASL XOAUTH2.
    OAuth2 {
        /// Login name, the email address.
        username: String,
        /// A current access token.
        access_token: String,
    },
}

impl Credentials {
    /// The login name.
    #[must_use]
    pub fn username(&self) -> &str {
        match self {
            Self::Password { username, .. } | Self::OAuth2 { username, .. } => username,
        }
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Password { .. } => "Password",
            Self::OAuth2 { .. } => "OAuth2",
        };
        f.debug_struct(kind)
            .field("username", &self.username())
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// Suggested settings for an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerSuggestion {
    /// Incoming mail server.
    pub imap: ServerConfig,
    /// Outgoing mail server.
    pub smtp: ServerConfig,
    /// How the provider expects you to sign in.
    pub auth: AuthKind,
}

fn tls(host: &str, port: u16) -> ServerConfig {
    ServerConfig {
        host: host.to_owned(),
        port,
        security: Security::Tls,
    }
}

fn starttls(host: &str, port: u16) -> ServerConfig {
    ServerConfig {
        host: host.to_owned(),
        port,
        security: Security::Starttls,
    }
}

/// Suggest server settings for an email address: known providers first,
/// then the common `mail.<domain>` convention. Returns `None` for input
/// that is not an address. The user can always edit the result.
#[must_use]
pub fn suggest_servers(email: &str) -> Option<ServerSuggestion> {
    let email = email.trim();
    if !mail_core::compose::is_valid_address(email) {
        return None;
    }
    let domain = email.rsplit_once('@')?.1.to_ascii_lowercase();
    let suggestion = match domain.as_str() {
        "gmail.com" | "googlemail.com" => ServerSuggestion {
            imap: tls("imap.gmail.com", 993),
            smtp: tls("smtp.gmail.com", 465),
            auth: AuthKind::Google,
        },
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => ServerSuggestion {
            imap: tls("outlook.office365.com", 993),
            smtp: starttls("smtp.office365.com", 587),
            auth: AuthKind::Microsoft,
        },
        "yahoo.com" | "ymail.com" => ServerSuggestion {
            imap: tls("imap.mail.yahoo.com", 993),
            smtp: tls("smtp.mail.yahoo.com", 465),
            auth: AuthKind::Password,
        },
        "icloud.com" | "me.com" | "mac.com" => ServerSuggestion {
            imap: tls("imap.mail.me.com", 993),
            smtp: starttls("smtp.mail.me.com", 587),
            auth: AuthKind::Password,
        },
        "zoho.com" => ServerSuggestion {
            imap: tls("imap.zoho.com", 993),
            smtp: tls("smtp.zoho.com", 465),
            auth: AuthKind::Password,
        },
        "fastmail.com" => ServerSuggestion {
            imap: tls("imap.fastmail.com", 993),
            smtp: tls("smtp.fastmail.com", 465),
            auth: AuthKind::Password,
        },
        _ => {
            let host = format!("mail.{domain}");
            ServerSuggestion {
                imap: tls(&host, 993),
                smtp: tls(&host, 465),
                auth: AuthKind::Password,
            }
        }
    };
    Some(suggestion)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers() {
        let g = suggest_servers("Someone@Gmail.com").unwrap();
        assert_eq!(g.imap.host, "imap.gmail.com");
        assert_eq!(g.auth, AuthKind::Google);
        let o = suggest_servers("a@hotmail.com").unwrap();
        assert_eq!(o.smtp.security, Security::Starttls);
        assert_eq!(o.smtp.port, 587);
    }

    #[test]
    fn custom_domains_use_mail_subdomain() {
        let s = suggest_servers("info@mocky.co.ke").unwrap();
        assert_eq!(s.imap, tls("mail.mocky.co.ke", 993));
        assert_eq!(s.smtp, tls("mail.mocky.co.ke", 465));
        assert_eq!(s.auth, AuthKind::Password);
    }

    #[test]
    fn invalid_addresses_get_no_suggestion() {
        assert_eq!(suggest_servers("not an address"), None);
        assert_eq!(suggest_servers(""), None);
    }

    #[test]
    fn validates_hosts_and_ports() {
        assert!(tls("mail.example.com", 993).validate().is_ok());
        assert!(tls("", 993).validate().is_err());
        assert!(tls("bad host", 993).validate().is_err());
        assert!(tls("evil.com/path", 993).validate().is_err());
        assert!(tls("ok.example", 0).validate().is_err());
    }

    #[test]
    fn debug_never_shows_secrets() {
        let c = Credentials::Password {
            username: "me@x.co".into(),
            password: "hunter2".into(),
        };
        let shown = format!("{c:?}");
        assert!(shown.contains("me@x.co"));
        assert!(!shown.contains("hunter2"));
        let t = Credentials::OAuth2 {
            username: "me@x.co".into(),
            access_token: "ya29.secret".into(),
        };
        assert!(!format!("{t:?}").contains("ya29"));
    }
}
