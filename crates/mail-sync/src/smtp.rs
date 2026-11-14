//! SMTP: logging in and handing a finished message to the server.

use std::time::Duration;

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::{Credentials as SmtpCredentials, Mechanism};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::{Credentials, Security, ServerConfig, SyncError};

/// How long one SMTP exchange may take. Large attachments on slow links
/// need room, so this is generous.
pub const SMTP_TIMEOUT: Duration = Duration::from_mins(3);

fn transport(
    server: &ServerConfig,
    credentials: &Credentials,
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
        .timeout(Some(SMTP_TIMEOUT))
        .build())
}

fn classify(host: &str, error: &lettre::transport::smtp::Error) -> SyncError {
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
    SyncError::Send {
        message: error.to_string(),
        permanent: error.is_permanent(),
    }
}

/// Connect and log in, then disconnect. Used when adding an account.
///
/// # Errors
/// [`SyncError::Auth`] for refused credentials; connect or TLS errors.
pub async fn test_login(server: &ServerConfig, credentials: &Credentials) -> Result<(), SyncError> {
    let transport = transport(server, credentials)?;
    match transport.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err(SyncError::Connect {
            host: server.host.clone(),
            port: server.port,
            reason: "the server did not accept the connection".to_owned(),
        }),
        Err(e) => Err(classify(&server.host, &e)),
    }
}

/// Send an already-built message to the given recipients.
///
/// # Errors
/// [`SyncError::Send`] with `permanent` set for 5xx replies, or connection
/// errors (all of which are worth retrying).
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
    transport(server, credentials)?
        .send_raw(&envelope, raw)
        .await
        .map(|_| ())
        .map_err(|e| classify(&server.host, &e))
}
