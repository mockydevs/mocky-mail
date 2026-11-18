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
                }
                path.push(name);
                text.clear();
            }
            Event::Empty(e) => {
                let name = local(e.local_name().as_ref());
                if let Some(r) = current.as_mut() {
                    mark_type(r, &path, &name);
                    if name == "comp"
                        && path.last().map(String::as_str)
                            == Some("supported-calendar-component-set")
                        && let Ok(Some(attr)) = e.try_get_attribute("name")
                    {
                        r.components.push(attr.value.to_ascii_uppercase());
                    }
                }
            }
            Event::Text(t) => text.push_str(&unescape_xml(t.as_ref())),
            Event::GeneralRef(r) => {
                // An entity such as &amp; arrives on its own.
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
    if path.last().map(String::as_str) == Some("resourcetype") {
        match name {
            "addressbook" => r.is_addressbook = true,
            "calendar" => r.is_calendar = true,
            _ => {}
        }
    }
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#13;", "\r")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
}

/// A `WebDAV` connection to one server.
#[derive(Debug, Clone)]
pub struct DavClient {
    http: Client,
    login: DavLogin,
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

fn check_scheme(url: &Url) -> Result<(), SyncError> {
    if url.scheme() == "https" || (cfg!(feature = "insecure-plaintext") && url.scheme() == "http") {
        Ok(())
    } else {
        Err(SyncError::Config(format!(
            "{url} isn't a secure (https) address, so Mocky Mail won't send your password to it."
        )))
    }
}

fn http_error(url: &Url, error: &reqwest::Error) -> SyncError {
    let host = url.host_str().unwrap_or_default().to_owned();
    if error.is_timeout() {
        SyncError::Timeout { host }
    } else if error.is_connect() {
        SyncError::Connect {
            port: url.port_or_known_default().unwrap_or(443),
            host,
            reason: error.to_string(),
        }
    } else {
        SyncError::Protocol(error.to_string())
    }
}

impl DavClient {
    /// A client for this login.
    ///
    /// # Errors
    /// When the HTTP client can't be set up.
    pub fn new(login: DavLogin) -> Result<Self, SyncError> {
        let http = Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Mocky Mail")
            .build()
            .map_err(|e| SyncError::Config(e.to_string()))?;
        Ok(Self { http, login })
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
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .basic_auth(&self.login.username, Some(&self.login.password))
                .headers(headers.clone());
            if let Some(body) = &body {
                request = request.body(body.clone());
            }
            let response = request.send().await.map_err(|e| http_error(&url, &e))?;
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
            if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                return Err(SyncError::Auth);
            }
            let headers = response.headers().clone();
            let bytes = response.bytes().await.map_err(|e| http_error(&url, &e))?;
            if bytes.len() > MAX_RESPONSE_BYTES {
                return Err(SyncError::Protocol(
                    "the server's reply was too large".to_owned(),
                ));
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
        let (at, found) = self
            .home_collections(server, "caldav", "calendars", |r| r.calendar_home.clone())
            .await?;
        Ok(found
            .into_iter()
            .filter(|r| r.is_calendar)
            .filter(|r| r.components.is_empty() || r.components.iter().any(|c| c == "VEVENT"))
            .filter_map(|r| {
                Some(CalendarCollection {
                    url: at.join(&r.href).ok()?,
                    name: r.display_name,
                    color: r.color,
                })
            })
            .collect())
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
        let url = match href {
            Some(href) => book.join(href),
            None => book.join(&format!("{}.{}", safe_name(uid), kind.extension())),
        }
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(kind.content_type()));
        if matches!(kind, Kind::Answered) {
            headers.insert("Schedule-Reply", HeaderValue::from_static("F"));
        }
        match (href, if_match) {
            (None, _) => {
                headers.insert("If-None-Match", HeaderValue::from_static("*"));
            }
            (Some(_), Some(etag)) => {
                if let Ok(value) = HeaderValue::from_str(etag) {
                    headers.insert("If-Match", value);
                }
            }
            (Some(_), None) => {}
        }
        let path = url.path().to_owned();
        let (_, status, headers, text) = self
            .send(&Method::PUT, url, headers, Some(vcard.to_owned()))
            .await?;
        if status.is_success() {
            let etag = headers
                .get(ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            return Ok((path, etag));
        }
        Err(SyncError::Refused(format!(
            "the server didn't save the {} ({status}): {}",
            kind.noun(),
            text.chars().take(200).collect::<String>()
        )))
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
        let url = book
            .join(href)
            .map_err(|e| SyncError::Invalid(e.to_string()))?;
        let mut headers = HeaderMap::new();
        if let Some(value) = if_match.and_then(|e| HeaderValue::from_str(e).ok()) {
            headers.insert("If-Match", value);
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

/// A file name from a uid: letters, digits, `-`, `_`, `.` only.
fn safe_name(uid: &str) -> String {
    let name: String = uid
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(100)
        .collect();
    if name.is_empty() {
        "contact".to_owned()
    } else {
        name
    }
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
const PROPFIND_HOMES: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav" xmlns:cal="urn:ietf:params:xml:ns:caldav"><d:prop><card:addressbook-home-set/><cal:calendar-home-set/></d:prop></d:propfind>"#;
const PROPFIND_COLLECTIONS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/"><d:prop><d:resourcetype/><d:displayname/><a:calendar-color/><cal:supported-calendar-component-set/></d:prop></d:propfind>"#;
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
        assert_eq!(safe_name("../../etc/passwd"), "....etcpasswd");
        assert_eq!(safe_name("ü/€"), "contact");
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
