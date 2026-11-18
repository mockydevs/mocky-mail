# Gmail and Microsoft calendars and contacts: setup checklist

Calendars and contacts already sync for password accounts (SOGo and other
CalDAV/CardDAV servers). Gmail and Microsoft accounts need the steps below,
done once by the account owner, before the app can be extended to them.

## Google (Gmail accounts)

Google exposes calendars over CalDAV and contacts over CardDAV, which the app
already speaks; it only needs an OAuth token with the right permissions.

In the Google Cloud project that holds Mocky Mail's sign-in client (the one in
`.env.oauth`, `MOCKY_MAIL_GOOGLE_CLIENT_ID`):

1. APIs & Services → Library: enable **CalDAV API** and **Google Contacts
   CardDAV API**.
2. OAuth consent screen → Data access → Add or remove scopes: add
   `https://www.googleapis.com/auth/calendar` and
   `https://www.googleapis.com/auth/carddav`.
3. Both are "sensitive" scopes. Until Google verifies them, only test users
   on the consent screen can grant them, and others see an "unverified app"
   warning. Submit the app for verification (privacy policy at
   https://mailer.co.ke/privacy.html, a short video of the calendar and
   contacts screens, and why the app needs each scope).
4. The Android sign-in (Google Play services) uses the same project. Nothing
   extra is needed there beyond the scopes.

The app will ask for calendar and contacts separately ("Connect Gmail
calendar"), so Gmail sign-in for mail is never affected while verification
is pending.

## Microsoft (Outlook.com and Microsoft 365 accounts)

Microsoft has no CalDAV or CardDAV; calendars and contacts go through
Microsoft Graph. Microsoft mail sign-in isn't set up in builds yet either
(`MOCKY_MAIL_MS_CLIENT_ID` is empty), so this also turns on Microsoft mail.

1. https://entra.microsoft.com → App registrations → New registration:
   name "Mocky Mail", supported accounts "Accounts in any organizational
   directory and personal Microsoft accounts".
2. Authentication → Add a platform → Mobile and desktop applications:
   redirect URI `http://localhost`; allow public client flows: Yes.
3. API permissions → Add → Microsoft Graph → Delegated: `Calendars.ReadWrite`,
   `Contacts.ReadWrite`, `offline_access`, `email`, `openid`. Then add the
   Office 365 Exchange Online permissions `IMAP.AccessAsUser.All` and
   `SMTP.Send` for mail.
4. Copy the Application (client) ID into `.env.oauth` as
   `MOCKY_MAIL_MS_CLIENT_ID`. No client secret is needed (public client).
5. For other companies' Microsoft 365 accounts, Microsoft may require
   publisher verification (a Microsoft Partner Network ID) before their
   admins allow the app.

When these are done, tell Claude and the app work can start: Google first
(it reuses the existing calendar and contacts sync), then Microsoft Graph.
