//! Finding a domain's mail servers from DNS, so most people only type
//! their address.
//!
//! Order: known providers, then RFC 6186 SRV records (`_imaps._tcp`,
//! `_submissions._tcp`, `_submission._tcp`), then the MX host (which also
//! reveals Google Workspace and Microsoft 365 domains), then the
//! `mail.<domain>` guess. Results are only suggestions the user sees and can
//! edit, and every connection still checks the server's certificate for the
//! chosen host name.
//!
//! The records are asked for twice at most: first from the DNS servers the
//! system is set up with, and, when those give nothing in time (a resolver
//! that drops SRV queries, a dead server listed first, no readable system
//! settings), from a public resolver over HTTPS. Only the domain name is
//! sent, never the address.

use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::{RData, RecordType};
use serde::Deserialize;

use crate::config::{AuthKind, Security, ServerConfig, ServerSuggestion, suggest_servers};

/// Longest the system's DNS servers may take before the HTTPS lookup is tried.
const SYSTEM_DNS_TIMEOUT: Duration = Duration::from_secs(3);
/// Longest one DNS server may take to answer one question. The default of
/// five seconds would spend the whole budget on a single dead server.
const DNS_QUERY_TIMEOUT: Duration = Duration::from_millis(1200);
/// Longest the lookup over HTTPS may take before falling back to a guess.
const HTTPS_DNS_TIMEOUT: Duration = Duration::from_secs(5);
/// The public resolver asked over HTTPS (JSON answers).
const HTTPS_DNS_ENDPOINT: &str = "https://cloudflare-dns.com/dns-query";
/// Largest answer read from the public resolver, bytes.
const HTTPS_DNS_MAX_BYTES: usize = 64 * 1024;
/// The MX record type number, as it appears in JSON answers.
const TYPE_MX: u16 = 15;
/// The SRV record type number, as it appears in JSON answers.
const TYPE_SRV: u16 = 33;

/// One SRV answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrvRecord {
    /// Lower is preferred.
    pub priority: u16,
    /// Higher is preferred among equal priorities.
    pub weight: u16,
    /// Port.
    pub port: u16,
    /// Host, without the trailing dot.
    pub target: String,
}

/// The best SRV answer, skipping the "service not offered" marker (`.`).
#[must_use]
pub fn best_srv(records: &[SrvRecord]) -> Option<&SrvRecord> {
    records
        .iter()
        .filter(|r| !r.target.is_empty() && r.target != "." && r.port != 0)
        .min_by_key(|r| (r.priority, std::cmp::Reverse(r.weight)))
}

fn clean_host(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// Hosted providers recognised from an MX host.
#[must_use]
pub fn provider_from_mx(mx_host: &str) -> Option<AuthKind> {
    let host = clean_host(mx_host);
    if host.ends_with(".google.com") || host.ends_with(".googlemail.com") {
        Some(AuthKind::Google)
    } else if host.ends_with(".outlook.com") || host.ends_with(".office365.com") {
        Some(AuthKind::Microsoft)
    } else {
        None
    }
}

fn hosted(auth: AuthKind) -> Option<ServerSuggestion> {
    let probe = match auth {
        AuthKind::Google => "x@gmail.com",
        AuthKind::Microsoft => "x@outlook.com",
        AuthKind::Password => return None,
    };
    suggest_servers(probe)
}

/// Combine whatever DNS returned into a suggestion. Pure, for testing.
#[must_use]
pub fn from_records(
    imaps: &[SrvRecord],
    submissions: &[SrvRecord],
    submission: &[SrvRecord],
    mx: Option<&str>,
) -> Option<ServerSuggestion> {
    if let Some(imap) = best_srv(imaps) {
        let imap_config = ServerConfig {
            host: clean_host(&imap.target),
            port: imap.port,
            security: Security::Tls,
        };
        let smtp = best_srv(submissions)
            .map(|s| ServerConfig {
                host: clean_host(&s.target),
                port: s.port,
                security: Security::Tls,
            })
            .or_else(|| {
                best_srv(submission).map(|s| ServerConfig {
                    host: clean_host(&s.target),
                    port: s.port,
                    security: Security::Starttls,
                })
            })
            .unwrap_or_else(|| ServerConfig {
                host: imap_config.host.clone(),
                port: 465,
                security: Security::Tls,
            });
        return Some(ServerSuggestion {
            imap: imap_config,
            smtp,
            auth: AuthKind::Password,
        });
    }
    let mx = mx.map(clean_host).filter(|h| !h.is_empty())?;
    if let Some(auth) = provider_from_mx(&mx) {
        return hosted(auth);
    }
    Some(ServerSuggestion {
        imap: ServerConfig {
            host: mx.clone(),
            port: 993,
            security: Security::Tls,
        },
        smtp: ServerConfig {
            host: mx,
            port: 465,
            security: Security::Tls,
        },
        auth: AuthKind::Password,
    })
}

async fn srv(resolver: &TokioResolver, name: &str) -> Vec<SrvRecord> {
    let Ok(lookup) = resolver.lookup(name, RecordType::SRV).await else {
        return Vec::new();
    };
    lookup
        .answers()
        .iter()
        .filter_map(|record| match &record.data {
            RData::SRV(s) => Some(SrvRecord {
                priority: s.priority,
                weight: s.weight,
                port: s.port,
                target: clean_host(&s.target.to_string()),
            }),
            _ => None,
        })
        .collect()
}

async fn mx(resolver: &TokioResolver, domain: &str) -> Option<String> {
    let lookup = resolver.lookup(domain, RecordType::MX).await.ok()?;
    lookup
        .answers()
        .iter()
        .filter_map(|record| match &record.data {
            RData::MX(m) => Some((m.preference, clean_host(&m.exchange.to_string()))),
            _ => None,
        })
        .filter(|(_, host)| !host.is_empty())
        .min()
        .map(|(_, host)| host)
}

async fn lookup(domain: &str) -> Option<ServerSuggestion> {
    let mut builder = TokioResolver::builder_tokio().ok()?;
    let options = builder.options_mut();
    options.timeout = DNS_QUERY_TIMEOUT;
    options.attempts = 2;
    let resolver = builder.build().ok()?;
    let (imaps_name, implicit_tls_name, starttls_name, mx_name) = (
        format!("_imaps._tcp.{domain}."),
        format!("_submissions._tcp.{domain}."),
        format!("_submission._tcp.{domain}."),
        format!("{domain}."),
    );
    let (imaps, submissions, submission, mx) = tokio::join!(
        srv(&resolver, &imaps_name),
        srv(&resolver, &implicit_tls_name),
        srv(&resolver, &starttls_name),
        mx(&resolver, &mx_name),
    );
    from_records(&imaps, &submissions, &submission, mx.as_deref())
}

/// One answer in a JSON DNS response.
#[derive(Deserialize)]
struct JsonAnswer {
    #[serde(rename = "type")]
    record_type: u16,
    data: String,
}

/// A JSON DNS response; only the answers matter here.
#[derive(Deserialize)]
struct JsonResponse {
    #[serde(rename = "Answer", default)]
    answers: Vec<JsonAnswer>,
}

/// The record data of one type from a JSON DNS response. Pure, for testing.
fn json_answers(body: &[u8], record_type: u16) -> Vec<String> {
    let Ok(response) = serde_json::from_slice::<JsonResponse>(body) else {
        return Vec::new();
    };
    response
        .answers
        .into_iter()
        .filter(|answer| answer.record_type == record_type)
        .map(|answer| answer.data)
        .collect()
}

/// An SRV record from its text form, `priority weight port target`.
fn parse_srv(data: &str) -> Option<SrvRecord> {
    let mut parts = data.split_ascii_whitespace();
    let priority = parts.next()?.parse().ok()?;
    let weight = parts.next()?.parse().ok()?;
    let port = parts.next()?.parse().ok()?;
    let target = clean_host(parts.next()?);
    Some(SrvRecord {
        priority,
        weight,
        port,
        target,
    })
}

/// An MX record from its text form, `preference host`.
fn parse_mx(data: &str) -> Option<(u16, String)> {
    let mut parts = data.split_ascii_whitespace();
    let preference = parts.next()?.parse().ok()?;
    let host = clean_host(parts.next()?);
    Some((preference, host))
}

fn parse_all_srv(answers: &[String]) -> Vec<SrvRecord> {
    answers.iter().filter_map(|data| parse_srv(data)).collect()
}

/// The preferred MX host among text-form answers.
fn best_mx(answers: &[String]) -> Option<String> {
    answers
        .iter()
        .filter_map(|data| parse_mx(data))
        .filter(|(_, host)| !host.is_empty())
        .min()
        .map(|(_, host)| host)
}

/// Whether a domain can go into a query string as it is.
fn is_plain_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Ask the public resolver for one record set. Empty on any failure.
async fn https_query(
    http: &reqwest::Client,
    name: &str,
    type_name: &str,
    record_type: u16,
) -> Vec<String> {
    let url = format!("{HTTPS_DNS_ENDPOINT}?name={name}&type={type_name}");
    let request = http
        .get(url)
        .header(reqwest::header::ACCEPT, "application/dns-json");
    let Ok(response) = request.send().await else {
        return Vec::new();
    };
    if !response.status().is_success() {
        return Vec::new();
    }
    let Ok(body) = response.bytes().await else {
        return Vec::new();
    };
    if body.len() > HTTPS_DNS_MAX_BYTES {
        return Vec::new();
    }
    json_answers(&body, record_type)
}

/// The same records as [`lookup`], asked from a public resolver over HTTPS.
/// This goes through the system's ordinary name lookup and HTTPS, so it
/// works where SRV and MX queries to the configured DNS servers don't.
async fn lookup_over_https(domain: &str) -> Option<ServerSuggestion> {
    if !is_plain_domain(domain) {
        return None;
    }
    let http = reqwest::Client::builder()
        .timeout(HTTPS_DNS_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mocky Mail")
        .build()
        .ok()?;
    let (imaps_name, implicit_tls_name, starttls_name) = (
        format!("_imaps._tcp.{domain}"),
        format!("_submissions._tcp.{domain}"),
        format!("_submission._tcp.{domain}"),
    );
    let (imaps, submissions, submission, mx) = tokio::join!(
        https_query(&http, &imaps_name, "SRV", TYPE_SRV),
        https_query(&http, &implicit_tls_name, "SRV", TYPE_SRV),
        https_query(&http, &starttls_name, "SRV", TYPE_SRV),
        https_query(&http, domain, "MX", TYPE_MX),
    );
    from_records(
        &parse_all_srv(&imaps),
        &parse_all_srv(&submissions),
        &parse_all_srv(&submission),
        best_mx(&mx).as_deref(),
    )
}

fn usable(found: &ServerSuggestion) -> bool {
    found.imap.validate().is_ok() && found.smtp.validate().is_ok()
}

/// Suggest servers for an address, using DNS when the provider isn't known.
/// Falls back to [`suggest_servers`] (the `mail.<domain>` guess) when DNS
/// has nothing or takes too long. `None` only for an invalid address.
pub async fn discover_servers(email: &str) -> Option<ServerSuggestion> {
    let fallback = suggest_servers(email)?;
    let domain = email.trim().rsplit_once('@')?.1.to_ascii_lowercase();
    let is_custom =
        fallback.auth == AuthKind::Password && fallback.imap.host == format!("mail.{domain}");
    if !is_custom {
        return Some(fallback);
    }
    let from_system = tokio::time::timeout(SYSTEM_DNS_TIMEOUT, lookup(&domain))
        .await
        .ok()
        .flatten()
        .filter(usable);
    if from_system.is_some() {
        return from_system;
    }
    let over_https = tokio::time::timeout(HTTPS_DNS_TIMEOUT, lookup_over_https(&domain))
        .await
        .ok()
        .flatten()
        .filter(usable);
    Some(over_https.unwrap_or(fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(priority: u16, weight: u16, port: u16, target: &str) -> SrvRecord {
        SrvRecord {
            priority,
            weight,
            port,
            target: target.into(),
        }
    }

    #[test]
    fn mailcow_style_srv_records() {
        // What mocky.co.ke publishes.
        let s = from_records(
            &[rec(0, 1, 993, "mail.mailer.co.ke")],
            &[],
            &[rec(0, 1, 587, "mail.mailer.co.ke")],
            Some("mail.mailer.co.ke"),
        )
        .unwrap();
        assert_eq!(
            s.imap,
            ServerConfig {
                host: "mail.mailer.co.ke".into(),
                port: 993,
                security: Security::Tls
            }
        );
        assert_eq!(
            s.smtp,
            ServerConfig {
                host: "mail.mailer.co.ke".into(),
                port: 587,
                security: Security::Starttls
            }
        );
        assert_eq!(s.auth, AuthKind::Password);
    }

    #[test]
    fn implicit_tls_submission_is_preferred() {
        let s = from_records(
            &[rec(0, 0, 993, "imap.x.co")],
            &[rec(0, 0, 465, "smtp.x.co")],
            &[rec(0, 0, 587, "smtp.x.co")],
            None,
        )
        .unwrap();
        assert_eq!(s.smtp.port, 465);
        assert_eq!(s.smtp.security, Security::Tls);
    }

    #[test]
    fn best_srv_uses_priority_then_weight_and_skips_dot() {
        let records = [
            rec(10, 5, 993, "b.x.co"),
            rec(0, 1, 993, "."),
            rec(5, 1, 993, "c.x.co"),
            rec(5, 9, 993, "d.x.co"),
        ];
        assert_eq!(
            best_srv(&records).map(|r| r.target.as_str()),
            Some("d.x.co")
        );
        assert_eq!(best_srv(&[rec(0, 0, 993, ".")]), None);
    }

    #[test]
    fn workspace_and_microsoft_365_domains_are_recognised_by_mx() {
        assert_eq!(
            from_records(&[], &[], &[], Some("aspmx.l.google.com."))
                .unwrap()
                .auth,
            AuthKind::Google
        );
        let m = from_records(
            &[],
            &[],
            &[],
            Some("company-co-ke.mail.protection.outlook.com"),
        )
        .unwrap();
        assert_eq!(m.auth, AuthKind::Microsoft);
        assert_eq!(m.imap.host, "outlook.office365.com");
    }

    #[test]
    fn plain_mx_host_is_used_directly() {
        let s = from_records(&[], &[], &[], Some("MX1.Host.Example.")).unwrap();
        assert_eq!(s.imap.host, "mx1.host.example");
        assert_eq!(s.smtp.port, 465);
    }

    #[test]
    fn nothing_found_means_no_dns_suggestion() {
        assert_eq!(from_records(&[], &[], &[], None), None);
    }

    #[test]
    fn json_answers_are_read_by_record_type() {
        // What the public resolver returns for a Mailcow-hosted domain.
        let srv = br#"{"Status":0,"Answer":[{"name":"_imaps._tcp.x.co","type":33,"TTL":300,"data":"0 1 993 mail.mailer.co.ke."}]}"#;
        let records = parse_all_srv(&json_answers(srv, TYPE_SRV));
        assert_eq!(records, vec![rec(0, 1, 993, "mail.mailer.co.ke")]);
        assert!(json_answers(srv, TYPE_MX).is_empty());

        // A CNAME in front of the answer is skipped.
        let mx = br#"{"Status":0,"Answer":[{"name":"x.co","type":5,"TTL":1,"data":"y.co."},{"name":"y.co","type":15,"TTL":1,"data":"20 backup.y.co."},{"name":"y.co","type":15,"TTL":1,"data":"10 MX.y.co."}]}"#;
        assert_eq!(
            best_mx(&json_answers(mx, TYPE_MX)).as_deref(),
            Some("mx.y.co")
        );

        // No such name, and anything that isn't the expected JSON.
        let missing = br#"{"Status":3,"Authority":[{"name":"x.co","type":6,"TTL":1,"data":"ns. host. 1 2 3 4 5"}]}"#;
        assert!(json_answers(missing, TYPE_SRV).is_empty());
        assert!(json_answers(b"<html>", TYPE_SRV).is_empty());
    }

    #[test]
    fn text_records_are_parsed_strictly() {
        assert_eq!(
            parse_srv("5 10 587 Smtp.X.Co."),
            Some(rec(5, 10, 587, "smtp.x.co"))
        );
        assert_eq!(parse_srv("5 10 smtp.x.co"), None);
        assert_eq!(parse_srv("5 10 99999 smtp.x.co"), None);
        assert_eq!(parse_mx("10 mail.x.co."), Some((10, "mail.x.co".into())));
        assert_eq!(parse_mx("mail.x.co."), None);
        // The "no mail here" MX (`0 .`) is not a host.
        assert_eq!(best_mx(&["0 .".to_owned()]), None);
    }

    #[test]
    fn only_plain_domains_go_into_a_query() {
        assert!(is_plain_domain("winstar-health.co.ke"));
        assert!(!is_plain_domain(""));
        assert!(!is_plain_domain("x.co&type=A"));
        assert!(!is_plain_domain("x.co/../y"));
    }

    #[tokio::test]
    async fn known_providers_skip_dns() {
        let s = discover_servers("someone@gmail.com").await.unwrap();
        assert_eq!(s.auth, AuthKind::Google);
        assert_eq!(discover_servers("not an address").await, None);
    }

    /// Real DNS; run with `cargo test -p mail-sync -- --ignored live_dns`.
    #[tokio::test]
    #[ignore = "needs internet access"]
    async fn live_dns_finds_mailcow_for_mocky_co_ke() {
        let s = discover_servers("info@mocky.co.ke").await.unwrap();
        assert_eq!(s.imap.host, "mail.mailer.co.ke");
        assert_eq!(s.smtp.host, "mail.mailer.co.ke");
    }
}
