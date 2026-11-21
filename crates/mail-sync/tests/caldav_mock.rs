//! `CalDAV` client against a small in-process fake server shaped like
//! `SOGo`: discovery (`.well-known` redirect, calendar home, an event
//! calendar beside the scheduling inbox), listing, fetching, creating,
//! editing with `If-Match`, skipping one date of a series, and deleting.
//!
//! `cargo test -p mail-sync --features insecure-plaintext --test caldav_mock`
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
type Items = Arc<Mutex<HashMap<String, (u32, String)>>>;

const BOOK: &str = "/SOGo/dav/me/Calendar/personal/";

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
        r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:A="http://apple.com/ns/ical/">{responses}</D:multistatus>"#
    )
}

fn handle(
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    body: &str,
    cards: &Items,
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
        ("PROPFIND", "/.well-known/caldav") => reply(
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
                "<D:response><D:href>/SOGo/dav/me/</D:href><D:propstat><D:prop><C:calendar-home-set><D:href>/SOGo/dav/me/Calendar/</D:href></C:calendar-home-set></D:prop></D:propstat></D:response>",
            ),
        ),
        ("PROPFIND", "/SOGo/dav/me/Calendar/") => reply(
            "207 Multi-Status",
            &[],
            &multistatus(&format!(
                "<D:response><D:href>/SOGo/dav/me/Calendar/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response><D:response><D:href>/SOGo/dav/me/Calendar/inbox/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:schedule-inbox/></D:resourcetype></D:prop></D:propstat></D:response><D:response><D:href>{BOOK}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:calendar/></D:resourcetype><D:displayname>Personal Calendar</D:displayname><A:calendar-color>#3A87ADFF</A:calendar-color><C:supported-calendar-component-set><C:comp name=\"VEVENT\"/><C:comp name=\"VTODO\"/></C:supported-calendar-component-set></D:prop></D:propstat></D:response><D:response><D:href>/SOGo/dav/me/Calendar/tasks/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:calendar/></D:resourcetype><C:supported-calendar-component-set><C:comp name=\"VTODO\"/></C:supported-calendar-component-set></D:prop></D:propstat></D:response>"
            )),
        ),
        ("PROPFIND", BOOK) => {
            let mut items = format!(
                "<D:response><D:href>{BOOK}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:calendar/></D:resourcetype></D:prop></D:propstat></D:response>"
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
                    items.push_str(&format!("<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:getetag>\"{etag}\"</D:getetag><C:calendar-data>{escaped}</C:calendar-data></D:prop></D:propstat></D:response>"));
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

async fn serve(cards: Items) -> Url {
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
async fn finds_the_calendar_and_reads_and_writes_events() {
    let items: Items = Arc::default();
    items.lock().unwrap().insert(
        format!("{BOOK}standup.ics"),
        (1, "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:standup\r\nDTSTART:20261102T060000Z\r\nDTEND:20261102T061500Z\r\nRRULE:FREQ=DAILY;COUNT=5\r\nSUMMARY:Stand-up & plan\r\nBEGIN:VALARM\r\nTRIGGER:-PT5M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into()),
    );
    let server = serve(Arc::clone(&items)).await;
    let dav = client("secret");

    // Only the calendar that holds events; not the inbox or a task list.
    let calendars = dav.discover_calendars(&server).await.unwrap();
    assert_eq!(calendars.len(), 1);
    assert_eq!(calendars[0].name.as_deref(), Some("Personal Calendar"));
    assert_eq!(calendars[0].color.as_deref(), Some("#3A87ADFF"));
    let cal = calendars[0].url.clone();

    let listed = dav.list_resources(&cal).await.unwrap();
    assert_eq!(
        listed,
        [(format!("{BOOK}standup.ics"), Some("\"1\"".to_owned()))]
    );
    let fetched = dav
        .fetch_events(&cal, &[listed[0].0.clone()])
        .await
        .unwrap();
    let all = mail_core::ical::occurrences(&fetched[0].vcard, 1_793_491_200, 1_796_083_200);
    assert_eq!(all.len(), 5);
    assert_eq!(all[0].title, "Stand-up & plan");

    // Skip one date: the alarm and the rest of the series stay.
    let skipped = mail_core::ical::skip_occurrence(&fetched[0].vcard, all[2].start).unwrap();
    let (href, etag) = dav
        .put_event(
            &cal,
            Some(&fetched[0].href),
            "standup",
            &skipped,
            Some("\"1\""),
        )
        .await
        .unwrap();
    assert_eq!(etag.as_deref(), Some("\"2\""));
    let stored = items.lock().unwrap()[&href].1.clone();
    assert!(stored.contains("BEGIN:VALARM"));
    assert_eq!(mail_core::ical::occurrences(&stored, 0, i64::MAX).len(), 4);
    // A stale version is refused rather than overwriting.
    let stale = dav
        .put_event(&cal, Some(&href), "standup", &skipped, Some("\"1\""))
        .await;
    assert!(matches!(stale, Err(SyncError::Refused(_))));

    // Create, then delete.
    let input = mail_core::ical::EventInput {
        title: "Site visit".into(),
        location: "Kilimani".into(),
        notes: String::new(),
        all_day: false,
        start: 1_795_000_000,
        end: 1_795_003_600,
        repeat: mail_core::ical::Repeat::None,
        time_zone: "Africa/Nairobi".into(),
        reminders: Some(vec![15]),
    };
    let text = mail_core::ical::new_event("new-1@mocky.mail", &input, 1_794_000_000);
    let (new_href, _) = dav
        .put_event(&cal, None, "new-1@mocky.mail", &text, None)
        .await
        .unwrap();
    // The "@" can't go in the name, so a hash of the whole uid is added.
    assert!(new_href.starts_with(&format!("{BOOK}new-1mocky.mail-")));
    assert_eq!(
        new_href.len(),
        BOOK.len() + "new-1mocky.mail-0123456789abcdef.ics".len()
    );
    assert_eq!(dav.list_resources(&cal).await.unwrap().len(), 2);
    dav.delete_event(&cal, &new_href, None).await.unwrap();
    dav.delete_event(&cal, &new_href, None).await.unwrap();
    assert_eq!(dav.list_resources(&cal).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_wrong_password_is_an_auth_error() {
    let server = serve(Arc::default()).await;
    let result = client("wrong").discover_calendars(&server).await;
    assert!(matches!(result, Err(SyncError::Auth)));
}
