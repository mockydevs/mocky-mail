# Contributing

Thanks for helping. A few rules keep the code dependable:

1. **Rules go in `mail-core`.** If it decides something (what to show, when
   to send, what is allowed), it is a pure function in `crates/mail-core`
   with unit tests and, where it fits, property tests. The Tauri shell and
   the interface stay thin.
2. **No shortcuts on safety.** No `unsafe`, `unwrap`, `expect` or `panic`
   outside tests. No `dangerouslySetInnerHTML`. Untrusted text is passed
   through `display_safe` before it is shown.
3. **Every bug fix comes with a test** that fails without the fix.
4. **All checks pass** (see the README) before review. CI runs them on
   Linux, Windows and macOS.
5. **Small pull requests**, one change each, with a clear description.

By contributing you agree your work is licensed under GPL-3.0-or-later.
