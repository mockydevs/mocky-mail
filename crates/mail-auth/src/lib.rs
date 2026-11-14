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
    /// Malformed response.
    #[error("The sign-in service returned an invalid response")]
    Protocol,
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
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        let slot = match k.as_ref() {
            "state" => &mut state,
            "code" => &mut code,
            "error" => &mut error,
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
        (None, Some("access_denied")) => Err(AuthError::Denied),
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
        if provider == Provider::Google {
            q.append_pair("access_type", "offline")
                .append_pair("prompt", "consent");
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
}
#[derive(Deserialize)]
struct TokenError {
    error: String,
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
        return Err(
            match response
                .json::<TokenError>()
                .await
                .ok()
                .map(|e| e.error)
                .as_deref()
            {
                Some("invalid_grant") => AuthError::Reauthorize,
                Some("access_denied") => AuthError::Denied,
                _ => AuthError::Protocol,
            },
        );
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
        granted_email: None,
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
