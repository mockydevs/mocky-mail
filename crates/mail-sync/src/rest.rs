//! A small JSON-over-HTTPS client for the Google and Microsoft APIs, with
//! an OAuth access token. Replies are size-limited and errors are turned
//! into plain words; tokens never appear in errors or `Debug` output.

use std::fmt::Write as _;
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Client, Method, StatusCode, Url};
use serde_json::Value;

use crate::SyncError;

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Largest reply read, bytes.
const MAX_REPLY_BYTES: usize = 16 * 1024 * 1024;

/// Which API a client talks to, for its error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    /// Google (Calendar, Tasks, People, Gmail settings).
    Google,
    /// Microsoft Graph.
    Graph,
}

/// A signed-in client for one API.
#[derive(Clone)]
pub struct RestClient {
    http: Client,
    token: String,
    api: Api,
    /// Extra headers on every request (Graph's `Prefer` time zone).
    headers: HeaderMap,
}

impl std::fmt::Debug for RestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestClient")
            .field("api", &self.api)
            .finish_non_exhaustive()
    }
}

/// A reply: status and parsed body (`Value::Null` when empty).
#[derive(Debug, Clone)]
pub struct Reply {
    /// HTTP status.
    pub status: StatusCode,
    /// The JSON body.
    pub body: Value,
}

impl RestClient {
    /// A client with this access token.
    ///
    /// # Errors
    /// When the HTTP client can't be set up.
    pub fn new(api: Api, access_token: String) -> Result<Self, SyncError> {
        let http = Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent("Mocky Mail")
            .build()
            .map_err(|e| SyncError::Config(e.to_string()))?;
        let mut headers = HeaderMap::new();
        if api == Api::Graph {
            // Times come back in UTC, so they read the same everywhere.
            headers.insert(
                "Prefer",
                HeaderValue::from_static(
                    "outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"",
                ),
            );
        }
        Ok(Self {
            http,
            token: access_token,
            api,
            headers,
        })
    }

    /// GET a JSON document.
    ///
    /// # Errors
    /// Network errors and error replies, in plain words.
    pub async fn get(&self, url: &str) -> Result<Value, SyncError> {
        Ok(self.send(Method::GET, url, None).await?.body)
    }

    /// Send a request with an optional JSON body.
    ///
    /// # Errors
    /// Network errors and error replies (any status not 2xx), in plain
    /// words.
    pub async fn send(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Reply, SyncError> {
        let parsed = Url::parse(url).map_err(|e| SyncError::Invalid(e.to_string()))?;
        if parsed.scheme() != "https"
            && !(cfg!(feature = "insecure-plaintext") && parsed.scheme() == "http")
        {
            return Err(SyncError::Config(format!("{url} isn't a secure address")));
        }
        let host = parsed.host_str().unwrap_or_default().to_owned();
        let mut auth = HeaderValue::from_str(&format!("Bearer {}", self.token))
            .map_err(|_| SyncError::Auth)?;
        auth.set_sensitive(true);
        let mut request = self
            .http
            .request(method, parsed)
            .headers(self.headers.clone())
            .header(AUTHORIZATION, auth);
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(serde_json::to_string(body).map_err(|e| SyncError::Invalid(e.to_string()))?);
        } else {
            request = request.header("Content-Length", "0");
        }
        let mut response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                SyncError::Timeout { host: host.clone() }
            } else if e.is_connect() {
                SyncError::Connect {
                    host: host.clone(),
                    port: 443,
                    reason: crate::error::cause_reason(&e),
                }
            } else {
                SyncError::Protocol(crate::error::innermost_text(&e))
            }
        })?;
        let status = response.status();
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| SyncError::Protocol(crate::error::innermost_text(&e)))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_REPLY_BYTES {
                return Err(SyncError::Protocol("the reply was too large".to_owned()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value = if bytes.iter().all(u8::is_ascii_whitespace) {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        if status.is_success() {
            return Ok(Reply { status, body });
        }
        Err(error_for(self.api, status, &body))
    }
}

/// The error message an API put in its reply, if any.
fn api_message(body: &Value) -> String {
    let error = body.get("error");
    error
        .and_then(|e| e.get("message"))
        .or_else(|| body.get("error_description"))
        .and_then(Value::as_str)
        .or_else(|| error.and_then(Value::as_str))
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

/// An error reply in plain words.
#[must_use]
pub fn error_for(api: Api, status: StatusCode, body: &Value) -> SyncError {
    let text = body.to_string();
    let message = api_message(body);
    match api {
        Api::Google => {
            if let Some(setup) = crate::google::setup_problem(&text) {
                return setup;
            }
        }
        Api::Graph => {
            if let Some(setup) = crate::graph::setup_problem(&text) {
                return setup;
            }
        }
    }
    match status {
        StatusCode::UNAUTHORIZED => SyncError::Auth,
        StatusCode::TOO_MANY_REQUESTS => {
            SyncError::Protocol("the service is busy; it will be tried again".to_owned())
        }
        s if s.is_server_error() => SyncError::Protocol(format!(
            "the service had a problem ({s}); it will be tried again"
        )),
        s => SyncError::Refused(if message.is_empty() {
            format!("the service refused ({s})")
        } else {
            format!("{message} ({s})")
        }),
    }
}

/// Every page of a listing: the items under `key` of each page, following
/// `next` (a function giving the next page's URL from a page, if any).
///
/// # Errors
/// As [`RestClient::get`].
pub async fn all_pages(
    client: &RestClient,
    first: String,
    key: &str,
    next: impl Fn(&Value, &str) -> Option<String>,
    max_pages: usize,
) -> Result<(Vec<Value>, Value), SyncError> {
    let mut url = first;
    let mut out = Vec::new();
    let mut last = Value::Null;
    for _ in 0..max_pages {
        let page = client.get(&url).await?;
        if let Some(items) = page.get(key).and_then(Value::as_array) {
            out.extend(items.iter().cloned());
        }
        match next(&page, &url) {
            Some(more) => {
                url = more;
                last = page;
            }
            None => return Ok((out, page)),
        }
    }
    Ok((out, last))
}

/// Set a field of a JSON object (nothing happens to anything else).
pub fn set(target: &mut Value, key: &str, value: Value) {
    if let Some(object) = target.as_object_mut() {
        object.insert(key.to_owned(), value);
    }
}

/// A string field of a JSON object.
#[must_use]
pub fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A URL with query pairs added.
///
/// # Errors
/// [`SyncError::Invalid`] for a bad base.
pub fn with_query(base: &str, pairs: &[(&str, &str)]) -> Result<String, SyncError> {
    let mut url = Url::parse(base).map_err(|e| SyncError::Invalid(e.to_string()))?;
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in pairs {
            q.append_pair(k, v);
        }
    }
    Ok(url.to_string())
}

/// One part of a URL path, percent-encoded (calendar ids hold `@` and `#`).
#[must_use]
pub fn segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_plain_and_setup_problems_are_named() {
        let disabled = serde_json::json!({"error": {"code": 403, "message": "Google Calendar API has not been used in project 123 before or it is disabled.", "status": "PERMISSION_DENIED", "details": [{"reason": "SERVICE_DISABLED"}]}});
        let e = error_for(Api::Google, StatusCode::FORBIDDEN, &disabled);
        assert!(matches!(e, SyncError::Config(_)), "{e:?}");
        assert!(e.to_string().contains("Google hasn't"), "{e}");
        let scope = serde_json::json!({"error": {"code": 403, "message": "Request had insufficient authentication scopes.", "status": "PERMISSION_DENIED", "details": [{"reason": "ACCESS_TOKEN_SCOPE_INSUFFICIENT"}]}});
        assert!(
            error_for(Api::Google, StatusCode::FORBIDDEN, &scope)
                .to_string()
                .contains("Connect")
        );
        let graph = serde_json::json!({"error": {"code": "ErrorAccessDenied", "message": "Access is denied. Check credentials and try again."}});
        assert!(matches!(
            error_for(Api::Graph, StatusCode::FORBIDDEN, &graph),
            SyncError::Config(_)
        ));
        assert!(matches!(
            error_for(Api::Graph, StatusCode::UNAUTHORIZED, &Value::Null),
            SyncError::Auth
        ));
        assert!(matches!(
            error_for(Api::Graph, StatusCode::SERVICE_UNAVAILABLE, &Value::Null),
            SyncError::Protocol(_)
        ));
        let gone = error_for(
            Api::Google,
            StatusCode::NOT_FOUND,
            &serde_json::json!({"error": {"message": "Not Found"}}),
        );
        assert_eq!(
            gone.to_string(),
            "the mail server refused: Not Found (404 Not Found)"
        );
    }

    #[test]
    fn ids_are_encoded_in_paths() {
        assert_eq!(segment("me@gmail.com"), "me%40gmail.com");
        assert_eq!(
            segment("en.ke#holiday@group.v.calendar.google.com"),
            "en.ke%23holiday%40group.v.calendar.google.com"
        );
        assert_eq!(
            with_query("https://x.example/a", &[("q", "a b&c")]).unwrap(),
            "https://x.example/a?q=a+b%26c"
        );
    }

    #[test]
    fn the_token_is_never_printed() {
        let client = RestClient::new(Api::Graph, "eyJ0eXAi.secret".into()).unwrap();
        assert!(!format!("{client:?}").contains("secret"));
    }
}
