# Security

Email clients read hostile input all day. We treat every message, header,
file name and HTML body as untrusted.

## Reporting a vulnerability

Please email **info@mocky.co.ke** with "Mocky Mail security" in the subject.
Do not open a public issue. We aim to reply within 3 working days.

## How the app protects you

- **HTML email** is cleaned in Rust with an allow-list (`mail-core/src/sanitize.rs`):
  no scripts, event handlers, forms, iframes, SVG, `<style>` blocks,
  `javascript:` or `data:` links, or CSS that can position content or load
  URLs. The result is then shown in an `<iframe sandbox="">` with its own
  Content-Security-Policy, so even a sanitizer bug cannot run code.
- **Remote images are blocked** until you choose to show them.
- **Sender names, subjects and file names** have control characters and
  invisible direction marks removed, which defeats tricks like
  `invoice<RLO>fdp.exe` displaying as `invoiceexe.pdf`. Attachment names
  lose any folder path before saving.
- **Size limits** on every input from the interface to the backend.
- **Outbox** never sends a message twice: if the app stops mid-send, the
  message is checked against your Sent folder before it is retried.
- **Minimal permissions.** The window gets only Tauri's core permissions and
  link opening. The app has a strict Content-Security-Policy.
- **Rust code** forbids `unsafe`, `unwrap`, `panic` and unchecked arithmetic
  via lints, and dependencies are audited in CI.
