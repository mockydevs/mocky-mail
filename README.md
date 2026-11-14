# Mocky Mail

A fast, private desktop email client for Windows, macOS and Linux, by
[Mocky Digital](https://mocky.co.ke). Free and open source under the GPL-3.0.

> Status: early development. The mail engine's rules and the interface are in
> place; connecting to real mail servers (IMAP, SMTP, OAuth) and the local
> database come next.

## What makes it different

- **Works on bad connections.** Mail is saved on your computer. Messages you
  send wait in the Outbox and go out by themselves when the connection
  returns, with sensible retries and no double-sends after a crash.
- **Private by default.** Remote images (tracking pixels) are blocked until
  you allow them. HTML email is cleaned and then shown in a locked-down
  sandbox.
- **Small and light.** Built with Rust and Tauri, not a bundled browser.

## Install

Download Mocky Mail from **[mailer.co.ke](https://mailer.co.ke)** (Windows
`.exe` and Linux `.deb` now; macOS coming soon). Builds are also attached to
[GitHub releases](https://github.com/mockydevs/mocky-mail/releases/latest).
On Ubuntu or Debian:

```bash
sudo apt install ./mocky-mail_0.1.0_amd64.deb
```

Step-by-step instructions for each system are in [docs/install.md](docs/install.md).

## Layout

```
crates/mail-core/        Pure mail rules: parsing, HTML cleaning, outbox, size limits.
                         No network, disk or clock, so every rule is unit tested.
apps/desktop/            React + TypeScript interface (Vite).
apps/desktop/src-tauri/  Tauri shell: window, permissions, and thin IPC commands.
```

## Develop

Requirements: Rust 1.95 (pinned in `rust-toolchain.toml`), Node 24, and the
[Tauri system dependencies](https://v2.tauri.app/start/prerequisites/) for
your OS. On Debian or Ubuntu:

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

```bash
cargo test                      # mail engine tests (no system libraries needed)
cd apps/desktop && npm ci
npm run dev                     # interface in the browser with sample data
npx @tauri-apps/cli@2.12.4 dev  # full desktop app (needs the system libraries)
```

Before opening a pull request, all of these must pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/desktop && npm run lint && npm run typecheck && npm test && npm run build
```

See [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md).

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
