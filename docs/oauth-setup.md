# Desktop mailbox sign-in

Mocky Mail uses the system browser (opened through the Tauri opener plugin, which can also open it on Android), authorization code with PKCE S256, a random state, and an ephemeral loopback listener. The Rust process exchanges the code. Keep tokens out of IPC responses, logs, telemetry and the renderer. Store the refresh token in the operating system keyring. When refresh returns a replacement token, persist it before the next sync. If it omits a replacement, retain the old one. Reauthorize after an invalid grant.

## Google

Create an OAuth client of type **Desktop app** in Google Auth Platform. During development choose External / Testing, configure support and developer contacts, add the intended tester under Audience, and configure the scopes `openid`, `https://www.googleapis.com/auth/userinfo.email` and `https://mail.google.com/`. IMAP and SMTP use the full Gmail mail scope. The client requests offline access and uses a loopback redirect at `http://127.0.0.1:<port>/`. A native application's bundled Google client secret is recoverable by anyone with its binary.

Before public distribution, review Google's restricted-scope verification, branding, domain, privacy policy and data-handling requirements. Security assessment requirements depend on how restricted data is stored or transmitted. Testing-mode grants with the Gmail scope generally expire after seven days. See [Google native OAuth](https://developers.google.com/identity/protocols/oauth2/native-app) and [Gmail scopes](https://developers.google.com/workspace/gmail/api/auth/scopes).

## Microsoft

Register a public **Mobile and desktop applications** client in Entra. Choose accounts in any organizational directory and personal Microsoft accounts. Register `http://localhost` as the native redirect. The browser returns to localhost at the ephemeral port chosen by the application. No client secret is needed.

Configure delegated IMAP.AccessAsUser.All and SMTP.Send (listed under Microsoft Graph in the portal). Request the Outlook resource scopes at runtime, together with openid, email and offline_access. Individual users consent, subject to tenant policy. No application-only mailbox permissions are needed. Check that IMAP and authenticated SMTP are allowed for the mailbox being tested. See [Microsoft's Exchange OAuth guide](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth).

## Build and verify

Provide these variables to Cargo at compile time. Restart an already-running build/dev shell after setting them.

- MOCKY_MAIL_GOOGLE_CLIENT_ID
- MOCKY_MAIL_GOOGLE_CLIENT_SECRET
- MOCKY_MAIL_MS_CLIENT_ID

On the development machine the configuration is in the checkout's gitignored `.env.oauth`. In a trusted local shell use `set -a`, source that file, and run `set +a`. Keep its permissions at 0600. Do not print it. A provider without its registration returns a configuration error.

Run `cargo test -p mail-auth` for the local protocol tests. For the desktop smoke check, use each provider's own test mailbox, select it in the system browser and approve the requested scopes. Verify a read-only IMAP operation and an authenticated SMTP connection without sending a message. Restart the app and verify keyring-backed refresh. A login hint is advisory and does not establish mailbox ownership; use a successfully authenticated mailbox before persisting the account association. The helper returns base64 XOAUTH2; a SASL library that base64-encodes its input needs the decoded bytes.
