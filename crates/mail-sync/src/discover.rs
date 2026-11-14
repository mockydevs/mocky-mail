//! Finding a domain's mail servers from DNS, so most people only type
//! their address.
//!
//! Order: known providers, then RFC 6186 SRV records (`_imaps._tcp`,
//! `_submissions._tcp`, `_submission._tcp`), then the MX host (which also
//! reveals Google Workspace and Microsoft 365 domains), then the
//! `mail.<domain>` guess. Results are only suggestions the user sees and can
//! edit, and every connection still checks the server's certificate for the
//! chosen host name.

use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::{RData, RecordType};

use crate::config::{AuthKind, Security, ServerConfig, ServerSuggestion, suggest_servers};

/// Longest the whole DNS lookup may take before falling back to a guess.
const DNS_TIMEOUT: Duration = Duration::from_secs(5);

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
    let resolver = TokioResolver::builder_tokio().ok()?.build().ok()?;
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
    match tokio::time::timeout(DNS_TIMEOUT, lookup(&domain)).await {
        Ok(Some(found)) if found.imap.validate().is_ok() && found.smtp.validate().is_ok() => {
            Some(found)
        }
        _ => Some(fallback),
    }
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
