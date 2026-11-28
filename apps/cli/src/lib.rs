//! `mocky-mail`, the command line for Mocky Mail, and its MCP server
//! (`mocky-mail mcp`). Both talk to the running app over its local
//! automation channel (Settings › Automation): a named pipe on Windows, a
//! Unix socket in the app's data folder elsewhere, with the secret from the
//! `automation-token` file there. Nothing goes over the network.
//!
//! See `docs/automation.md`.

#![allow(clippy::missing_errors_doc)]

pub mod args;
pub mod channel;
pub mod mcp;
pub mod output;
pub mod paths;

use serde_json::{Value, json};

use crate::args::{Command, Options};
use crate::channel::{CallError, Channel, LocalChannel};

/// The help text.
pub const HELP: &str = "\
mocky-mail: Mocky Mail from the command line, and its MCP server.

Mocky Mail must be running with Settings > Automation turned on (Read only,
or Full access to send, move and change things).

Usage: mocky-mail [options] <command> [arguments]

Commands:
  accounts                     List the accounts.
  folders <account>            List an account's folders (account: id or address).
  list [--folder ID] [--account A] [--search QUERY] [--unread] [--limit N]
                               List messages (the inboxes when no folder is given).
  search <query> [--limit N]   Search every account.
  read <message-id>            Show a message.
  send --from ACCOUNT --to ADDRESSES --subject TEXT --body TEXT
       [--cc ADDRESSES] [--bcc ADDRESSES] [--html FILE]
                               Send a message (goes through the Outbox, Undo Send applies).
  commands                     List every command the app allows in its current mode.
  call <name> [json-args]      Run any command with JSON arguments, e.g.
                               mocky-mail call get_message '{\"id\":\"...\",\"allowRemote\":false}'
  mcp                          Run the MCP server on standard input and output.

Options:
  --endpoint PATH      The app's automation endpoint (default: the app's own).
  --token-file FILE    The file with the automation secret (default: the app's).
  --json               Print results as JSON.
  -h, --help           Show this help.
  -V, --version        Show the version.

The endpoint and secret can also be set with MOCKY_MAIL_ENDPOINT and
MOCKY_MAIL_TOKEN_FILE.";

/// Run the program with its arguments (without the program name); the exit
/// code.
pub async fn run(arguments: Vec<String>) -> i32 {
    let (options, command) = match args::parse(&arguments) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("mocky-mail: {error}\nRun mocky-mail --help for the commands.");
            return 2;
        }
    };
    match command {
        Command::Help => {
            println!("{HELP}");
            return 0;
        }
        Command::Version => {
            println!("mocky-mail {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        _ => {}
    }
    let channel = LocalChannel::from_options(&options);
    if command == Command::Mcp {
        mcp::serve(channel, tokio::io::stdin(), tokio::io::stdout()).await;
        return 0;
    }
    let mut channel = channel;
    match execute(&mut channel, &options, command).await {
        Ok(text) => {
            if !text.is_empty() {
                println!("{text}");
            }
            0
        }
        Err(error) => {
            eprintln!("mocky-mail: {}", error.message);
            1
        }
    }
}

/// The account an argument names: its id, or its address.
async fn account_id<C: Channel>(channel: &mut C, wanted: &str) -> Result<String, CallError> {
    let accounts = channel.call("list_accounts", Value::Null).await?;
    accounts
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| {
            a.get("id").and_then(Value::as_str) == Some(wanted)
                || a.get("email")
                    .and_then(Value::as_str)
                    .is_some_and(|e| e.eq_ignore_ascii_case(wanted))
        })
        .and_then(|a| a.get("id").and_then(Value::as_str).map(str::to_owned))
        .ok_or_else(|| CallError::new("not_found", format!("There is no account {wanted}.")))
}

/// Run one command; the text to print.
#[allow(clippy::too_many_lines, reason = "one arm per command")]
pub async fn execute<C: Channel>(
    channel: &mut C,
    options: &Options,
    command: Command,
) -> Result<String, CallError> {
    let print = |value: &Value, friendly: fn(&Value) -> String| {
        if options.json {
            serde_json::to_string_pretty(value).unwrap_or_default()
        } else {
            friendly(value)
        }
    };
    match command {
        Command::Help | Command::Version | Command::Mcp => Ok(String::new()),
        Command::Commands => {
            let listed = channel.call("__commands", Value::Null).await?;
            Ok(print(&listed, output::commands))
        }
        Command::Call { name, args } => {
            let result = channel.call(&name, args).await?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        Command::Accounts => {
            let accounts = channel.call("list_accounts", Value::Null).await?;
            Ok(print(&accounts, output::accounts))
        }
        Command::Folders { account } => {
            let id = account_id(channel, &account).await?;
            let folders = channel
                .call("list_folders", json!({ "accountId": id }))
                .await?;
            Ok(print(&folders, output::folders))
        }
        Command::List {
            folder,
            account,
            search,
            unread,
            limit,
        } => {
            let account = match account {
                Some(account) => Some(account_id(channel, &account).await?),
                None => None,
            };
            let query = list_query(folder, account, search, unread, limit);
            let page = channel
                .call("list_messages", json!({ "query": query, "threads": false }))
                .await?;
            Ok(print(&page, output::messages))
        }
        Command::Search { query, limit } => {
            let query = json!({
                "search": query,
                "searchScope": "all",
                "filter": "all",
                "offset": 0,
                "limit": limit,
            });
            let page = channel
                .call("list_messages", json!({ "query": query, "threads": false }))
                .await?;
            Ok(print(&page, output::messages))
        }
        Command::Read { id } => {
            let message = channel
                .call("get_message", json!({ "id": id, "allowRemote": false }))
                .await?;
            Ok(print(&message, output::message))
        }
        Command::Send {
            from,
            to,
            cc,
            bcc,
            subject,
            body,
            html,
        } => {
            let account = account_id(channel, &from).await?;
            let body_html = match html {
                Some(path) => Some(std::fs::read_to_string(&path).map_err(|error| {
                    CallError::new(
                        "invalid",
                        format!("Couldn't read {}: {error}", path.display()),
                    )
                })?),
                None => None,
            };
            let draft = json!({
                "accountId": account,
                "to": to,
                "cc": cc,
                "bcc": bcc,
                "subject": subject,
                "bodyText": body,
                "bodyHtml": body_html,
                "inReplyTo": null,
                "references": [],
                "attachments": [],
                "replyToMessageId": null,
            });
            let queued = channel
                .call("send_message", json!({ "draft": draft }))
                .await?;
            Ok(print(&queued, |_| "Queued in the Outbox.".to_owned()))
        }
    }
}

/// The `list_messages` query for `list`.
#[must_use]
pub fn list_query(
    folder: Option<String>,
    account: Option<String>,
    search: Option<String>,
    unread: bool,
    limit: u32,
) -> Value {
    let mut query = serde_json::Map::new();
    query.insert(
        "filter".into(),
        json!(if unread { "unread" } else { "all" }),
    );
    query.insert("offset".into(), json!(0));
    query.insert("limit".into(), json!(limit));
    let selected = folder.is_some();
    if let Some(folder) = folder {
        query.insert("folderId".into(), json!(folder));
    }
    if let Some(account) = account {
        query.insert("accountId".into(), json!(account));
    }
    if !selected {
        query.insert("role".into(), json!("inbox"));
    }
    if let Some(search) = search {
        query.insert("search".into(), json!(search));
        query.insert(
            "searchScope".into(),
            json!(if selected { "folder" } else { "all" }),
        );
    }
    Value::Object(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::tests::Fake;

    #[tokio::test]
    async fn friendly_commands_call_the_app() {
        let mut fake = Fake::new();
        let options = Options::default();
        let text = execute(&mut fake, &options, Command::Accounts)
            .await
            .unwrap();
        assert!(text.contains("don@mocky.co.ke"), "{text}");
        let text = execute(
            &mut fake,
            &options,
            Command::Folders {
                account: "DON@mocky.co.ke".into(),
            },
        )
        .await
        .unwrap();
        assert!(text.contains("f1  inbox    INBOX (2 unread)"), "{text}");
        assert_eq!(
            fake.calls.last(),
            Some(&("list_folders".to_owned(), json!({ "accountId": "a1" })))
        );
        let missing = execute(
            &mut fake,
            &options,
            Command::Folders {
                account: "who@else".into(),
            },
        )
        .await;
        assert_eq!(missing.map_err(|e| e.kind), Err("not_found".to_owned()));
    }

    #[tokio::test]
    async fn send_builds_a_draft() {
        let mut fake = Fake::new();
        let options = Options::default();
        execute(
            &mut fake,
            &options,
            Command::Send {
                from: "don@mocky.co.ke".into(),
                to: "ann@example.com".into(),
                cc: String::new(),
                bcc: String::new(),
                subject: "Hi".into(),
                body: "Hello".into(),
                html: None,
            },
        )
        .await
        .unwrap();
        let (name, args) = fake.calls.last().cloned().unwrap();
        assert_eq!(name, "send_message");
        assert_eq!(args["draft"]["accountId"], "a1");
        assert_eq!(args["draft"]["to"], "ann@example.com");
        assert_eq!(args["draft"]["bodyText"], "Hello");
        assert_eq!(args["draft"]["references"], json!([]));
    }

    #[test]
    fn list_queries() {
        assert_eq!(
            list_query(None, None, None, false, 20),
            json!({ "filter": "all", "offset": 0, "limit": 20, "role": "inbox" })
        );
        assert_eq!(
            list_query(Some("f1".into()), None, Some("from:ann".into()), true, 5),
            json!({ "filter": "unread", "offset": 0, "limit": 5, "folderId": "f1", "search": "from:ann", "searchScope": "folder" })
        );
    }
}
