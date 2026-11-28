//! Results as text for people (`--json` prints them as they came).

use std::fmt::Write as _;

use serde_json::Value;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A mailbox as `Name <address>`, or the address.
fn mailbox(value: &Value) -> String {
    let address = text(value, "address");
    match value.get("name").and_then(Value::as_str) {
        Some(name) if !name.is_empty() => format!("{name} <{address}>"),
        _ => address.to_owned(),
    }
}

fn mailboxes(value: &Value) -> String {
    value
        .as_array()
        .map(|list| list.iter().map(mailbox).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

/// A Unix time as `YYYY-MM-DD HH:MM` (UTC).
#[must_use]
#[allow(
    clippy::arithmetic_side_effects,
    reason = "days since 1970 of an i64 of seconds stay far inside i64"
)]
pub fn date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil.
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// `accounts`.
#[must_use]
pub fn accounts(value: &Value) -> String {
    let mut out = String::new();
    for account in value.as_array().into_iter().flatten() {
        let _ = writeln!(
            out,
            "{}  {}  {} ({})",
            text(account, "id"),
            text(account, "email"),
            text(account, "displayName"),
            account
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("imap"),
        );
    }
    if out.is_empty() {
        "No accounts.".into()
    } else {
        out.trim_end().to_owned()
    }
}

/// `folders`.
#[must_use]
pub fn folders(value: &Value) -> String {
    let mut out = String::new();
    for folder in value.as_array().into_iter().flatten() {
        let unread = folder.get("unread").and_then(Value::as_u64).unwrap_or(0);
        let _ = writeln!(
            out,
            "{}  {:<8} {}{}",
            text(folder, "id"),
            text(folder, "role"),
            text(folder, "path"),
            if unread > 0 {
                format!(" ({unread} unread)")
            } else {
                String::new()
            }
        );
    }
    out.trim_end().to_owned()
}

/// `list` and `search`.
#[must_use]
pub fn messages(value: &Value) -> String {
    let mut out = String::new();
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in &items {
        let when = item
            .get("date")
            .and_then(Value::as_i64)
            .map(date)
            .unwrap_or_default();
        let from = item.get("from").map(mailbox).unwrap_or_default();
        let unread = if item.get("unread").and_then(Value::as_bool) == Some(true) {
            "*"
        } else {
            " "
        };
        let _ = writeln!(
            out,
            "{unread} {}  {when}  {from}  {}",
            text(item, "id"),
            text(item, "subject")
        );
    }
    let total = value.get("total").and_then(Value::as_u64).unwrap_or(0);
    let _ = write!(out, "{} of {total} messages", items.len());
    out
}

/// `read`.
#[must_use]
pub fn message(value: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "From:    {}",
        value.get("from").map(mailbox).unwrap_or_default()
    );
    let _ = writeln!(
        out,
        "To:      {}",
        mailboxes(value.get("to").unwrap_or(&Value::Null))
    );
    let cc = mailboxes(value.get("cc").unwrap_or(&Value::Null));
    if !cc.is_empty() {
        let _ = writeln!(out, "Cc:      {cc}");
    }
    if let Some(when) = value.get("date").and_then(Value::as_i64) {
        let _ = writeln!(out, "Date:    {} UTC", date(when));
    }
    let _ = writeln!(out, "Subject: {}", text(value, "subject"));
    let attachments = value
        .get("attachments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !attachments.is_empty() {
        let names: Vec<&str> = attachments
            .iter()
            .map(|a| a.get("fileName").and_then(Value::as_str).unwrap_or("?"))
            .collect();
        let _ = writeln!(out, "Attached: {}", names.join(", "));
    }
    out.push('\n');
    let body = value
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| text(value, "preview"));
    out.push_str(body.trim_end());
    out
}

/// `commands`.
#[must_use]
pub fn commands(value: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Automation is set to {}.",
        match text(value, "mode") {
            "full" => "full access",
            "read" => "read only",
            other => other,
        }
    );
    for command in value
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let args: Vec<String> = command
            .get("args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|a| {
                let name = text(a, "name");
                if a.get("required").and_then(Value::as_bool) == Some(true) {
                    name.to_owned()
                } else {
                    format!("[{name}]")
                }
            })
            .collect();
        let _ = writeln!(
            out,
            "{}({})  {}",
            text(command, "name"),
            args.join(", "),
            text(command, "description")
        );
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn dates_are_utc() {
        assert_eq!(date(0), "1970-01-01 00:00");
        assert_eq!(date(1_798_675_200), "2026-12-31 00:00");
        assert_eq!(date(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn messages_and_a_message() {
        let page = json!({ "items": [
            { "id": "m1", "subject": "Invoice", "date": 0, "unread": true, "from": { "name": "Ann", "address": "ann@example.com" } }
        ], "total": 3 });
        let shown = messages(&page);
        assert!(
            shown.contains("* m1  1970-01-01 00:00  Ann <ann@example.com>  Invoice"),
            "{shown}"
        );
        assert!(shown.ends_with("1 of 3 messages"));
        let detail = json!({
            "subject": "Invoice", "from": { "name": null, "address": "ann@example.com" },
            "to": [{ "name": "Don", "address": "don@mocky.co.ke" }], "cc": [],
            "text": "Please pay.\n", "attachments": [{ "fileName": "invoice.pdf" }]
        });
        let shown = message(&detail);
        assert!(shown.contains("From:    ann@example.com"));
        assert!(shown.contains("To:      Don <don@mocky.co.ke>"));
        assert!(shown.contains("Attached: invoice.pdf"));
        assert!(shown.ends_with("Please pay."));
    }
}
