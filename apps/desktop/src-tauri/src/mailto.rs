//! `mailto:` links handed to the app by the system: on the command line
//! when the app starts (Linux and Windows), from a second launch while it
//! runs (`mail://mailto` event), or from Android's intent (read by the page
//! through `MockyIntents`). They only fill in a new message.

use std::sync::Mutex;

use mail_core::mailto::{Mailto, parse};

use crate::state::lock;

/// A link the app was started with, until the page asks for it.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

/// The first `mailto:` argument, if any.
pub fn from_args<'a>(args: impl IntoIterator<Item = &'a str>) -> Option<String> {
    args.into_iter()
        .find(|a| {
            a.get(..7)
                .is_some_and(|p| p.eq_ignore_ascii_case("mailto:"))
        })
        .map(str::to_owned)
}

/// Remember the link this process was started with.
pub fn remember_launch_link() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    *lock(&PENDING) = from_args(args.iter().map(String::as_str));
}

/// `take_mailto`: the link the app was started with, once.
#[tauri::command]
pub fn take_mailto() -> Option<Mailto> {
    lock(&PENDING).take().and_then(|url| parse(&url))
}

/// `parse_mailto`: read a link handed over another way.
#[tauri::command]
pub fn parse_mailto(url: String) -> Option<Mailto> {
    parse(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_link_among_arguments() {
        assert_eq!(
            from_args(["--minimized", "mailto:info@mocky.co.ke"]).as_deref(),
            Some("mailto:info@mocky.co.ke")
        );
        assert_eq!(from_args(["--help"]), None);
    }
}
