//! Adding, changing and removing accounts. Nothing is saved until a real
//! login to both the incoming and outgoing server has worked.

use std::sync::Arc;

use mail_auth::Provider;
use mail_store::{Account, AuthKind, NewAccount};
use mail_sync::imap::ImapConnection;
use mail_sync::{Credentials, Security, ServerConfig, ServerSuggestion, smtp};
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::convert::{to_store, to_sync};
use crate::credentials::{cache_token, oauth_key, password_key};
use crate::error::{AppError, AppResult};
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPatch {
    display_name: Option<String>,
    password: Option<String>,
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
#[tauri::command]
pub async fn suggest_servers(email: String) -> Option<ServerSuggestion> {
    mail_sync::discover_servers(&email).await
}

/// `list_accounts`.
#[tauri::command]
pub fn list_accounts(state: State<'_, Arc<AppState>>) -> AppResult<Vec<Account>> {
    Ok(state.store.list_accounts()?)
}

/// `add_account`: password sign-in.
#[tauri::command]
pub async fn add_account(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    input: AddAccountInput,
) -> AppResult<Account> {
    let email = clean_email(&input.email)?;
    ensure_new(&state, &email)?;
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
        color: next_color(&state)?,
        imap: to_store(&input.imap),
        smtp: to_store(&input.smtp),
        size_limit_bytes: None,
        created_at: now(),
    };
    let account = save(&state, &new, password_key, &password)?;
    sync::start(&app, state.inner(), &account.id);
    events::accounts_changed(&app);
    Ok(account)
}

/// `add_oauth_account`: Google or Microsoft sign-in in the system browser.
#[tauri::command]
pub async fn add_oauth_account(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    input: AddOAuthInput,
) -> AppResult<Account> {
    let email = clean_email(&input.email)?;
    ensure_new(&state, &email)?;
    let (provider, auth) = match input.provider.as_str() {
        "google" => (Provider::Google, AuthKind::Google),
        "microsoft" => (Provider::Microsoft, AuthKind::Microsoft),
        _ => return Err(AppError::invalid("Unknown sign-in provider.")),
    };
    let tokens = mail_auth::authorize(provider, &email).await?;
    let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
        AppError::new(
            "auth",
            "The provider didn't grant offline access. Please try signing in again.",
        )
    })?;
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
    // The token says nothing reliable about which mailbox was chosen; logging
    // in to the typed address proves the grant covers that mailbox.
    let credentials = Credentials::OAuth2 {
        username: email.clone(),
        access_token: tokens.access_token.clone(),
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
        color: next_color(&state)?,
        imap: to_store(&imap),
        smtp: to_store(&smtp_server),
        size_limit_bytes: Some(25 * 1024 * 1024),
        created_at: now(),
    };
    let account = save(&state, &new, oauth_key, &refresh_token)?;
    cache_token(
        &state,
        &account.id,
        &tokens.access_token,
        tokens.expires_at_unix,
    );
    sync::start(&app, state.inner(), &account.id);
    events::accounts_changed(&app);
    Ok(account)
}

/// `update_account`: rename, or replace the password after testing it.
#[tauri::command]
pub async fn update_account(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    patch: AccountPatch,
) -> AppResult<Account> {
    let account = state.store.get_account(&id)?;
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
    events::accounts_changed(&app);
    Ok(state.store.get_account(&id)?)
}

/// `remove_account`: stop syncing, forget secrets and local mail. Mail on
/// the server is not touched.
#[tauri::command]
pub fn remove_account(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> AppResult<()> {
    if let Some(engine) = lock(&state.engines).remove(&id) {
        engine.task.abort();
    }
    lock(&state.access_tokens).remove(&id);
    state.secrets.delete(&password_key(&id))?;
    state.secrets.delete(&oauth_key(&id))?;
    state.store.delete_account(&id)?;
    events::accounts_changed(&app);
    events::outbox_changed(&app, &state);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
