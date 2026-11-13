//! TLS setup shared by IMAP. Certificates are checked against the
//! operating system's trust store (via `rustls-platform-verifier`), so
//! company-installed roots work and expired or wrong-host certificates fail.

use std::sync::Arc;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::SyncError;

fn connector(host: &str) -> Result<TlsConnector, SyncError> {
    let tls_error = |detail: String| SyncError::Tls {
        host: host.to_owned(),
        detail,
    };
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| tls_error(e.to_string()))?
            .with_platform_verifier()
            .map_err(|e| tls_error(e.to_string()))?
            .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Wrap an open TCP connection in TLS for `host`.
pub(crate) async fn wrap(tcp: TcpStream, host: &str) -> Result<TlsStream<TcpStream>, SyncError> {
    let name = ServerName::try_from(host.to_owned())
        .map_err(|_| SyncError::Config(format!("\"{host}\" is not a valid server name.")))?;
    connector(host)?
        .connect(name, tcp)
        .await
        .map_err(|e| SyncError::Tls {
            host: host.to_owned(),
            detail: e.to_string(),
        })
}
