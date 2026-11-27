//! Turning a stored account into login credentials: the password from the
//! keyring, or a fresh OAuth access token from the stored refresh token.

use mail_auth::Provider;
use mail_store::{Account, AuthKind};
use mail_sync::Credentials;

use crate::error::{AppError, AppResult};
use crate::state::{AppState, lock, now};

/// Keyring key for an account's password.
pub fn password_key(account_id: &str) -> String {
    account_id.to_owned()
}

/// Keyring key for an account's OAuth refresh token.
pub fn oauth_key(account_id: &str) -> String {
    format!("{account_id}:oauth")
}

/// The OAuth provider for an account, if it uses OAuth.
pub const fn provider(auth: AuthKind) -> Option<Provider> {
    match auth {
        AuthKind::Password => None,
        AuthKind::Google => Some(Provider::Google),
        AuthKind::Microsoft => Some(Provider::Microsoft),
    }
}

/// Stored instead of a refresh token for Gmail accounts added with
/// Android's own Google sign-in: Android hands out fresh tokens itself.
pub const ANDROID_GOOGLE: &str = "android-google-play-services";

/// Seconds of validity a cached token must have left to be reused.
const TOKEN_MARGIN: u64 = 120;

/// Remember a fresh access token in memory.
pub fn cache_token(state: &AppState, account_id: &str, token: &str, expires_at: u64) {
    lock(&state.access_tokens).insert(account_id.to_owned(), (token.to_owned(), expires_at));
}

/// Credentials for logging in to this account now.
///
/// # Errors
/// `auth` when the password or refresh token is missing or refused.
pub async fn for_account(state: &AppState, account: &Account) -> AppResult<Credentials> {
    let username = account.email.clone();
    let Some(provider) = provider(account.auth) else {
        let password = state
            .secrets
            .get(&password_key(&account.id))?
            .ok_or_else(|| {
                AppError::new(
                    "auth",
                    "The password for this account is missing. Update it in Settings.",
                )
            })?;
        return Ok(Credentials::Password { username, password });
    };

    let cached = lock(&state.access_tokens).get(&account.id).cloned();
    if let Some((token, expires_at)) = cached
        && expires_at > now().saturating_add(TOKEN_MARGIN)
    {
        return Ok(Credentials::OAuth2 {
            username,
            access_token: token,
        });
    }
    let refresh_token = state
        .secrets
        .get(&oauth_key(&account.id))?
        .ok_or_else(|| AppError::new("auth", "Sign in again to reconnect this account."))?;
    if refresh_token == ANDROID_GOOGLE {
        return android_google(state, account).await;
    }
    let tokens = mail_auth::refresh(provider, &refresh_token).await?;
    if let Some(rotated) = tokens.refresh_token.as_deref() {
        state.secrets.set(&oauth_key(&account.id), rotated)?;
    }
    cache_token(
        state,
        &account.id,
        &tokens.access_token,
        tokens.expires_at_unix,
    );
    Ok(Credentials::OAuth2 {
        username,
        access_token: tokens.access_token,
    })
}

/// A Gmail account added with Android's own Google sign-in: ask Android for
/// a fresh token, with no screen shown.
#[cfg(target_os = "android")]
async fn android_google(state: &AppState, account: &Account) -> AppResult<Credentials> {
    let extra = kept_scopes(state, account);
    let access_token = crate::google_android::token(&account.email, &extra).await?;
    cache_token(
        state,
        &account.id,
        &access_token,
        now().saturating_add(crate::google_android::TOKEN_LIFETIME_SECS),
    );
    Ok(Credentials::OAuth2 {
        username: account.email.clone(),
        access_token,
    })
}

/// Such accounts only exist on Android; elsewhere they must sign in again.
#[cfg(not(target_os = "android"))]
#[allow(clippy::unused_async, reason = "same signature as the Android version")]
async fn android_google(_state: &AppState, _account: &Account) -> AppResult<Credentials> {
    Err(AppError::new(
        "auth",
        "Sign in again to reconnect this account.",
    ))
}

/// The permissions beyond mail an account was granted when it connected
/// calendar and contacts, which signing in again (or renewing a token on
/// Android) must ask for too, so they are never dropped.
pub fn kept_scopes(state: &AppState, account: &Account) -> Vec<String> {
    let Some(provider) = provider(account.auth) else {
        return Vec::new();
    };
    match state.store.account_connection(&account.id) {
        Ok(Some(granted)) => mail_auth::kept_scopes(provider, &granted),
        _ => Vec::new(),
    }
}
