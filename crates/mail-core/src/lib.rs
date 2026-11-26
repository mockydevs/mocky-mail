//! Pure mail logic for Mocky Mail.
//!
//! Nothing in this crate touches the network, the disk or a clock. Every
//! function takes its inputs explicitly (including "now"), so each rule can
//! be tested exhaustively and the same code runs in the app, the sync
//! engine and the test suite.

pub mod attachment_policy;
pub mod categories;
pub mod compose;
pub mod focus;
pub mod folders;
pub mod formatting;
pub mod ical;
pub mod importance;
pub mod limits;
pub mod mailto;
pub mod message;
pub mod outbox;
pub mod preview;
pub mod sanitize;
pub mod signature;
pub mod sync_window;
pub mod vacation;
pub mod vcard;
