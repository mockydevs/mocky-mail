//! `CardDAV` client against a small in-process fake server: discovery
//! (with the `.well-known` redirect `SOGo` sends), listing, fetching,
//! creating, editing with `If-Match`, and deleting.
//!
//! `cargo test -p mail-sync --features insecure-plaintext --test carddav_mock`
#![cfg(feature = "insecure-plaintext")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::format_push_string
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mail_sync::SyncError;
use mail_sync::dav::{DavClient, DavLogin};
use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Cards by path: (etag, text).
type Cards = Arc<Mutex<HashMap<String, (u32, String)>>>;

const BOOK: &str = "/SOGo/dav/me/Contacts/personal/";

fn reply(status: &str, headers: &[(&str, String)], body: &str) -> String {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    out
}

fn multistatus(responses: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav">{responses}</D:multistatus>"#
    )
}

fn handle(
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    body: &str,
    cards: &Cards,
) -> String {
    // "me:secret" in Basic auth.
    if headers.get("authorization").map(String::as_str) != Some("Basic bWU6c2VjcmV0") {
        return reply(
            "401 Unauthorized",
            &[("WWW-Authenticate", "Basic realm=\"SOGo\"".into())],
            "",
        );
    }
    let mut cards = cards.lock().unwrap();
    match (method, path) {
        ("PROPFIND", "/.well-known/carddav") => reply(
            "301 Moved Permanently",
            &[("Location", "/SOGo/dav/".into())],
            "",
        ),
        ("PROPFIND", "/SOGo/dav/") => reply(
            "207 Multi-Status",
            &[],
            &multistatus(
                "<D:response><D:href>/SOGo/dav/</D:href><D:propstat><D:prop><D:current-user-principal><D:href>/SOGo/dav/me/</D:href></D:current-user-principal></D:prop></D:propstat></D:response>",
            ),
        ),
        ("PROPFIND", "/SOGo/dav/me/") => reply(
            "207 Multi-Status",
            &[],
            &multistatus(
                "<D:response><D:href>/SOGo/dav/me/</D:href><D:propstat><D:prop><C:addressbook-home-set><D:href>/SOGo/dav/me/Contacts/</D:href></C:addressbook-home-set></D:prop></D:propstat></D:response>",
            ),
        ),
        ("PROPFIND", "/SOGo/dav/me/Contacts/") => reply(
            "207 Multi-Status",
            &[],
            &multistatus(&format!(
                "<D:response><D:href>/SOGo/dav/me/Contacts/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response><D:response><D:href>{BOOK}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:addressbook/></D:resourcetype><D:displayname>Personal Address Book</D:displayname></D:prop></D:propstat></D:response>"
            )),
        ),
        ("PROPFIND", BOOK) => {
            let mut items = format!(
                "<D:response><D:href>{BOOK}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:addressbook/></D:resourcetype></D:prop></D:propstat></D:response>"
            );
            for (href, (etag, _)) in cards.iter() {
                items.push_str(&format!("<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:getetag>\"{etag}\"</D:getetag><D:resourcetype/></D:prop></D:propstat></D:response>"));
            }
            reply("207 Multi-Status", &[], &multistatus(&items))
        }
        ("REPORT", BOOK) => {
            let mut items = String::new();
            for (href, (etag, text)) in cards.iter() {
                if body.contains(href.as_str()) {
                    let escaped = text
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('\r', "&#13;");
                    items.push_str(&format!("<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:getetag>\"{etag}\"</D:getetag><C:address-data>{escaped}</C:address-data></D:prop></D:propstat></D:response>"));
                }
            }
            reply("207 Multi-Status", &[], &multistatus(&items))
        }
        ("PUT", path) => {
            let current = cards.get(path).map(|(e, _)| *e);
            if headers.get("if-none-match").is_some() && current.is_some() {
                return reply("412 Precondition Failed", &[], "");
            }
            if let Some(expected) = headers.get("if-match")
                && current.map(|e| format!("\"{e}\"")) != Some(expected.clone())
            {
                return reply("412 Precondition Failed", &[], "");
            }
            let etag = current.unwrap_or(0) + 1;
            cards.insert(path.to_owned(), (etag, body.to_owned()));
            reply(
                if current.is_some() {
                    "204 No Content"
                } else {
                    "201 Created"
                },
                &[("ETag", format!("\"{etag}\""))],
                "",
            )
        }
        ("DELETE", path) => {
            if cards.remove(path).is_some() {
                reply("204 No Content", &[], "")
            } else {
                reply("404 Not Found", &[], "")
            }
        }
        _ => reply("404 Not Found", &[], ""),
    }
}

async fn serve(cards: Cards) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let cards = Arc::clone(&cards);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                // Read headers, then the body by Content-Length.
                let (head_end, length) = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                        break (pos + 4, length);
                    }
                };
                while buf.len() < head_end + length {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
                let mut lines = head.lines();
                let request = lines.next().unwrap_or_default();
                let mut parts = request.split(' ');
                let method = parts.next().unwrap_or_default().to_owned();
                let path = parts.next().unwrap_or_default().to_owned();
                let headers: HashMap<String, String> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_owned()))
                    .collect();
                let answer = handle(&method, &path, &headers, &body, &cards);
                socket.write_all(answer.as_bytes()).await.unwrap();
            });
        }
    });
    Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap()
}

fn client(password: &str) -> DavClient {
    DavClient::new(DavLogin {
        username: "me".into(),
        password: password.into(),
    })
    .unwrap()
}

#[tokio::test]
async fn finds_the_address_book_and_syncs_both_ways() {
    let cards: Cards = Arc::default();
    cards.lock().unwrap().insert(
        format!("{BOOK}peter.vcf"),
        (1, "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:peter\r\nFN:Peter Mwangi\r\nEMAIL:peter@client.example\r\nPHOTO:abc\r\nEND:VCARD\r\n".into()),
    );
    let server = serve(Arc::clone(&cards)).await;
    let dav = client("secret");

    let books = dav.discover_address_books(&server).await.unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].name.as_deref(), Some("Personal Address Book"));
    let book = books[0].url.clone();

    let listed = dav.list_cards(&book).await.unwrap();
    assert_eq!(
        listed,
        [(format!("{BOOK}peter.vcf"), Some("\"1\"".to_owned()))]
    );
    let fetched = dav
        .fetch_cards(&book, &[listed[0].0.clone()])
        .await
        .unwrap();
    let card = mail_core::vcard::parse(&fetched[0].vcard).unwrap();
    assert_eq!(card.name, "Peter Mwangi");

    // Edit with If-Match; the photo the app doesn't show survives.
    let mut edited = card.clone();
    edited.title = "Site manager".into();
    let text = mail_core::vcard::write(&edited, Some(&fetched[0].vcard));
    let (href, etag) = dav
        .put_card(
            &book,
            Some(&fetched[0].href),
            &edited.uid,
            &text,
            Some("\"1\""),
        )
        .await
        .unwrap();
    assert_eq!(etag.as_deref(), Some("\"2\""));
    assert!(cards.lock().unwrap()[&href].1.contains("PHOTO:abc"));
    // A stale If-Match is refused rather than overwriting.
    let stale = dav
        .put_card(&book, Some(&href), &edited.uid, &text, Some("\"1\""))
        .await;
    assert!(matches!(stale, Err(SyncError::Refused(_))));

    // Create, then delete.
    let new = mail_core::vcard::ContactCard {
        uid: "new-1".into(),
        name: "Wanjiru".into(),
        emails: vec!["w@shop.example".into()],
        ..Default::default()
    };
    let (new_href, _) = dav
        .put_card(
            &book,
            None,
            &new.uid,
            &mail_core::vcard::write(&new, None),
            None,
        )
        .await
        .unwrap();
    assert_eq!(new_href, format!("{BOOK}new-1.vcf"));
    assert_eq!(dav.list_cards(&book).await.unwrap().len(), 2);
    dav.delete_card(&book, &new_href, None).await.unwrap();
    // Deleting something already gone is fine.
    dav.delete_card(&book, &new_href, None).await.unwrap();
    assert_eq!(dav.list_cards(&book).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_wrong_password_is_an_auth_error() {
    let server = serve(Arc::default()).await;
    let result = client("wrong").discover_address_books(&server).await;
    assert!(matches!(result, Err(SyncError::Auth)));
}
