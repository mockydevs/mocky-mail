# Gmail and Microsoft calendars, contacts and tasks: setup checklist

Calendars, contacts and tasks sync for password accounts (SOGo and other
CalDAV/CardDAV servers) with no setup. Gmail and Microsoft accounts are
connected from the app ("Connect calendar and contacts",
`connect_calendar_and_contacts`), which signs in again in the browser asking
for more permissions. That only works once the console steps below are done
for the OAuth clients built into the app. Until then, connecting fails with
"Google hasn't enabled calendar access for Mocky Mail yet" or "Microsoft
hasn't allowed calendar access for Mocky Mail yet", and mail keeps working.

The client IDs come from the build environment, as for mail sign-in
(`.env.oauth`, see `oauth-setup.md`): `MOCKY_MAIL_GOOGLE_CLIENT_ID`,
`MOCKY_MAIL_GOOGLE_CLIENT_SECRET`, `MOCKY_MAIL_MS_CLIENT_ID`. Nothing new is
added to the build; the extra permissions are asked for at run time.

## Google (Gmail accounts)

In the Google Cloud project that holds Mocky Mail's sign-in client:

1. **APIs & Services → Library**: enable each of these. The app calls all of
   them; a missing one shows "Google hasn't enabled calendar and contacts
   access for Mocky Mail yet (one of its Google APIs is switched off)".
   - **CalDAV API** (events, `apidata.googleusercontent.com/caldav/v2/`)
   - **Google Calendar API** (the calendar list and Find a Time free/busy,
     `www.googleapis.com/calendar/v3/`)
   - **Google Contacts CardDAV API** (contacts,
     `www.googleapis.com/carddav/v1/`)
   - **People API** (contact groups, `people.googleapis.com/v1/`)
   - **Google Tasks API** (`tasks.googleapis.com/tasks/v1/`)
   - **Gmail API** (automatic replies, `gmail.googleapis.com/gmail/v1/users/me/settings/vacation`)
2. **OAuth consent screen → Data access → Add or remove scopes**: add
   - `https://www.googleapis.com/auth/calendar`
   - `https://www.googleapis.com/auth/carddav`
   - `https://www.googleapis.com/auth/contacts`
   - `https://www.googleapis.com/auth/tasks`
   - `https://www.googleapis.com/auth/gmail.settings.basic`
   (`https://mail.google.com/`, `openid` and `email` are there already for
   mail.)
3. These are sensitive (and `mail.google.com` restricted) scopes. Until
   Google verifies the app, only test users on the consent screen can grant
   them, and others see an "unverified app" warning. Submit for
   verification: privacy policy at https://mailer.co.ke/privacy.html, a short
   video of the calendar, contacts, tasks and automatic-reply screens, and
   why each scope is needed (calendar: show and edit events and answer
   invitations; carddav/contacts: sync the address book and its groups;
   tasks: sync to-dos; gmail.settings.basic: set automatic replies).
4. The sign-in is incremental (`include_granted_scopes=true`): mail access
   stays as it was, and the new refresh token covers both.

Android: Gmail accounts on Android sign in through Google Play services
(`GoogleAuthPlugin.kt`), which asks for the mail scope only. Connecting a
Gmail calendar on Android is refused with a plain message for now; it needs
the Kotlin plugin to request the scopes above (`Scope(...)` in its
`AuthorizationRequest`) and return the extra grant. Microsoft accounts
connect on Android through the browser like on computers.

## Microsoft (Outlook.com and Microsoft 365 accounts)

Microsoft has no CalDAV or CardDAV; calendars, contacts, tasks and automatic
replies go through Microsoft Graph. Microsoft issues a token per resource,
so this is a second grant beside the mail one (IMAP/SMTP), with its own
refresh token.

In the Entra app registration (https://entra.microsoft.com → App
registrations → Mocky Mail, the client in `MOCKY_MAIL_MS_CLIENT_ID`):

1. If it doesn't exist yet: New registration, name "Mocky Mail", supported
   accounts "Accounts in any organizational directory and personal Microsoft
   accounts"; Authentication → Add a platform → Mobile and desktop
   applications: redirect URI `http://localhost`; Allow public client
   flows: Yes. No client secret (public client).
2. **API permissions → Add a permission → Microsoft Graph → Delegated
   permissions**: add
   - `Calendars.ReadWrite`
   - `Contacts.ReadWrite`
   - `Tasks.ReadWrite`
   - `MailboxSettings.ReadWrite`
   - `offline_access`, `openid`, `email` (already there for mail)
   and keep the Office 365 Exchange Online `IMAP.AccessAsUser.All` and
   `SMTP.Send` for mail.
3. None of these need admin consent for personal accounts. In other
   companies' Microsoft 365 tenants an administrator may have to allow the
   app (Enterprise applications → Mocky Mail → Grant admin consent), and
   Microsoft may require publisher verification (a Microsoft Partner Network
   ID) first. Until then those users see "Microsoft hasn't allowed calendar
   access for Mocky Mail yet. A work account's administrator may need to
   allow it".
4. Mailboxes on a company's own (on-premises) Exchange server can't be
   reached through Graph; they show "Microsoft can't reach this mailbox's
   calendar and contacts for Mocky Mail".

## What works how

| | Password (CalDAV/CardDAV) | Gmail | Microsoft |
|---|---|---|---|
| Calendars and events | CalDAV | Calendar list from the Calendar API, events over Google's CalDAV | Graph `calendarView` (90 days back, a year on) kept as iCalendar |
| Invitations from the app | the server's own scheduling if it has it, else iMIP emails through the Outbox | Google's CalDAV scheduling | Graph sends them |
| Find a Time | CalDAV scheduling outbox (RFC 6638), if the server has one | Calendar API `freeBusy` | Graph `getSchedule` |
| Contacts | CardDAV | Google's CardDAV | Graph contacts (delta) |
| Contact pictures, favourites | in the vCard | this device only | this device only |
| Contact groups | group vCards (Apple style) | People API contact groups | this device only (Outlook contact folders aren't groups) |
| Tasks | CalDAV calendars that hold `VTODO` | Google Tasks (reminders, importance, categories stay on this device) | Microsoft To Do |
| Automatic replies | ManageSieve `vacation` | Gmail `settings/vacation` | Graph `automaticRepliesSetting` (inside/outside replies, outside audience) |
