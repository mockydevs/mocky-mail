//! Pure mail logic for Mocky Mail.
//!
//! Nothing in this crate touches the network, the disk or a clock. Every
//! function takes its inputs explicitly (including "now"), so each rule can
//! be tested exhaustively and the same code runs in the app, the sync
//! engine and the test suite.

pub mod compose;
pub mod folders;
pub mod limits;
pub mod message;
pub mod outbox;
pub mod sanitize;
