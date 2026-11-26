//! Conversions between the database's and the network layer's server types.

use mail_store::{Security as StoreSecurity, ServerConfig as StoreServer};
use mail_sync::{Security, ServerConfig};

/// Database server settings → network settings.
pub fn to_sync(server: &StoreServer) -> ServerConfig {
    ServerConfig {
        host: server.host.clone(),
        port: server.port,
        security: match server.security {
            StoreSecurity::Tls => Security::Tls,
            StoreSecurity::Starttls => Security::Starttls,
        },
    }
}

/// For `#[serde(deserialize_with)]` on an `Option<Option<T>>` field with
/// `#[serde(default)]`: left out is `None`, null is `Some(None)`, a value
/// `Some(Some(v))`. The interface uses null to mean "clear it".
///
/// # Errors
/// When the value isn't a `T` or null.
#[allow(clippy::option_option, reason = "the three cases are the point")]
pub fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

/// Network settings → database server settings.
pub fn to_store(server: &ServerConfig) -> StoreServer {
    StoreServer {
        host: server.host.trim().to_ascii_lowercase(),
        port: server.port,
        security: match server.security {
            Security::Tls => StoreSecurity::Tls,
            Security::Starttls => StoreSecurity::Starttls,
        },
    }
}
