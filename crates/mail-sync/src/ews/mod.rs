//! Exchange Server over Exchange Web Services (EWS): SOAP over HTTPS, with
//! `NTLMv2` sign-in (and Basic where the server offers only that), and
//! Autodiscover to find the server.
//!
//! Mail is kept as MIME, as IMAP mail is: list entries come from each
//! message's header block (`SyncFolderItems`, then `GetItem`), bodies from
//! `GetItem` with `IncludeMimeContent`, and sending hands the whole MIME
//! message to `CreateItem` (`SendAndSaveCopy`), so signing, encryption and
//! HTML work unchanged. Calendars are read as iCalendar and written with
//! EWS's own fields; contacts, automatic replies and free/busy have their
//! own operations.
//!
//! Every request runs on one connection at a time per account, because an
//! NTLM sign-in belongs to the connection it was made on.

pub mod autodiscover;
pub mod convert;
pub mod ntlm;
pub mod parse;
pub mod soap;
pub mod xml;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue, LOCATION, WWW_AUTHENTICATE};
use reqwest::{Client, StatusCode, Url};

pub use parse::{
    Busy, Contact, Folder, Headers, HierarchyChanges, ItemChange, ItemChanges, ItemState, Mime,
};
pub use soap::{ContactFields, EventFields, FolderRef, MessageChange, Oof};

use self::xml::Element;
use crate::SyncError;

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(90);
/// Largest answer read, bytes.
const MAX_REPLY_BYTES: usize = 160 * 1024 * 1024;
/// Items per `GetItem` for list entries, and for MIME content.
const HEADER_BATCH: usize = 50;
const MIME_BATCH: usize = 10;
/// Items per change (`UpdateItem`, `MoveItem`, `DeleteItem`).
const CHANGE_BATCH: usize = 100;
/// Content type of SOAP and Autodiscover requests.
const XML: &str = "text/xml; charset=utf-8";

/// How an account signs in to Exchange.
#[derive(Clone, PartialEq, Eq)]
pub struct EwsLogin {
    /// `DOMAIN\user`, `user@domain`, or the address.
    pub username: String,
    /// The password.
    pub password: String,
}

impl std::fmt::Debug for EwsLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EwsLogin")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Ntlm,
    Basic,
}

/// What came back from one HTTP request.
struct Reply {
    status: StatusCode,
    challenges: Vec<String>,
    location: Option<String>,
    body: String,
}

enum Auth<'a> {
    None,
    Header(&'a str),
    Basic,
}

/// HTTPS with the account's sign-in, one request at a time.
struct Transport {
    http: Client,
    login: EwsLogin,
    ntlm: Option<ntlm::Credentials>,
    scheme: Mutex<Option<Scheme>>,
    serial: tokio::sync::Mutex<()>,
}

fn secure(url: &Url) -> Result<(), SyncError> {
    if url.scheme() == "https" || (cfg!(feature = "insecure-plaintext") && url.scheme() == "http") {
        Ok(())
    } else {
        Err(SyncError::Config(format!(
            "{} isn't a secure (https) address",
            url.host_str().unwrap_or_default()
        )))
    }
}

impl Transport {
    fn new(login: EwsLogin) -> Result<Self, SyncError> {
        let http = Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Mocky Mail")
            // An NTLM sign-in lives on its connection: keep just one.
            .pool_max_idle_per_host(1)
            .http1_only()
            .build()
            .map_err(|e| SyncError::Config(e.to_string()))?;
        let ntlm = ntlm::Credentials::new(&login.username, &login.password).ok();
        Ok(Self {
            http,
            login,
            ntlm,
            scheme: Mutex::new(None),
            serial: tokio::sync::Mutex::new(()),
        })
    }

    fn scheme(&self) -> Option<Scheme> {
        *self.scheme.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_scheme(&self, scheme: Scheme) {
        *self.scheme.lock().unwrap_or_else(PoisonError::into_inner) = Some(scheme);
    }

    async fn send(
        &self,
        url: &Url,
        body: Option<&str>,
        auth: Auth<'_>,
    ) -> Result<Reply, SyncError> {
        secure(url)?;
        let mut request = self
            .http
            .post(url.clone())
            .header(CONTENT_TYPE, XML)
            .body(body.unwrap_or_default().to_owned());
        match auth {
            Auth::None => {}
            Auth::Header(value) => {
                let mut value = HeaderValue::from_str(value).map_err(|_| SyncError::Auth)?;
                value.set_sensitive(true);
                request = request.header(AUTHORIZATION, value);
            }
            Auth::Basic => {
                request = request.basic_auth(&self.login.username, Some(&self.login.password));
            }
        }
        let mut response = request
            .send()
            .await
            .map_err(|e| crate::dav::http_error(url, &e))?;
        let status = response.status();
        let challenges = response
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_owned))
            .collect();
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let too_large = || SyncError::Protocol("Exchange's answer was too large".to_owned());
        if response
            .content_length()
            .is_some_and(|n| usize::try_from(n).map_or(true, |n| n > MAX_REPLY_BYTES))
        {
            return Err(too_large());
        }
        // Read to the end, so the connection (and its sign-in) is kept.
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| crate::dav::http_error(url, &e))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_REPLY_BYTES {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Reply {
            status,
            challenges,
            location,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }

    /// The three NTLM legs, the last carrying the request.
    async fn ntlm_round(&self, url: &Url, body: &str) -> Result<Reply, SyncError> {
        let Some(credentials) = &self.ntlm else {
            return Err(SyncError::Auth);
        };
        let (handshake, negotiate) = ntlm::Handshake::negotiate(credentials);
        let leg = self.send(url, None, Auth::Header(&negotiate)).await?;
        if leg.status != StatusCode::UNAUTHORIZED {
            return Err(SyncError::Protocol(format!(
                "Exchange answered {} to the NTLM sign-in instead of a challenge",
                leg.status
            )));
        }
        let token = ntlm::ntlm_token(&leg.challenges).ok_or(SyncError::Auth)?;
        let mut client_challenge = [0u8; 8];
        getrandom::fill(&mut client_challenge)
            .map_err(|_| SyncError::Protocol("couldn't make a random challenge".into()))?;
        let context = ntlm::Context {
            client_challenge,
            unix_seconds: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            host: url.host_str().unwrap_or_default(),
            channel_binding: None,
        };
        let header = handshake
            .authenticate(token, &context)
            .map_err(|e| SyncError::Protocol(e.to_string()))?;
        self.send(url, Some(body), Auth::Header(&header)).await
    }

    /// POST with the sign-in the server asks for: NTLM when offered (the
    /// connection stays signed in), else Basic (over HTTPS only).
    async fn post(&self, url: &Url, body: &str) -> Result<Reply, SyncError> {
        let _one_at_a_time = self.serial.lock().await;
        let known = self.scheme();
        let first = match known {
            Some(Scheme::Basic) => self.send(url, Some(body), Auth::Basic).await?,
            _ => self.send(url, Some(body), Auth::None).await?,
        };
        if first.status != StatusCode::UNAUTHORIZED {
            return Ok(first);
        }
        if known == Some(Scheme::Basic) {
            return Err(SyncError::Auth);
        }
        let offers_ntlm = ntlm::offers(&first.challenges, "ntlm");
        let offers_basic = ntlm::offers(&first.challenges, "basic");
        if offers_ntlm && self.ntlm.is_some() {
            let reply = self.ntlm_round(url, body).await?;
            if reply.status != StatusCode::UNAUTHORIZED {
                self.set_scheme(Scheme::Ntlm);
                return Ok(reply);
            }
        }
        if offers_basic {
            let reply = self.send(url, Some(body), Auth::Basic).await?;
            if reply.status != StatusCode::UNAUTHORIZED {
                self.set_scheme(Scheme::Basic);
                return Ok(reply);
            }
            return Err(SyncError::Auth);
        }
        if offers_ntlm {
            return Err(SyncError::Auth);
        }
        Err(SyncError::Config(
            "This Exchange server only allows a kind of sign-in Mocky Mail doesn't support (Kerberos or modern authentication). Ask your IT department to allow NTLM or Basic sign-in for Exchange Web Services.".to_owned(),
        ))
    }
}

/// A signed-in connection to one mailbox's EWS address.
#[derive(Clone)]
pub struct EwsClient {
    url: Url,
    transport: Arc<Transport>,
}

impl std::fmt::Debug for EwsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EwsClient")
            .field("host", &self.url.host_str())
            .finish_non_exhaustive()
    }
}

/// The well-known folders' ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WellKnown {
    /// Inbox.
    pub inbox: Option<String>,
    /// Sent Items.
    pub sent: Option<String>,
    /// Drafts.
    pub drafts: Option<String>,
    /// Deleted Items.
    pub trash: Option<String>,
    /// Junk Email.
    pub junk: Option<String>,
    /// Archive (Exchange 2016 and later).
    pub archive: Option<String>,
    /// Outbox.
    pub outbox: Option<String>,
    /// The top of the mailbox.
    pub root: Option<String>,
    /// Calendar.
    pub calendar: Option<String>,
    /// Contacts.
    pub contacts: Option<String>,
    /// Tasks.
    pub tasks: Option<String>,
}

const WELL_KNOWN: [&str; 10] = [
    "inbox",
    "sentitems",
    "drafts",
    "deleteditems",
    "junkemail",
    "outbox",
    "msgfolderroot",
    "calendar",
    "contacts",
    "tasks",
];

impl EwsClient {
    /// A client for this EWS address.
    ///
    /// # Errors
    /// An address that isn't `https`, or no HTTP client.
    pub fn new(ews_url: &str, login: EwsLogin) -> Result<Self, SyncError> {
        let url = Url::parse(ews_url.trim())
            .map_err(|_| SyncError::Config(format!("\"{ews_url}\" isn't a web address")))?;
        secure(&url)?;
        Ok(Self {
            url,
            transport: Arc::new(Transport::new(login)?),
        })
    }

    /// The EWS address.
    #[must_use]
    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    /// Run one operation; the answer's XML.
    async fn call(&self, operation: &str) -> Result<Element, SyncError> {
        let reply = self
            .transport
            .post(&self.url, &soap::envelope(operation))
            .await?;
        let host = || self.url.host_str().unwrap_or_default().to_owned();
        match reply.status.as_u16() {
            200 => Element::parse(&reply.body),
            500 => {
                let root = Element::parse(&reply.body).map_err(|_| {
                    SyncError::Protocol(format!("{} had an internal error", host()))
                })?;
                parse::body(&root)?;
                Ok(root)
            }
            403 => Err(SyncError::Refused(
                "Exchange doesn't allow this account to use Exchange Web Services".to_owned(),
            )),
            404 => Err(SyncError::Config(format!(
                "There is no Exchange Web Services at {}. Check the EWS address.",
                self.url
            ))),
            429 | 503 => Err(SyncError::Protocol(format!("{} is busy", host()))),
            300..=399 => Err(SyncError::Config(format!(
                "{} sent Mocky Mail to another address ({}). Use that EWS address instead.",
                host(),
                reply.location.unwrap_or_default()
            ))),
            other => Err(SyncError::Protocol(format!("{} answered {other}", host()))),
        }
    }

    /// Sign in and read the inbox: proves the address, name and password.
    ///
    /// # Errors
    /// [`SyncError::Auth`] for a refused sign-in, and the rest.
    pub async fn test(&self) -> Result<(), SyncError> {
        let root = self.call(&soap::get_folders(&["inbox"])).await?;
        parse::single(&root).map(|_| ())
    }

    /// The well-known folders' ids.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn well_known(&self) -> Result<WellKnown, SyncError> {
        let root = self.call(&soap::get_folders(&WELL_KNOWN)).await?;
        let ids = parse::folder_ids(&root)?;
        let at = |n: usize| ids.get(n).cloned().flatten();
        // Archive is newer than the schema asked for: asked on its own, and
        // a server that doesn't know it simply has none.
        let archive = match self.call(&soap::get_folders(&["archive"])).await {
            Ok(root) => parse::folder_ids(&root)
                .ok()
                .and_then(|ids| ids.into_iter().next().flatten()),
            Err(error) if error.is_transient() => return Err(error),
            Err(_) => None,
        };
        Ok(WellKnown {
            inbox: at(0),
            sent: at(1),
            drafts: at(2),
            trash: at(3),
            junk: at(4),
            outbox: at(5),
            root: at(6),
            calendar: at(7),
            contacts: at(8),
            tasks: at(9),
            archive,
        })
    }

    /// Every folder change since `state` (all folders without one).
    ///
    /// # Errors
    /// [`SyncError::Refused`] with [`parse::INVALID_SYNC_STATE`] when the
    /// state is stale; network errors and refusals.
    pub async fn sync_hierarchy(&self, state: Option<&str>) -> Result<HierarchyChanges, SyncError> {
        let mut out = HierarchyChanges::default();
        let mut state = state.map(str::to_owned);
        for _ in 0..100 {
            let root = self
                .call(&soap::sync_folder_hierarchy(state.as_deref()))
                .await?;
            let page = parse::hierarchy(&root)?;
            out.changed.extend(page.changed);
            out.deleted.extend(page.deleted);
            out.sync_state.clone_from(&page.sync_state);
            state = Some(page.sync_state);
            if page.done {
                out.done = true;
                break;
            }
        }
        Ok(out)
    }

    /// One page of changes in a folder since `state`.
    ///
    /// # Errors
    /// As [`EwsClient::sync_hierarchy`].
    pub async fn sync_items(
        &self,
        folder: &str,
        state: Option<&str>,
        max: u32,
        mail: bool,
    ) -> Result<ItemChanges, SyncError> {
        let root = self
            .call(&soap::sync_folder_items(
                &FolderRef::Id(folder),
                state,
                max,
                mail,
            ))
            .await?;
        parse::item_changes(&root)
    }

    /// A page of a folder's newest items.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn find_newest(
        &self,
        folder: &str,
        offset: u32,
        max: u32,
        since: Option<i64>,
    ) -> Result<parse::FoundItems, SyncError> {
        let since = since.map(convert::utc);
        let root = self
            .call(&soap::find_newest(
                &FolderRef::Id(folder),
                offset,
                max,
                since.as_deref(),
            ))
            .await?;
        parse::found_items(&root)
    }

    /// The ids of a folder's items with this Message-ID.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn find_by_message_id(
        &self,
        folder: &str,
        message_id: &str,
    ) -> Result<Vec<String>, SyncError> {
        let root = self
            .call(&soap::find_by_message_id(
                &FolderRef::Id(folder),
                message_id,
            ))
            .await?;
        Ok(parse::found_items(&root)?
            .items
            .into_iter()
            .map(|i| i.id)
            .collect())
    }

    /// List entries of messages (those gone meanwhile are left out).
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn get_headers(&self, ids: &[String]) -> Result<Vec<Headers>, SyncError> {
        let mut out = Vec::new();
        for chunk in ids.chunks(HEADER_BATCH) {
            let root = self.call(&soap::get_headers(chunk)).await?;
            out.extend(parse::headers(&root)?);
        }
        Ok(out)
    }

    /// Items' whole MIME content (those gone meanwhile are left out).
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn get_mime(&self, ids: &[String]) -> Result<Vec<Mime>, SyncError> {
        let mut out = Vec::new();
        for chunk in ids.chunks(MIME_BATCH) {
            let root = self.call(&soap::get_mime(chunk)).await?;
            out.extend(parse::mime(&root)?);
        }
        Ok(out)
    }

    /// One message's MIME content, `None` when it is gone.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn get_message(&self, id: &str) -> Result<Option<Vec<u8>>, SyncError> {
        Ok(self
            .get_mime(&[id.to_owned()])
            .await?
            .into_iter()
            .next()
            .map(|m| m.raw))
    }

    /// Change read, flag or categories on messages.
    ///
    /// # Errors
    /// Network errors and refusals (messages gone count as done).
    pub async fn update_messages(
        &self,
        ids: &[String],
        change: &MessageChange<'_>,
    ) -> Result<(), SyncError> {
        for chunk in ids.chunks(CHANGE_BATCH) {
            let root = self.call(&soap::update_messages(chunk, change)).await?;
            parse::all_done(&root)?;
        }
        Ok(())
    }

    /// Move or copy items to a folder.
    ///
    /// # Errors
    /// Network errors and refusals (items gone count as done).
    pub async fn move_or_copy(
        &self,
        ids: &[String],
        to: &str,
        copy: bool,
    ) -> Result<(), SyncError> {
        for chunk in ids.chunks(CHANGE_BATCH) {
            let root = self
                .call(&soap::move_or_copy(chunk, &FolderRef::Id(to), copy))
                .await?;
            parse::all_done(&root)?;
        }
        Ok(())
    }

    /// Delete items, for good or to Deleted Items.
    ///
    /// # Errors
    /// Network errors and refusals (items gone count as done).
    pub async fn delete_items(&self, ids: &[String], hard: bool) -> Result<(), SyncError> {
        for chunk in ids.chunks(CHANGE_BATCH) {
            let root = self.call(&soap::delete_items(chunk, hard)).await?;
            parse::all_done(&root)?;
        }
        Ok(())
    }

    /// Send a whole MIME message (a copy goes to Sent Items). `bcc`: blind
    /// copies.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn send_mime(&self, raw: &[u8], bcc: &[String]) -> Result<(), SyncError> {
        let mime = base64::engine::general_purpose::STANDARD.encode(raw);
        let root = self.call(&soap::send_mime(&mime, bcc)).await?;
        parse::single(&root).map(|_| ())
    }

    /// Save a whole MIME message in a folder (a draft, or an imported
    /// message); its new id.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn save_mime(
        &self,
        raw: &[u8],
        folder: &str,
        draft: bool,
    ) -> Result<Option<String>, SyncError> {
        let mime = base64::engine::general_purpose::STANDARD.encode(raw);
        let root = self
            .call(&soap::save_mime(&mime, &FolderRef::Id(folder), draft))
            .await?;
        Ok(parse::created_items(&root)?
            .into_iter()
            .next()
            .map(|(id, _)| id))
    }

    /// Make a mail folder; its id.
    ///
    /// # Errors
    /// Network errors and refusals (a folder of that name already there).
    pub async fn create_folder(&self, parent: &str, name: &str) -> Result<String, SyncError> {
        let root = self
            .call(&soap::create_folder(&FolderRef::Id(parent), name))
            .await?;
        parse::created_folder(&root)
    }

    /// Rename a folder.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn rename_folder(&self, id: &str, name: &str) -> Result<(), SyncError> {
        let root = self.call(&soap::rename_folder(id, name)).await?;
        parse::single(&root).map(|_| ())
    }

    /// Delete a folder (to Deleted Items, with what is in it).
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn delete_folder(&self, id: &str) -> Result<(), SyncError> {
        let root = self.call(&soap::delete_folder(id)).await?;
        parse::single(&root).map(|_| ())
    }

    /// The mailbox's automatic replies.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn get_oof(&self, address: &str) -> Result<Oof, SyncError> {
        let root = self.call(&soap::get_oof(address)).await?;
        parse::oof(&root)
    }

    /// Save the mailbox's automatic replies.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn set_oof(&self, address: &str, oof: &Oof) -> Result<(), SyncError> {
        let root = self.call(&soap::set_oof(address, oof)).await?;
        parse::oof_saved(&root)
    }

    /// When people are busy between two times (Unix seconds): per address
    /// asked, in order, `None` when Exchange couldn't tell.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn availability(
        &self,
        emails: &[String],
        from: i64,
        to: i64,
    ) -> Result<Vec<Option<Vec<mail_core::ical::BusyPeriod>>>, SyncError> {
        let root = self
            .call(&soap::get_availability(
                emails,
                &convert::utc_plain(from),
                &convert::utc_plain(to),
            ))
            .await?;
        Ok(parse::availability(&root)?
            .into_iter()
            .map(|busy| busy.map(|b| convert::busy_periods(&b)))
            .collect())
    }

    /// Make a calendar item; its id.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn create_event(
        &self,
        calendar: &str,
        fields: &EventFields,
        notify: bool,
    ) -> Result<String, SyncError> {
        let root = self
            .call(&soap::create_event(
                &FolderRef::Id(calendar),
                fields,
                notify,
            ))
            .await?;
        parse::created_items(&root)?
            .into_iter()
            .next()
            .map(|(id, _)| id)
            .ok_or_else(|| {
                SyncError::Protocol("Exchange made the event but didn't say its id".into())
            })
    }

    /// Change a calendar item.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn update_event(
        &self,
        id: &str,
        fields: &EventFields,
        recurrence: Option<&str>,
        notify: bool,
    ) -> Result<(), SyncError> {
        let root = self
            .call(&soap::update_event(id, fields, recurrence, notify))
            .await?;
        parse::single(&root).map(|_| ())
    }

    /// Delete a calendar item.
    ///
    /// # Errors
    /// Network errors and refusals (one already gone counts as deleted).
    pub async fn delete_event(&self, id: &str, notify: bool) -> Result<(), SyncError> {
        let root = self.call(&soap::delete_event(id, notify)).await?;
        parse::all_done(&root)
    }

    /// Contacts' fields.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn get_contacts(&self, ids: &[String]) -> Result<Vec<Contact>, SyncError> {
        let mut out = Vec::new();
        for chunk in ids.chunks(HEADER_BATCH) {
            let root = self.call(&soap::get_contacts(chunk)).await?;
            out.extend(parse::contacts(&root)?);
        }
        Ok(out)
    }

    /// Make a contact; its id and change key.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn create_contact(
        &self,
        folder: &str,
        contact: &ContactFields,
    ) -> Result<(String, Option<String>), SyncError> {
        let root = self
            .call(&soap::create_contact(&FolderRef::Id(folder), contact))
            .await?;
        parse::created_items(&root)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                SyncError::Protocol("Exchange made the contact but didn't say its id".into())
            })
    }

    /// Change a contact's simple fields; its new change key.
    ///
    /// # Errors
    /// Network errors and refusals.
    pub async fn update_contact(
        &self,
        id: &str,
        contact: &ContactFields,
    ) -> Result<Option<String>, SyncError> {
        let root = self.call(&soap::update_contact(id, contact)).await?;
        Ok(parse::created_items(&root)?
            .into_iter()
            .next()
            .and_then(|(_, key)| key))
    }

    /// Autodiscover with this client's sign-in: the EWS address for `email`.
    ///
    /// # Errors
    /// [`SyncError::Config`] when no address could be found.
    pub async fn discover(email: &str, login: &EwsLogin) -> Result<String, SyncError> {
        discover(email, login).await
    }
}

/// One Autodiscover request to `url`; the answer.
async fn ask_autodiscover(
    transport: &Transport,
    url: &Url,
    email: &str,
) -> Result<autodiscover::Answer, SyncError> {
    let reply = transport.post(url, &autodiscover::request(email)).await?;
    match reply.status.as_u16() {
        200 => autodiscover::parse(&reply.body),
        // Sent on by HTTP: followed only to another https address.
        301 | 302 | 303 | 307 | 308 => reply
            .location
            .and_then(|l| url.join(&l).ok())
            .filter(|next| autodiscover::acceptable(next.as_str()).is_some())
            .map(|next| autodiscover::Answer::RedirectUrl(next.to_string()))
            .ok_or_else(|| SyncError::Protocol("Autodiscover redirected nowhere safe".into())),
        401 => Err(SyncError::Auth),
        status => Err(SyncError::Protocol(format!(
            "Autodiscover answered {status}"
        ))),
    }
}

/// The SRV record `_autodiscover._tcp.<domain>`'s target, if it has one.
async fn srv_target(domain: &str) -> Option<(String, u16)> {
    use hickory_resolver::proto::rr::{RData, RecordType};
    let resolver = hickory_resolver::TokioResolver::builder_tokio()
        .ok()?
        .build()
        .ok()?;
    let lookup = tokio::time::timeout(
        Duration::from_secs(5),
        resolver.lookup(format!("_autodiscover._tcp.{domain}."), RecordType::SRV),
    )
    .await
    .ok()?
    .ok()?;
    let mut records: Vec<(u16, u16, String, u16)> = lookup
        .answers()
        .iter()
        .filter_map(|record| match &record.data {
            RData::SRV(srv) => Some((
                srv.priority,
                u16::MAX.saturating_sub(srv.weight),
                srv.target.to_string(),
                srv.port,
            )),
            _ => None,
        })
        .collect();
    records.sort();
    records
        .into_iter()
        .next()
        .map(|(_, _, target, port)| (target, port))
}

/// The plain-HTTP redirect method: where `http://autodiscover.<domain>`
/// sends Autodiscover, asked without the sign-in.
async fn redirect_target(domain: &str) -> Option<String> {
    let http = Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mocky Mail")
        .build()
        .ok()?;
    let probe = autodiscover::redirect_probe(domain);
    let response = http.get(&probe).send().await.ok()?;
    if !response.status().is_redirection() {
        return None;
    }
    let location = response.headers().get(LOCATION)?.to_str().ok()?;
    let next = Url::parse(&probe).ok()?.join(location).ok()?;
    autodiscover::acceptable(next.as_str()).map(|u| u.to_string())
}

/// Autodiscover: the EWS address for `email`, asking the places
/// [`autodiscover`] lists in order and following its redirects.
///
/// # Errors
/// [`SyncError::Auth`] when a server refused the sign-in;
/// [`SyncError::Config`] when no address could be found.
pub async fn discover(email: &str, login: &EwsLogin) -> Result<String, SyncError> {
    let transport = Transport::new(login.clone())?;
    let mut hops = autodiscover::Hops::default();
    let mut address = email.trim().to_owned();
    let mut refused = false;
    'addresses: loop {
        let domain = autodiscover::domain_of(&address)
            .ok_or_else(|| SyncError::Invalid(format!("{address} isn't an email address")))?;
        let mut queue: Vec<String> = autodiscover::candidates(&domain);
        let mut asked_extra = false;
        let mut at = 0usize;
        loop {
            let Some(next) = queue.get(at).cloned() else {
                if asked_extra {
                    break;
                }
                asked_extra = true;
                // Then the redirect method and the SRV record.
                if let Some(target) = redirect_target(&domain).await {
                    queue.push(target);
                }
                if let Some(candidate) = srv_target(&domain)
                    .await
                    .and_then(|(target, port)| autodiscover::srv_candidate(&target, port))
                {
                    queue.push(candidate);
                }
                continue;
            };
            at = at.saturating_add(1);
            let Some(url) = autodiscover::acceptable(&next) else {
                continue;
            };
            match ask_autodiscover(&transport, &url, &address).await {
                Ok(autodiscover::Answer::Settings {
                    ews_url: Some(ews), ..
                }) => return Ok(ews),
                Ok(autodiscover::Answer::RedirectAddr(other)) => {
                    hops.take(&other)?;
                    address = other;
                    continue 'addresses;
                }
                Ok(autodiscover::Answer::RedirectUrl(other)) => {
                    hops.take(&other)?;
                    if autodiscover::acceptable(&other).is_some() {
                        // Asked next, before the rest.
                        queue.insert(at, other);
                    }
                }
                Err(SyncError::Auth) => refused = true,
                Ok(_) | Err(_) => {}
            }
        }
        break;
    }
    if refused {
        return Err(SyncError::Auth);
    }
    Err(SyncError::Config(
        "Couldn't find the Exchange server with Autodiscover. Enter the EWS address (ask your IT department; it often ends in /EWS/Exchange.asmx).".to_owned(),
    ))
}

#[cfg(test)]
mod tests;
