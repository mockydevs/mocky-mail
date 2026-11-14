# Installing Mocky Mail

Download the installer for your computer from **[mailer.co.ke](https://mailer.co.ke)**,
or from the [latest GitHub release](https://github.com/mockydevs/mocky-mail/releases/latest).

| Computer | File to download |
| --- | --- |
| Linux (Ubuntu, Debian, Mint, Pop!_OS) | `mocky-mail_<version>_amd64.deb` |
| Windows 10 or 11 | `mocky-mail_<version>_x64-setup.exe` (or the `.msi` for managed installs) |
| macOS 11 or newer (Apple Silicon and Intel) | `mocky-mail_<version>_universal.dmg` |

## Linux (.deb)

Needs a 64-bit (amd64) system based on Ubuntu 22.04, Debian 12 or newer.

In a terminal, in the folder you downloaded it to:

```bash
sudo apt install ./mocky-mail_0.1.0_amd64.deb
```

Use `apt` rather than `dpkg -i`: `apt` also installs the libraries the app
needs (WebKitGTK and GTK). Mocky Mail then appears in your applications menu.
You can also start it from a terminal with `mocky-mail-desktop`.

To update, download the newer `.deb` and run the same command.

To remove it:

```bash
sudo apt remove mocky-mail
```

## Windows

1. Run `mocky-mail_<version>_x64-setup.exe`.
2. The installer is not code-signed yet, so Windows may show "Windows protected
   your PC". Click **More info**, then **Run anyway**.
3. Follow the installer. Mocky Mail is added to the Start menu.

The `.msi` file installs the same app and suits IT tools such as Intune or
Group Policy. To remove the app, use **Settings > Apps > Installed apps**.

## macOS

1. Open `mocky-mail_<version>_universal.dmg` and drag **Mocky Mail** into
   **Applications**.
2. The app is not notarized by Apple yet, so the first time you open it macOS
   says it cannot check it for malicious software. Click **Done**, then open
   **System Settings > Privacy & Security**, scroll down and click
   **Open Anyway** next to Mocky Mail.
3. After that it opens normally.

To remove it, drag Mocky Mail from Applications to the Bin.

## Your mail and passwords

Mail is stored on your computer. Account passwords and sign-in tokens are kept
in the system keychain (Windows Credential Manager, macOS Keychain, or the
Secret Service keyring on Linux), never in plain files. Removing the app does
not delete them.

## For maintainers: making a release

1. Set the new version in `apps/desktop/src-tauri/tauri.conf.json`.
2. Commit, then tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. The **Release** workflow builds the `.deb`, Windows and macOS installers
   and attaches them to a draft release. Check it, then publish it.

The workflow needs these repository secrets so Gmail and Outlook sign-in work
in the installers: `MOCKY_MAIL_GOOGLE_CLIENT_ID`,
`MOCKY_MAIL_GOOGLE_CLIENT_SECRET` and `MOCKY_MAIL_MS_CLIENT_ID`. Without them
the build still succeeds, but those two sign-in options show a configuration
error.

The download site at https://mailer.co.ke is a static page served by the mail
server (`data/web/mailer-site/` with `data/conf/nginx/mailer-site.conf` in the
Mailcow checkout). To publish a new version there, upload the new installer to
its `downloads/` folder, regenerate `SHA256SUMS`, and update the version, size
and checksum in its `index.html`.

To build the installers on a Linux machine (load the sign-in IDs from
`.env.oauth` first, see `docs/oauth-setup.md`):

```bash
cd apps/desktop
npx @tauri-apps/cli@2.12.4 build --bundles deb
```

The Windows `.exe` can be cross-built on Linux too, after
`sudo apt install nsis mingw-w64` and `rustup target add x86_64-pc-windows-gnu`:

```bash
cd apps/desktop
npx @tauri-apps/cli@2.12.4 build --target x86_64-pc-windows-gnu --bundles nsis
```

It lands in `target/x86_64-pc-windows-gnu/release/bundle/nsis/` and includes
`WebView2Loader.dll`. The `.msi` can only be built on Windows.

### Signing the Windows installer

Unsigned installers trigger "Windows protected your PC" and some antivirus
warnings. Signing them with Mocky Digital's code-signing certificate shows
"Mocky Digital" as the verified publisher. Once the certificate is set up
(see the variables at the top of `scripts/sign-windows.sh`), build with:

```bash
cd apps/desktop
npx @tauri-apps/cli@2.12.4 build --target x86_64-pc-windows-gnu --bundles nsis \
  --config src-tauri/tauri.sign.conf.json
```

This signs the app, the installer and the uninstaller, and stops with an
error if no certificate is configured, so an unsigned file is never shipped
as a signed release by mistake. Needs `sudo apt install osslsigncode` for the
certificate-file method.

