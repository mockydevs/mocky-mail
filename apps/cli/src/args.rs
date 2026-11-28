//! The command line's arguments.

use std::path::PathBuf;

use serde_json::Value;

/// Options for every command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// `--endpoint`: the app's automation endpoint.
    pub endpoint: Option<String>,
    /// `--token-file`: the file with the automation secret.
    pub token_file: Option<PathBuf>,
    /// `--json`: print results as JSON.
    pub json: bool,
}

/// What to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `--help`.
    Help,
    /// `--version`.
    Version,
    /// `commands`.
    Commands,
    /// `call <name> [json-args]`.
    Call {
        /// The command.
        name: String,
        /// Its arguments.
        args: Value,
    },
    /// `accounts`.
    Accounts,
    /// `folders <account>`.
    Folders {
        /// An account id or address.
        account: String,
    },
    /// `list`.
    List {
        /// `--folder`.
        folder: Option<String>,
        /// `--account`.
        account: Option<String>,
        /// `--search`.
        search: Option<String>,
        /// `--unread`.
        unread: bool,
        /// `--limit`.
        limit: u32,
    },
    /// `search <query>`.
    Search {
        /// The search.
        query: String,
        /// `--limit`.
        limit: u32,
    },
    /// `read <message-id>`.
    Read {
        /// The message.
        id: String,
    },
    /// `send`.
    Send {
        /// `--from`: an account id or address.
        from: String,
        /// `--to`.
        to: String,
        /// `--cc`.
        cc: String,
        /// `--bcc`.
        bcc: String,
        /// `--subject`.
        subject: String,
        /// `--body`.
        body: String,
        /// `--html`: a file with the HTML body.
        html: Option<PathBuf>,
    },
    /// `mcp`.
    Mcp,
}

/// Messages listed when `--limit` isn't given.
pub const DEFAULT_LIMIT: u32 = 20;
/// Most messages one `list` or `search` asks for.
pub const MAX_LIMIT: u32 = 200;

/// Split `--name=value` and `--name value` options from the rest.
struct Words {
    options: Vec<(String, Option<String>)>,
    positional: Vec<String>,
}

/// Options that take a value.
const VALUED: &[&str] = &[
    "--endpoint",
    "--token-file",
    "--folder",
    "--account",
    "--search",
    "--limit",
    "--from",
    "--to",
    "--cc",
    "--bcc",
    "--subject",
    "--body",
    "--html",
];

fn split(arguments: &[String]) -> Result<Words, String> {
    let mut words = Words {
        options: Vec::new(),
        positional: Vec::new(),
    };
    let mut rest = arguments.iter();
    let mut only_positional = false;
    while let Some(word) = rest.next() {
        if only_positional || !word.starts_with('-') || word == "-" {
            words.positional.push(word.clone());
            continue;
        }
        if word == "--" {
            only_positional = true;
            continue;
        }
        let (name, inline) = match word.split_once('=') {
            Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
            None => (word.clone(), None),
        };
        if VALUED.contains(&name.as_str()) {
            let value = match inline {
                Some(value) => value,
                None => rest
                    .next()
                    .cloned()
                    .ok_or_else(|| format!("{name} needs a value"))?,
            };
            words.options.push((name, Some(value)));
        } else if inline.is_some() {
            return Err(format!("{name} takes no value"));
        } else {
            words.options.push((name, None));
        }
    }
    Ok(words)
}

impl Words {
    fn take(&mut self, name: &str) -> Option<String> {
        let at = self.options.iter().position(|(n, _)| n == name)?;
        self.options.remove(at).1
    }

    fn flag(&mut self, names: &[&str]) -> bool {
        let before = self.options.len();
        self.options.retain(|(n, _)| !names.contains(&n.as_str()));
        self.options.len() != before
    }

    fn limit(&mut self) -> Result<u32, String> {
        match self.take("--limit") {
            None => Ok(DEFAULT_LIMIT),
            Some(text) => match text.parse::<u32>() {
                Ok(n) if (1..=MAX_LIMIT).contains(&n) => Ok(n),
                _ => Err(format!("--limit must be a number from 1 to {MAX_LIMIT}")),
            },
        }
    }

    fn required(&mut self, name: &str) -> Result<String, String> {
        self.take(name).ok_or_else(|| format!("{name} is required"))
    }

    fn done(self, command: &str) -> Result<(), String> {
        if let Some((name, _)) = self.options.first() {
            return Err(format!("{command} doesn't take {name}"));
        }
        Ok(())
    }
}

/// Parse the arguments (without the program name).
#[allow(clippy::too_many_lines, reason = "one arm per command")]
pub fn parse(arguments: &[String]) -> Result<(Options, Command), String> {
    let mut words = split(arguments)?;
    let options = Options {
        endpoint: words.take("--endpoint"),
        token_file: words.take("--token-file").map(PathBuf::from),
        json: words.flag(&["--json"]),
    };
    if words.flag(&["-h", "--help"]) {
        return Ok((options, Command::Help));
    }
    if words.flag(&["-V", "--version"]) {
        return Ok((options, Command::Version));
    }
    let mut positional = std::mem::take(&mut words.positional).into_iter();
    let Some(name) = positional.next() else {
        return Ok((options, Command::Help));
    };
    let rest: Vec<String> = positional.collect();
    let at_most = |count: usize| -> Result<(), String> {
        if rest.len() > count {
            Err(format!("{name}: too many arguments"))
        } else {
            Ok(())
        }
    };
    let command = match name.as_str() {
        "help" => Command::Help,
        "commands" => {
            at_most(0)?;
            Command::Commands
        }
        "accounts" => {
            at_most(0)?;
            Command::Accounts
        }
        "mcp" => {
            at_most(0)?;
            Command::Mcp
        }
        "call" => {
            at_most(2)?;
            let command = rest
                .first()
                .cloned()
                .ok_or("call needs a command name (see mocky-mail commands)")?;
            let args = match rest.get(1) {
                Some(text) => serde_json::from_str::<Value>(text)
                    .map_err(|e| format!("the arguments aren't JSON: {e}"))?,
                None => Value::Null,
            };
            if !(args.is_object() || args.is_null()) {
                return Err("the arguments must be a JSON object".into());
            }
            Command::Call {
                name: command,
                args,
            }
        }
        "folders" => {
            at_most(1)?;
            Command::Folders {
                account: rest
                    .first()
                    .cloned()
                    .ok_or("folders needs an account (id or address)")?,
            }
        }
        "list" => {
            at_most(0)?;
            Command::List {
                folder: words.take("--folder"),
                account: words.take("--account"),
                search: words.take("--search"),
                unread: words.flag(&["--unread"]),
                limit: words.limit()?,
            }
        }
        "search" => {
            if rest.is_empty() {
                return Err("search needs something to look for".into());
            }
            Command::Search {
                query: rest.join(" "),
                limit: words.limit()?,
            }
        }
        "read" => {
            at_most(1)?;
            Command::Read {
                id: rest.first().cloned().ok_or("read needs a message id")?,
            }
        }
        "send" => {
            at_most(0)?;
            Command::Send {
                from: words.required("--from")?,
                to: words.required("--to")?,
                cc: words.take("--cc").unwrap_or_default(),
                bcc: words.take("--bcc").unwrap_or_default(),
                subject: words.take("--subject").unwrap_or_default(),
                body: words.take("--body").unwrap_or_default(),
                html: words.take("--html").map(PathBuf::from),
            }
        }
        other => return Err(format!("there is no command called {other}")),
    };
    words.done(&name)?;
    Ok((options, command))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &[&str]) -> Result<(Options, Command), String> {
        let words: Vec<String> = line.iter().map(|w| (*w).to_owned()).collect();
        parse(&words)
    }

    #[test]
    fn commands_and_their_options() {
        assert_eq!(parsed(&[]).unwrap().1, Command::Help);
        assert_eq!(parsed(&["--version"]).unwrap().1, Command::Version);
        assert_eq!(parsed(&["accounts"]).unwrap().1, Command::Accounts);
        assert_eq!(
            parsed(&["list", "--folder", "f1", "--limit=5", "--unread"])
                .unwrap()
                .1,
            Command::List {
                folder: Some("f1".into()),
                account: None,
                search: None,
                unread: true,
                limit: 5
            }
        );
        assert_eq!(
            parsed(&["search", "from:ann", "invoice"]).unwrap().1,
            Command::Search {
                query: "from:ann invoice".into(),
                limit: DEFAULT_LIMIT
            }
        );
        assert_eq!(
            parsed(&["call", "get_message", r#"{"id":"m1","allowRemote":false}"#])
                .unwrap()
                .1,
            Command::Call {
                name: "get_message".into(),
                args: serde_json::json!({ "id": "m1", "allowRemote": false })
            }
        );
        let (options, send) = parsed(&[
            "--json",
            "send",
            "--from",
            "don@mocky.co.ke",
            "--to",
            "ann@example.com",
            "--subject",
            "Hi",
            "--body",
            "Hello",
            "--endpoint",
            "/tmp/x.sock",
        ])
        .unwrap();
        assert!(options.json);
        assert_eq!(options.endpoint.as_deref(), Some("/tmp/x.sock"));
        assert!(
            matches!(send, Command::Send { ref subject, ref cc, .. } if subject == "Hi" && cc.is_empty())
        );
        assert_eq!(
            parsed(&["mcp", "--token-file", "t"]).unwrap().0.token_file,
            Some(PathBuf::from("t"))
        );
    }

    #[test]
    fn mistakes_are_explained() {
        for (line, wanted) in [
            (&["launch"][..], "no command called launch"),
            (&["send", "--to", "a@b.c"][..], "--from is required"),
            (&["list", "--limit", "0"][..], "--limit must be"),
            (&["list", "--limit"][..], "--limit needs a value"),
            (&["call"][..], "call needs a command name"),
            (&["call", "x", "[1]"][..], "must be a JSON object"),
            (&["call", "x", "{"][..], "aren't JSON"),
            (&["accounts", "extra"][..], "too many arguments"),
            (&["accounts", "--folder", "f"][..], "doesn't take --folder"),
            (&["read"][..], "needs a message id"),
        ] {
            let error = parsed(line).unwrap_err();
            assert!(error.contains(wanted), "{line:?}: {error}");
        }
    }
}
