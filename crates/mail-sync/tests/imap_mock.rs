//! IMAP client against a small in-process fake server: the commands the
//! 0.5.0 mail-handling features send (copy, keywords and PERMANENTFLAGS,
//! Gmail labels, SEARCH SINCE) and what is read back.
//!
//! `cargo test -p mail-sync --features insecure-plaintext --test imap_mock`
#![cfg(feature = "insecure-plaintext")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::format_push_string
)]

use std::sync::{Arc, Mutex};

use mail_sync::imap::ImapConnection;
use mail_sync::{Credentials, Security, ServerConfig};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// How the fake server behaves.
#[derive(Clone, Copy)]
struct Server {
    gmail: bool,
    /// PERMANENTFLAGS includes `\*`.
    keywords: bool,
}

/// Everything the client sent, without tags.
type Log = Arc<Mutex<Vec<String>>>;

fn answer(server: Server, command: &str) -> String {
    let upper = command.to_ascii_uppercase();
    if upper.starts_with("CAPABILITY") {
        let gmail = if server.gmail { " X-GM-EXT-1" } else { "" };
        return format!("* CAPABILITY IMAP4rev1 UIDPLUS MOVE IDLE{gmail}\r\n");
    }
    if upper.starts_with("SELECT") {
        let star = if server.keywords { " \\*" } else { "" };
        return format!(
            "* FLAGS (\\Seen \\Flagged \\Answered \\Deleted \\Draft $Junk)\r\n\
             * OK [PERMANENTFLAGS (\\Seen \\Flagged \\Answered \\Deleted \\Draft $Junk{star})] Flags\r\n\
             * 2 EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 7] UIDs\r\n* OK [UIDNEXT 9] next\r\n"
        );
    }
    if upper.starts_with("UID SEARCH SINCE") {
        return "* SEARCH 5 8\r\n".to_owned();
    }
    if upper.starts_with("UID FETCH") {
        let labels = if server.gmail {
            " X-GM-LABELS (\"\\\\Inbox\" \\Important Clients \"Entw&APw-rfe\" \"Old \\\"one\\\"\")"
        } else {
            ""
        };
        return format!(
            "* 1 FETCH (UID 5 FLAGS (\\Seen $Cat_Red_20team $label1){labels})\r\n\
             * 2 FETCH (UID 8 FLAGS (){labels})\r\n"
        );
    }
    String::new()
}

/// Serve one connection; returns the port and the log of commands.
async fn start(server: Server) -> (u16, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log: Log = Arc::default();
    let seen = Arc::clone(&log);
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        write.write_all(b"* OK fake IMAP ready\r\n").await.unwrap();
        while let Ok(Some(line)) = lines.next_line().await {
            let (tag, command) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            seen.lock().unwrap().push(command.to_owned());
            let mut reply = answer(server, command);
            if command.to_ascii_uppercase().starts_with("LOGOUT") {
                reply.push_str("* BYE\r\n");
            }
            reply.push_str(&format!("{tag} OK done\r\n"));
            if write.write_all(reply.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    (port, log)
}

async fn connect(port: u16) -> ImapConnection {
    ImapConnection::connect(
        &ServerConfig {
            host: "127.0.0.1".into(),
            port,
            security: Security::Plain,
        },
        &Credentials::Password {
            username: "me@example.com".into(),
            password: "secret".into(),
        },
    )
    .await
    .unwrap()
}

fn sent(log: &Log, prefix: &str) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|c| c.starts_with(prefix))
        .cloned()
        .collect()
}

fn owned(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test]
async fn keywords_copies_and_the_sync_window() {
    let (port, log) = start(Server {
        gmail: false,
        keywords: true,
    })
    .await;
    let mut conn = connect(port).await;
    assert!(!conn.is_gmail());
    let status = conn.select("INBOX").await.unwrap();
    assert!(status.accepts_keywords);

    assert!(
        conn.store_keywords("INBOX", &[5, 6], &owned(&["$Junk"]), &owned(&["$NotJunk"]))
            .await
            .unwrap()
    );
    conn.copy_messages("INBOX", &[5, 6, 7], "Clients")
        .await
        .unwrap();
    let since = conn.uids_since("INBOX", "26-Oct-2026").await.unwrap();
    assert_eq!(since.len(), 2);
    assert!(since.contains(&5) && since.contains(&8));
    // A date that isn't one never reaches the server.
    assert!(conn.uids_since("INBOX", "1 OR ALL").await.is_err());

    let tagged = conn.fetch_flags_and_tags("INBOX", "5,8").await.unwrap();
    let (uid, flags, tags) = &tagged[0];
    assert_eq!(*uid, 5);
    assert!(flags.seen);
    assert_eq!(tags.keywords, owned(&["$Cat_Red_20team", "$label1"]));
    assert_eq!(tags.labels, None);
    conn.logout().await;

    assert_eq!(
        sent(&log, "UID STORE"),
        [
            "UID STORE 5:6 +FLAGS.SILENT ($Junk)",
            "UID STORE 5:6 -FLAGS.SILENT ($NotJunk)"
        ]
    );
    assert_eq!(sent(&log, "UID COPY"), ["UID COPY 5:7 \"Clients\""]);
    assert_eq!(sent(&log, "UID SEARCH"), ["UID SEARCH SINCE 26-Oct-2026"]);
    assert_eq!(sent(&log, "UID FETCH"), ["UID FETCH 5,8 (UID FLAGS)"]);
}

#[tokio::test]
async fn a_folder_without_keywords_keeps_them_local() {
    let (port, log) = start(Server {
        gmail: false,
        keywords: false,
    })
    .await;
    let mut conn = connect(port).await;
    assert!(!conn.select("INBOX").await.unwrap().accepts_keywords);
    assert!(
        !conn
            .store_keywords("INBOX", &[5], &owned(&["$Cat_Red"]), &[])
            .await
            .unwrap()
    );
    conn.logout().await;
    assert!(sent(&log, "UID STORE").is_empty());
}

#[tokio::test]
async fn gmail_labels_are_read_and_written() {
    let (port, log) = start(Server {
        gmail: true,
        keywords: true,
    })
    .await;
    let mut conn = connect(port).await;
    assert!(conn.is_gmail());
    let tagged = conn.fetch_flags_and_tags("INBOX", "5").await.unwrap();
    assert_eq!(
        tagged[0].2.labels.as_deref(),
        Some(
            &owned(&[
                "\\Inbox",
                "\\Important",
                "Clients",
                "Entw&APw-rfe",
                "Old \"one\""
            ])[..]
        )
    );
    conn.store_labels(
        "INBOX",
        &[5],
        &owned(&["Entw&APw-rfe"]),
        &owned(&["Old \"one\""]),
    )
    .await
    .unwrap();
    conn.logout().await;
    assert_eq!(
        sent(&log, "UID FETCH"),
        ["UID FETCH 5 (UID FLAGS X-GM-LABELS)"]
    );
    assert_eq!(
        sent(&log, "UID STORE"),
        [
            "UID STORE 5 +X-GM-LABELS.SILENT (\"Entw&APw-rfe\")",
            "UID STORE 5 -X-GM-LABELS.SILENT (\"Old \\\"one\\\"\")"
        ]
    );
}
