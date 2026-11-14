//! Fixtures shared by the unit tests.

use mail_core::folders::FolderRole;
use mail_core::message::{AttachmentInfo, Mailbox, MessageSummary};
use mail_core::outbox::OutboxItem;

use crate::{
    AuthKind, MessageFlags, NewAccount, NewFolder, NewOutbox, Security, ServerConfig, Store,
};

pub(crate) fn new_account(email: &str) -> NewAccount {
    NewAccount {
        email: email.to_owned(),
        display_name: "Test User".to_owned(),
        auth: AuthKind::Password,
        color: "#B81D24".to_owned(),
        imap: ServerConfig {
            host: "imap.example.com".to_owned(),
            port: 993,
            security: Security::Tls,
        },
        smtp: ServerConfig {
            host: "smtp.example.com".to_owned(),
            port: 465,
            security: Security::Tls,
        },
        size_limit_bytes: None,
        created_at: 1_700_000_000,
    }
}

pub(crate) fn add_account(store: &Store, email: &str) -> String {
    store.insert_account(&new_account(email)).unwrap().id
}

pub(crate) fn add_folder(store: &Store, account_id: &str, path: &str, role: FolderRole) -> String {
    store
        .upsert_folder(
            account_id,
            &NewFolder {
                path,
                name: path,
                role,
                delimiter: Some("/"),
            },
        )
        .unwrap()
}

pub(crate) fn summary(subject: &str, date: i64) -> MessageSummary {
    MessageSummary {
        message_id: Some(format!("{subject}-{date}@example.com")),
        subject: subject.to_owned(),
        from: Some(Mailbox {
            name: Some("Alice Sender".to_owned()),
            address: "alice@example.com".to_owned(),
        }),
        reply_to: vec![],
        to: vec![Mailbox {
            name: Some("Bob Receiver".to_owned()),
            address: "bob@example.org".to_owned(),
        }],
        cc: vec![Mailbox {
            name: None,
            address: "carol@example.net".to_owned(),
        }],
        date: Some(date),
        preview: format!("preview of {subject}"),
        has_html: false,
        attachments: vec![],
        in_reply_to: None,
        references: vec![],
    }
}

pub(crate) fn with_attachment(mut summary: MessageSummary) -> MessageSummary {
    summary.attachments.push(AttachmentInfo {
        file_name: "invoice.pdf".to_owned(),
        mime_type: "application/pdf".to_owned(),
        size: 1234,
    });
    summary
}

pub(crate) fn add_message(
    store: &Store,
    folder_id: &str,
    uid: u32,
    summary: &MessageSummary,
) -> String {
    store
        .upsert_message(folder_id, uid, summary, MessageFlags::default(), 100)
        .unwrap()
}

pub(crate) fn new_outbox(account_id: &str) -> NewOutbox {
    NewOutbox {
        account_id: account_id.to_owned(),
        created_at: 100,
        to: vec![Mailbox {
            name: None,
            address: "bob@example.org".to_owned(),
        }],
        subject: "Quote".to_owned(),
        preview: "Here is the quote".to_owned(),
        raw: b"Subject: Quote\r\n\r\nHere is the quote\r\n".to_vec(),
        recipients: vec!["to@x.co".into(), "hidden@x.co".into()],
        message_id_header: Some("<q1@example.com>".to_owned()),
        reply_to_message_id: None,
        sent_folder_hint: Some("Sent".to_owned()),
        item: OutboxItem::new(100),
    }
}
