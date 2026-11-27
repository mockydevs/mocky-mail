# Mocky Mail

A fast, private email app for **Windows, Linux and Android** (macOS coming
soon), by [Mocky Digital](https://mocky.co.ke). Free and open source under the
GPL-3.0.

> Status: version 0.10, in testing. It syncs and sends real mail over IMAP
> and SMTP, with password sign-in for any provider and browser sign-in for
> Gmail and Microsoft 365/Outlook.

## What makes it different

- **Works on bad connections.** Mail is saved on your device, so it opens
  offline. Messages you send wait in the Outbox and go out by themselves when
  the connection returns, with sensible retries. After a crash, Mocky Mail
  checks your Sent folder before resending, and asks you if it can't tell.
  Moves, deletes and read marks made offline reach the server later.
- **Private by default.** Remote images (tracking pixels) are blocked until
  you allow them. HTML email is cleaned and then shown in a locked-down
  sandbox. Passwords and sign-in tokens live in your system's keyring, never
  in the app's files.
- **Small and light.** Built with Rust and Tauri, not a bundled browser.

## Features

- Several accounts side by side, each with its full folder tree.
- Gmail and Microsoft 365/Outlook sign-in through your browser, or a
  password for any IMAP/SMTP provider (server settings are suggested).
- On Android, pick a Google account already on the phone; no typing.
- Fast search across all your mail, even offline.
- Undo send (5 seconds by default, adjustable) and Undo after archive or
  delete. A toast tells you when a message is actually sent, or why not.
- Recipient suggestions from people you've written to or heard from.
- The list is grouped by day: Today, Yesterday, weekdays, then months.
- Choose when mail counts as read: on open, after 3 seconds, or by hand.
- Programs and scripts can't be attached by mistake (blocked by name,
  content and inside zips, as Gmail does).
- On phones: pull down to check for new mail, a full-screen message view,
  quick reply and Back-button support.
- Compose with Cc and Bcc, attachments and saved drafts. Reply, reply all
  and forward.
- New-mail alerts with an optional sound.
- Light and dark themes. In dark mode, email is darkened too, with a
  "Show original colours" button.
- Keyboard shortcuts: `j`/`k` next and previous, `r` reply, `e` archive,
  `#` delete, `c` new message, `/` search.

## Install

| System | How |
| --- | --- |
| **Linux** (Ubuntu, Debian, Mint, Pop!_OS) | One command, below |
| **Windows** 10 and 11 | Download the installer from **[mailer.co.ke](https://mailer.co.ke)** |
| **Android** 7 and newer | **[Google Play](https://play.google.com/store/apps/details?id=ke.co.mocky.mail)** (in review; the link works once Google approves it) |
| macOS | Coming soon |

### Linux in one command

```bash
curl -fsSL https://raw.githubusercontent.com/mockydevs/mocky-mail/main/install.sh | bash
```

It downloads the latest `.deb` from mailer.co.ke, checks it against the
published SHA-256 checksum, and installs it with `apt`, so `apt` can update and
remove it later. It asks for your password only for `apt`. Run the same
command again to update. Prefer to read it first? [`install.sh`](install.sh) is
short. Options: `--dry-run` shows what it would do, `--uninstall` removes the
app (your mail and settings stay), `--yes` skips the question.

Or download the `.deb` from [mailer.co.ke](https://mailer.co.ke) and install
it yourself:

```bash
sudo apt install ./mocky-mail_*_amd64.deb
```

Step-by-step instructions for each system are in [docs/install.md](docs/install.md).

## Layout

```
crates/mail-core/        Pure mail rules: parsing, HTML cleaning, outbox, size limits.
                         No network, disk or clock, so every rule is unit tested.
crates/mail-store/       Local SQLite database: accounts, folders, messages, search,
                         the outbox and offline changes waiting for the server.
crates/mail-sync/        IMAP and SMTP connections, and the system keyring.
crates/mail-auth/        Browser sign-in (OAuth with PKCE) for Gmail and Microsoft.
apps/desktop/            React + TypeScript interface (Vite).
apps/desktop/src-tauri/  Tauri shell: window, sync and outbox workers, IPC commands.
docs/                    IPC contract, install guide, OAuth setup, store release notes.
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

Gmail and Microsoft sign-in need OAuth client IDs set when the app is built;
without them those two options show a configuration error and password
sign-in still works. See [docs/oauth-setup.md](docs/oauth-setup.md).

The interface and the native side talk through the commands in
[docs/ipc.md](docs/ipc.md).

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
