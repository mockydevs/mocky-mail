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
