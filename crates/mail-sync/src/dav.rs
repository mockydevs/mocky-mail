//! `WebDAV` client for address books (`CardDAV`, RFC 6352) and calendars
//! (`CalDAV`, RFC 4791).
//!
//! Servers such as `SOGo` use the account's mail password with HTTP Basic
//! auth, over HTTPS only. The address book is found the standard way:
//! `/.well-known/carddav`, then the user's principal, then its
//! address-book home, then the address books in it.
//!
//! Redirects are followed by hand: HTTP clients turn a redirected
//! `PROPFIND` into a `GET`, which `WebDAV` servers answer with a web page.

use std::time::Duration;

use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::header::{CONTENT_TYPE, ETAG, HeaderMap, HeaderValue, LOCATION};
use reqwest::{Client, Method, StatusCode, Url};

use crate::SyncError;

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Cards fetched per `addressbook-multiget`.
const MULTIGET_BATCH: usize = 50;
/// Most redirects followed for one request.
const MAX_REDIRECTS: usize = 5;
/// Largest response read, bytes.
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Login for a `WebDAV` server.
#[derive(Clone)]
pub struct DavLogin {
    /// User name, usually the email address.
    pub username: String,
    /// The password.
    pub password: String,
}

impl std::fmt::Debug for DavLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DavLogin")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// One `<response>` of a `207 Multi-Status`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "what the server said, as it said it"
)]
pub struct DavResponse {
    /// The resource's path, as the server wrote it.
    pub href: String,
    /// `getetag`, if given.
    pub etag: Option<String>,
    /// `resourcetype` includes `addressbook`.
    pub is_addressbook: bool,
    /// `resourcetype` includes `calendar`.
    pub is_calendar: bool,
    /// `displayname`.
    pub display_name: Option<String>,
    /// `current-user-principal`.
    pub principal: Option<String>,
    /// `addressbook-home-set`.
    pub addressbook_home: Option<String>,
    /// `calendar-home-set`.
    pub calendar_home: Option<String>,
    /// `address-data` or `calendar-data`.
    pub data: Option<String>,
    /// Apple's `calendar-color`, such as `#3A87ADFF`.
    pub color: Option<String>,
    /// `supported-calendar-component-set`: `VEVENT`, `VTODO`, ...
    pub components: Vec<String>,
    /// The resource itself answered 404 (deleted).
    pub missing: bool,
    /// `current-user-privilege-set` was given.
    pub has_privileges: bool,
    /// Its privileges: `read`, `write`, `write-content`, `all`, ...
    pub privileges: Vec<String>,
    /// `schedule-outbox-URL` (RFC 6638).
    pub schedule_outbox: Option<String>,
}

impl DavResponse {
    /// Whether this login may add and change items in it (when the server
    /// says; one that doesn't is taken as writable).
    #[must_use]
    pub fn writable(&self) -> bool {
        !self.has_privileges
            || self
                .privileges
                .iter()
                .any(|p| matches!(p.as_str(), "write" | "write-content" | "all" | "bind"))
    }
}

/// Read a `207 Multi-Status` body. Namespace prefixes vary by server, so
/// elements are matched by local name.
///
/// # Errors
/// [`SyncError::Protocol`] for XML that can't be read.
pub fn parse_multistatus(xml: &str) -> Result<Vec<DavResponse>, SyncError> {
    let mut reader = Reader::from_str(xml);
    // No trimming: it would eat the spaces around &amp; in card text.
    reader.config_mut().trim_text(false);
    let mut out = Vec::new();
    let mut current: Option<DavResponse> = None;
    // Element names from the root to here, local names only.
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| SyncError::Protocol(format!("unreadable reply from the server: {e}")))?;
        match event {
            Event::Start(e) => {
                let name = local(e.local_name().as_ref());
                if name == "response" {
                    current = Some(DavResponse::default());
                }
                if let Some(r) = current.as_mut() {
                    mark_type(r, &path, &name);
                    // <comp name="VTODO"></comp>, as some servers write it.
                    mark_component(r, &path, &name, &e);
                }
                path.push(name);
                text.clear();
            }
            Event::Empty(e) => {
                let name = local(e.local_name().as_ref());
                if let Some(r) = current.as_mut() {
                    mark_type(r, &path, &name);
                    mark_component(r, &path, &name, &e);
                }
            }
            Event::Text(t) => text.push_str(&unescape_xml(t.as_ref())),
            Event::GeneralRef(r) => {
                // An entity such as &amp; or &#13; arrives on its own.
                let name: &str = r.as_ref();
                text.push_str(&unescape_xml(&format!("&{name};")));
            }
            Event::CData(c) => text.push_str(c.as_ref()),
            Event::End(_) => {
                let name = path.pop().unwrap_or_default();
                let parent = path.last().map_or("", String::as_str);
                if let Some(r) = current.as_mut() {
                    let value = text.trim().to_owned();
                    match (parent, name.as_str()) {
                        ("response", "href") => r.href = value,
                        ("response", "status") if value.contains(" 404") => r.missing = true,
                        (_, "getetag") if !value.is_empty() => r.etag = Some(value),
                        (_, "displayname") if !value.is_empty() => r.display_name = Some(value),
                        (_, "calendar-color") if !value.is_empty() => r.color = Some(value),
                        ("current-user-principal", "href") => r.principal = Some(value),
                        ("addressbook-home-set", "href") => r.addressbook_home = Some(value),
                        ("calendar-home-set", "href") => r.calendar_home = Some(value),
                        ("schedule-outbox-url", "href") => r.schedule_outbox = Some(value),
                        (_, "address-data" | "calendar-data") => {
                            // Card text keeps its own line breaks.
                            r.data = Some(text.clone());
                        }
                        _ => {}
                    }
                }
                if name == "response"
                    && let Some(r) = current.take()
                {
                    out.push(r);
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn local(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn mark_type(r: &mut DavResponse, path: &[String], name: &str) {
    if name == "current-user-privilege-set" {
        r.has_privileges = true;
    }
    if path.last().map(String::as_str) == Some("privilege")
        && path
            .iter()
            .rev()
            .nth(1)
            .is_some_and(|p| p == "current-user-privilege-set")
        && !r.privileges.iter().any(|p| p == name)
    {
        r.privileges.push(name.to_owned());
    }
    if path.last().map(String::as_str) == Some("resourcetype") {
        match name {
            "addressbook" => r.is_addressbook = true,
            "calendar" => r.is_calendar = true,
            _ => {}
        }
    }
}

/// Note a `<comp name="VEVENT">` inside `supported-calendar-component-set`.
fn mark_component(
    r: &mut DavResponse,
    path: &[String],
    name: &str,
    element: &quick_xml::events::BytesStart<'_>,
) {
    if name == "comp"
        && path.last().map(String::as_str) == Some("supported-calendar-component-set")
        && let Ok(Some(attr)) = element.try_get_attribute("name")
    {
        r.components.push(attr.value.to_ascii_uppercase());
    }
}

/// Longest reference name looked for after an `&` (`#x10FFFF` is 8).
const MAX_REFERENCE_LEN: usize = 10;

/// Replace XML references with their characters: the five named ones and
/// numeric ones in decimal (`&#13;`) or hex (`&#xD;`), which servers use
/// for the line breaks and non-ASCII letters of card and event text.
/// Anything else after an `&` is kept as written.
pub(crate) fn unescape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        let (before, tail) = rest.split_at(at);
        out.push_str(before);
        // `tail` starts with the one-byte `&`.
        let after = tail.get(1..).unwrap_or_default();
        let decoded = after
            .find(';')
            .filter(|end| *end <= MAX_REFERENCE_LEN)
            .and_then(|end| Some((reference_char(after.get(..end)?)?, end)));
        if let Some((c, end)) = decoded {
            out.push(c);
            rest = after.get(end.saturating_add(1)..).unwrap_or_default();
        } else {
            out.push('&');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The character a reference names (`amp`, `#13`, `#xD`), if it is one.
fn reference_char(name: &str) -> Option<char> {
    match name {
        "lt" => return Some('<'),
        "gt" => return Some('>'),
        "quot" => return Some('"'),
        "apos" => return Some('\''),
        "amp" => return Some('&'),
        _ => {}
    }
    let number = name.strip_prefix('#')?;
    let code = match number.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => number.parse().ok()?,
    };
    // NUL is not a character XML can carry.
    char::from_u32(code).filter(|c| *c != '\0')
}

/// A `WebDAV` connection to one server.
#[derive(Debug, Clone)]
pub struct DavClient {
    http: Client,
    auth: DavAuth,
}

/// How requests sign in.
#[derive(Clone)]
enum DavAuth {
    /// HTTP Basic with the mail password.
    Basic(DavLogin),
    /// An OAuth access token (Google's `CalDAV` and `CardDAV`).
    Bearer(String),
}

impl std::fmt::Debug for DavAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Basic(login) => f.debug_tuple("Basic").field(login).finish(),
            Self::Bearer(_) => f.write_str("Bearer(..)"),
        }
    }
}

/// An address book on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressBook {
    /// Its URL.
    pub url: Url,
    /// Its name, if the server gave one.
    pub name: Option<String>,
}

/// A calendar on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarCollection {
    /// Its URL.
    pub url: Url,
    /// Its name, if the server gave one.
    pub name: Option<String>,
    /// Its colour (`#RRGGBB` or `#RRGGBBAA`), if the server gave one.
    pub color: Option<String>,
    /// What it holds (`VEVENT`, `VTODO`); empty when the server didn't say.
    pub components: Vec<String>,
    /// Items can be added and changed.
    pub writable: bool,
}

/// One recipient's part of a scheduling answer (RFC 6638 `schedule-response`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleAnswer {
    /// The recipient's address (without `mailto:`).
    pub recipient: String,
    /// `request-status`, such as `2.0;Success`.
    pub status: String,
    /// The calendar data returned (a `VFREEBUSY`).
    pub data: Option<String>,
}

/// Read a `schedule-response`: each recipient with its status and data.
///
/// # Errors
/// [`SyncError::Protocol`] for XML that can't be read.
pub fn parse_schedule_response(xml: &str) -> Result<Vec<ScheduleAnswer>, SyncError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut out = Vec::new();
    let mut current: Option<ScheduleAnswer> = None;
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| SyncError::Protocol(format!("unreadable reply from the server: {e}")))?;
        match event {
            Event::Start(e) => {
                let name = local(e.local_name().as_ref());
                if name == "response" {
                    current = Some(ScheduleAnswer {
                        recipient: String::new(),
                        status: String::new(),
                        data: None,
                    });
                }
                path.push(name);
                text.clear();
            }
            Event::Text(t) => text.push_str(&unescape_xml(t.as_ref())),
            Event::GeneralRef(r) => {
                let name: &str = r.as_ref();
                text.push_str(&unescape_xml(&format!("&{name};")));
            }
            Event::CData(c) => text.push_str(c.as_ref()),
            Event::End(_) => {
                let name = path.pop().unwrap_or_default();
                let parent = path.last().map_or("", String::as_str);
                if let Some(answer) = current.as_mut() {
                    match (parent, name.as_str()) {
                        ("recipient", "href") => {
                            let value = text.trim();
                            value
                                .get(..7)
                                .filter(|p| p.eq_ignore_ascii_case("mailto:"))
                                .and_then(|_| value.get(7..))
                                .unwrap_or(value)
                                .clone_into(&mut answer.recipient);
                        }
                        (_, "request-status") => text.trim().clone_into(&mut answer.status),
                        (_, "calendar-data") => answer.data = Some(text.clone()),
                        _ => {}
                    }
                }
                if name == "response"
                    && let Some(answer) = current.take()
                {
                    out.push(answer);
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// A card as read from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCard {
    /// Its path.
    pub href: String,
    /// Its version tag.
    pub etag: Option<String>,
    /// The vCard text.
    pub vcard: String,
}

/// Whether a reply of the length the server announced is within
/// [`MAX_RESPONSE_BYTES`].
fn fits_in_memory(length: u64) -> bool {
    usize::try_from(length).is_ok_and(|length| length <= MAX_RESPONSE_BYTES)
}

fn check_scheme(url: &Url) -> Result<(), SyncError> {
    if url.scheme() == "https" || (cfg!(feature = "insecure-plaintext") && url.scheme() == "http") {
        Ok(())
    } else {
        Err(SyncError::Config(format!(
            "{url} isn't a secure (https) address, so Mocky Mail won't send your password to it."
        )))
    }
}

pub(crate) fn http_error(url: &Url, error: &reqwest::Error) -> SyncError {
    let host = url.host_str().unwrap_or_default().to_owned();
    if error.is_timeout() {
        SyncError::Timeout { host }
    } else if let Some(detail) = crate::error::tls_detail(error) {
        // A certificate that can't be trusted is not "offline": trying
        // again won't help, and the person needs to know which it is.
        SyncError::Tls { host, detail }
    } else if error.is_connect() {
        SyncError::Connect {
            port: url.port_or_known_default().unwrap_or(443),
            host,
            // The error's own text is only "error sending request for
            // url"; the cause underneath says why.
            reason: crate::error::cause_reason(error),
        }
    } else {
        SyncError::Protocol(crate::error::innermost_text(error))
    }
}

/// A collection's URL ending in `/`. Joining a name onto one without the
/// slash would replace its last part and store the item in the parent.
fn as_collection(url: &Url) -> Url {
    let mut url = url.clone();
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url
}

/// An `If-Match` value for a version tag. A tag that can't be sent is an
/// error: leaving the header out would turn a guarded write or delete into
/// one that overwrites whatever is on the server.
fn if_match_value(etag: &str, kind: Kind) -> Result<HeaderValue, SyncError> {
    HeaderValue::from_bytes(etag.as_bytes()).map_err(|_| {
        SyncError::Refused(format!(
            "the {}'s version on the server can't be checked (its version tag isn't usable)",
            kind.noun()
        ))
    })
}

/// Whether card or event text has this `UID`. Lines may be folded, and the
/// property may carry parameters (`UID;X-A=b:value`).
fn has_uid(text: &str, uid: &str) -> bool {
    let unfolded = text
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "");
    unfolded.lines().any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        let name = name.split(';').next().unwrap_or_default().trim();
        name.eq_ignore_ascii_case("UID") && value.trim() == uid
    })
}

impl DavClient {
    /// A client for this login.
    ///
    /// # Errors
    /// When the HTTP client can't be set up.
    pub fn new(login: DavLogin) -> Result<Self, SyncError> {
        Self::with_auth(DavAuth::Basic(login))
    }

    /// A client that signs in with an OAuth access token (`Authorization:
    /// Bearer`), for Google's `CalDAV` and `CardDAV`.
    ///
    /// # Errors
    /// When the HTTP client can't be set up.
    pub fn bearer(access_token: String) -> Result<Self, SyncError> {
        Self::with_auth(DavAuth::Bearer(access_token))
    }

    fn with_auth(auth: DavAuth) -> Result<Self, SyncError> {
        let http = Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Mocky Mail")
            .build()
            .map_err(|e| SyncError::Config(e.to_string()))?;
        Ok(Self { http, auth })
    }

    /// Send one request, following redirects by hand with the same method.
    async fn send(
        &self,
        method: &Method,
        mut url: Url,
        headers: HeaderMap,
        body: Option<String>,
    ) -> Result<(Url, StatusCode, HeaderMap, String), SyncError> {
        for _ in 0..=MAX_REDIRECTS {
            check_scheme(&url)?;
            let request = self.http.request(method.clone(), url.clone());
            let mut request = match &self.auth {
                DavAuth::Basic(login) => request.basic_auth(&login.username, Some(&login.password)),
                DavAuth::Bearer(token) => request.bearer_auth(token),
            }
            .headers(headers.clone());
            if let Some(body) = &body {
                request = request.body(body.clone());
            }
            let mut response = request.send().await.map_err(|e| http_error(&url, &e))?;
            let status = response.status();
            if status.is_redirection() {
                let next = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|loc| url.join(loc).ok())
                    .ok_or_else(|| {
                        SyncError::Protocol("the server redirected nowhere".to_owned())
                    })?;
                // Never follow a redirect to another site with the password.
                if next.host_str() != url.host_str() {
                    return Err(SyncError::Config(format!(
                        "the server sent Mocky Mail to another site ({}); not following it with your password",
                        next.host_str().unwrap_or_default()
                    )));
                }
                url = next;
                continue;
            }
            if status == StatusCode::UNAUTHORIZED {
                return Err(SyncError::Auth);
            }
            // Signed in, but not allowed here: a read-only or shared
            // collection, or a web server with no address book or calendar.
            // The password is not the problem.
            if status == StatusCode::FORBIDDEN {
                if matches!(self.auth, DavAuth::Bearer(_)) {
                    let body: String = response
                        .text()
                        .await
                        .unwrap_or_default()
                        .chars()
                        .take(4000)
                        .collect();
                    if let Some(setup) = crate::google::setup_problem(&body) {
                        return Err(setup);
                    }
                }
                return Err(SyncError::Refused(
                    "the server doesn't allow that here (403 Forbidden)".to_owned(),
                ));
            }
            let headers = response.headers().clone();
            // The limit is applied as the body arrives, not after: a huge
            // or endless reply must not be held in memory first.
            let too_large = || SyncError::Protocol("the server's reply was too large".to_owned());
            if response
                .content_length()
                .is_some_and(|length| !fits_in_memory(length))
            {
                return Err(too_large());
            }
            let mut bytes: Vec<u8> = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| http_error(&url, &e))? {
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(too_large());
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok((
                url,
                status,
                headers,
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
        Err(SyncError::Protocol(
            "the server redirected too many times".to_owned(),
        ))
    }

    async fn xml_request(
        &self,
        method: &str,
        url: Url,
        depth: &str,
        body: &str,
    ) -> Result<(Url, Vec<DavResponse>), SyncError> {
        let method =
            Method::from_bytes(method.as_bytes()).map_err(|e| SyncError::Invalid(e.to_string()))?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "Depth",
            HeaderValue::from_str(depth).map_err(|e| SyncError::Invalid(e.to_string()))?,
        );
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/xml; charset=utf-8"),
        );
        let (url, status, _, text) = self
            .send(&method, url, headers, Some(body.to_owned()))
            .await?;
        if status != StatusCode::MULTI_STATUS {
            return Err(SyncError::Protocol(format!(
                "the server answered {status} to {method}"
            )));
        }
        Ok((url, parse_multistatus(&text)?))
    }

    /// The collections in this login's home of one kind: the server's
    /// `/.well-known/<service>`, then the user's principal, then the home
    /// it names, then what's in it.
    async fn home_collections(
        &self,
        server: &Url,
        service: &str,
        what: &str,
        pick_home: fn(&DavResponse) -> Option<String>,
    ) -> Result<(Url, Vec<DavResponse>), SyncError> {
        let start = server
            .join(&format!("/.well-known/{service}"))
            .map_err(|e| SyncError::Config(e.to_string()))?;
        let (at, found) = self
            .xml_request("PROPFIND", start, "0", PROPFIND_PRINCIPAL)
            .await?;
        let principal = found
            .iter()
            .find_map(|r| r.principal.clone())
            .ok_or_else(|| {
                SyncError::Config(format!("this server doesn't offer {what} (no principal)"))
            })?;
        let principal = at
            .join(&principal)
            .map_err(|e| SyncError::Protocol(e.to_string()))?;
        let (at, found) = self
            .xml_request("PROPFIND", principal, "0", PROPFIND_HOMES)
            .await?;
        let home = found.iter().find_map(pick_home).ok_or_else(|| {
            SyncError::Config(format!("this server doesn't offer {what} (no home)"))
        })?;
        let home = at
            .join(&home)
            .map_err(|e| SyncError::Protocol(e.to_string()))?;
        self.xml_request("PROPFIND", home, "1", PROPFIND_COLLECTIONS)
            .await
    }

    /// Find the address books for this login, starting from the server's
    /// address (for example `https://mail.example.com`).
    ///
    /// # Errors
    /// [`SyncError::Auth`] for a refused password; others when the server
    /// has no address book service.
    pub async fn discover_address_books(
        &self,
        server: &Url,
    ) -> Result<Vec<AddressBook>, SyncError> {
        let (at, found) = self
            .home_collections(server, "carddav", "contacts", |r| {
                r.addressbook_home.clone()
            })
            .await?;
        Ok(found
            .into_iter()
            .filter(|r| r.is_addressbook)
            .filter_map(|r| {
                Some(AddressBook {
                    url: at.join(&r.href).ok()?,
                    name: r.display_name,
                })
            })
            .collect())
    }

    /// Find the calendars that hold events for this login.
    ///
    /// # Errors
    /// [`SyncError::Auth`] for a refused password; others when the server
    /// has no calendar service.
    pub async fn discover_calendars(
        &self,
        server: &Url,
    ) -> Result<Vec<CalendarCollection>, SyncError> {
        Ok(self
            .discover_collections(server)
            .await?
            .into_iter()
            .filter(|c| c.components.is_empty() || c.components.iter().any(|x| x == "VEVENT"))
            .collect())
    }

    /// Find the calendars that hold to-dos (`VTODO`) for this login. Only
    /// those the server says hold to-dos count, so a server that doesn't
    /// say gets no task lists rather than one per calendar.
    ///
    /// # Errors
    /// As [`Self::discover_calendars`].
    pub async fn discover_task_lists(
        &self,
        server: &Url,
    ) -> Result<Vec<CalendarCollection>, SyncError> {
        Ok(self
            .discover_collections(server)
            .await?
            .into_iter()
            .filter(|c| c.components.iter().any(|x| x == "VTODO"))
            .collect())
    }

    /// Every calendar collection in this login's calendar home.
    ///
    /// # Errors
    /// As [`Self::discover_calendars`].
    pub async fn discover_collections(
        &self,
        server: &Url,
    ) -> Result<Vec<CalendarCollection>, SyncError> {
        let (at, found) = self
            .home_collections(server, "caldav", "calendars", |r| r.calendar_home.clone())
            .await?;
        Ok(found
            .into_iter()
            .filter(|r| r.is_calendar)
            .filter_map(|r| {
                Some(CalendarCollection {
                    url: at.join(&r.href).ok()?,
                    writable: r.writable(),
                    name: r.display_name,
                    color: r.color,
                    components: r.components,
                })
            })
            .collect())
    }

    /// The paths and version tags of the to-dos in a calendar
    /// (`calendar-query` for `VTODO`), so its events are not fetched again.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn list_todos(
        &self,
        collection: &Url,
    ) -> Result<Vec<(String, Option<String>)>, SyncError> {
        let (_, found) = self
            .xml_request("REPORT", collection.clone(), "1", TODO_QUERY)
            .await?;
        let own_path = collection.path().trim_end_matches('/').to_owned();
        Ok(found
            .into_iter()
            .filter(|r| !r.missing && !r.is_calendar)
            .filter(|r| r.href.trim_end_matches('/') != own_path && !r.href.ends_with('/'))
            .map(|r| (r.href, r.etag))
            .collect())
    }

    /// The login's scheduling outbox (RFC 6638), where free/busy requests
    /// go, if the server has one.
    ///
    /// # Errors
    /// Network errors; a server without scheduling gives `Ok(None)`.
    pub async fn scheduling_outbox(&self, server: &Url) -> Result<Option<Url>, SyncError> {
        let start = server
            .join("/.well-known/caldav")
            .map_err(|e| SyncError::Config(e.to_string()))?;
        let (at, found) = match self
            .xml_request("PROPFIND", start, "0", PROPFIND_PRINCIPAL)
            .await
        {
            Ok(found) => found,
            Err(SyncError::Protocol(_) | SyncError::Refused(_) | SyncError::Config(_)) => {
                return Ok(None);
            }
            Err(other) => return Err(other),
        };
        let Some(principal) = found
            .iter()
            .find_map(|r| r.principal.clone())
            .and_then(|p| at.join(&p).ok())
        else {
            return Ok(None);
        };
        let (at, found) = match self
            .xml_request("PROPFIND", principal, "0", PROPFIND_OUTBOX)
            .await
        {
            Ok(found) => found,
            Err(SyncError::Protocol(_) | SyncError::Refused(_)) => return Ok(None),
            Err(other) => return Err(other),
        };
        Ok(found
            .iter()
            .find_map(|r| r.schedule_outbox.clone())
            .and_then(|o| at.join(&o).ok()))
    }

    /// Ask the scheduling outbox when people are busy: `request` is a
    /// `VFREEBUSY` `METHOD:REQUEST` from `originator`. Returns each
    /// recipient's answer.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn schedule_free_busy(
        &self,
        outbox: &Url,
        originator: &str,
        recipients: &[String],
        request: &str,
    ) -> Result<Vec<ScheduleAnswer>, SyncError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/calendar; charset=utf-8; method=REQUEST"),
        );
        let value = |text: String| {
            HeaderValue::from_str(&text).map_err(|e| SyncError::Invalid(e.to_string()))
        };
        headers.insert("Originator", value(format!("mailto:{originator}"))?);
        for recipient in recipients {
            headers.append("Recipient", value(format!("mailto:{recipient}"))?);
        }
        let (_, status, _, text) = self
            .send(
                &Method::POST,
                outbox.clone(),
                headers,
                Some(request.to_owned()),
            )
            .await?;
        if !status.is_success() {
            return Err(SyncError::Refused(format!(
                "the server didn't answer the free/busy request ({status})"
            )));
        }
        parse_schedule_response(&text)
    }

    /// When this login is busy in one calendar (`free-busy-query`, RFC 4791
    /// 7.10): the `VFREEBUSY` text.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn free_busy_query(
        &self,
        calendar: &Url,
        from: &str,
        to: &str,
    ) -> Result<String, SyncError> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:free-busy-query xmlns:c="urn:ietf:params:xml:ns:caldav"><c:time-range start="{}" end="{}"/></c:free-busy-query>"#,
            escape_xml(from),
            escape_xml(to)
        );
        let mut headers = HeaderMap::new();
        headers.insert("Depth", HeaderValue::from_static("1"));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/xml; charset=utf-8"),
        );
        let method =
            Method::from_bytes(b"REPORT").map_err(|e| SyncError::Invalid(e.to_string()))?;
        let (_, status, _, text) = self
            .send(&method, calendar.clone(), headers, Some(body))
            .await?;
        if status.is_success() {
            Ok(text)
        } else {
            Err(SyncError::Refused(format!(
                "the server didn't answer the free/busy query ({status})"
            )))
        }
    }

    /// Every card's path and version tag in an address book.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn list_cards(&self, book: &Url) -> Result<Vec<(String, Option<String>)>, SyncError> {
        self.list_resources(book).await
    }

    /// Every item's path and version tag in a collection (address book
    /// or calendar).
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn list_resources(
        &self,
        collection: &Url,
    ) -> Result<Vec<(String, Option<String>)>, SyncError> {
        let book = collection;
        let (_, found) = self
            .xml_request("PROPFIND", book.clone(), "1", PROPFIND_ETAGS)
            .await?;
        let own_path = book.path().trim_end_matches('/').to_owned();
        Ok(found
            .into_iter()
            .filter(|r| !r.missing && !r.is_addressbook && !r.is_calendar)
            .filter(|r| r.href.trim_end_matches('/') != own_path && !r.href.ends_with('/'))
            .map(|r| (r.href, r.etag))
            .collect())
    }

    /// Fetch cards by path, in batches.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn fetch_cards(
        &self,
        book: &Url,
        hrefs: &[String],
    ) -> Result<Vec<RemoteCard>, SyncError> {
        self.multiget(book, hrefs, MULTIGET_HEAD, "</c:addressbook-multiget>")
            .await
    }

    /// Fetch calendar items (`.ics` text in `vcard`) by path, in batches.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn fetch_events(
        &self,
        calendar: &Url,
        hrefs: &[String],
    ) -> Result<Vec<RemoteCard>, SyncError> {
        self.multiget(
            calendar,
            hrefs,
            CALENDAR_MULTIGET_HEAD,
            "</c:calendar-multiget>",
        )
        .await
    }

    async fn multiget(
        &self,
        collection: &Url,
        hrefs: &[String],
        head: &str,
        tail: &str,
    ) -> Result<Vec<RemoteCard>, SyncError> {
        let mut out = Vec::new();
        for batch in hrefs.chunks(MULTIGET_BATCH) {
            let mut body = String::from(head);
            for href in batch {
                body.push_str("<d:href>");
                body.push_str(&escape_xml(href));
                body.push_str("</d:href>");
            }
            body.push_str(tail);
            let (_, found) = self
                .xml_request("REPORT", collection.clone(), "1", &body)
                .await?;
            out.extend(found.into_iter().filter_map(|r| {
                Some(RemoteCard {
                    vcard: r.data?,
                    href: r.href,
                    etag: r.etag,
                })
            }));
        }
        Ok(out)
    }

    /// Store a card. `href` `None` creates it (named after its uid);
    /// `if_match` guards against overwriting someone else's edit. Returns
    /// its path and new version tag (if the server gave one).
    ///
    /// # Errors
    /// [`SyncError::Refused`] when the server's copy changed since
    /// `if_match` (412) or the card was refused.
    pub async fn put_card(
        &self,
        book: &Url,
        href: Option<&str>,
        uid: &str,
        vcard: &str,
        if_match: Option<&str>,
    ) -> Result<(String, Option<String>), SyncError> {
        self.put(book, href, uid, vcard, if_match, Kind::Card).await
    }

    /// Store a calendar item, like [`Self::put_card`].
    ///
    /// # Errors
    /// [`SyncError::Refused`] when the server's copy changed since
    /// `if_match` (412) or the event was refused.
    pub async fn put_event(
        &self,
        calendar: &Url,
        href: Option<&str>,
        uid: &str,
        ics: &str,
        if_match: Option<&str>,
    ) -> Result<(String, Option<String>), SyncError> {
        self.put(calendar, href, uid, ics, if_match, Kind::Event)
            .await
    }

    /// Whether the server emails organisers and attendees itself when a
    /// calendar changes (`calendar-auto-schedule` in its `DAV` header,
    /// RFC 6638), as `SOGo` does. Then the app must not email them too.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn auto_schedules(&self, calendar: &Url) -> Result<bool, SyncError> {
        let (_, _, headers, _) = self
            .send(&Method::OPTIONS, calendar.clone(), HeaderMap::new(), None)
            .await?;
        Ok(headers
            .get_all("dav")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|v| v.trim().eq_ignore_ascii_case("calendar-auto-schedule")))
    }

    /// Store an answered invitation in a calendar, asking the server not to
    /// email the organiser itself (`Schedule-Reply: F`, RFC 6638): the app
    /// sends that answer.
    ///
    /// # Errors
    /// As [`Self::put_event`].
    pub async fn put_answered_event(
        &self,
        calendar: &Url,
        href: Option<&str>,
        uid: &str,
        ics: &str,
        if_match: Option<&str>,
    ) -> Result<(String, Option<String>), SyncError> {
        self.put(calendar, href, uid, ics, if_match, Kind::Answered)
            .await
    }

    async fn put(
        &self,
        book: &Url,
        href: Option<&str>,
        uid: &str,
        vcard: &str,
        if_match: Option<&str>,
        kind: Kind,
    ) -> Result<(String, Option<String>), SyncError> {
        let book = as_collection(book);
        let url = match href {
            Some(href) => book.join(href),
            None => book.join(&format!("{}.{}", safe_name(uid), kind.extension())),
        }
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
        let mut base = HeaderMap::new();
        base.insert(CONTENT_TYPE, HeaderValue::from_static(kind.content_type()));
        if matches!(kind, Kind::Answered) {
            base.insert("Schedule-Reply", HeaderValue::from_static("F"));
        }
        let mut headers = base.clone();
        match (href, if_match) {
            (None, _) => {
                headers.insert("If-None-Match", HeaderValue::from_static("*"));
            }
            (Some(_), Some(etag)) => {
                headers.insert("If-Match", if_match_value(etag, kind)?);
            }
            (Some(_), None) => {}
        }
        let path = url.path().to_owned();
        let (_, status, headers, text) = self
            .send(&Method::PUT, url.clone(), headers, Some(vcard.to_owned()))
            .await?;
        if status.is_success() {
            return Ok((path, etag_of(&headers)));
        }
        let refused = |status: StatusCode, text: &str| {
            SyncError::Refused(format!(
                "the server didn't save the {} ({status}): {}",
                kind.noun(),
                text.chars().take(200).collect::<String>()
            ))
        };
        if href.is_some() || status != StatusCode::PRECONDITION_FAILED {
            return Err(refused(status, &text));
        }
        // A create answered "already there". If an earlier try got through
        // and only its reply was lost, the item there is this one, and
        // every further try would be refused the same way: take it as ours.
        let (_, found, found_headers, stored) = self
            .send(&Method::GET, url.clone(), HeaderMap::new(), None)
            .await?;
        if !found.is_success() || !has_uid(&stored, uid) {
            return Err(refused(status, &text));
        }
        let etag = etag_of(&found_headers);
        if stored.trim() == vcard.trim() {
            return Ok((path, etag));
        }
        // It was edited again while the first try was in doubt (or the
        // server rewrote it): store the text asked for now, guarded by the
        // version just read.
        let mut headers = base;
        if let Some(etag) = &etag {
            headers.insert("If-Match", if_match_value(etag, kind)?);
        }
        let (_, status, headers, text) = self
            .send(&Method::PUT, url, headers, Some(vcard.to_owned()))
            .await?;
        if status.is_success() {
            Ok((path, etag_of(&headers)))
        } else {
            Err(refused(status, &text))
        }
    }

    /// Delete a card. One already gone counts as deleted.
    ///
    /// # Errors
    /// Network or server errors.
    pub async fn delete_card(
        &self,
        book: &Url,
        href: &str,
        if_match: Option<&str>,
    ) -> Result<(), SyncError> {
        self.delete(book, href, if_match, Kind::Card).await
    }

    /// Delete a calendar item. One already gone counts as deleted.
    ///
    /// # Errors
    /// Network or server errors; [`SyncError::Refused`] when it changed
    /// on the server since `if_match`.
    pub async fn delete_event(
        &self,
        calendar: &Url,
        href: &str,
        if_match: Option<&str>,
    ) -> Result<(), SyncError> {
        self.delete(calendar, href, if_match, Kind::Event).await
    }

    /// Remove a declined or cancelled invitation from a calendar, without
    /// the server emailing the organiser (`Schedule-Reply: F`).
    ///
    /// # Errors
    /// As [`Self::delete_event`].
    pub async fn delete_answered_event(
        &self,
        calendar: &Url,
        href: &str,
        if_match: Option<&str>,
    ) -> Result<(), SyncError> {
        self.delete(calendar, href, if_match, Kind::Answered).await
    }

    async fn delete(
        &self,
        book: &Url,
        href: &str,
        if_match: Option<&str>,
        kind: Kind,
    ) -> Result<(), SyncError> {
        let url = as_collection(book)
            .join(href)
            .map_err(|e| SyncError::Invalid(e.to_string()))?;
        let mut headers = HeaderMap::new();
        if let Some(etag) = if_match {
            headers.insert("If-Match", if_match_value(etag, kind)?);
        }
        if matches!(kind, Kind::Answered) {
            headers.insert("Schedule-Reply", HeaderValue::from_static("F"));
        }
        let (_, status, _, _) = self.send(&Method::DELETE, url, headers, None).await?;
        if status.is_success() || status == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(SyncError::Refused(format!(
                "the server didn't delete the {} ({status})",
                kind.noun()
            )))
        }
    }
}

/// What is being stored.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Card,
    Event,
    /// An event that answers an invitation.
    Answered,
}

impl Kind {
    fn extension(self) -> &'static str {
        match self {
            Self::Card => "vcf",
            Self::Event | Self::Answered => "ics",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Card => "text/vcard; charset=utf-8",
            Self::Event | Self::Answered => "text/calendar; charset=utf-8",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Card => "contact",
            Self::Event | Self::Answered => "event",
        }
    }
}

/// The `ETag` header of a reply, if it has one.
fn etag_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Longest part of a uid kept in a file name.
const MAX_NAME_CHARS: usize = 100;

/// A file name from a uid: letters, digits, `-`, `_`, `.` only. A uid that
/// loses characters this way (or is cut short) gets a hash of the whole uid
/// added, so two different uids never share a name: the second would be
/// refused as "already there" for ever. A uid that fits is used as it is.
fn safe_name(uid: &str) -> String {
    let safe = |c: &char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    let name: String = uid.chars().filter(safe).take(MAX_NAME_CHARS).collect();
    if !name.is_empty() && name.len() == uid.len() {
        return name;
    }
    let kept = if name.is_empty() { "item" } else { &name };
    format!("{kept}-{:016x}", fnv1a(uid.as_bytes()))
}

/// 64-bit FNV-1a. Written out because the name it goes into must come out
/// the same on every run and every version of the app, which the standard
/// library's hasher doesn't promise.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
const PROPFIND_HOMES: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav" xmlns:cal="urn:ietf:params:xml:ns:caldav"><d:prop><card:addressbook-home-set/><cal:calendar-home-set/></d:prop></d:propfind>"#;
const PROPFIND_COLLECTIONS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/"><d:prop><d:resourcetype/><d:displayname/><a:calendar-color/><cal:supported-calendar-component-set/><d:current-user-privilege-set/></d:prop></d:propfind>"#;
const PROPFIND_OUTBOX: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav"><d:prop><cal:schedule-outbox-URL/></d:prop></d:propfind>"#;
const TODO_QUERY: &str = r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VTODO"/></c:comp-filter></c:filter></c:calendar-query>"#;
const PROPFIND_ETAGS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>"#;
const CALENDAR_MULTIGET_HEAD: &str = r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data/></d:prop>"#;
const MULTIGET_HEAD: &str = r#"<?xml version="1.0" encoding="utf-8"?><c:addressbook-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><d:getetag/><c:address-data/></d:prop>"#;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn reads_principal_and_homes() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav">
 <D:response><D:href>/SOGo/dav/</D:href>
  <D:propstat><D:prop><D:current-user-principal><D:href>/SOGo/dav/info%40mocky.co.ke/</D:href></D:current-user-principal></D:prop>
  <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#;
        let r = parse_multistatus(xml).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].href, "/SOGo/dav/");
        assert_eq!(
            r[0].principal.as_deref(),
            Some("/SOGo/dav/info%40mocky.co.ke/")
        );

        let homes = r#"<multistatus xmlns="DAV:"><response><href>/p/</href><propstat><prop>
<addressbook-home-set xmlns="urn:ietf:params:xml:ns:carddav"><href>/SOGo/dav/u/Contacts/</href></addressbook-home-set>
</prop></propstat></response></multistatus>"#;
        assert_eq!(
            parse_multistatus(homes).unwrap()[0]
                .addressbook_home
                .as_deref(),
            Some("/SOGo/dav/u/Contacts/")
        );
    }

    #[test]
    fn finds_address_books_and_cards() {
        let xml = r#"<a:multistatus xmlns:a="DAV:" xmlns:b="urn:ietf:params:xml:ns:carddav">
<a:response><a:href>/SOGo/dav/u/Contacts/personal/</a:href><a:propstat><a:prop>
 <a:resourcetype><a:collection/><b:addressbook/></a:resourcetype><a:displayname>Personal Address Book</a:displayname>
</a:prop><a:status>HTTP/1.1 200 OK</a:status></a:propstat></a:response>
<a:response><a:href>/SOGo/dav/u/Contacts/personal/5A1.vcf</a:href><a:propstat><a:prop>
 <a:getetag>"gcs00000001"</a:getetag><a:resourcetype/>
</a:prop><a:status>HTTP/1.1 200 OK</a:status></a:propstat></a:response>
</a:multistatus>"#;
        let r = parse_multistatus(xml).unwrap();
        assert!(r[0].is_addressbook);
        assert_eq!(r[0].display_name.as_deref(), Some("Personal Address Book"));
        assert!(!r[1].is_addressbook);
        assert_eq!(r[1].etag.as_deref(), Some("\"gcs00000001\""));
    }

    #[test]
    fn reads_card_text_with_entities_and_line_breaks() {
        let xml = "<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:carddav\"><d:response><d:href>/b/1.vcf</d:href><d:propstat><d:prop><d:getetag>\"e\"</d:getetag><c:address-data>BEGIN:VCARD&#13;\nVERSION:3.0&#13;\nFN:Kamau &amp; Sons&#13;\nEND:VCARD&#13;\n</c:address-data></d:prop></d:propstat></d:response><d:response><d:href>/b/gone.vcf</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response></d:multistatus>";
        let r = parse_multistatus(xml).unwrap();
        let data = r[0].data.as_deref().unwrap();
        assert!(data.contains("FN:Kamau & Sons"));
        assert!(data.contains("VERSION:3.0\r\n"));
        assert!(r[1].missing);
    }

    #[test]
    fn finds_event_calendars_with_colour() {
        let xml = r#"<D:multistatus xmlns:D="DAV:" xmlns:b="http://apple.com/ns/ical/"><D:response><D:href>/SOGo/dav/u/Calendar/personal/</D:href><D:propstat><D:status>HTTP/1.1 200 OK</D:status><D:prop><D:resourcetype><D:collection/><calendar xmlns="urn:ietf:params:xml:ns:caldav"/></D:resourcetype><D:displayname>Personal Calendar</D:displayname><b:calendar-color>#AAAAAAFF</b:calendar-color><n1:supported-calendar-component-set xmlns:n1="urn:ietf:params:xml:ns:caldav"><n1:comp name="VEVENT"/><n1:comp name="VTODO"/></n1:supported-calendar-component-set></D:prop></D:propstat></D:response><D:response><D:href>/SOGo/dav/u/Calendar/inbox/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><schedule-inbox xmlns="urn:ietf:params:xml:ns:caldav"/></D:resourcetype></D:prop></D:propstat></D:response></D:multistatus>"#;
        let r = parse_multistatus(xml).unwrap();
        assert!(r[0].is_calendar);
        assert_eq!(r[0].color.as_deref(), Some("#AAAAAAFF"));
        assert_eq!(r[0].components, ["VEVENT", "VTODO"]);
        assert!(!r[1].is_calendar);
    }

    #[test]
    fn bad_xml_is_an_error_not_a_panic() {
        assert!(parse_multistatus("<multistatus><response></oops>").is_err());
        assert!(parse_multistatus("").unwrap().is_empty());
    }

    #[test]
    fn only_https_gets_the_password() {
        assert!(check_scheme(&Url::parse("https://mail.example.com/").unwrap()).is_ok());
        if !cfg!(feature = "insecure-plaintext") {
            assert!(check_scheme(&Url::parse("http://mail.example.com/").unwrap()).is_err());
        }
    }

    #[test]
    fn card_names_are_safe() {
        assert_eq!(safe_name("a1b2-c3.vcf"), "a1b2-c3.vcf");
        let stripped = safe_name("../../etc/passwd");
        assert!(stripped.starts_with("....etcpasswd-"), "{stripped}");
        assert!(!stripped.contains('/'));
        let foreign = safe_name("ü/€");
        assert!(foreign.starts_with("item-"), "{foreign}");
        assert!(foreign.is_ascii());
    }

    #[test]
    fn different_uids_never_share_a_name() {
        // All non-ASCII: both used to become "contact".
        assert_ne!(safe_name("ü/€"), safe_name("名前"));
        // The same once the unsafe characters are dropped.
        assert_ne!(safe_name("a/b@x"), safe_name("a/bx"));
        assert_ne!(safe_name("a/b@x"), safe_name("ab@x"));
        // The same first hundred characters.
        let long = "u".repeat(150);
        let longer = format!("{long}2");
        assert_ne!(safe_name(&long), safe_name(&longer));
        assert!(safe_name(&long).len() <= MAX_NAME_CHARS + 17);
        // A name is the same every time, or a retry would miss the first try.
        assert_eq!(safe_name("ü/€"), safe_name("ü/€"));
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        // A uid that fits keeps its plain name, as before.
        let plain = "5A1F-22.33_x";
        assert_eq!(safe_name(plain), plain);
        assert_eq!(safe_name(&"u".repeat(100)), "u".repeat(100));
    }

    #[test]
    fn numeric_references_become_their_characters() {
        assert_eq!(unescape_xml("a&#13;&#10;b"), "a\r\nb");
        assert_eq!(unescape_xml("a&#xD;&#xA;b&#x9;c&#9;d"), "a\r\nb\tc\td");
        assert_eq!(unescape_xml("Caf&#233; &#xE9; &#x1F600;"), "Café é 😀");
        assert_eq!(
            unescape_xml("&lt;b&gt; &quot;x&quot; &apos;y&apos;"),
            "<b> \"x\" 'y'"
        );
        // Decoded once: &amp;lt; is the text "&lt;", not "<".
        assert_eq!(unescape_xml("&amp;lt; &amp;#13;"), "&lt; &#13;");
        // Not references: kept as written.
        assert_eq!(
            unescape_xml("a & b; &nbsp; &#; &#xZZ; &#0; &#xD800; &"),
            "a & b; &nbsp; &#; &#xZZ; &#0; &#xD800; &"
        );
        assert_eq!(unescape_xml("&#99999999999;"), "&#99999999999;");
        assert_eq!(reference_char("#X41"), Some('A'));
        assert_eq!(reference_char("13"), None);
    }

    #[test]
    fn reads_card_text_with_hex_and_other_numeric_references() {
        let xml = "<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:carddav\"><d:response><d:href>/b/1.vcf</d:href><d:propstat><d:prop><c:address-data>BEGIN:VCARD&#xD;&#xA;FN:Ren&#233;e &#x1F600;&#xD;&#xA;NOTE:a&#9;b&#xD;&#xA;END:VCARD&#xD;&#xA;</c:address-data></d:prop></d:propstat></d:response></d:multistatus>";
        let r = parse_multistatus(xml).unwrap();
        assert_eq!(
            r[0].data.as_deref(),
            Some("BEGIN:VCARD\r\nFN:Renée 😀\r\nNOTE:a\tb\r\nEND:VCARD\r\n")
        );
    }

    #[test]
    fn components_are_read_in_either_form() {
        let xml = r#"<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/c/tasks/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:calendar/></D:resourcetype><C:supported-calendar-component-set><C:comp name="VTODO"></C:comp></C:supported-calendar-component-set></D:prop></D:propstat></D:response></D:multistatus>"#;
        let r = parse_multistatus(xml).unwrap();
        assert!(r[0].is_calendar);
        assert_eq!(r[0].components, ["VTODO"]);
    }

    #[test]
    fn items_are_stored_inside_the_collection() {
        let bare = Url::parse("https://x.example/dav/me/Contacts/personal").unwrap();
        assert_eq!(
            as_collection(&bare).join("a.vcf").unwrap().path(),
            "/dav/me/Contacts/personal/a.vcf"
        );
        let slashed = Url::parse("https://x.example/dav/personal/").unwrap();
        assert_eq!(as_collection(&slashed), slashed);
        // A full path from the server still wins.
        assert_eq!(
            as_collection(&bare)
                .join("/dav/other/b.vcf")
                .unwrap()
                .path(),
            "/dav/other/b.vcf"
        );
    }

    #[test]
    fn a_version_tag_that_cannot_be_sent_is_an_error() {
        assert!(if_match_value("\"gcs00000001\"", Kind::Card).is_ok());
        assert!(if_match_value("W/\"é\"", Kind::Card).is_ok());
        let bad = if_match_value("\"a\nb\"", Kind::Event).unwrap_err();
        assert!(matches!(bad, SyncError::Refused(_)), "{bad:?}");
    }

    #[test]
    fn finds_the_uid_of_stored_text() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:abc-1\r\nFN:A\r\nEND:VCARD\r\n";
        assert!(has_uid(card, "abc-1"));
        assert!(!has_uid(card, "abc"));
        assert!(!has_uid(card, "A"));
        assert!(has_uid("uid;x-a=b:urn:uuid:12\n", "urn:uuid:12"));
        // Folded across lines.
        assert!(has_uid("UID:long-\r\n uid-value\r\n", "long-uid-value"));
        assert!(!has_uid("NOTE:UID:abc-1\r\n", "abc-1"));
    }

    #[test]
    fn an_announced_length_over_the_limit_is_refused_unread() {
        assert!(fits_in_memory(0));
        assert!(fits_in_memory(MAX_RESPONSE_BYTES as u64));
        assert!(!fits_in_memory(MAX_RESPONSE_BYTES as u64 + 1));
        assert!(!fits_in_memory(u64::MAX));
    }

    #[test]
    fn reads_privileges_and_the_scheduling_outbox() {
        let xml = r#"<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/c/shared/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:calendar/></D:resourcetype><D:current-user-privilege-set><D:privilege><D:read/></D:privilege><D:privilege><D:read-free-busy/></D:privilege></D:current-user-privilege-set></D:prop></D:propstat></D:response><D:response><D:href>/c/mine/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:calendar/></D:resourcetype><D:current-user-privilege-set><D:privilege><D:all/></D:privilege></D:current-user-privilege-set></D:prop></D:propstat></D:response><D:response><D:href>/c/plain/</D:href><D:propstat><D:prop><D:resourcetype><C:calendar/></D:resourcetype></D:prop></D:propstat></D:response><D:response><D:href>/p/</D:href><D:propstat><D:prop><C:schedule-outbox-URL><D:href>/c/outbox/</D:href></C:schedule-outbox-URL></D:prop></D:propstat></D:response></D:multistatus>"#;
        let r = parse_multistatus(xml).unwrap();
        assert_eq!(r[0].privileges, ["read", "read-free-busy"]);
        assert!(!r[0].writable());
        assert!(r[1].writable());
        // A server that doesn't say: writable.
        assert!(r[2].writable());
        assert_eq!(r[3].schedule_outbox.as_deref(), Some("/c/outbox/"));
    }

    #[test]
    fn reads_a_schedule_response() {
        let xml = "<?xml version=\"1.0\"?><C:schedule-response xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\"><C:response><C:recipient><D:href>mailto:peter@client.example</D:href></C:recipient><C:request-status>2.0;Success</C:request-status><C:calendar-data>BEGIN:VCALENDAR&#13;\nBEGIN:VFREEBUSY&#13;\nFREEBUSY:20261118T060000Z/20261118T070000Z&#13;\nEND:VFREEBUSY&#13;\nEND:VCALENDAR&#13;\n</C:calendar-data></C:response><C:response><C:recipient><D:href>mailto:who@elsewhere.example</D:href></C:recipient><C:request-status>3.7;Invalid calendar user</C:request-status></C:response></C:schedule-response>";
        let answers = parse_schedule_response(xml).unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].recipient, "peter@client.example");
        assert!(answers[0].status.starts_with("2.0"));
        let busy = mail_core::ical::free_busy_periods(answers[0].data.as_deref().unwrap());
        assert_eq!(busy.len(), 1);
        assert!(answers[1].data.is_none());
        assert!(answers[1].status.starts_with("3.7"));
    }

    #[test]
    fn a_bearer_token_is_never_printed() {
        let client = DavClient::bearer("ya29.secret".into()).unwrap();
        assert!(!format!("{client:?}").contains("ya29"));
    }

    #[test]
    fn the_login_is_never_printed() {
        let login = DavLogin {
            username: "me@x.example".into(),
            password: "hunter2".into(),
        };
        assert!(!format!("{login:?}").contains("hunter2"));
    }
}
