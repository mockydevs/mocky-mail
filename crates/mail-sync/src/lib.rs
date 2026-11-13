//! Network layer for Mocky Mail: talking to mail servers and the system
//! keyring.
//!
//! Decisions (what to show, when to retry, what is safe) live in
//! `mail-core`; this crate only moves bytes. Every network call has a
//! timeout, every connection is TLS-encrypted and certificate-checked with
//! the operating system's trust store, and credentials never appear in
//! errors, logs or `Debug` output.

pub mod config;
pub mod error;
pub mod imap;
pub mod outgoing;
pub mod secrets;
pub mod smtp;
mod tls;

pub use config::{
    AuthKind, Credentials, Security, ServerConfig, ServerSuggestion, suggest_servers,
};
pub use error::{ErrorKind, SyncError};
