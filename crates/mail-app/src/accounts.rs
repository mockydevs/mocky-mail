//! Adding, changing and removing accounts. Nothing is saved until a real
//! login to both the incoming and outgoing server has worked.

use std::sync::Arc;

use mail_auth::Provider;
use mail_store::{Account, AuthKind, NewAccount};
use mail_sync::imap::ImapConnection;
use mail_sync::{Credentials, Security, ServerConfig, ServerSuggestion, smtp};
use serde::Deserialize;

use crate::Ctx;
use crate::convert::{to_store, to_sync};
use crate::credentials::{cache_token, oauth_key, password_key};
use crate::error::{AppError, AppResult};
use crate::host::DEVICE_GOOGLE_TOKEN_SECS;
use crate::state::{AppState, lock, now};
use crate::{events, sync};

/// Colours handed out to accounts in order.
const PALETTE: &[&str] = &[
    "#2450C8", "#0F7B6C", "#8A3FB0", "#B5471B", "#1E6B3A", "#A1356B",
];

/// Input for `add_account`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAccountInput {
    email: String,
    display_name: String,
    password: String,
    imap: ServerConfig,
    smtp: ServerConfig,
}

/// Input for `add_oauth_account`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddOAuthInput {
    email: String,
    display_name: String,
    provider: String,
}

/// Changes for `update_account`.
#[allow(
    clippy::option_option,
    reason = "a syncMonths of null differs from one left out"
)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPatch {
    display_name: Option<String>,
    password: Option<String>,
    /// Left out: unchanged; null: keep all mail in full; months otherwise.
    #[serde(default, deserialize_with = "crate::convert::present")]
    sync_months: Option<Option<u32>>,
}

fn clean_email(email: &str) -> AppResult<String> {
    let email = email.trim().to_owned();
    if mail_core::compose::is_valid_address(&email) {
        Ok(email)
    } else {
        Err(AppError::invalid("Enter a valid email address."))
    }
}

fn clean_name(name: &str, email: &str) -> String {
    let name = mail_core::message::display_safe(name);
    if name.is_empty() {
        email.split('@').next().unwrap_or(email).to_owned()
    } else {
        name.chars().take(100).collect()
    }
}

/// The result of signing in with Google or Microsoft.
struct SignedIn {
    /// The mailbox address chosen.
    email: String,
    access_token: String,
    expires_at: u64,
    /// What to keep in the keyring: the refresh token, or for Android's own
    /// Google sign-in a marker (Android renews those tokens itself).
    stored_secret: String,
}

/// Sign in with the provider. On Android, Google uses the phone's own
/// accounts; everything else opens the provider's page in the browser.
/// `keep` are permissions beyond mail the account was granted before
/// (calendar, contacts, tasks), asked for again so they aren't dropped.
async fn sign_in(
    ctx: &Ctx,
    provider: Provider,
    typed: &str,
    keep: &[String],
) -> AppResult<SignedIn> {
    if provider == Provider::Google && ctx.host.device_google() {
        let granted = ctx.host.device_google_sign_in(keep).await?;
        return Ok(SignedIn {
            email: clean_email(&granted.email)?,
            access_token: granted.access_token,
            expires_at: now().saturating_add(DEVICE_GOOGLE_TOKEN_SECS),
            stored_secret: crate::credentials::ANDROID_GOOGLE.to_owned(),
        });
    }
    let opener = Arc::clone(&ctx.host);
    let tokens = mail_auth::authorize_keeping(provider, typed, keep, move |url| {
        // Opening links works on every platform, Android included.
        match opener.open_url(url) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("couldn't open the browser for sign-in: {error}");
                false
            }
        }
    })
    .await?;
    let email = chosen_address(typed, tokens.granted_email.as_deref())?;
    let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
        AppError::new(
            "auth",
            "The provider didn't grant offline access. Please try signing in again.",
        )
    })?;
    Ok(SignedIn {
        email,
        access_token: tokens.access_token.clone(),
        expires_at: tokens.expires_at_unix,
        stored_secret: refresh_token,
    })
}

/// The account the user signed in as: the one picked in the provider's
/// chooser, or else the address typed before signing in.
fn chosen_address(typed: &str, granted: Option<&str>) -> AppResult<String> {
    match granted {
        Some(granted) => clean_email(granted),
        None if !typed.is_empty() => clean_email(typed),
        None => Err(AppError::new(
            "auth",
            "Couldn't tell which account you chose. Type your email address and try again.",
        )),
    }
}

fn ensure_new(state: &AppState, email: &str) -> AppResult<()> {
    if state
        .store
        .list_accounts()?
        .iter()
        .any(|a| a.email.eq_ignore_ascii_case(email))
    {
        return Err(AppError::invalid(format!("{email} is already added.")));
    }
    Ok(())
}

async fn verify(
    imap: &ServerConfig,
    smtp_server: &ServerConfig,
    credentials: &Credentials,
) -> AppResult<()> {
    ImapConnection::connect(imap, credentials)
        .await?
        .logout()
        .await;
    smtp::test_login(smtp_server, credentials).await?;
    Ok(())
}

fn next_color(state: &AppState) -> AppResult<String> {
    let count = state.store.list_accounts()?.len();
    Ok(PALETTE
        .get(count.checked_rem(PALETTE.len()).unwrap_or(0))
        .copied()
        .unwrap_or("#2450C8")
        .to_owned())
}

fn save(
    state: &AppState,
    new: &NewAccount,
    secret_key: impl Fn(&str) -> String,
    secret: &str,
) -> AppResult<Account> {
    let account = state.store.insert_account(new)?;
    if let Err(error) = state.secrets.set(&secret_key(&account.id), secret) {
        // Never keep an account we can't log in to later.
        let _ = state.store.delete_account(&account.id);
        return Err(error.into());
    }
    Ok(account)
}

/// `suggest_servers`: settings to pre-fill for an address. Known providers
/// first, then the domain's DNS (SRV and MX records), then a guess.
pub async fn suggest_servers(email: String) -> Option<ServerSuggestion> {
    mail_sync::discover_servers(&email).await
}

/// `list_accounts`.
pub fn list_accounts(ctx: &Ctx) -> AppResult<Vec<Account>> {
    let state = &ctx.state;
    Ok(state.store.list_accounts()?)
}

/// `add_account`: password sign-in.
pub async fn add_account(ctx: &Ctx, input: AddAccountInput) -> AppResult<Account> {
    let state = &ctx.state;
    let email = clean_email(&input.email)?;
    ensure_new(state, &email)?;
    if input.password.is_empty() {
        return Err(AppError::invalid("Enter the password."));
    }
    input.imap.validate()?;
    input.smtp.validate()?;
    let credentials = Credentials::Password {
        username: email.clone(),
        password: input.password,
    };
    verify(&input.imap, &input.smtp, &credentials).await?;
    let Credentials::Password { password, .. } = credentials else {
        return Err(AppError::internal("unexpected credentials"));
    };
    let new = NewAccount {
        display_name: clean_name(&input.display_name, &email),
        email,
        auth: AuthKind::Password,
        color: next_color(state)?,
        imap: to_store(&input.imap),
        smtp: to_store(&input.smtp),
        size_limit_bytes: None,
        created_at: now(),
    };
    let account = save(state, &new, password_key, &password)?;
    sync::start(ctx, state, &account.id);
    events::accounts_changed(ctx);
    Ok(account)
}

/// `add_oauth_account`: Google or Microsoft sign-in in the system browser.
pub async fn add_oauth_account(ctx: &Ctx, input: AddOAuthInput) -> AppResult<Account> {
    let state = &ctx.state;
    // The address is optional: the provider's account chooser picks it.
    let typed = input.email.trim();
    if !typed.is_empty() {
        ensure_new(state, &clean_email(typed)?)?;
    }
    let (provider, auth) = match input.provider.as_str() {
        "google" => (Provider::Google, AuthKind::Google),
        "microsoft" => (Provider::Microsoft, AuthKind::Microsoft),
        _ => return Err(AppError::invalid("Unknown sign-in provider.")),
    };
    let signed_in = sign_in(ctx, provider, typed, &[]).await?;
    let email = signed_in.email;
    ensure_new(state, &email)?;
    let servers = provider.servers();
    let imap = ServerConfig {
        host: servers.imap.to_owned(),
        port: 993,
        security: Security::Tls,
    };
    let smtp_server = ServerConfig {
        host: servers.smtp.to_owned(),
        port: servers.smtp_port,
        security: if servers.smtp_starttls {
            Security::Starttls
        } else {
            Security::Tls
        },
    };
    // Logging in to the mailbox with the access token proves the grant
    // really covers this address.
    let credentials = Credentials::OAuth2 {
        username: email.clone(),
        access_token: signed_in.access_token.clone(),
    };
    verify(&imap, &smtp_server, &credentials).await.map_err(|error| {
        if error.kind == "auth" {
            AppError::new("auth", format!("You signed in, but not as {email}. Sign in with that account and try again."))
        } else {
            error
        }
    })?;
    let new = NewAccount {
        display_name: clean_name(&input.display_name, &email),
        email,
        auth,
        color: next_color(state)?,
        imap: to_store(&imap),
        smtp: to_store(&smtp_server),
        size_limit_bytes: Some(25 * 1024 * 1024),
        created_at: now(),
    };
    let account = save(state, &new, oauth_key, &signed_in.stored_secret)?;
    cache_token(
        state,
        &account.id,
        &signed_in.access_token,
        signed_in.expires_at,
    );
    sync::start(ctx, state, &account.id);
    events::accounts_changed(ctx);
    Ok(account)
}

/// `update_account`: rename, or replace the password after testing it.
pub async fn update_account(ctx: &Ctx, id: String, patch: AccountPatch) -> AppResult<Account> {
    let state = &ctx.state;
    let account = state.store.get_account(&id)?;
    if let Some(Some(months)) = patch.sync_months
        && !mail_core::sync_window::is_valid(months)
    {
        return Err(AppError::invalid(format!(
            "Choose between 1 and {} months.",
            mail_core::sync_window::MAX_MONTHS
        )));
    }
    if let Some(password) = patch.password.filter(|p| !p.is_empty()) {
        if account.auth != AuthKind::Password {
            return Err(AppError::invalid(
                "This account signs in with its provider, not a password.",
            ));
        }
        let credentials = Credentials::Password {
            username: account.email.clone(),
            password,
        };
        verify(
            &to_sync(&account.imap),
            &to_sync(&account.smtp),
            &credentials,
        )
        .await?;
        if let Credentials::Password { password, .. } = credentials {
            state.secrets.set(&password_key(&id), &password)?;
        }
        state.wake_sync(Some(&id));
        state.outbox_wake.notify_one();
    }
    if let Some(name) = patch.display_name {
        state
            .store
            .update_account_display_name(&id, &clean_name(&name, &account.email))?;
    }
    if let Some(months) = patch.sync_months
        && months != account.sync_months
    {
        state.store.set_account_sync_months(&id, months)?;
        // A wider window downloads the rest now. Bodies already kept stay.
        state.wake_sync(Some(&id));
    }
    events::accounts_changed(ctx);
    Ok(state.store.get_account(&id)?)
}

/// `reconnect_account`: sign in again to a Google or Microsoft account
/// whose saved sign-in has expired or been revoked. Without this the only
/// way back was removing the account, which also deletes its queued
/// messages and the contacts kept only on this device.
pub async fn reconnect_account(ctx: &Ctx, id: String) -> AppResult<Account> {
    let state = &ctx.state;
    let account = state.store.get_account(&id)?;
    let provider = match account.auth {
        AuthKind::Google => Provider::Google,
        AuthKind::Microsoft => Provider::Microsoft,
        AuthKind::Password => {
            return Err(AppError::invalid(
                "This account signs in with a password. Update it in Settings instead.",
            ));
        }
    };
    let keep = crate::credentials::kept_scopes(state, &account);
    let signed_in = sign_in(ctx, provider, &account.email, &keep).await?;
    // The provider's chooser lets any account be picked; only this one fits.
    if !signed_in.email.eq_ignore_ascii_case(&account.email) {
        return Err(AppError::new(
            "auth",
            format!(
                "You signed in as {}, but this account is {}. Sign in with that account.",
                signed_in.email, account.email
            ),
        ));
    }
    let credentials = Credentials::OAuth2 {
        username: account.email.clone(),
        access_token: signed_in.access_token.clone(),
    };
    verify(
        &to_sync(&account.imap),
        &to_sync(&account.smtp),
        &credentials,
    )
    .await?;
    state
        .secrets
        .set(&oauth_key(&id), &signed_in.stored_secret)?;
    cache_token(state, &id, &signed_in.access_token, signed_in.expires_at);
    state.wake_sync(Some(&id));
    state.outbox_wake.notify_one();
    events::accounts_changed(ctx);
    Ok(account)
}

/// `remove_account`: stop syncing, forget secrets and local mail. Mail on
/// the server is not touched.
pub fn remove_account(ctx: &Ctx, id: String) -> AppResult<()> {
    let state = &ctx.state;
    if let Some(engine) = lock(&state.engines).remove(&id) {
        engine.task.abort();
    }
    lock(&state.access_tokens).remove(&id);
    lock(&state.access_tokens).remove(&crate::connections::graph_key(&id));
    // The account's row goes first. If that fails, nothing is lost and the
    // account still works; had its password been deleted first, it would
    // be left listed but unable to sign in. A secret that can't be removed
    // afterwards is harmless without its account.
    state.store.delete_account(&id)?;
    for key in [
        password_key(&id),
        oauth_key(&id),
        crate::connections::graph_key(&id),
    ] {
        if let Err(error) = state.secrets.delete(&key) {
            eprintln!("couldn't remove a saved sign-in of a removed account: {error}");
        }
    }
    crate::signatures::remove(state, &id)?;
    events::accounts_changed(ctx);
    events::outbox_changed(ctx, state);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_chosen_at_the_provider_wins() {
        assert_eq!(
            chosen_address("", Some("don@gmail.com")).ok().as_deref(),
            Some("don@gmail.com")
        );
        // Picked a different account than the one typed: use the one signed in.
        assert_eq!(
            chosen_address("old@gmail.com", Some("don@gmail.com"))
                .ok()
                .as_deref(),
            Some("don@gmail.com")
        );
        assert_eq!(
            chosen_address("don@outlook.com", None).ok().as_deref(),
            Some("don@outlook.com")
        );
        assert_eq!(chosen_address("", None).map_err(|e| e.kind), Err("auth"));
    }

    #[test]
    fn a_null_sync_window_is_told_from_one_left_out() {
        let patch = |json: &str| serde_json::from_str::<AccountPatch>(json).ok();
        assert!(patch(r#"{"displayName":"Don"}"#).is_some_and(|p| p.sync_months.is_none()));
        assert!(patch(r#"{"syncMonths":null}"#).is_some_and(|p| p.sync_months == Some(None)));
        assert!(patch(r#"{"syncMonths":6}"#).is_some_and(|p| p.sync_months == Some(Some(6))));
    }

    #[test]
    fn email_and_name_cleanup() {
        assert_eq!(
            clean_email("  me@work.example ").ok().as_deref(),
            Some("me@work.example")
        );
        assert!(clean_email("nope").is_err());
        assert_eq!(clean_name("", "don@mocky.co.ke"), "don");
        assert_eq!(clean_name("Don\u{202E}\r\n", "x@y.co"), "Don");
    }
}
