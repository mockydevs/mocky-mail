//! End-to-end tests against a real IMAP/SMTP server.
//!
//! They run only when `MOCKY_TEST_SERVER` is set and the crate is built with
//! the `insecure-plaintext` feature, against a disposable local server:
//!
//! ```text
//! docker run --rm -d --name mocky-greenmail -p 3025:3025 -p 3143:3143 \
//!   -e GREENMAIL_OPTS='-Dgreenmail.setup.test.smtp -Dgreenmail.setup.test.imap -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.users=alice:secret@local.test,bob:secret@local.test -Dgreenmail.users.login=email -Dgreenmail.verbose' \
//!   greenmail/standalone:2.1.3
//! MOCKY_TEST_SERVER=127.0.0.1 cargo test -p mail-sync --features insecure-plaintext --test live_server
//! ```
#![cfg(feature = "insecure-plaintext")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::time::Duration;

use mail_core::message::{Mailbox, parse_body, parse_summary};
use mail_sync::imap::{FlagChange, ImapConnection};
use mail_sync::outgoing::{OutgoingAttachment, OutgoingMessage, build};
use mail_sync::{Credentials, Security, ServerConfig, SyncError, smtp};

fn host() -> Option<String> {
    std::env::var("MOCKY_TEST_SERVER").ok()
}

fn imap(host: &str) -> ServerConfig {
    ServerConfig {
        host: host.to_owned(),
        port: 3143,
        security: Security::Plain,
    }
}

fn smtp_server(host: &str) -> ServerConfig {
    ServerConfig {
        host: host.to_owned(),
        port: 3025,
        security: Security::Plain,
    }
}

fn login(user: &str) -> Credentials {
    Credentials::Password {
        username: format!("{user}@local.test"),
        password: "secret".into(),
    }
}

fn mb(user: &str) -> Mailbox {
    Mailbox {
        name: None,
        address: format!("{user}@local.test"),
    }
}

async fn send(host: &str, subject: &str, with_attachment: bool) -> String {
    let built = build(&OutgoingMessage {
        from: Mailbox {
            name: Some("Alice".into()),
            address: "alice@local.test".into(),
        },
        to: vec![mb("bob")],
        cc: vec![],
        bcc: vec![],
        subject: subject.into(),
        body_text: "Hello Bob,\n\nThis is a test.".into(),
        in_reply_to: None,
        references: vec![],
        attachments: if with_attachment {
            vec![OutgoingAttachment {
                file_name: "notes.txt".into(),
                mime_type: "text/plain".into(),
                data: b"attached".to_vec(),
            }]
        } else {
            vec![]
        },
    })
    .unwrap();
    smtp::send(
        &smtp_server(host),
        &login("alice"),
        &built.envelope_from,
        &built.recipients,
        &built.raw,
    )
    .await
    .unwrap();
    built.message_id
}

async fn wait_for(conn: &mut ImapConnection, folder: &str, count: usize) -> Vec<u32> {
    for _ in 0..50 {
        conn.select(folder).await.unwrap();
        let uids = conn.all_uids(folder).await.unwrap();
        if uids.len() >= count {
            let mut v: Vec<u32> = uids.into_iter().collect();
            v.sort_unstable();
            return v;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("{folder} never reached {count} messages");
}

#[tokio::test]
async fn full_mailbox_round_trip() {
    let Some(host) = host() else { return };

    // Wrong password is reported as an auth error, for IMAP and SMTP.
    let bad = Credentials::Password {
        username: "bob@local.test".into(),
        password: "wrong".into(),
    };
    assert!(matches!(
        ImapConnection::connect(&imap(&host), &bad).await,
        Err(SyncError::Auth)
    ));
    smtp::test_login(&smtp_server(&host), &login("alice"))
        .await
        .unwrap();

    let subject = format!("Round trip {}", std::process::id());
    let message_id = send(&host, &subject, true).await;

    let mut bob = ImapConnection::connect(&imap(&host), &login("bob"))
        .await
        .unwrap();
    let folders = bob.list_folders().await.unwrap();
    assert!(folders.iter().any(|f| f.path.eq_ignore_ascii_case("INBOX")));

    let uids = wait_for(&mut bob, "INBOX", 1).await;
    let headers = bob.fetch_headers("INBOX", "1:*").await.unwrap();
    let ours = headers
        .iter()
        .find(|m| m.summary.subject == subject)
        .expect("sent message arrives");
    assert!(!ours.flags.seen);
    assert_eq!(
        ours.summary.message_id.as_deref(),
        Some(message_id.as_str())
    );

    // Reading the body must not mark it read (BODY.PEEK).
    let raw = bob.fetch_raw("INBOX", ours.uid).await.unwrap().unwrap();
    let body = parse_body(&raw).unwrap();
    assert!(body.text.unwrap().contains("This is a test."));
    assert_eq!(body.attachments[0].file_name, "notes.txt");
    let flags = bob
        .fetch_flags("INBOX", &ours.uid.to_string())
        .await
        .unwrap();
    assert!(
        !flags[0].1.seen,
        "fetching the body marked the message read"
    );

    bob.store_flags("INBOX", &[ours.uid], FlagChange::Seen(true))
        .await
        .unwrap();
    bob.store_flags("INBOX", &[ours.uid], FlagChange::Flagged(true))
        .await
        .unwrap();
    let flags = bob
        .fetch_flags("INBOX", &ours.uid.to_string())
        .await
        .unwrap();
    assert!(flags[0].1.seen && flags[0].1.flagged);

    assert!(bob.contains_message_id("INBOX", &message_id).await.unwrap());
    assert!(
        !bob.contains_message_id("INBOX", "nope@local.test")
            .await
            .unwrap()
    );

    // Move to a new Archive folder, then delete for good.
    let archive = format!("Archive{}", std::process::id());
    bob.create_folder(&archive).await.unwrap();
    bob.move_messages("INBOX", &[ours.uid], &archive)
        .await
        .unwrap();
    bob.select("INBOX").await.unwrap();
    assert!(!bob.all_uids("INBOX").await.unwrap().contains(&ours.uid));
    let moved = wait_for(&mut bob, &archive, 1).await;
    bob.delete_forever(&archive, &moved).await.unwrap();
    bob.select(&archive).await.unwrap();
    assert!(bob.all_uids(&archive).await.unwrap().is_empty());

    // Append a copy to Sent.
    let sent = format!("Sent{}", std::process::id());
    bob.create_folder(&sent).await.unwrap();
    bob.append(&sent, &raw, "(\\Seen)").await.unwrap();
    let in_sent = wait_for(&mut bob, &sent, 1).await;
    let copy = bob
        .fetch_headers(&sent, &in_sent[0].to_string())
        .await
        .unwrap();
    assert!(copy[0].flags.seen);
    assert_eq!(
        parse_summary(&raw).unwrap().subject,
        copy[0].summary.subject
    );

    // Replacing a saved draft: find the old copy by Message-ID and remove it.
    let drafts = format!("Drafts{}", std::process::id());
    bob.create_folder(&drafts).await.unwrap();
    bob.append(&drafts, &raw, "(\\Draft \\Seen)").await.unwrap();
    wait_for(&mut bob, &drafts, 1).await;
    let old_copy = bob
        .uids_with_message_id(&drafts, &message_id)
        .await
        .unwrap();
    assert_eq!(old_copy.len(), 1);
    bob.delete_forever(&drafts, &old_copy).await.unwrap();
    assert!(
        bob.uids_with_message_id(&drafts, &message_id)
            .await
            .unwrap()
            .is_empty()
    );

    drop(uids);
    bob.logout().await;
}

#[tokio::test]
async fn idle_reports_new_mail() {
    let Some(host) = host() else { return };
    let mut carol = ImapConnection::connect(&imap(&host), &login("bob"))
        .await
        .unwrap();
    carol.select("INBOX").await.unwrap();
    if !carol.supports_idle() {
        eprintln!("server has no IDLE; skipping");
        return;
    }
    let waiter = tokio::spawn(async move { carol.idle("INBOX").await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    send(&host, "Wake up", false).await;
    let (_conn, changed) = tokio::time::timeout(Duration::from_secs(30), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(changed);
}
