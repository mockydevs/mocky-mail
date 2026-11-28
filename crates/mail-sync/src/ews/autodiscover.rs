//! Finding an Exchange Server's EWS address from an email address: POX
//! Autodiscover (`autodiscover.xml`, [MS-OXDSCLI]).
//!
//! Where it asks, in order: `https://<domain>/autodiscover/autodiscover.xml`,
//! `https://autodiscover.<domain>/autodiscover/autodiscover.xml`, the HTTP
//! redirect method (`http://autodiscover.<domain>/…`, asked without the
//! password, whose redirect must lead to an `https` address), then the
//! `_autodiscover._tcp.<domain>` SRV record. An answer may send it to
//! another address (`redirectAddr`, asked again from the start) or another
//! server (`redirectUrl`, `https` only); at most [`MAX_HOPS`] such steps
//! are taken. Every request but the HTTP redirect probe carries the
//! sign-in, and only ever over `https`.

use super::xml::{Element, escape};
use crate::SyncError;

/// Most redirects (to another address or server) followed.
pub const MAX_HOPS: usize = 10;

/// The request schema and the answer asked for.
const REQUEST_SCHEMA: &str =
    "http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006";
const RESPONSE_SCHEMA: &str =
    "http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a";

/// The POX request for an address.
#[must_use]
pub fn request(email: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<Autodiscover xmlns="{request}"><Request>"#,
            "<EMailAddress>{email}</EMailAddress>",
            "<AcceptableResponseSchema>{response}</AcceptableResponseSchema>",
            "</Request></Autodiscover>"
        ),
        request = REQUEST_SCHEMA,
        email = escape(email),
        response = RESPONSE_SCHEMA
    )
}

/// The `https` addresses asked first, in order, for an address's domain.
#[must_use]
pub fn candidates(domain: &str) -> Vec<String> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    vec![
        format!("https://{domain}/autodiscover/autodiscover.xml"),
        format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"),
    ]
}

/// The plain-HTTP address of the redirect method (asked without the
/// sign-in; only its redirect is used).
#[must_use]
pub fn redirect_probe(domain: &str) -> String {
    format!(
        "http://autodiscover.{}/autodiscover/autodiscover.xml",
        domain.trim().trim_end_matches('.').to_ascii_lowercase()
    )
}

/// The Autodiscover address on a host named by an SRV record.
#[must_use]
pub fn srv_candidate(target: &str, port: u16) -> Option<String> {
    let host = target.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || !is_host(&host) {
        return None;
    }
    Some(if port == 443 {
        format!("https://{host}/autodiscover/autodiscover.xml")
    } else {
        format!("https://{host}:{port}/autodiscover/autodiscover.xml")
    })
}

fn is_host(host: &str) -> bool {
    host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// An address Autodiscover may be sent on to (a `redirectUrl`, an HTTP
/// redirect, a settings answer's EWS address): `https` only, with a host,
/// and no user name or password in it.
#[must_use]
pub fn acceptable(url: &str) -> Option<reqwest::Url> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    let secure = parsed.scheme() == "https"
        || (cfg!(feature = "insecure-plaintext") && parsed.scheme() == "http");
    (secure
        && parsed.host_str().is_some_and(|h| !h.is_empty())
        && parsed.username().is_empty()
        && parsed.password().is_none())
    .then_some(parsed)
}

/// What an Autodiscover answer says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The mailbox's settings.
    Settings {
        /// The EWS address (inside the organisation first, then outside).
        ews_url: Option<String>,
        /// The other EWS addresses offered.
        others: Vec<String>,
        /// The mailbox's own address, if the server said.
        address: Option<String>,
    },
    /// Ask again for this address.
    RedirectAddr(String),
    /// Ask this server instead.
    RedirectUrl(String),
    /// The server refused (for example: no such mailbox).
    Error(String),
}

/// Read an Autodiscover answer.
///
/// # Errors
/// [`SyncError::Protocol`] for one that isn't an Autodiscover answer.
pub fn parse(xml: &str) -> Result<Answer, SyncError> {
    let root = Element::parse(xml)?;
    let not_one = || SyncError::Protocol("that address doesn't answer Autodiscover".into());
    if root.name != "Autodiscover" {
        return Err(not_one());
    }
    let response = root.child("Response").ok_or_else(not_one)?;
    if let Some(error) = response.child("Error") {
        let code = error.text_of("ErrorCode").unwrap_or("");
        let message = error.text_of("Message").unwrap_or(code);
        return Ok(Answer::Error(message.to_owned()));
    }
    let account = response.child("Account").ok_or_else(not_one)?;
    match account.text_of("Action").unwrap_or("settings") {
        "redirectAddr" => account
            .text_of("RedirectAddr")
            .map(|a| Answer::RedirectAddr(a.to_owned()))
            .ok_or_else(not_one),
        "redirectUrl" => account
            .text_of("RedirectUrl")
            .map(|u| Answer::RedirectUrl(u.to_owned()))
            .ok_or_else(not_one),
        _ => {
            // EXCH: inside the organisation; EXPR: from outside; WEB: the
            // addresses of Outlook on the web (EWS under it, 2010 on).
            let mut urls: Vec<String> = Vec::new();
            for kind in ["EXCH", "EXPR", "WEB"] {
                for protocol in account.children_named("Protocol") {
                    if protocol.text_of("Type") != Some(kind) {
                        continue;
                    }
                    for name in ["EwsUrl", "ExternalEwsUrl"] {
                        if let Some(url) = protocol.text_of(name)
                            && !urls.iter().any(|u| u.eq_ignore_ascii_case(url))
                        {
                            urls.push(url.to_owned());
                        }
                    }
                }
            }
            let address = response
                .at(&["User", "AutoDiscoverSMTPAddress"])
                .map(|e| e.text.trim().to_owned())
                .filter(|a| !a.is_empty());
            let mut secure = urls.into_iter().filter(|u| acceptable(u).is_some());
            Ok(Answer::Settings {
                ews_url: secure.next(),
                others: secure.collect(),
                address,
            })
        }
    }
}

/// The steps taken so far, so a loop of redirects stops.
#[derive(Debug, Default)]
pub struct Hops {
    taken: usize,
    seen: Vec<String>,
}

impl Hops {
    /// Take one more step to `to` (an address or a server).
    ///
    /// # Errors
    /// [`SyncError::Config`] past [`MAX_HOPS`], or back to a place already
    /// visited.
    pub fn take(&mut self, to: &str) -> Result<(), SyncError> {
        let key = to.trim().to_ascii_lowercase();
        if self.taken >= MAX_HOPS || self.seen.contains(&key) {
            return Err(SyncError::Config(
                "Autodiscover sent Mocky Mail round in circles. Enter the EWS address yourself (ask your IT department)."
                    .into(),
            ));
        }
        self.taken = self.taken.saturating_add(1);
        self.seen.push(key);
        Ok(())
    }
}

/// The domain of an address.
#[must_use]
pub fn domain_of(email: &str) -> Option<String> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    (!local.is_empty() && is_host(&domain)).then_some(domain)
}

/// The EWS address under a server, when only the server is known
/// (`https://mail.example.org` → `…/EWS/Exchange.asmx`).
#[must_use]
pub fn ews_url_for(server: &str) -> Option<String> {
    let trimmed = server.trim().trim_end_matches('/');
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let url = acceptable(&with_scheme)?;
    if url.path().to_ascii_lowercase().ends_with("/exchange.asmx") {
        return Some(url.to_string());
    }
    url.join("/EWS/Exchange.asmx").ok().map(|u| u.to_string())
}
