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
