//! The single error type returned by every store operation.

use thiserror::Error;

/// Something went wrong reading or writing the local database.
///
/// Messages never contain user mail content, passwords or tokens, so they
/// are safe to log.
#[derive(Debug, Error)]
pub enum StoreError {
    /// `SQLite` reported an error (I/O, constraint, busy, corrupt file).
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A JSON column could not be encoded or decoded.
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// The requested record does not exist.
    #[error("{0} not found")]
    NotFound(&'static str),
    /// The caller passed a value the store cannot hold.
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    /// A stored value is outside the range this version understands.
    #[error("stored value is invalid: {0}")]
    Corrupt(&'static str),
    /// The database was written by a newer version of the app.
    #[error("database schema version {found} is newer than supported version {supported}")]
    SchemaTooNew {
        /// Version found in the file.
        found: i64,
        /// Highest version this build knows.
        supported: i64,
    },
    /// The operating system could not supply random bytes for a new ID.
    #[error("could not generate a random id: {0}")]
    Random(String),
}

/// Shorthand for results from this crate.
pub type Result<T, E = StoreError> = std::result::Result<T, E>;
