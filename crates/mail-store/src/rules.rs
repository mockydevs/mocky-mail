//! Mail rules, what they see of messages, and the senders whose pictures
//! always load.
//!
//! Matching and actions are `mail_core::rules`; this module keeps the
//! rules (as JSON, in the user's order) and reads messages for them.

use mail_core::importance::Importance;
use mail_core::message::Mailbox;
use mail_core::rules::{MessageFacts, Rule, RuleSpec, ServerStatus, header_lines};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{Result, StoreError};
use crate::ids::{new_id, to_u32, to_u64};

/// A message as rules see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSubject {
    /// Message id.
    pub id: String,
    /// Its account.
    pub account_id: String,
    /// Its folder.
    pub folder_id: String,
    /// What rules look at.
    pub facts: MessageFacts,
}

/// A sender whose remote pictures always load (the interface's
/// `ImageSender`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSender {
    /// An address, or a domain when `whole_domain`.
    pub sender: String,
    /// Everyone at the domain (and its subdomains).
    pub whole_domain: bool,
}

/// The key a sender is trusted under: the address in lower case, or with
/// `whole_domain` its domain. `None` for something that isn't an address
/// (or, with `whole_domain`, a domain).
#[must_use]
pub fn image_sender_key(sender: &str, whole_domain: bool) -> Option<String> {
    let s = sender.trim().trim_matches(['<', '>']).to_lowercase();
    let valid_domain = |d: &str| {
        d.contains('.')
            && !d.starts_with('.')
            && !d.ends_with('.')
            && !d.contains("..")
            && d.chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
    };
    let Some((local, domain)) = s.rsplit_once('@') else {
        // A bare domain.
        return (whole_domain && valid_domain(&s)).then(|| s.clone());
    };
    if !valid_domain(domain) {
        None
    } else if whole_domain {
        Some(domain.to_owned())
    } else if local.is_empty() || local.chars().any(char::is_whitespace) {
        None
    } else {
        Some(s.clone())
    }
}

fn read_rule(row: &Row<'_>) -> rusqlite::Result<(String, String, String, Option<String>, i64)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn into_rule(
    (id, spec, status, note, order): (String, String, String, Option<String>, i64),
) -> Result<Rule> {
    Ok(Rule {
        id,
        spec: serde_json::from_str(&spec)?,
        server_status: ServerStatus::parse(&status),
        server_note: note,
        order,
    })
}

const RULE_COLUMNS: &str = "SELECT id, spec, server_status, server_note, sort_order FROM rules";

/// Columns for [`RuleSubject`]; `?1` asks for the text, `?2` for the raw
/// message (for headers).
macro_rules! subject_columns {
    () => {
        "SELECT m.id, m.account_id, m.folder_id, m.from_name, m.from_address, m.to_json,
                m.cc_json, m.subject, m.has_attachments, m.importance, m.size, m.categories,
                lower(a.email),
                CASE WHEN ?1 THEN coalesce(m.body_text, nullif(m.preview, '')) END,
                CASE WHEN ?2 THEN m.raw END
         FROM messages m JOIN accounts a ON a.id = m.account_id "
    };
}

fn read_subject(row: &Row<'_>) -> rusqlite::Result<SubjectRow> {
    Ok(SubjectRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        from_name: row.get(3)?,
        from_address: row.get(4)?,
        to_json: row.get(5)?,
        cc_json: row.get(6)?,
        subject: row.get(7)?,
        has_attachments: row.get(8)?,
        importance: row.get(9)?,
        size: row.get(10)?,
        categories: row.get(11)?,
        email: row.get(12)?,
        text: row.get(13)?,
        raw: row.get(14)?,
    })
}

struct SubjectRow {
    id: String,
    account_id: String,
    folder_id: String,
    from_name: Option<String>,
    from_address: Option<String>,
    to_json: String,
    cc_json: String,
    subject: String,
    has_attachments: bool,
    importance: String,
    size: i64,
    categories: String,
    email: String,
    text: Option<String>,
    raw: Option<Vec<u8>>,
}

impl SubjectRow {
    fn into_subject(self) -> Result<RuleSubject> {
        let from = self.from_address.map(|address| Mailbox {
            name: self.from_name,
            address,
        });
        Ok(RuleSubject {
            id: self.id,
            account_id: self.account_id,
            folder_id: self.folder_id,
            facts: MessageFacts {
                from,
                to: serde_json::from_str(&self.to_json)?,
                cc: serde_json::from_str(&self.cc_json)?,
                subject: self.subject,
                text: self.text,
                headers: self.raw.as_deref().map(header_lines),
                has_attachments: self.has_attachments,
                importance: Importance::parse(&self.importance).unwrap_or_default(),
                size: to_u64(self.size.max(0), "message size")?,
                categories: serde_json::from_str(&self.categories)?,
                own_addresses: vec![self.email],
            },
        })
    }
}

impl Store {
    /// Every rule, in order.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_rules(&self) -> Result<Vec<Rule>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!("{RULE_COLUMNS} ORDER BY sort_order, id"))?;
        let rows = stmt
            .query_map([], read_rule)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(into_rule).collect()
    }

    /// One rule.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id.
    pub fn get_rule(&self, id: &str) -> Result<Rule> {
        let row = self
            .conn()
            .query_row(
                &format!("{RULE_COLUMNS} WHERE id = ?1"),
                params![id],
                read_rule,
            )
            .optional()?
            .ok_or(StoreError::NotFound("rule"))?;
        into_rule(row)
    }

    /// Add a rule (last in the order) or, with `id`, change one (keeping
    /// its place). Returns its id. The spec must already be checked.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown `id` or account.
    pub fn save_rule(
        &self,
        id: Option<&str>,
        spec: &RuleSpec,
        status: ServerStatus,
        note: Option<&str>,
    ) -> Result<String> {
        let json = serde_json::to_string(spec)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if let Some(account) = &spec.account_id {
            let known: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM accounts WHERE id = ?1",
                    params![account],
                    |row| row.get(0),
                )
                .optional()?;
            if known.is_none() {
                return Err(StoreError::NotFound("account"));
            }
        }
        let id = if let Some(id) = id {
            let changed = tx.execute(
                "UPDATE rules SET account_id = ?2, spec = ?3, server_status = ?4, server_note = ?5
                 WHERE id = ?1",
                params![id, spec.account_id, json, status.as_str(), note],
            )?;
            if changed == 0 {
                return Err(StoreError::NotFound("rule"));
            }
            id.to_owned()
        } else {
            let id = new_id()?;
            tx.execute(
                "INSERT INTO rules (id, account_id, sort_order, spec, server_status, server_note)
                 VALUES (?1, ?2, (SELECT coalesce(max(sort_order) + 1, 0) FROM rules), ?3, ?4, ?5)",
                params![id, spec.account_id, json, status.as_str(), note],
            )?;
            id
        };
        tx.commit()?;
        Ok(id)
    }

    /// Remove a rule.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown id.
    pub fn delete_rule(&self, id: &str) -> Result<()> {
        if self
            .conn()
            .execute("DELETE FROM rules WHERE id = ?1", params![id])?
            == 0
        {
            return Err(StoreError::NotFound("rule"));
        }
        Ok(())
    }

    /// Put the rules in this order: every rule id exactly once.
    ///
    /// # Errors
    /// [`StoreError::Invalid`] when `ids` isn't every rule once.
    pub fn reorder_rules<S: AsRef<str>>(&self, ids: &[S]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut known: Vec<String> = tx
            .prepare("SELECT id FROM rules")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut wanted: Vec<&str> = ids.iter().map(AsRef::as_ref).collect();
        known.sort();
        wanted.sort_unstable();
        if known.iter().map(String::as_str).ne(wanted.iter().copied()) {
            return Err(StoreError::Invalid("every rule exactly once"));
        }
        {
            let mut stmt = tx.prepare("UPDATE rules SET sort_order = ?2 WHERE id = ?1")?;
            for (order, id) in ids.iter().enumerate() {
                stmt.execute(params![
                    id.as_ref(),
                    i64::try_from(order).unwrap_or(i64::MAX)
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Record whether the mail server runs a rule. A rule removed since is
    /// skipped.
    ///
    /// # Errors
    /// Database errors.
    pub fn set_rule_server_status(
        &self,
        id: &str,
        status: ServerStatus,
        note: Option<&str>,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE rules SET server_status = ?2, server_note = ?3 WHERE id = ?1",
            params![id, status.as_str(), note],
        )?;
        Ok(())
    }

    /// Of these new messages, the ones rules should still run on, now
    /// marked as done: not done before, and no message with the same
    /// Message-ID in the account was done either (a copy a rule made).
    ///
    /// # Errors
    /// Database errors.
    pub fn claim_for_rules<S: AsRef<str>>(&self, ids: &[S]) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut claimed = Vec::new();
        {
            let mut check = tx.prepare(
                "SELECT m.rules_done OR EXISTS (
                     SELECT 1 FROM messages o
                     WHERE o.account_id = m.account_id AND o.rules_done = 1 AND o.id <> m.id
                       AND m.message_id_header IS NOT NULL
                       AND o.message_id_header = m.message_id_header)
                 FROM messages m WHERE m.id = ?1",
            )?;
            let mut mark = tx.prepare("UPDATE messages SET rules_done = 1 WHERE id = ?1")?;
            for id in ids {
                let done: Option<bool> = check
                    .query_row(params![id.as_ref()], |row| row.get(0))
                    .optional()?;
                match done {
                    None => {}
                    Some(true) => {
                        mark.execute(params![id.as_ref()])?;
                    }
                    Some(false) => {
                        mark.execute(params![id.as_ref()])?;
                        claimed.push(id.as_ref().to_owned());
                    }
                }
            }
        }
        tx.commit()?;
        Ok(claimed)
    }

    /// What rules see of these messages (unknown ids are skipped).
    /// `text`: include the message text (or preview); `headers`: the
    /// header lines of downloaded messages.
    ///
    /// # Errors
    /// Database errors.
    pub fn rule_subjects<S: AsRef<str>>(
        &self,
        ids: &[S],
        text: bool,
        headers: bool,
    ) -> Result<Vec<RuleSubject>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(concat!(subject_columns!(), "WHERE m.id = ?3"))?;
        let mut out = Vec::new();
        for id in ids {
            if let Some(row) = stmt
                .query_row(params![text, headers, id.as_ref()], read_subject)
                .optional()?
            {
                out.push(row.into_subject()?);
            }
        }
        Ok(out)
    }

    /// What rules see of a folder's newest `limit` messages.
    ///
    /// # Errors
    /// Database errors.
    pub fn folder_rule_subjects(
        &self,
        folder_id: &str,
        limit: u32,
        text: bool,
        headers: bool,
    ) -> Result<Vec<RuleSubject>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(concat!(
            subject_columns!(),
            "WHERE m.folder_id = ?3 ORDER BY m.date IS NULL, m.date DESC, m.seq DESC LIMIT ?4"
        ))?;
        let rows = stmt
            .query_map(params![text, headers, folder_id, limit], read_subject)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(SubjectRow::into_subject).collect()
    }

    /// How many messages a folder holds (for a rule run's count).
    ///
    /// # Errors
    /// Database errors.
    pub fn folder_message_count(&self, folder_id: &str) -> Result<u32> {
        let n: i64 = self.conn().query_row(
            "SELECT count(*) FROM messages WHERE folder_id = ?1",
            params![folder_id],
            |row| row.get(0),
        )?;
        to_u32(n, "message count")
    }

    /// Every sender whose pictures load, by name.
    ///
    /// # Errors
    /// Database errors.
    pub fn list_image_senders(&self) -> Result<Vec<ImageSender>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached("SELECT sender, whole_domain FROM image_senders ORDER BY sender")?;
        let rows = stmt.query_map([], |row| {
            Ok(ImageSender {
                sender: row.get(0)?,
                whole_domain: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Always load pictures from this key ([`image_sender_key`]).
    ///
    /// # Errors
    /// Database errors.
    pub fn trust_image_sender(&self, key: &str, whole_domain: bool) -> Result<()> {
        self.conn().execute(
            "INSERT INTO image_senders (sender, whole_domain) VALUES (?1, ?2)
             ON CONFLICT (sender) DO UPDATE SET whole_domain = excluded.whole_domain",
            params![key, whole_domain],
        )?;
        Ok(())
    }

    /// Stop trusting a sender as listed (any case). Returns whether it was
    /// there.
    ///
    /// # Errors
    /// Database errors.
    pub fn untrust_image_sender(&self, sender: &str) -> Result<bool> {
        let key = sender.trim().trim_start_matches('@').to_lowercase();
        Ok(self
            .conn()
            .execute("DELETE FROM image_senders WHERE sender = ?1", params![key])?
            > 0)
    }

    /// Whether pictures from this address always load: the address is
    /// listed, or its domain (or a domain it is under) is, as a whole.
    ///
    /// # Errors
    /// Database errors.
    pub fn images_trusted(&self, address: &str) -> Result<bool> {
        let address = address.trim().to_lowercase();
        let Some((_, domain)) = address.rsplit_once('@') else {
            return Ok(false);
        };
        let conn = self.conn();
        let listed: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM image_senders WHERE sender = ?1 AND whole_domain = 0",
                params![address],
                |row| row.get(0),
            )
            .optional()?;
        if listed.is_some() {
            return Ok(true);
        }
        let mut stmt =
            conn.prepare_cached("SELECT sender FROM image_senders WHERE whole_domain = 1")?;
        let domains = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for trusted in domains {
            let trusted = trusted?;
            if domain == trusted || domain.ends_with(&format!(".{trusted}")) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{add_account, add_folder, add_message, summary};
    use mail_core::folders::FolderRole;
    use mail_core::rules::{ActionKind, Match, RuleAction};

    fn spec(name: &str, account: Option<&str>) -> RuleSpec {
        RuleSpec {
            name: name.into(),
            enabled: true,
            account_id: account.map(str::to_owned),
            match_kind: Match::All,
            conditions: vec![],
            exceptions: vec![],
            actions: vec![RuleAction {
                kind: ActionKind::Flag,
                folder_id: None,
                role: None,
                category: None,
                importance: None,
            }],
            stop_processing: false,
            keep_on_server: false,
        }
    }

    fn names(store: &Store) -> Vec<String> {
        store
            .list_rules()
            .unwrap()
            .into_iter()
            .map(|r| r.spec.name)
            .collect()
    }

    #[test]
    fn rules_keep_their_order_and_status() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let a = store
            .save_rule(None, &spec("A", Some(&account)), ServerStatus::Local, None)
            .unwrap();
        let b = store
            .save_rule(
                None,
                &spec("B", None),
                ServerStatus::Unsupported,
                Some("why"),
            )
            .unwrap();
        let c = store
            .save_rule(None, &spec("C", None), ServerStatus::Local, None)
            .unwrap();
        assert_eq!(names(&store), ["A", "B", "C"]);
        let rules = store.list_rules().unwrap();
        assert_eq!(rules[1].server_status, ServerStatus::Unsupported);
        assert_eq!(rules[1].server_note.as_deref(), Some("why"));
        assert_eq!(rules[0].order, 0);
        assert_eq!(rules[2].order, 2);
        // Editing keeps the place.
        store
            .save_rule(
                Some(&a),
                &spec("A2", Some(&account)),
                ServerStatus::Local,
                None,
            )
            .unwrap();
        assert_eq!(names(&store), ["A2", "B", "C"]);
        store.reorder_rules(&[&c, &a, &b]).unwrap();
        assert_eq!(names(&store), ["C", "A2", "B"]);
        assert!(matches!(
            store.reorder_rules(&[&c, &a]),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            store.reorder_rules(&[&c, &a, &a]),
            Err(StoreError::Invalid(_))
        ));
        store
            .set_rule_server_status(&c, ServerStatus::Server, None)
            .unwrap();
        assert_eq!(
            store.get_rule(&c).unwrap().server_status,
            ServerStatus::Server
        );
        store.delete_rule(&b).unwrap();
        assert!(matches!(
            store.delete_rule(&b),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            store.save_rule(Some(&b), &spec("B", None), ServerStatus::Local, None),
            Err(StoreError::NotFound("rule"))
        ));
        assert!(matches!(
            store.save_rule(None, &spec("X", Some("nope")), ServerStatus::Local, None),
            Err(StoreError::NotFound("account"))
        ));
        // A rule goes with its account.
        store.delete_account(&account).unwrap();
        assert_eq!(names(&store), ["C"]);
        // New rules still go last.
        store
            .save_rule(None, &spec("D", None), ServerStatus::Local, None)
            .unwrap();
        assert_eq!(names(&store), ["C", "D"]);
    }

    #[test]
    fn subjects_carry_what_rules_look_at() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "Me@Example.com");
        let inbox = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        let mut s = summary("Invoice", 10);
        s.importance = Importance::High;
        let id = add_message(&store, &inbox, 1, &s);
        let old = add_message(&store, &inbox, 2, &summary("Older", 5));
        store
            .set_raw(
                &id,
                b"List-Id: <news.example>\r\nSubject: Invoice\r\n\r\nbody",
                Some("Pay now"),
            )
            .unwrap();
        let bare = store
            .rule_subjects(&[&id, &"missing".to_owned()], false, false)
            .unwrap();
        assert_eq!(bare.len(), 1);
        let f = &bare[0].facts;
        assert_eq!(f.subject, "Invoice");
        assert_eq!(f.from.as_ref().unwrap().address, "alice@example.com");
        assert_eq!(f.cc[0].address, "carol@example.net");
        assert_eq!(f.importance, Importance::High);
        assert_eq!(f.own_addresses, ["me@example.com"]);
        assert_eq!(f.text, None);
        assert_eq!(f.headers, None);
        let full = store.rule_subjects(&[&id], true, true).unwrap();
        assert_eq!(full[0].facts.text.as_deref(), Some("Pay now"));
        assert_eq!(
            full[0].facts.headers.as_ref().unwrap()[0],
            ("List-Id".to_owned(), "<news.example>".to_owned())
        );
        // Not downloaded: the preview stands in for the text.
        let older = store.rule_subjects(&[&old], true, true).unwrap();
        assert_eq!(older[0].facts.text.as_deref(), Some("preview of Older"));
        assert_eq!(older[0].facts.headers, None);
        let folder = store.folder_rule_subjects(&inbox, 1, false, false).unwrap();
        assert_eq!(folder.len(), 1);
        assert_eq!(folder[0].id, id);
        assert_eq!(store.folder_message_count(&inbox).unwrap(), 2);
    }

    #[test]
    fn rules_run_once_per_message_and_not_on_its_copies() {
        let store = Store::open_in_memory().unwrap();
        let account = add_account(&store, "a@example.com");
        let inbox = add_folder(&store, &account, "INBOX", FolderRole::Inbox);
        let first = add_message(&store, &inbox, 1, &summary("Hello", 1));
        assert_eq!(
            store.claim_for_rules(&[&first]).unwrap(),
            std::slice::from_ref(&first)
        );
        assert!(store.claim_for_rules(&[&first]).unwrap().is_empty());
        // A copy of it (same Message-ID) arriving later.
        let copy = add_message(&store, &inbox, 2, &summary("Hello", 1));
        assert_ne!(copy, first);
        assert!(store.claim_for_rules(&[&copy]).unwrap().is_empty());
        // Moved into the inbox here: the server's copy isn't new mail.
        let archive = add_folder(&store, &account, "Archive", FolderRole::Archive);
        let filed = add_message(&store, &archive, 1, &summary("Filed", 3));
        store.move_local(&[&filed], &inbox).unwrap();
        let adopted = add_message(&store, &inbox, 9, &summary("Filed", 3));
        assert_eq!(adopted, filed);
        assert!(store.claim_for_rules(&[&adopted]).unwrap().is_empty());
        let other = add_message(&store, &inbox, 3, &summary("Other", 2));
        assert_eq!(
            store
                .claim_for_rules(&[&other, &"missing".to_owned()])
                .unwrap(),
            [other]
        );
    }

    #[test]
    fn trusted_picture_senders() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            image_sender_key(" News@Shop.Example ", false).as_deref(),
            Some("news@shop.example")
        );
        assert_eq!(
            image_sender_key("news@shop.example", true).as_deref(),
            Some("shop.example")
        );
        assert_eq!(
            image_sender_key("@shop.example", true).as_deref(),
            Some("shop.example")
        );
        assert_eq!(
            image_sender_key("shop.example", true).as_deref(),
            Some("shop.example")
        );
        assert_eq!(image_sender_key("shop.example", false), None);
        assert_eq!(image_sender_key("a@b", false), None);
        assert_eq!(image_sender_key("a b@c.example", false), None);
        assert_eq!(image_sender_key("x@..example", true), None);

        store
            .trust_image_sender("news@shop.example", false)
            .unwrap();
        store.trust_image_sender("bank.example", true).unwrap();
        assert!(store.images_trusted("NEWS@shop.example").unwrap());
        assert!(!store.images_trusted("sales@shop.example").unwrap());
        assert!(store.images_trusted("alerts@bank.example").unwrap());
        assert!(store.images_trusted("x@mail.bank.example").unwrap());
        assert!(!store.images_trusted("x@notbank.example").unwrap());
        assert!(!store.images_trusted("x@bank.example.evil").unwrap());
        assert!(!store.images_trusted("not an address").unwrap());
        assert_eq!(
            store.list_image_senders().unwrap(),
            vec![
                ImageSender {
                    sender: "bank.example".into(),
                    whole_domain: true
                },
                ImageSender {
                    sender: "news@shop.example".into(),
                    whole_domain: false
                },
            ]
        );
        assert!(store.untrust_image_sender("@Bank.Example").unwrap());
        assert!(!store.untrust_image_sender("bank.example").unwrap());
        assert!(!store.images_trusted("alerts@bank.example").unwrap());
        assert_eq!(
            serde_json::to_value(&store.list_image_senders().unwrap()[0]).unwrap(),
            serde_json::json!({ "sender": "news@shop.example", "wholeDomain": false })
        );
    }
}
