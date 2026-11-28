//! Native OAuth for Gmail and Outlook. Tokens stay in the native process.
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use reqwest::{Client, redirect::Policy};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    time::timeout,
};
use url::Url;

/// Supported hosted mailbox provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// Gmail and Google Workspace.
    Google,
    /// Outlook.com and Microsoft 365.
    Microsoft,
}
/// Mail endpoints. IMAP always uses implicit TLS.
pub struct MailServers {
    /// IMAP hostname (port 993).
    pub imap: &'static str,
    /// SMTP hostname.
    pub smtp: &'static str,
    /// SMTP port.
    pub smtp_port: u16,
    /// Require STARTTLS before authentication on SMTP.
    pub smtp_starttls: bool,
}
impl Provider {
    /// TLS settings for this provider.
    #[must_use]
    pub fn servers(self) -> MailServers {
        match self {
            Self::Google => MailServers {
                imap: "imap.gmail.com",
                smtp: "smtp.gmail.com",
                smtp_port: 465,
                smtp_starttls: false,
            },
            Self::Microsoft => MailServers {
                imap: "outlook.office365.com",
                smtp: "smtp.office365.com",
                smtp_port: 587,
                smtp_starttls: true,
            },
        }
    }
    fn config(self) -> Result<Config<'static>, AuthError> {
        let (id, secret, authority, token, scope, host) = match self {
            Self::Google => (
                option_env!("MOCKY_MAIL_GOOGLE_CLIENT_ID"),
                option_env!("MOCKY_MAIL_GOOGLE_CLIENT_SECRET"),
                "https://accounts.google.com/o/oauth2/v2/auth",
                "https://oauth2.googleapis.com/token",
                "openid email https://mail.google.com/",
                "127.0.0.1",
            ),
            Self::Microsoft => (
                option_env!("MOCKY_MAIL_MS_CLIENT_ID"),
                None,
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                "openid email offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send",
                "localhost",
            ),
        };
        if self == Self::Google && secret.is_none_or(str::is_empty) {
            return Err(AuthError::NotConfigured(self));
        }
        Ok(Config {
            id: id
                .filter(|s| !s.is_empty())
                .ok_or(AuthError::NotConfigured(self))?,
            secret,
            authority,
            token,
            scope,
            host,
        })
    }
}
struct Config<'a> {
    id: &'a str,
    secret: Option<&'a str>,
    authority: &'a str,
    token: &'a str,
    scope: &'a str,
    host: &'a str,
}
/// Guess only known consumer domains. Enterprise domains require explicit selection.
#[must_use]
pub fn provider_for_address(email: &str) -> Option<Provider> {
    let (name, domain) = email.trim().split_once('@')?;
    if name.is_empty() {
        return None;
    }
    match domain.to_ascii_lowercase().as_str() {
        "gmail.com" | "googlemail.com" => Some(Provider::Google),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => Some(Provider::Microsoft),
        _ => None,
    }
}
/// Secret-bearing response; deliberately has no Debug or serialization implementation.
pub struct OAuthTokens {
    /// Bearer credential.
    pub access_token: String,
    /// Replace the stored refresh token only if one is returned.
    pub refresh_token: Option<String>,
    /// Unix seconds at expiry.
    pub expires_at_unix: u64,
    /// Verified identity if available. A login hint alone never proves identity.
    pub granted_email: Option<String>,
    /// The permissions granted, as the provider listed them (`scope`).
    pub granted_scopes: Vec<String>,
}

/// What Gmail accounts are asked for on top of mail to connect calendar,
/// contacts, tasks and automatic replies.
pub const GOOGLE_EXTRA_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/carddav",
    "https://www.googleapis.com/auth/contacts",
    "https://www.googleapis.com/auth/tasks",
    "https://www.googleapis.com/auth/gmail.settings.basic",
];

/// The Microsoft Graph permissions asked for to connect calendar,
/// contacts, tasks and automatic replies. Microsoft issues a token per
/// resource, so this is a separate grant from the mail (IMAP/SMTP) one.
pub const GRAPH_SCOPES: &[&str] = &[
    "https://graph.microsoft.com/Calendars.ReadWrite",
    "https://graph.microsoft.com/Contacts.ReadWrite",
    "https://graph.microsoft.com/Tasks.ReadWrite",
    "https://graph.microsoft.com/MailboxSettings.ReadWrite",
];

/// The scope string for connecting calendar and contacts.
fn extra_scope(provider: Provider, mail_scope: &str) -> String {
    match provider {
        // Incremental: mail stays granted, the rest is added.
        Provider::Google => format!("{mail_scope} {}", GOOGLE_EXTRA_SCOPES.join(" ")),
        Provider::Microsoft => format!("openid email offline_access {}", GRAPH_SCOPES.join(" ")),
    }
}

/// The extra permissions (beyond mail sign-in) in a list of granted
/// scopes that a mail sign-in must ask for again so they aren't dropped:
/// Google's calendar, contacts, tasks and settings scopes. Microsoft's are
/// a separate grant, untouched by a mail sign-in, so none.
#[must_use]
pub fn kept_scopes(provider: Provider, granted: &[String]) -> Vec<String> {
    match provider {
        Provider::Microsoft => Vec::new(),
        Provider::Google => {
            let mut out: Vec<String> = Vec::new();
            for scope in granted {
                let scope = scope.trim();
                if scope.starts_with("https://www.googleapis.com/auth/")
                    && !out.iter().any(|s| s == scope)
                {
                    out.push(scope.to_owned());
                }
            }
            out
        }
    }
}

/// The scope string for a mail sign-in that keeps `extra` permissions.
fn mail_scope_with(provider: Provider, mail_scope: &str, extra: &[String]) -> String {
    let kept = kept_scopes(provider, extra);
    let mut scope = mail_scope.to_owned();
    for s in kept {
        if !scope.split_whitespace().any(|have| have == s) {
            scope.push(' ');
            scope.push_str(&s);
        }
    }
    scope
}

/// [`authorize`] for an account that connected calendar and contacts:
/// `granted` (the scopes it was granted) are asked for again with mail, so
/// signing in again never drops them (Google, incrementally).
///
/// # Errors
/// As [`authorize`].
pub async fn authorize_keeping(
    provider: Provider,
    login_hint: &str,
    granted: &[String],
    open_browser: impl FnOnce(&str) -> bool,
) -> Result<OAuthTokens, AuthError> {
    let base = provider.config()?;
    let scope = mail_scope_with(provider, base.scope, granted);
    let incremental = scope != base.scope;
    let cfg = Config {
        scope: &scope,
        ..base
    };
    authorize_with_options(provider, &cfg, login_hint, incremental, open_browser).await
}

/// Sign in again in the browser, asking for calendar, contacts, tasks and
/// automatic replies as well (Google: incrementally, with
/// `include_granted_scopes`, so mail access stays; Microsoft: a Graph grant
/// beside the mail one).
///
/// # Errors
/// As [`authorize`]; [`AuthError::NotEnabled`] when the provider refuses
/// the permissions (not set up for the app).
pub async fn authorize_calendar_and_contacts(
    provider: Provider,
    login_hint: &str,
    open_browser: impl FnOnce(&str) -> bool,
) -> Result<OAuthTokens, AuthError> {
    let base = provider.config()?;
    let scope = extra_scope(provider, base.scope);
    let cfg = Config {
        id: base.id,
        secret: base.secret,
        authority: base.authority,
        token: base.token,
        scope: &scope,
        host: base.host,
    };
    authorize_with_options(provider, &cfg, login_hint, true, open_browser).await
}

/// Refresh the Microsoft Graph grant (stored apart from the mail one).
///
/// # Errors
/// As [`refresh`].
pub async fn refresh_graph(refresh_token: &str) -> Result<OAuthTokens, AuthError> {
    let base = Provider::Microsoft.config()?;
    let scope = extra_scope(Provider::Microsoft, base.scope);
    let cfg = Config {
        scope: &scope,
        ..base
    };
    exchange(
        &cfg,
        vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", &scope),
        ],
    )
    .await
}
/// User-safe errors. Never include raw HTTP responses, callback URLs or tokens.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    /// Missing build-time registration.
    #[error("{0:?} OAuth is not configured")]
    NotConfigured(Provider),
    /// User declined or cancelled.
    #[error("Sign-in was declined")]
    Denied,
    /// Invalid, expired or revoked grant.
    #[error("Sign in again to renew mailbox access")]
    Reauthorize,
    /// The local callback listener (or another local step) failed.
    #[error("Cannot start the local sign-in callback")]
    Local,
    /// The system browser could not be opened for sign-in.
    #[error("Cannot open a web browser to sign in")]
    Browser,
    /// Sign-in timed out.
    #[error("Sign-in timed out")]
    Timeout,
    /// Token endpoint unreachable or temporarily unavailable.
    #[error("The sign-in service is unavailable")]
    Network,
    /// The provider hasn't allowed what was asked (an API or permission
    /// not set up for the app yet).
    #[error("The provider hasn't allowed this access for Mocky Mail yet")]
    NotEnabled,
    /// Malformed response.
    #[error("The sign-in service returned an invalid response")]
    Protocol,
    /// Microsoft 365: the organisation lets only an administrator approve
    /// new apps (AADSTS65001, AADSTS90094, "Need admin approval").
    /// [`admin_consent_url`] is the page the administrator opens.
    #[error(
        "Your organisation lets only an administrator approve new apps. Ask your IT administrator to approve Mocky Mail (send them the approval link), then sign in again."
    )]
    AdminConsent {
        /// The organisation's tenant id, when the error named it.
        tenant: Option<String>,
    },
}

/// Microsoft's sign-in error codes that mean an administrator must approve
/// the app first: consent not given, the grant needs an administrator, or
/// an approval request is waiting for one.
const ADMIN_CONSENT_CODES: &[&str] = &["AADSTS65001", "AADSTS90094", "AADSTS90095"];

/// Whether a sign-in error (its `error` and `error_description`) says an
/// administrator must approve the app.
fn needs_admin_consent(error: Option<&str>, description: Option<&str>) -> bool {
    let description = description.unwrap_or_default();
    let lower = description.to_ascii_lowercase();
    error == Some("consent_required")
        || ADMIN_CONSENT_CODES
            .iter()
            .any(|code| description.contains(code))
        || lower.contains("admin approval")
        || lower.contains("admin consent")
        || lower.contains("requires admin permission")
}

/// A tenant id (a GUID) named after the word "tenant" in an error.
fn tenant_in(description: &str) -> Option<String> {
    let lower = description.to_ascii_lowercase();
    let at = lower.find("tenant")?;
    let after = lower.get(at..)?;
    after
        .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .take(8)
        .find(|word| is_guid(word))
        .map(str::to_owned)
}

fn is_guid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8usize, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && group.chars().all(|c| c.is_ascii_hexdigit()))
}

fn admin_consent(description: Option<&str>) -> AuthError {
    AuthError::AdminConsent {
        tenant: description.and_then(tenant_in),
    }
}

/// The page where an administrator of a Microsoft 365 organisation approves
/// Mocky Mail for everyone there: the tenant's own when known, else
/// `organizations` (the administrator signs in and their tenant is used).
/// Only the app's client id is in it, nothing about the user. `None` when
/// this build has no Microsoft registration.
#[must_use]
pub fn admin_consent_url(tenant: Option<&str>) -> Option<String> {
    let client_id = option_env!("MOCKY_MAIL_MS_CLIENT_ID").filter(|id| !id.is_empty())?;
    Some(admin_consent_url_for(client_id, tenant))
}

fn admin_consent_url_for(client_id: &str, tenant: Option<&str>) -> String {
    let tenant = tenant.filter(|t| is_guid(t)).unwrap_or("organizations");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", client_id)
        .finish();
    format!("https://login.microsoftonline.com/{tenant}/adminconsent?{query}")
}
fn nonce() -> Result<String, AuthError> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::Local)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn callback(line: &str, expected: &str) -> Option<Result<String, AuthError>> {
    let mut words = line.split_whitespace();
    if words.next()? != "GET" {
        return None;
    }
    let path = words.next()?;
    let query = path.strip_prefix("/?")?;
    let mut state = None;
    let mut code = None;
    let mut error = None;
    let mut description = None;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        let slot = match k.as_ref() {
            "state" => &mut state,
            "code" => &mut code,
            "error" => &mut error,
            "error_description" => &mut description,
            _ => continue,
        };
        if slot.replace(v.into_owned()).is_some() {
            return None;
        }
    }
    if state.as_deref() != Some(expected) {
        return None;
    }
    Some(match (code, error.as_deref()) {
        (Some(c), None) if !c.is_empty() => Ok(c),
        (None, error @ Some(_)) if needs_admin_consent(error, description.as_deref()) => {
            Err(admin_consent(description.as_deref()))
        }
        (None, Some("access_denied")) => Err(AuthError::Denied),
        (None, Some("invalid_scope" | "unauthorized_client")) => Err(AuthError::NotEnabled),
        _ => Err(AuthError::Protocol),
    })
}
async fn wait_code(listener: &TcpListener, state: &str) -> Result<String, AuthError> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|_| AuthError::Local)?;
        let mut first = String::new();
        let read = timeout(
            Duration::from_secs(2),
            BufReader::new((&mut stream).take(8192)).read_line(&mut first),
        )
        .await;
        let result = if matches!(read, Ok(Ok(_))) && first.ends_with('\n') {
            callback(&first, state)
        } else {
            None
        };
        let response = if result.is_some() {
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nReturn to Mocky Mail to continue."
        } else {
            "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n"
        };
        let _ = stream.write_all(response.as_bytes()).await;
        if let Some(code) = result {
            return code;
        }
    }
}
/// Open the system browser and exchange its one-time authorization code.
///
/// `open_browser` shows the sign-in page in the user's browser and returns
/// whether it could. The app passes its own opener, because how to open a
/// browser differs by platform (on Android it is an intent, not a program).
///
/// # Errors
/// Returns a user-safe error for missing configuration, cancellation or provider failure.
pub async fn authorize(
    provider: Provider,
    login_hint: &str,
    open_browser: impl FnOnce(&str) -> bool,
) -> Result<OAuthTokens, AuthError> {
    let cfg = provider.config()?;
    authorize_with(provider, &cfg, login_hint, open_browser).await
}

async fn authorize_with(
    provider: Provider,
    cfg: &Config<'_>,
    login_hint: &str,
    open_browser: impl FnOnce(&str) -> bool,
) -> Result<OAuthTokens, AuthError> {
    authorize_with_options(provider, cfg, login_hint, false, open_browser).await
}

async fn authorize_with_options(
    provider: Provider,
    cfg: &Config<'_>,
    login_hint: &str,
    incremental: bool,
    open_browser: impl FnOnce(&str) -> bool,
) -> Result<OAuthTokens, AuthError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|_| AuthError::Local)?;
    let redirect = format!(
        "http://{}:{}/",
        cfg.host,
        listener.local_addr().map_err(|_| AuthError::Local)?.port()
    );
    let verifier = nonce()?;
    let state = nonce()?;
    let mut url = Url::parse(cfg.authority).map_err(|_| AuthError::Protocol)?;
    {
        let mut q = url.query_pairs_mut();
        q.extend_pairs([
            ("client_id", cfg.id),
            ("response_type", "code"),
            ("scope", cfg.scope),
            ("redirect_uri", &redirect),
            ("state", &state),
            ("code_challenge", &challenge(&verifier)),
            ("code_challenge_method", "S256"),
        ]);
        if !login_hint.is_empty() {
            q.append_pair("login_hint", login_hint);
        }
        // Show the provider's account chooser, so the user picks one of the
        // accounts already on the device instead of typing an address.
        if provider == Provider::Google {
            q.append_pair("access_type", "offline")
                .append_pair("prompt", "select_account consent");
            if incremental {
                q.append_pair("include_granted_scopes", "true");
            }
        } else {
            q.append_pair("prompt", "select_account");
        }
    }
    if !open_browser(url.as_str()) {
        return Err(AuthError::Browser);
    }
    let code = timeout(Duration::from_mins(5), wait_code(&listener, &state))
        .await
        .map_err(|_| AuthError::Timeout)??;
    exchange(
        cfg,
        vec![
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("code_verifier", &verifier),
        ],
    )
    .await
}
/// Refresh a grant; an omitted replacement refresh token means keep the current one.
///
/// # Errors
/// Returns Reauthorize for a revoked grant or Network for transport failure.
pub async fn refresh(provider: Provider, refresh_token: &str) -> Result<OAuthTokens, AuthError> {
    exchange(
        &provider.config()?,
        vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    )
    .await
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    token_type: String,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}
#[derive(Deserialize)]
struct IdClaims {
    email: Option<String>,
    email_verified: Option<bool>,
    preferred_username: Option<String>,
}
/// The mailbox address the user chose, from the ID token's claims.
///
/// The token comes straight from the provider's token endpoint over TLS, so
/// its signature isn't checked here (`OpenID` Connect 3.1.3.7). It only
/// names the account; the app then logs in to that mailbox with the access
/// token, which is what proves the grant really covers it.
fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let claims: IdClaims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    let email = match claims.email {
        Some(email) if claims.email_verified != Some(false) => Some(email),
        // Microsoft work accounts often leave out `email`; their sign-in
        // name is the mailbox address.
        _ => claims.preferred_username,
    }?;
    let email = email.trim();
    (email.contains('@') && !email.contains(char::is_whitespace)).then(|| email.to_owned())
}
#[derive(Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// What a refused token request means.
fn token_error(error: &TokenError) -> AuthError {
    let description = error.error_description.as_deref();
    if needs_admin_consent(Some(&error.error), description) {
        return admin_consent(description);
    }
    match error.error.as_str() {
        "invalid_grant" => AuthError::Reauthorize,
        "access_denied" => AuthError::Denied,
        "invalid_scope" | "unauthorized_client" => AuthError::NotEnabled,
        _ => AuthError::Protocol,
    }
}
fn expires(now: u64, lifetime: u64) -> Result<u64, AuthError> {
    now.checked_add(lifetime)
        .filter(|_| lifetime > 0)
        .ok_or(AuthError::Protocol)
}
async fn exchange(
    cfg: &Config<'_>,
    mut fields: Vec<(&str, &str)>,
) -> Result<OAuthTokens, AuthError> {
    fields.push(("client_id", cfg.id));
    if let Some(secret) = cfg.secret {
        fields.push(("client_secret", secret));
    }
    let response = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| AuthError::Local)?
        .post(cfg.token)
        .form(&fields)
        .send()
        .await
        .map_err(|_| AuthError::Network)?;
    if !response.status().is_success() {
        if response.status().is_server_error() || response.status().as_u16() == 429 {
            return Err(AuthError::Network);
        }
        return Err(response
            .json::<TokenError>()
            .await
            .ok()
            .as_ref()
            .map_or(AuthError::Protocol, token_error));
    }
    let token = response
        .json::<TokenResponse>()
        .await
        .map_err(|_| AuthError::Protocol)?;
    if token.access_token.is_empty() || !token.token_type.eq_ignore_ascii_case("bearer") {
        return Err(AuthError::Protocol);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthError::Local)?
        .as_secs();
    Ok(OAuthTokens {
        access_token: token.access_token,
        refresh_token: token.refresh_token.filter(|v| !v.is_empty()),
        expires_at_unix: expires(now, token.expires_in)?,
        granted_email: token.id_token.as_deref().and_then(email_from_id_token),
        granted_scopes: token
            .scope
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    })
}
/// Base64-encoded SASL XOAUTH2 initial response, ready for a wire protocol.
/// Pass the decoded bytes to SASL libraries which perform their own base64 encoding.
#[must_use]
pub fn xoauth2(user: &str, access_token: &str) -> String {
    STANDARD.encode(format!("user={user}\x01auth=Bearer {access_token}\x01\x01"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_config() -> Config<'static> {
        Config {
            id: "test-client",
            secret: None,
            authority: "https://sign-in.invalid/authorize",
            token: "https://sign-in.invalid/token",
            scope: "email",
            host: "127.0.0.1",
        }
    }

    #[tokio::test]
    async fn a_browser_that_will_not_open_says_so() {
        let result = authorize_with(Provider::Google, &test_config(), "a@x.co", |_| false).await;
        assert!(matches!(result, Err(AuthError::Browser)));
    }

    #[tokio::test]
    async fn the_page_opened_comes_back_to_the_app() {
        // Plays the browser: reads the sign-in URL it was asked to open, then
        // follows the redirect back to the app's local callback, as if the
        // user had pressed "Cancel" at the provider.
        let opened = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen = std::sync::Arc::clone(&opened);
        let result = authorize_with(Provider::Google, &test_config(), "a@x.co", move |url| {
            *seen.lock().unwrap() = url.to_owned();
            let page = Url::parse(url).unwrap();
            let param = |name: &str| {
                page.query_pairs()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.into_owned())
                    .unwrap()
            };
            let redirect = Url::parse(&param("redirect_uri")).unwrap();
            let state = param("state");
            tokio::spawn(async move {
                let mut stream = tokio::net::TcpStream::connect((
                    redirect.host_str().unwrap(),
                    redirect.port().unwrap(),
                ))
                .await
                .unwrap();
                let request = format!("GET /?state={state}&error=access_denied HTTP/1.1\r\n\r\n");
                stream.write_all(request.as_bytes()).await.unwrap();
            });
            true
        })
        .await;
        assert!(matches!(result, Err(AuthError::Denied)));
        let url = opened.lock().unwrap().clone();
        assert!(url.starts_with("https://sign-in.invalid/authorize?"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("login_hint=a%40x.co"));
    }

    #[test]
    fn an_organisation_that_needs_its_administrator_is_told_apart() {
        // The browser comes back with Microsoft's admin-consent errors.
        for request in [
            "GET /?state=x&error=consent_required HTTP/1.1",
            "GET /?state=x&error=access_denied&error_description=AADSTS90094%3A+The+grant+requires+admin+permission. HTTP/1.1",
            "GET /?state=x&error=invalid_request&error_description=AADSTS65001%3A+The+user+or+administrator+has+not+consented HTTP/1.1",
        ] {
            assert_eq!(
                callback(request, "x"),
                Some(Err(AuthError::AdminConsent { tenant: None })),
                "{request}"
            );
        }
        // A user who just pressed Cancel is not sent to an administrator.
        assert_eq!(
            callback(
                "GET /?state=x&error=access_denied&error_description=AADSTS65004%3A+User+declined+to+consent HTTP/1.1",
                "x"
            ),
            Some(Err(AuthError::Denied))
        );
        // The token endpoint says so too, sometimes naming the tenant.
        let refused = |error: &str, description: Option<&str>| {
            token_error(&TokenError {
                error: error.into(),
                error_description: description.map(str::to_owned),
            })
        };
        assert_eq!(
            refused(
                "invalid_grant",
                Some(
                    "AADSTS65001: The user or administrator has not consented to use the application with ID 'abc' in tenant '72f988bf-86f1-41af-91ab-2d7cd011db47'. Trace ID: 0b0e8c6a-1111-2222-3333-444455556666"
                )
            ),
            AuthError::AdminConsent {
                tenant: Some("72f988bf-86f1-41af-91ab-2d7cd011db47".into())
            }
        );
        assert_eq!(
            refused("invalid_grant", Some("AADSTS70008: expired")),
            AuthError::Reauthorize
        );
        assert_eq!(refused("invalid_scope", None), AuthError::NotEnabled);
        assert_eq!(refused("weird", None), AuthError::Protocol);
    }

    #[test]
    fn the_approval_page_has_only_the_app_and_the_tenant() {
        assert_eq!(
            admin_consent_url_for("client-1", None),
            "https://login.microsoftonline.com/organizations/adminconsent?client_id=client-1"
        );
        assert_eq!(
            admin_consent_url_for("client-1", Some("72f988bf-86f1-41af-91ab-2d7cd011db47")),
            "https://login.microsoftonline.com/72f988bf-86f1-41af-91ab-2d7cd011db47/adminconsent?client_id=client-1"
        );
        // Anything but a tenant id is not put in the address.
        assert_eq!(
            admin_consent_url_for("client-1", Some("../evil?x=1")),
            "https://login.microsoftonline.com/organizations/adminconsent?client_id=client-1"
        );
        // Trace and correlation ids are not tenants.
        assert_eq!(
            tenant_in("Trace ID: 0b0e8c6a-1111-2222-3333-444455556666"),
            None
        );
    }

    fn id_token(claims: &str) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims))
    }

    #[test]
    fn the_chosen_address_comes_from_the_id_token() {
        let google = id_token(r#"{"email":"don@gmail.com","email_verified":true}"#);
        assert_eq!(
            email_from_id_token(&google).as_deref(),
            Some("don@gmail.com")
        );
        // Microsoft work accounts: no email claim, the sign-in name instead.
        let work = id_token(r#"{"preferred_username":"don@mocky.co.ke"}"#);
        assert_eq!(
            email_from_id_token(&work).as_deref(),
            Some("don@mocky.co.ke")
        );
        // An unverified email is not used; nor is junk.
        let unverified = id_token(r#"{"email":"x@y.co","email_verified":false}"#);
        assert_eq!(email_from_id_token(&unverified), None);
        assert_eq!(email_from_id_token("not-a-token"), None);
        assert_eq!(
            email_from_id_token(&id_token(r#"{"email":"no at sign"}"#)),
            None
        );
    }

    #[test]
    fn connecting_asks_for_calendar_contacts_and_tasks() {
        let google = extra_scope(Provider::Google, "openid email https://mail.google.com/");
        assert!(google.starts_with("openid email https://mail.google.com/ "));
        for scope in GOOGLE_EXTRA_SCOPES {
            assert!(google.contains(scope), "{scope}");
        }
        let microsoft = extra_scope(Provider::Microsoft, "ignored");
        assert!(microsoft.contains("offline_access"));
        assert!(microsoft.contains("https://graph.microsoft.com/Calendars.ReadWrite"));
        // Graph and Outlook mail can't share one token.
        assert!(!microsoft.contains("outlook.office.com"));
        // Permissions the provider won't give the app are a setup problem.
        assert_eq!(
            callback("GET /?state=x&error=invalid_scope HTTP/1.1", "x"),
            Some(Err(AuthError::NotEnabled))
        );
    }

    #[test]
    fn signing_in_again_keeps_the_extra_permissions() {
        let granted: Vec<String> = [
            "openid",
            "https://www.googleapis.com/auth/userinfo.email",
            "https://mail.google.com/",
            "https://www.googleapis.com/auth/calendar",
            "https://www.googleapis.com/auth/carddav",
            "https://www.googleapis.com/auth/tasks",
            "https://www.googleapis.com/auth/calendar",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        let mail = "openid email https://mail.google.com/";
        let scope = mail_scope_with(Provider::Google, mail, &granted);
        assert!(scope.starts_with("openid email https://mail.google.com/ "));
        for kept in [
            "https://www.googleapis.com/auth/calendar",
            "https://www.googleapis.com/auth/carddav",
            "https://www.googleapis.com/auth/tasks",
        ] {
            assert_eq!(
                scope.split_whitespace().filter(|s| *s == kept).count(),
                1,
                "{kept}"
            );
        }
        assert_eq!(scope.matches("https://mail.google.com/").count(), 1);
        // Not connected: the plain mail sign-in.
        assert_eq!(mail_scope_with(Provider::Google, mail, &[]), mail);
        // Microsoft's Graph grant is separate and stays as it is.
        let graph: Vec<String> = GRAPH_SCOPES.iter().map(|s| (*s).to_owned()).collect();
        assert!(kept_scopes(Provider::Microsoft, &graph).is_empty());
        assert_eq!(mail_scope_with(Provider::Microsoft, "x y", &graph), "x y");
    }

    #[tokio::test]
    async fn the_incremental_page_keeps_mail_access() {
        let opened = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen = std::sync::Arc::clone(&opened);
        let result = authorize_with_options(
            Provider::Google,
            &test_config(),
            "a@x.co",
            true,
            move |url| {
                *seen.lock().unwrap() = url.to_owned();
                false
            },
        )
        .await;
        assert!(matches!(result, Err(AuthError::Browser)));
        assert!(
            opened
                .lock()
                .unwrap()
                .contains("include_granted_scopes=true")
        );
    }

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_ne!(nonce().unwrap(), nonce().unwrap());
    }
    #[test]
    fn callbacks_require_matching_unique_state_and_code() {
        assert_eq!(
            callback("GET /?state=x&code=a%2Bb HTTP/1.1", "x"),
            Some(Ok("a+b".into()))
        );
        for request in [
            "GET /?state=y&code=z HTTP/1.1",
            "GET /?state=x&state=x&code=z HTTP/1.1",
            "POST /?state=x&code=z HTTP/1.1",
            "GET /other?state=x&code=z HTTP/1.1",
        ] {
            assert!(callback(request, "x").is_none());
        }
        assert_eq!(
            callback("GET /?state=x&error=access_denied HTTP/1.1", "x"),
            Some(Err(AuthError::Denied))
        );
    }
    #[test]
    fn known_hosts_and_xoauth2() {
        assert_eq!(provider_for_address("Me@GMAIL.COM"), Some(Provider::Google));
        assert_eq!(
            provider_for_address("me@msn.com"),
            Some(Provider::Microsoft)
        );
        assert_eq!(provider_for_address("me@company.example"), None);
        assert_eq!(
            STANDARD.decode(xoauth2("a@b.example", "sample")).unwrap(),
            b"user=a@b.example\x01auth=Bearer sample\x01\x01"
        );
    }
    #[test]
    fn expiry_rejects_overflow_and_zero() {
        assert_eq!(expires(1, 5), Ok(6));
        assert_eq!(expires(u64::MAX, 1), Err(AuthError::Protocol));
        assert_eq!(expires(0, 0), Err(AuthError::Protocol));
    }
    async fn mock_exchange(status: &str, body: &str) -> Result<OAuthTokens, AuthError> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = format!("http://{address}/token");
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut line = String::new();
            let mut length = 0;
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).await.unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some((_, v)) = line.to_ascii_lowercase().split_once("content-length:") {
                    length = v.trim().parse().unwrap();
                }
            }
            let mut data = vec![0; length];
            reader.read_exact(&mut data).await.unwrap();
            assert!(
                String::from_utf8(data)
                    .unwrap()
                    .contains("grant_type=refresh_token")
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        });
        let config = Config {
            id: "test",
            secret: None,
            authority: "",
            token: &endpoint,
            scope: "",
            host: "localhost",
        };
        let result = exchange(
            &config,
            vec![
                ("grant_type", "refresh_token"),
                ("refresh_token", "fixture"),
            ],
        )
        .await;
        server.await.unwrap();
        result
    }
    #[tokio::test]
    async fn refresh_accepts_rotated_or_omitted_token() {
        for (extra, expected) in [("", None), (r#","refresh_token":"new""#, Some("new"))] {
            let payload = format!(
                r#"{{"access_token":"test","expires_in":10,"token_type":"Bearer"{extra}}}"#
            );
            assert_eq!(
                mock_exchange("200 OK", &payload)
                    .await
                    .unwrap()
                    .refresh_token
                    .as_deref(),
                expected
            );
        }
    }
    #[tokio::test]
    async fn errors_do_not_expose_provider_payload() {
        assert!(matches!(
            mock_exchange("400 Bad Request", r#"{"error":"invalid_grant"}"#).await,
            Err(AuthError::Reauthorize)
        ));
        assert!(matches!(
            mock_exchange("503 Service Unavailable", "").await,
            Err(AuthError::Network)
        ));
        assert!(matches!(
            mock_exchange("200 OK", "{}").await,
            Err(AuthError::Protocol)
        ));
    }
}
