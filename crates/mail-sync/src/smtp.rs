//! SMTP: logging in and handing a finished message to the server.

use std::time::Duration;

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::{Credentials as SmtpCredentials, Mechanism};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::{Credentials, Security, ServerConfig, SyncError};

/// How long one SMTP exchange may take. Large attachments on slow links
/// need room, so this is generous.
pub const SMTP_TIMEOUT: Duration = Duration::from_mins(3);

/// How long checking a login may take when adding an account. A wrong port
/// or security mode usually shows as silence, and the person is watching
/// the form.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);

fn transport(
    server: &ServerConfig,
    credentials: &Credentials,
    limit: Duration,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, SyncError> {
    server.validate()?;
    let tls_error = |e: lettre::transport::smtp::Error| SyncError::Tls {
        host: server.host.clone(),
        detail: e.to_string(),
    };
    let builder = match server.security {
        Security::Tls => {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&server.host).map_err(tls_error)?
        }
        Security::Starttls => {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&server.host).map_err(tls_error)?
        }
        #[cfg(feature = "insecure-plaintext")]
        Security::Plain => {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(server.host.clone())
        }
    };
    let (smtp_credentials, mechanisms) = match credentials {
        Credentials::Password { username, password } => (
            SmtpCredentials::new(username.clone(), password.clone()),
            vec![Mechanism::Plain, Mechanism::Login],
        ),
        Credentials::OAuth2 {
            username,
            access_token,
        } => (
            SmtpCredentials::new(username.clone(), access_token.clone()),
            vec![Mechanism::Xoauth2],
        ),
    };
    Ok(builder
        .port(server.port)
        .credentials(smtp_credentials)
        .authentication(mechanisms)
        .timeout(Some(limit))
        .build())
}

fn classify(host: &str, port: u16, error: &lettre::transport::smtp::Error) -> SyncError {
    if error.is_timeout() {
        return SyncError::Timeout {
            host: host.to_owned(),
        };
    }
    if error.is_tls() {
        return SyncError::Tls {
            host: host.to_owned(),
            detail: error.to_string(),
        };
    }
    let code = error.status().map(|c| c.to_string());
    if matches!(code.as_deref(), Some("535" | "534" | "530")) {
        return SyncError::Auth;
    }
    if code.is_none()
        && let Some(local) = classify_unanswered(host, port, &error.to_string(), error)
    {
        return local;
    }
    SyncError::Send {
        message: error.to_string(),
        permanent: error.is_permanent(),
    }
}

/// A failure with no SMTP reply behind it, sorted by what the person can
/// do about it. lettre keeps the error kind private; `text` (its message)
/// starts with the kind. `None` for anything not recognised.
fn classify_unanswered(
    host: &str,
    port: u16,
    text: &str,
    error: &(dyn std::error::Error + 'static),
) -> Option<SyncError> {
    // Never connected: the address lookup, the TCP connection or the TLS
    // handshake failed. lettre files all three under "Connection error".
    if text.starts_with("Connection error") {
        if let Some(detail) = crate::error::tls_detail(error) {
            return Some(SyncError::Tls {
                host: host.to_owned(),
                detail,
            });
        }
        return Some(SyncError::Connect {
            host: host.to_owned(),
            port,
            reason: crate::error::cause_reason(error),
        });
    }
    if text.contains("STARTTLS is not supported") {
        return Some(SyncError::Tls {
            host: host.to_owned(),
            detail: format!(
                "the server doesn't offer STARTTLS on port {port}. Check the port and the security setting."
            ),
        });
    }
    if text.contains("No compatible authentication mechanism") {
        return Some(SyncError::Config(format!(
            "{host} doesn't offer a way to sign in that Mocky Mail can use on port {port}. Check the outgoing server's port and security setting."
        )));
    }
    None
}

/// Whether the exchange broke off after the connection was up: a dropped
/// link or a timeout between commands, with no reply from the server. The
/// message may already have been accepted. Failing to connect, a TLS
/// failure and any SMTP reply are all definite answers instead.
fn mid_session(error: &lettre::transport::smtp::Error) -> bool {
    if error.status().is_some() || error.is_tls() {
        return false;
    }
    // lettre keeps the error kind private; its text starts with the kind.
    let text = error.to_string();
    text.starts_with("network error") || text.starts_with("response error")
}

/// Connect and log in, then disconnect. Used when adding an account, so it
/// gives up after [`LOGIN_TIMEOUT`] instead of the long limit for sending.
///
/// # Errors
/// [`SyncError::Auth`] for refused credentials; connect or TLS errors;
/// [`SyncError::Timeout`] when the server stays silent.
pub async fn test_login(server: &ServerConfig, credentials: &Credentials) -> Result<(), SyncError> {
    let transport = transport(server, credentials, LOGIN_TIMEOUT)?;
    // lettre's own limit is per step (connect, greeting, each command);
    // this one covers the whole check.
    let Ok(result) = tokio::time::timeout(LOGIN_TIMEOUT, transport.test_connection()).await else {
        return Err(SyncError::Timeout {
            host: server.host.clone(),
        });
    };
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(SyncError::Connect {
            host: server.host.clone(),
            port: server.port,
            reason: "the server did not accept the connection".to_owned(),
        }),
        Err(e) => Err(classify(&server.host, server.port, &e)),
    }
}

/// Send an already-built message to the given recipients.
///
/// # Errors
/// [`SyncError::Send`] with `permanent` set for 5xx replies.
/// [`SyncError::Connect`] or [`SyncError::Timeout`] when the server
/// couldn't be reached (worth retrying); [`SyncError::Tls`] or
/// [`SyncError::Config`] when it could but the settings don't fit it.
/// [`SyncError::SendUnconfirmed`] when the exchange broke off after it
/// began, so the message may have gone out.
pub async fn send(
    server: &ServerConfig,
    credentials: &Credentials,
    from: &str,
    recipients: &[String],
    raw: &[u8],
) -> Result<(), SyncError> {
    let parse = |a: &str| {
        a.parse()
            .map_err(|_| SyncError::Invalid(format!("\"{a}\" is not a valid address.")))
    };
    let to = recipients
        .iter()
        .map(|r| parse(r))
        .collect::<Result<Vec<_>, _>>()?;
    let envelope =
        Envelope::new(Some(parse(from)?), to).map_err(|e| SyncError::Invalid(e.to_string()))?;
    transport(server, credentials, SMTP_TIMEOUT)?
        .send_raw(&envelope, raw)
        .await
        .map(|_| ())
        .map_err(|e| {
            if mid_session(&e) {
                SyncError::SendUnconfirmed {
                    host: server.host.clone(),
                }
            } else {
                classify(&server.host, server.port, &e)
            }
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::io::{Error, ErrorKind as Io};

    use super::*;

    #[test]
    fn failures_before_any_reply_say_what_to_fix() {
        let refused = Error::from(Io::ConnectionRefused);
        let error = classify_unanswered(
            "smtp.example.com",
            465,
            "Connection error: refused",
            &refused,
        )
        .unwrap();
        let SyncError::Connect { host, port, reason } = &error else {
            panic!("{error:?}");
        };
        assert_eq!((host.as_str(), *port), ("smtp.example.com", 465));
        assert!(reason.contains("refused the connection"), "{reason}");
        assert!(error.is_transient());

        // lettre reports a failed handshake as a connection error too.
        let bad_cert = Error::new(
            Io::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidForName),
        );
        let error = classify_unanswered("h", 465, "Connection error: invalid", &bad_cert).unwrap();
        assert!(matches!(error, SyncError::Tls { .. }), "{error:?}");
        assert!(!error.is_transient());

        let none = Error::other("No compatible authentication mechanism was found");
        let text = format!("internal client error: {none}");
        let error = classify_unanswered("smtp.example.com", 25, &text, &none).unwrap();
        assert!(matches!(error, SyncError::Config(_)), "{error:?}");
        assert!(error.to_string().contains("smtp.example.com"));

        let other = Error::other("odd");
        assert!(classify_unanswered("h", 25, "internal client error: odd", &other).is_none());
    }

    #[tokio::test]
    async fn a_closed_port_is_a_connect_error_not_a_send_error() {
        // A port nothing listens on: bound, then released.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let server = ServerConfig {
            host: "127.0.0.1".into(),
            port,
            security: Security::Tls,
        };
        let credentials = Credentials::Password {
            username: "me@example.com".into(),
            password: "secret".into(),
        };
        let error = test_login(&server, &credentials).await.unwrap_err();
        assert!(
            matches!(error, SyncError::Connect { port: p, .. } if p == port),
            "{error:?}"
        );
    }

    #[test]
    fn checking_a_login_gives_up_long_before_sending_does() {
        assert!(LOGIN_TIMEOUT <= Duration::from_secs(30));
        assert!(LOGIN_TIMEOUT < SMTP_TIMEOUT);
    }
}
