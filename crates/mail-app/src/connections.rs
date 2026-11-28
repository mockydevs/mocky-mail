//! Connecting Gmail and Microsoft accounts for calendar, contacts, tasks
//! and automatic replies (`connect_calendar_and_contacts`), and the access
//! tokens those services need.
//!
//! Gmail: the browser sign-in runs again asking for the extra Google
//! permissions on top of mail (incremental, so mail access stays). The new
//! refresh token replaces the old one in the keyring and covers both, so
//! one access token serves IMAP, `CalDAV`, `CardDAV` and Google's APIs.
//! On Android, Gmail accounts use Google Play services instead: its
//! consent sheet adds the permissions (incremental authorization) and
//! every later token is asked for with them. Signing in again
//! (`reconnect_account`) asks for them too, so they are never dropped.
//!
//! Microsoft: tokens are issued per resource, so Graph gets its own grant
//! and refresh token (`<account>:graph` in the keyring) beside the mail
//! one, which is left as it is.
//!
//! Which accounts are connected, with the permissions granted, is kept in
//! the database (`account_connections`).

use std::sync::Arc;

use mail_auth::Provider;
use mail_store::{Account, AuthKind};
use mail_sync::rest::{Api, RestClient};

use crate::Ctx;
use crate::credentials::{cache_token, oauth_key};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, lock, now};

/// Keyring key for an account's Microsoft Graph refresh token.
#[must_use]
pub fn graph_key(account_id: &str) -> String {
    format!("{account_id}:graph")
}

/// The in-memory token cache key for an account's Graph token.
fn graph_cache_key(account_id: &str) -> String {
    format!("{account_id}:graph")
}

/// Which provider service an account's calendar, contacts and tasks use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    /// The mail server's own `CalDAV`/`CardDAV` (password accounts).
    Dav,
    /// Google, connected.
    Google,
    /// Microsoft Graph, connected.
    Microsoft,
    /// A Gmail or Microsoft account not connected yet.
    NotConnected,
    /// An Exchange Server account: Exchange Web Services.
    Exchange,
}

/// How an account's calendar, contacts and tasks are reached.
///
/// # Errors
/// Database errors.
pub fn service(state: &AppState, account: &Account) -> AppResult<Service> {
    Ok(match account.auth {
        AuthKind::Password => Service::Dav,
        AuthKind::Exchange => Service::Exchange,
        _ if state.store.account_connection(&account.id)?.is_none() => Service::NotConnected,
        AuthKind::Google => Service::Google,
        AuthKind::Microsoft => Service::Microsoft,
    })
}

/// What an account not connected yet shows.
pub const NOT_CONNECTED: &str = "Connect this account's calendar and contacts to see them here.";

/// A Gmail or Microsoft account that can be connected now.
pub fn connectable(state: &AppState, account: &Account) -> bool {
    matches!(service(state, account), Ok(Service::NotConnected))
}

/// An access token for Google's services (the account's own grant,
/// which covers calendar and contacts once connected).
///
/// # Errors
/// `auth` when the sign-in was refused.
pub async fn google_token(state: &AppState, account: &Account) -> AppResult<String> {
    match crate::credentials::for_account(state, account).await? {
        mail_sync::Credentials::OAuth2 { access_token, .. } => Ok(access_token),
        mail_sync::Credentials::Password { .. } => Err(AppError::invalid(
            "This account doesn't sign in with Google.",
        )),
    }
}

/// An access token for Microsoft Graph.
///
/// # Errors
/// `auth` when the Graph grant is missing or was refused.
pub async fn graph_token(state: &AppState, account: &Account) -> AppResult<String> {
    let key = graph_cache_key(&account.id);
    let cached = lock(&state.access_tokens).get(&key).cloned();
    if let Some((token, expires_at)) = cached
        && expires_at > now().saturating_add(120)
    {
        return Ok(token);
    }
    let refresh = state.secrets.get(&graph_key(&account.id))?.ok_or_else(|| {
        AppError::new(
            "auth",
            "Connect this account's calendar and contacts again.",
        )
    })?;
    let tokens = match mail_auth::refresh_graph(&refresh).await {
        Ok(tokens) => tokens,
        Err(mail_auth::AuthError::Reauthorize) => {
            return Err(AppError::new(
                "auth",
                "Microsoft wants you to connect this account's calendar and contacts again.",
            ));
        }
        Err(other) => return Err(other.into()),
    };
    if let Some(rotated) = tokens.refresh_token.as_deref() {
        state.secrets.set(&graph_key(&account.id), rotated)?;
    }
    cache_token(state, &key, &tokens.access_token, tokens.expires_at_unix);
    Ok(tokens.access_token)
}

/// A Google API client for a connected account.
///
/// # Errors
/// As [`google_token`].
pub async fn google_client(state: &AppState, account: &Account) -> AppResult<RestClient> {
    Ok(RestClient::new(
        Api::Google,
        google_token(state, account).await?,
    )?)
}

/// A Graph client for a connected account.
///
/// # Errors
/// As [`graph_token`].
pub async fn graph_client(state: &AppState, account: &Account) -> AppResult<RestClient> {
    Ok(RestClient::new(
        Api::Graph,
        graph_token(state, account).await?,
    )?)
}

/// The user's offset from UTC now (seconds east), for dates sent to Google
/// and Microsoft.
#[must_use]
pub fn local_offset() -> i32 {
    use chrono::Offset as _;
    chrono::Local::now().offset().fix().local_minus_utc()
}

/// Plain words for a refused connection.
fn connect_error(provider: Provider, error: mail_auth::AuthError) -> AppError {
    match (provider, error) {
        (Provider::Google, mail_auth::AuthError::NotEnabled) => AppError::new(
            "config",
            "Google hasn't enabled calendar access for Mocky Mail yet. Your mail is not affected; try again after the app's Google setup is finished.",
        ),
        (Provider::Microsoft, mail_auth::AuthError::NotEnabled) => AppError::new(
            "config",
            "Microsoft hasn't allowed calendar access for Mocky Mail yet. A work account's administrator may need to allow it; your mail is not affected.",
        ),
        (_, other) => other.into(),
    }
}

/// `connect_calendar_and_contacts`: sign in again in the browser asking for
/// calendar, contacts, tasks and automatic-reply access, then sync them.
pub async fn connect_calendar_and_contacts(ctx: &Ctx, account_id: String) -> AppResult<()> {
    let state = &ctx.state;
    let account = state.store.get_account(&account_id)?;
    let provider = match account.auth {
        AuthKind::Google => Provider::Google,
        AuthKind::Microsoft => Provider::Microsoft,
        AuthKind::Password | AuthKind::Exchange => {
            return Err(AppError::invalid(
                "This account's calendar and contacts come from its mail server; there is nothing to connect.",
            ));
        }
    };
    // Gmail accounts added with Android's own Google sign-in: Google Play
    // services asks for the extra permissions (incremental authorization)
    // and hands back one token covering mail and them; no refresh token.
    if provider == Provider::Google
        && state.secrets.get(&oauth_key(&account.id))?.as_deref()
            == Some(crate::credentials::ANDROID_GOOGLE)
    {
        return connect_android_google(ctx, state, &account).await;
    }
    let opener = Arc::clone(&ctx.host);
    let tokens =
        mail_auth::authorize_calendar_and_contacts(
            provider,
            &account.email,
            move |url| match opener.open_url(url) {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("couldn't open the browser to connect: {error}");
                    false
                }
            },
        )
        .await
        .map_err(|e| connect_error(provider, e))?;
    if let Some(granted) = tokens.granted_email.as_deref()
        && !granted.eq_ignore_ascii_case(&account.email)
    {
        return Err(AppError::new(
            "auth",
            format!(
                "You signed in as {granted}, but this account is {}. Connect with that account.",
                account.email
            ),
        ));
    }
    let refresh = tokens.refresh_token.clone().ok_or_else(|| {
        AppError::new(
            "auth",
            "The provider didn't grant offline access. Please try connecting again.",
        )
    })?;
    let scopes = keep_grant(state, &account, provider, &tokens, &refresh)?;
    connected(ctx, state, &account, &scopes)
}

/// Record a connection and sync what was connected, in the background.
fn connected(
    ctx: &Ctx,
    state: &Arc<AppState>,
    account: &Account,
    scopes: &[String],
) -> AppResult<()> {
    state
        .store
        .set_account_connection(&account.id, scopes, i64::try_from(now()).unwrap_or(0))?;
    crate::events::accounts_changed(ctx);
    let (task_ctx, task_state) = (ctx.clone(), Arc::clone(state));
    ctx.spawn(async move {
        crate::calendar::sync_all(&task_ctx, &task_state).await;
        crate::contacts::sync_all(&task_ctx, &task_state).await;
        crate::tasks::sync_all(&task_ctx, &task_state).await;
    });
    Ok(())
}

/// Whether Google's list of granted scopes covers calendar and mail. An
/// empty list (Google didn't say) counts as yes.
fn google_grant_ok(scopes: &[String]) -> AppResult<()> {
    if !scopes.is_empty()
        && !scopes
            .iter()
            .any(|s| s == "https://www.googleapis.com/auth/calendar")
    {
        return Err(AppError::new(
            "auth",
            "Calendar access wasn't allowed. Connect again and tick every box Google shows.",
        ));
    }
    if !scopes.is_empty() && !scopes.iter().any(|s| s == "https://mail.google.com/") {
        return Err(AppError::new(
            "auth",
            "Mail access was taken away in that sign-in. Connect again and allow mail too.",
        ));
    }
    Ok(())
}

/// Connect a Gmail account added with Android's own Google sign-in: Google
/// Play services shows its consent sheet for the extra permissions and
/// returns one access token for mail and them (renewed later the same way,
/// with no screen; see `credentials::android_google`).
async fn connect_android_google(
    ctx: &Ctx,
    state: &Arc<AppState>,
    account: &Account,
) -> AppResult<()> {
    if !ctx.host.device_google() {
        // Such accounts only exist on Android.
        return Err(AppError::new(
            "auth",
            "Sign this account in again, then connect its calendar and contacts.",
        ));
    }
    let extra: Vec<String> = mail_auth::GOOGLE_EXTRA_SCOPES
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let granted = ctx
        .host
        .device_google_authorize(&account.email, &extra)
        .await
        .map_err(|e| {
            if e.kind == "config" {
                AppError::new(
                    "config",
                    "Google hasn't enabled calendar access for Mocky Mail yet. Your mail is not affected; try again after the app's Google setup is finished.",
                )
            } else {
                e
            }
        })?;
    google_grant_ok(&granted.granted_scopes)?;
    cache_token(
        state,
        &account.id,
        &granted.access_token,
        now().saturating_add(crate::host::DEVICE_GOOGLE_TOKEN_SECS),
    );
    let scopes = if granted.granted_scopes.is_empty() {
        extra
    } else {
        granted.granted_scopes
    };
    connected(ctx, state, account, &scopes)
}

/// Whether a granted scope list includes one ending in `name`.
pub fn granted(state: &AppState, account: &Account, name: &str) -> bool {
    state
        .store
        .account_connection(&account.id)
        .ok()
        .flatten()
        .is_some_and(|scopes| scopes.iter().any(|s| s.ends_with(name)))
}

/// Keep a new grant: its refresh token in the keyring, its access token
/// in memory. Returns the permissions granted.
fn keep_grant(
    state: &AppState,
    account: &Account,
    provider: Provider,
    tokens: &mail_auth::OAuthTokens,
    refresh: &str,
) -> AppResult<Vec<String>> {
    let mut scopes = tokens.granted_scopes.clone();
    match provider {
        Provider::Google => {
            // Mail must still be covered, or the account would stop syncing.
            google_grant_ok(&scopes)?;
            state.secrets.set(&oauth_key(&account.id), refresh)?;
            cache_token(
                state,
                &account.id,
                &tokens.access_token,
                tokens.expires_at_unix,
            );
        }
        Provider::Microsoft => {
            state.secrets.set(&graph_key(&account.id), refresh)?;
            cache_token(
                state,
                &graph_cache_key(&account.id),
                &tokens.access_token,
                tokens.expires_at_unix,
            );
        }
    }
    if scopes.is_empty() {
        scopes = match provider {
            Provider::Google => mail_auth::GOOGLE_EXTRA_SCOPES
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            Provider::Microsoft => mail_auth::GRAPH_SCOPES
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        };
    }
    Ok(scopes)
}
