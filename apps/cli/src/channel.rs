//! The connection to the running app: one JSON request per line, carrying
//! the secret, and one answer per line (`docs/automation.md`).

use std::future::Future;
use std::path::PathBuf;

use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader, ReadHalf, WriteHalf,
};

use crate::args::Options;
use crate::paths;

/// What to tell someone whose app isn't reachable.
pub const TURN_ON: &str = "Turn on Settings › Automation in Mocky Mail";

/// An error from the app (its `AppError`), or from reaching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallError {
    /// `AppError.kind`; `connect` when the app can't be reached.
    pub kind: String,
    /// Plain words.
    pub message: String,
}

impl CallError {
    /// An error of this kind.
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.to_owned(),
            message: message.into(),
        }
    }
}

/// Something commands can be run through: the app, or a stand-in in tests.
pub trait Channel {
    /// Run a command with its arguments; its result.
    fn call(
        &mut self,
        command: &str,
        args: Value,
    ) -> impl Future<Output = Result<Value, CallError>>;
}

#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type Stream = tokio::net::UnixStream;

struct Connection {
    reader: BufReader<ReadHalf<Stream>>,
    writer: WriteHalf<Stream>,
}

/// The running app, connected to when first needed and again after it
/// restarts.
pub struct LocalChannel {
    endpoint: Option<String>,
    token_file: Option<PathBuf>,
    connection: Option<Connection>,
    token: Option<String>,
    next_id: u64,
}

/// The longest answer read, bytes.
const MAX_REPLY_BYTES: u64 = 256 * 1024 * 1024;

impl LocalChannel {
    /// The app's own endpoint and secret, or the ones the options name.
    #[must_use]
    pub fn from_options(options: &Options) -> Self {
        let data_dir = paths::data_dir();
        Self {
            endpoint: options
                .endpoint
                .clone()
                .or_else(|| paths::default_endpoint(data_dir.as_deref())),
            token_file: options
                .token_file
                .clone()
                .or_else(|| paths::default_token_file(data_dir.as_deref())),
            connection: None,
            token: None,
            next_id: 1,
        }
    }

    fn read_token(&self) -> Result<String, CallError> {
        let Some(path) = &self.token_file else {
            return Err(CallError::new(
                "auth",
                format!(
                    "Couldn't tell where Mocky Mail keeps its automation secret. {TURN_ON}, or pass --token-file."
                ),
            ));
        };
        let text = std::fs::read_to_string(path).map_err(|_| {
            CallError::new(
                "auth",
                format!(
                    "There is no automation secret at {}. {TURN_ON} (Read only or Full access).",
                    path.display()
                ),
            )
        })?;
        Ok(text.trim().to_owned())
    }

    async fn open(endpoint: &str) -> std::io::Result<Stream> {
        #[cfg(windows)]
        {
            // All instances busy: wait a little for one.
            const PIPE_BUSY: i32 = 231;
            let mut tries = 0u32;
            loop {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
                    Err(error) if error.raw_os_error() == Some(PIPE_BUSY) && tries < 50 => {
                        tries = tries.saturating_add(1);
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    other => return other,
                }
            }
        }
        #[cfg(unix)]
        {
            tokio::net::UnixStream::connect(endpoint).await
        }
    }

    async fn connect(&mut self) -> Result<(), CallError> {
        if self.connection.is_some() {
            return Ok(());
        }
        let Some(endpoint) = self.endpoint.clone() else {
            return Err(CallError::new(
                "connect",
                format!("Couldn't tell where Mocky Mail listens. {TURN_ON}, or pass --endpoint."),
            ));
        };
        let token = self.read_token()?;
        let stream = Self::open(&endpoint).await.map_err(|_| {
            CallError::new(
                "connect",
                format!("Mocky Mail isn't running, or automation is off. Start Mocky Mail, then {TURN_ON}."),
            )
        })?;
        let (read, writer) = tokio::io::split(stream);
        self.connection = Some(Connection {
            reader: BufReader::new(read),
            writer,
        });
        self.token = Some(token);
        Ok(())
    }

    /// Send one request and wait for its answer (event lines are skipped).
    async fn exchange(&mut self, command: &str, args: &Value) -> Result<Value, std::io::Error> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let token = self.token.clone().unwrap_or_default();
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotConnected))?;
        let request = json!({ "id": id, "token": token, "command": command, "args": args });
        let mut line = serde_json::to_vec(&request)?;
        line.push(b'\n');
        connection.writer.write_all(&line).await?;
        connection.writer.flush().await?;
        loop {
            let mut reply = String::new();
            let read = (&mut connection.reader)
                .take(MAX_REPLY_BYTES)
                .read_line(&mut reply)
                .await?;
            if read == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            let value: Value = serde_json::from_str(&reply)?;
            if value.get("id") == Some(&json!(id)) {
                return Ok(value);
            }
        }
    }
}

/// The answer's result, or its error.
fn result_of(reply: &Value) -> Result<Value, CallError> {
    if let Some(error) = reply.get("error") {
        return Err(CallError::new(
            error
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("internal"),
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Mocky Mail refused the request."),
        ));
    }
    Ok(reply.get("ok").cloned().unwrap_or(Value::Null))
}

impl Channel for LocalChannel {
    async fn call(&mut self, command: &str, args: Value) -> Result<Value, CallError> {
        // A connection the app closed (restarted, or automation changed)
        // is made again once.
        for attempt in 0..2 {
            self.connect().await?;
            match self.exchange(command, &args).await {
                Ok(reply) => {
                    let kind = reply
                        .get("error")
                        .and_then(|e| e.get("kind"))
                        .and_then(Value::as_str);
                    if kind == Some("auth") && attempt == 0 {
                        self.connection = None;
                        continue;
                    }
                    return result_of(&reply);
                }
                Err(_) => self.connection = None,
            }
        }
        Err(CallError::new(
            "connect",
            format!("Mocky Mail stopped answering. Check it is running and {TURN_ON}."),
        ))
    }
}

#[cfg(test)]
pub mod tests {
    //! A stand-in for the app.

    use super::*;

    /// Answers like the app with one account and a few commands.
    pub struct Fake {
        /// Every call made, in order.
        pub calls: Vec<(String, Value)>,
        /// Whether it allows changes (`full`) or only reads.
        pub full: bool,
    }

    impl Fake {
        /// One that allows everything.
        #[must_use]
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Self {
                calls: Vec::new(),
                full: true,
            }
        }
    }

    impl Channel for Fake {
        async fn call(&mut self, command: &str, args: Value) -> Result<Value, CallError> {
            self.calls.push((command.to_owned(), args.clone()));
            match command {
                "__commands" => {
                    let mut commands = vec![json!({
                        "name": "list_accounts", "description": "Every account, in sidebar order.", "writes": false,
                        "inputSchema": { "type": "object", "properties": {}, "required": [] }
                    })];
                    if self.full {
                        commands.push(json!({
                            "name": "send_message", "description": "Queue a message for sending.", "writes": true,
                            "inputSchema": { "type": "object", "properties": { "draft": { "type": "object" } }, "required": ["draft"] }
                        }));
                    }
                    Ok(
                        json!({ "mode": if self.full { "full" } else { "read" }, "commands": commands }),
                    )
                }
                "list_accounts" => Ok(json!([
                    { "id": "a1", "email": "don@mocky.co.ke", "displayName": "Don", "auth": "password", "protocol": "imap" }
                ])),
                "list_folders" => Ok(json!([
                    { "id": "f1", "accountId": "a1", "name": "Inbox", "path": "INBOX", "role": "inbox", "unread": 2, "total": 9 }
                ])),
                "send_message" if self.full => Ok(json!({ "id": "o1" })),
                "send_message" => Err(CallError::new(
                    "invalid",
                    "send_message changes things, and automation is set to read only.",
                )),
                _ => Err(CallError::new(
                    "invalid",
                    format!("There is no command called {command}."),
                )),
            }
        }
    }

    #[test]
    fn answers_become_results_or_errors() {
        assert_eq!(result_of(&json!({ "id": 1, "ok": [1] })), Ok(json!([1])));
        assert_eq!(
            result_of(&json!({ "id": 1, "error": { "kind": "auth", "message": "No." } })),
            Err(CallError::new("auth", "No."))
        );
    }

    #[tokio::test]
    async fn an_app_that_isnt_running_is_explained() {
        let dir = std::env::temp_dir().join(format!("mm-cli-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let token = dir.join("automation-token");
        let mut channel = LocalChannel::from_options(&Options {
            endpoint: Some(if cfg!(windows) {
                paths::pipe_name(&format!("nobody-{}", std::process::id()))
            } else {
                dir.join("none.sock").to_string_lossy().into_owned()
            }),
            token_file: Some(token.clone()),
            json: false,
        });
        let no_secret = channel
            .call("list_accounts", Value::Null)
            .await
            .unwrap_err();
        assert_eq!(no_secret.kind, "auth");
        assert!(
            no_secret.message.contains("Settings › Automation"),
            "{}",
            no_secret.message
        );
        std::fs::write(&token, "abc").unwrap();
        let not_running = channel
            .call("list_accounts", Value::Null)
            .await
            .unwrap_err();
        assert_eq!(not_running.kind, "connect");
        assert!(
            not_running.message.contains("Settings › Automation"),
            "{}",
            not_running.message
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
