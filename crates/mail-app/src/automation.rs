//! The automation channel: how the `mocky-mail` command line and its MCP
//! server reach the running app on a computer (Settings › Automation).
//!
//! The app listens on a local endpoint only: a named pipe on Windows
//! (`\\.\pipe\mocky-mail-<user>`, remote clients refused) and a Unix
//! socket in the app's data folder elsewhere (`automation.sock`, readable
//! and writable by the user only). Nothing listens on the network.
//!
//! The protocol is newline-delimited JSON. Each request carries the secret
//! from `<data dir>/automation-token` (32 random bytes, hex; readable by
//! the user only), made when automation is first turned on:
//!
//! ```text
//! {"id": 1, "token": "…", "command": "list_accounts", "args": {}}
//! {"id": 1, "ok": [...]}
//! {"id": 2, "error": {"kind": "invalid", "message": "…"}}
//! ```
//!
//! `{"command": "__commands"}` lists the commands this mode allows (with
//! their argument schemas); `{"command": "__subscribe"}` streams the
//! app's `mail://` events on the connection as `{"event": …, "payload": …}`
//! lines. In `read` mode only commands that change nothing run; `full`
//! runs every command except the ones that need the person at the
//! computer (pickers, dialogs, windows, browser sign-in). Phones never
//! listen.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::error::{AppError, AppResult};
use crate::registry::{self, CommandInfo};
use crate::state::lock;

/// The name of the secret's file in the data folder.
pub const TOKEN_FILE: &str = "automation-token";
/// The name of the Unix socket in the data folder.
pub const SOCKET_FILE: &str = "automation.sock";
/// The longest request line read, bytes (a message with attachments).
pub const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;
/// The command line's program name.
#[cfg(windows)]
const CLI_FILE: &str = "mocky-mail.exe";
#[cfg(not(windows))]
const CLI_FILE: &str = "mocky-mail";

/// Settings.automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nothing listens.
    Off,
    /// Commands that change nothing.
    Read,
    /// Every command a program may run.
    Full,
}

impl Mode {
    /// From the setting's text; anything else is off.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text {
            "read" => Self::Read,
            "full" => Self::Full,
            _ => Self::Off,
        }
    }

    /// The setting's text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Read => "read",
            Self::Full => "full",
        }
    }

    /// Whether a program may run this command in this mode.
    #[must_use]
    pub const fn allows(self, command: &CommandInfo) -> bool {
        match self {
            Self::Off => false,
            Self::Read => command.remote && !command.writes,
            Self::Full => command.remote,
        }
    }
}

/// `AutomationInfo` in `docs/ipc.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationInfo {
    /// Settings.automation.
    pub mode: &'static str,
    /// The local endpoint tools connect to; `None` when off (and on phones).
    pub endpoint: Option<String>,
    /// The file holding the secret; `None` when off.
    pub token_file: Option<String>,
    /// The command-line program installed with the app, if found.
    pub cli_path: Option<String>,
    /// An MCP client configuration to paste (computers).
    pub mcp_config: Option<String>,
}

/// The channel while it runs.
struct Running {
    task: tokio::task::JoinHandle<()>,
    endpoint: String,
    /// Bumped whenever the channel stops or restarts, so connections made
    /// before (with an older mode or secret) stop answering.
    generation: Arc<AtomicU64>,
}

/// The channel's state in [`AppState`](crate::AppState).
#[derive(Default)]
pub struct AutomationState {
    running: Mutex<Option<Running>>,
}

/// What a connection checks before each request.
#[derive(Clone)]
pub struct Session {
    /// The mode the channel was started in.
    pub mode: Mode,
    /// The secret requests must carry.
    pub token: Arc<str>,
    /// The channel's generation now, and the one this session belongs to.
    pub generation: (Arc<AtomicU64>, u64),
}

impl Session {
    /// A session that never goes stale (tests, tools).
    #[must_use]
    pub fn fixed(mode: Mode, token: &str) -> Self {
        Self {
            mode,
            token: Arc::from(token),
            generation: (Arc::new(AtomicU64::new(0)), 0),
        }
    }

    fn stale(&self) -> bool {
        self.generation.0.load(Ordering::SeqCst) != self.generation.1
    }
}

/// The current mode, from the settings.
fn mode(ctx: &Ctx) -> Mode {
    crate::settings::load(&ctx.state).map_or(Mode::Off, |s| Mode::parse(&s.automation))
}

/// Whether this host can listen at all: computers only.
fn supported(ctx: &Ctx) -> bool {
    cfg!(any(
        windows,
        all(unix, not(any(target_os = "android", target_os = "ios")))
    )) && ctx.host.desktop()
}

/// The endpoint for a data folder: the user's named pipe on Windows, the
/// socket in the data folder elsewhere.
#[must_use]
pub fn endpoint_for(data_dir: &Path) -> String {
    if cfg!(windows) {
        let user = std::env::var("USERNAME").unwrap_or_default();
        pipe_name(&user)
    } else {
        data_dir.join(SOCKET_FILE).to_string_lossy().into_owned()
    }
}

/// `\\.\pipe\mocky-mail-<user>`, the user name kept to characters a pipe
/// name takes.
#[must_use]
pub fn pipe_name(user: &str) -> String {
    let clean: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(64)
        .collect();
    let clean = if clean.is_empty() {
        "user".into()
    } else {
        clean
    };
    format!(r"\\.\pipe\mocky-mail-{}", clean.to_ascii_lowercase())
}

/// The secret's file.
#[must_use]
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

/// 32 random bytes, as hex.
fn new_token() -> AppResult<String> {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| AppError::internal("Couldn't make a secret for automation."))?;
    Ok(bytes.iter().fold(String::with_capacity(64), |mut hex, b| {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
        hex
    }))
}

/// Write the secret so only this user can read it: a new file (mode 0600
/// on Unix; on Windows the data folder is the user's own), moved into
/// place.
fn write_token(path: &Path, token: &str) -> AppResult<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("new");
    let _ = std::fs::remove_file(&temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// The secret, made if there is none yet.
///
/// # Errors
/// When the file can't be read or written.
pub fn ensure_token(data_dir: &Path) -> AppResult<String> {
    let path = token_path(data_dir);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let text = text.trim();
        if text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(text.to_owned());
        }
    }
    let token = new_token()?;
    write_token(&path, &token)?;
    Ok(token)
}

/// Compare secrets in time that doesn't depend on where they differ.
fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The command line next to this program, if it was installed with it.
fn cli_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(CLI_FILE);
    path.is_file().then_some(path)
}

/// The MCP client configuration for a command-line program.
#[must_use]
pub fn mcp_config(cli: &str) -> String {
    let config = json!({ "mcpServers": { "mocky-mail": { "command": cli, "args": ["mcp"] } } });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

fn info(ctx: &Ctx) -> AutomationInfo {
    let mode = mode(ctx);
    if !supported(ctx) {
        return AutomationInfo {
            mode: mode.as_str(),
            endpoint: None,
            token_file: None,
            cli_path: None,
            mcp_config: None,
        };
    }
    let running = lock(&ctx.state.automation.running)
        .as_ref()
        .map(|r| r.endpoint.clone());
    let data_dir = ctx.host.data_dir().ok();
    let cli = cli_path().map(|p| p.to_string_lossy().into_owned());
    AutomationInfo {
        mode: mode.as_str(),
        endpoint: if mode == Mode::Off { None } else { running },
        token_file: data_dir
            .filter(|_| mode != Mode::Off)
            .map(|dir| token_path(&dir).to_string_lossy().into_owned()),
        mcp_config: Some(mcp_config(cli.as_deref().unwrap_or("mocky-mail"))),
        cli_path: cli,
    }
}

/// `get_automation_info`.
pub fn get_automation_info(ctx: &Ctx) -> AppResult<AutomationInfo> {
    Ok(info(ctx))
}

/// `reset_automation_token`: a new secret; tools holding the old one are
/// refused from now on.
pub fn reset_automation_token(ctx: &Ctx) -> AppResult<AutomationInfo> {
    if !supported(ctx) {
        return Err(AppError::invalid(
            "Automation is available on computers only.",
        ));
    }
    let data_dir = ctx.host.data_dir()?;
    write_token(&token_path(&data_dir), &new_token()?)?;
    stop(ctx);
    apply(ctx);
    Ok(info(ctx))
}

/// Stop the channel, if it runs. Connections made before stop answering.
fn stop(ctx: &Ctx) {
    if let Some(running) = lock(&ctx.state.automation.running).take() {
        running.generation.fetch_add(1, Ordering::SeqCst);
        running.task.abort();
        #[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
        let _ = std::fs::remove_file(&running.endpoint);
    }
}

/// Start, restart or stop the channel to match the setting. Called at
/// start-up and whenever the settings are saved.
pub fn apply(ctx: &Ctx) {
    let wanted = mode(ctx);
    if !supported(ctx) {
        return;
    }
    stop(ctx);
    if wanted == Mode::Off {
        return;
    }
    if let Err(error) = start(ctx, wanted) {
        eprintln!("automation channel didn't start: {}", error.message);
    }
}

#[cfg(any(windows, all(unix, not(any(target_os = "android", target_os = "ios")))))]
fn start(ctx: &Ctx, mode: Mode) -> AppResult<()> {
    let data_dir = ctx.host.data_dir()?;
    let token: Arc<str> = Arc::from(ensure_token(&data_dir)?);
    let endpoint = endpoint_for(&data_dir);
    let generation = Arc::new(AtomicU64::new(0));
    let session = Session {
        mode,
        token,
        generation: (Arc::clone(&generation), 0),
    };
    let listener = transport::Listener::bind(&endpoint)?;
    let task = ctx.spawn(transport::serve(ctx.clone(), listener, session));
    *lock(&ctx.state.automation.running) = Some(Running {
        task,
        endpoint,
        generation,
    });
    Ok(())
}

#[cfg(not(any(windows, all(unix, not(any(target_os = "android", target_os = "ios"))))))]
fn start(_ctx: &Ctx, _mode: Mode) -> AppResult<()> {
    Ok(())
}

/// What a request line asks for.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Send this line back.
    Reply(Value),
    /// Send this line back, then stream events on the connection.
    Subscribe(Value),
    /// Send this line back and hang up (the channel was turned off or its
    /// mode or secret changed).
    Close(Value),
}

fn error_line(id: &Value, kind: &str, message: &str) -> Value {
    json!({ "id": id, "error": { "kind": kind, "message": message } })
}

/// The commands a mode allows, for `__commands`.
#[must_use]
pub fn allowed_commands(mode: Mode) -> Vec<Value> {
    registry::commands()
        .iter()
        .filter(|c| mode.allows(c))
        .map(|c| {
            json!({
                "name": c.name,
                "description": c.description,
                "writes": c.writes,
                "args": c.args,
                "inputSchema": c.input_schema(),
            })
        })
        .collect()
}

/// Answer one request line.
pub async fn answer(ctx: &Ctx, session: &Session, line: &str) -> Answer {
    let request: Value = match serde_json::from_str(line) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => {
            return Answer::Reply(error_line(
                &Value::Null,
                "invalid",
                "Each line must be one JSON object.",
            ));
        }
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if session.stale() {
        return Answer::Close(error_line(
            &id,
            "auth",
            "Automation was turned off or its secret changed. Connect again.",
        ));
    }
    let token = request.get("token").and_then(Value::as_str).unwrap_or("");
    if !same_secret(token, &session.token) {
        return Answer::Reply(error_line(
            &id,
            "auth",
            "The automation secret is missing or wrong. Read it from the automation-token file in Mocky Mail's data folder.",
        ));
    }
    let Some(name) = request.get("command").and_then(Value::as_str) else {
        return Answer::Reply(error_line(&id, "invalid", "Say which command to run."));
    };
    match name {
        "__commands" => {
            return Answer::Reply(
                json!({ "id": id, "ok": { "mode": session.mode.as_str(), "commands": allowed_commands(session.mode) } }),
            );
        }
        "__subscribe" => {
            return Answer::Subscribe(json!({ "id": id, "ok": { "subscribed": true } }));
        }
        _ => {}
    }
    let Some(command) = registry::command(name) else {
        return Answer::Reply(error_line(
            &id,
            "invalid",
            &format!("There is no command called {name}."),
        ));
    };
    if !command.remote {
        return Answer::Reply(error_line(
            &id,
            "invalid",
            &format!("{name} needs Mocky Mail's own window and can't be run by other programs."),
        ));
    }
    if !session.mode.allows(command) {
        return Answer::Reply(error_line(
            &id,
            "invalid",
            &format!(
                "{name} changes things, and automation is set to read only. Choose Full access in Settings › Automation to allow it."
            ),
        ));
    }
    let mut args = request.get("args").cloned().unwrap_or(Value::Null);
    if name == "set_settings" {
        // A program can't change its own access.
        if let Some(settings) = args.get_mut("settings").and_then(Value::as_object_mut) {
            settings.insert("automation".into(), json!(session.mode.as_str()));
        }
    }
    match registry::dispatch(ctx, name, args).await {
        Ok(value) => Answer::Reply(json!({ "id": id, "ok": value })),
        Err(error) => Answer::Reply(json!({ "id": id, "error": error })),
    }
}

/// An event as a line for subscribers.
#[must_use]
pub fn event_line(name: &str, payload: &Value) -> Value {
    json!({ "event": name, "payload": payload })
}

#[cfg(any(windows, all(unix, not(any(target_os = "android", target_os = "ios")))))]
mod transport {
    //! The local endpoint and the connections on it.

    use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};

    use super::{Answer, MAX_LINE_BYTES, Session, event_line};
    use crate::Ctx;
    use crate::error::{AppError, AppResult};

    /// Read one line of at most [`MAX_LINE_BYTES`]; `None` at the end.
    /// Safe to cancel: what was read so far stays in `buffer`.
    async fn read_line<R: AsyncRead + Unpin>(
        reader: &mut BufReader<R>,
        buffer: &mut Vec<u8>,
    ) -> std::io::Result<Option<String>> {
        loop {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                let rest = std::mem::take(buffer);
                return Ok((!rest.is_empty()).then(|| String::from_utf8_lossy(&rest).into()));
            }
            let (chunk, done) = match available.iter().position(|b| *b == b'\n') {
                Some(at) => (available.get(..at).unwrap_or_default(), Some(at)),
                None => (available, None),
            };
            if buffer.len().saturating_add(chunk.len()) > MAX_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "request line too long",
                ));
            }
            buffer.extend_from_slice(chunk);
            let used = done.map_or(chunk.len(), |at| at.saturating_add(1));
            reader.consume(used);
            if done.is_some() {
                let line = std::mem::take(buffer);
                return Ok(Some(String::from_utf8_lossy(&line).into()));
            }
        }
    }

    async fn write_line<W: AsyncWrite + Unpin>(
        writer: &mut W,
        value: &serde_json::Value,
    ) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(value).unwrap_or_default();
        line.push(b'\n');
        writer.write_all(&line).await?;
        writer.flush().await
    }

    /// Serve one connection: requests in, answers (and, once subscribed,
    /// events) out.
    pub async fn connection<S: AsyncRead + AsyncWrite + Unpin>(
        ctx: Ctx,
        stream: S,
        session: Session,
    ) {
        let (read, mut write) = tokio::io::split(stream);
        let mut reader = BufReader::new(read);
        let mut pending = Vec::new();
        let mut events: Option<tokio::sync::broadcast::Receiver<(String, serde_json::Value)>> =
            None;
        loop {
            let next = if let Some(receiver) = events.as_mut() {
                tokio::select! {
                    line = read_line(&mut reader, &mut pending) => Ok(line),
                    event = receiver.recv() => Err(event),
                }
            } else {
                Ok(read_line(&mut reader, &mut pending).await)
            };
            match next {
                Ok(Ok(Some(line))) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match super::answer(&ctx, &session, &line).await {
                        Answer::Reply(value) => {
                            if write_line(&mut write, &value).await.is_err() {
                                return;
                            }
                        }
                        Answer::Subscribe(value) => {
                            events.get_or_insert_with(|| ctx.state.events.subscribe());
                            if write_line(&mut write, &value).await.is_err() {
                                return;
                            }
                        }
                        Answer::Close(value) => {
                            let _ = write_line(&mut write, &value).await;
                            return;
                        }
                    }
                }
                Ok(Ok(None) | Err(_)) => return,
                Err(Ok((name, payload))) => {
                    if session.stale()
                        || write_line(&mut write, &event_line(&name, &payload))
                            .await
                            .is_err()
                    {
                        return;
                    }
                }
                // Fell behind: the missed events are gone; carry on.
                Err(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                Err(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                    events = None;
                }
            }
        }
    }

    #[cfg(unix)]
    pub struct Listener(tokio::net::UnixListener, std::path::PathBuf);

    #[cfg(unix)]
    impl Listener {
        /// Listen on a socket readable and writable by this user only. A
        /// socket left by a copy that stopped is replaced; one another
        /// running copy answers on is not.
        pub fn bind(endpoint: &str) -> AppResult<Self> {
            use std::os::unix::fs::PermissionsExt as _;
            let path = std::path::PathBuf::from(endpoint);
            if path.exists() {
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    return Err(AppError::invalid(
                        "Another copy of Mocky Mail is already answering automation requests.",
                    ));
                }
                std::fs::remove_file(&path)?;
            }
            let listener = tokio::net::UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self(listener, path))
        }
    }

    #[cfg(unix)]
    pub async fn serve(ctx: Ctx, listener: Listener, session: Session) {
        let Listener(listener, _path) = listener;
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    ctx.spawn(connection(ctx.clone(), stream, session.clone()));
                }
                Err(error) => {
                    eprintln!("automation channel: {error}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    #[cfg(windows)]
    pub struct Listener {
        name: String,
        first: tokio::net::windows::named_pipe::NamedPipeServer,
    }

    #[cfg(windows)]
    fn pipe(
        name: &str,
        first: bool,
    ) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
        tokio::net::windows::named_pipe::ServerOptions::new()
            // Fails if anyone else already made a pipe of this name.
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create(name)
    }

    #[cfg(windows)]
    impl Listener {
        /// Make the pipe. Refused when another program holds the name.
        pub fn bind(endpoint: &str) -> AppResult<Self> {
            let first = pipe(endpoint, true).map_err(|error| {
                AppError::invalid(format!(
                    "Another program is already using Mocky Mail's automation pipe ({error})."
                ))
            })?;
            Ok(Self {
                name: endpoint.to_owned(),
                first,
            })
        }
    }

    #[cfg(windows)]
    pub async fn serve(ctx: Ctx, listener: Listener, session: Session) {
        let Listener { name, first } = listener;
        let mut server = first;
        loop {
            if let Err(error) = server.connect().await {
                eprintln!("automation channel: {error}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            } else {
                let next = match pipe(&name, false) {
                    Ok(next) => next,
                    Err(error) => {
                        eprintln!("automation channel stopped: {error}");
                        return;
                    }
                };
                let connected = std::mem::replace(&mut server, next);
                ctx.spawn(connection(ctx.clone(), connected, session.clone()));
                continue;
            }
            server = match pipe(&name, false) {
                Ok(next) => next,
                Err(error) => {
                    eprintln!("automation channel stopped: {error}");
                    return;
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Ctx {
        let state = crate::AppState::new(
            mail_store::Store::open_in_memory().unwrap_or_else(|_| unreachable!()),
            Arc::new(mail_sync::secrets::MemoryStore::default()),
            std::env::temp_dir(),
        );
        Ctx::new(
            Arc::new(state),
            Arc::new(crate::host::NoHost),
            tokio::runtime::Handle::current(),
        )
    }

    const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    async fn ask(ctx: &Ctx, session: &Session, line: Value) -> Value {
        match answer(ctx, session, &line.to_string()).await {
            Answer::Reply(value) | Answer::Subscribe(value) | Answer::Close(value) => value,
        }
    }

    #[tokio::test]
    async fn requests_need_the_secret() {
        let ctx = context();
        let session = Session::fixed(Mode::Full, TOKEN);
        let missing = ask(
            &ctx,
            &session,
            json!({ "id": 1, "command": "list_accounts" }),
        )
        .await;
        assert_eq!(missing["id"], 1);
        assert_eq!(missing["error"]["kind"], "auth");
        let wrong = ask(
            &ctx,
            &session,
            json!({ "id": 2, "token": "00", "command": "list_accounts" }),
        )
        .await;
        assert_eq!(wrong["error"]["kind"], "auth");
        let right = ask(
            &ctx,
            &session,
            json!({ "id": "a", "token": TOKEN, "command": "list_accounts" }),
        )
        .await;
        assert_eq!(right, json!({ "id": "a", "ok": [] }));
    }

    #[tokio::test]
    async fn read_mode_runs_only_reads() {
        let ctx = context();
        let session = Session::fixed(Mode::Read, TOKEN);
        let read = ask(
            &ctx,
            &session,
            json!({ "id": 1, "token": TOKEN, "command": "list_categories" }),
        )
        .await;
        assert!(read.get("ok").is_some(), "{read}");
        let write = ask(
            &ctx,
            &session,
            json!({ "id": 2, "token": TOKEN, "command": "delete_forever", "args": { "ids": ["x"] } }),
        )
        .await;
        assert_eq!(write["error"]["kind"], "invalid");
        assert!(
            write["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("read only"))
        );
        let listed = ask(
            &ctx,
            &session,
            json!({ "id": 3, "token": TOKEN, "command": "__commands" }),
        )
        .await;
        let names: Vec<&str> = listed["ok"]["commands"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c["name"].as_str()).collect())
            .unwrap_or_default();
        assert!(names.contains(&"list_messages"));
        assert!(!names.contains(&"send_message"));
        assert!(!names.contains(&"pick_attachments"));
        assert_eq!(listed["ok"]["mode"], "read");
    }

    #[tokio::test]
    async fn full_mode_runs_writes_but_never_the_windows_own_commands() {
        let ctx = context();
        let session = Session::fixed(Mode::Full, TOKEN);
        let saved = ask(
            &ctx,
            &session,
            json!({ "id": 1, "token": TOKEN, "command": "save_category", "args": { "category": { "name": "Clients", "color": "#2450C8" } } }),
        )
        .await;
        assert!(saved.get("ok").is_some(), "{saved}");
        for local in [
            "pick_attachments",
            "open_external",
            "add_oauth_account",
            "install_update",
            "reset_automation_token",
        ] {
            let refused = ask(
                &ctx,
                &session,
                json!({ "id": 2, "token": TOKEN, "command": local }),
            )
            .await;
            assert_eq!(refused["error"]["kind"], "invalid", "{local}");
        }
        let listed = ask(
            &ctx,
            &session,
            json!({ "id": 3, "token": TOKEN, "command": "__commands" }),
        )
        .await;
        let commands = listed["ok"]["commands"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(commands.iter().any(|c| c["name"] == "send_message"));
        assert!(!commands.iter().any(|c| c["name"] == "save_attachment"));
        let get_message = commands.iter().find(|c| c["name"] == "get_message");
        assert_eq!(
            get_message.map(|c| c["inputSchema"]["required"].clone()),
            Some(json!(["id", "allowRemote"]))
        );
    }

    #[tokio::test]
    async fn a_program_cant_change_its_own_access() {
        let ctx = context();
        let session = Session::fixed(Mode::Full, TOKEN);
        let saved = ask(
            &ctx,
            &session,
            json!({ "id": 1, "token": TOKEN, "command": "set_settings", "args": { "settings": { "theme": "dark", "automation": "off" } } }),
        )
        .await;
        assert_eq!(saved["ok"]["theme"], "dark");
        assert_eq!(saved["ok"]["automation"], "full");
    }

    #[tokio::test]
    async fn unknown_commands_and_bad_lines_are_errors() {
        let ctx = context();
        let session = Session::fixed(Mode::Full, TOKEN);
        let unknown = ask(
            &ctx,
            &session,
            json!({ "id": 9, "token": TOKEN, "command": "launch_rockets" }),
        )
        .await;
        assert_eq!(unknown["id"], 9);
        assert_eq!(unknown["error"]["kind"], "invalid");
        assert!(matches!(
            answer(&ctx, &session, "not json").await,
            Answer::Reply(v) if v["error"]["kind"] == "invalid"
        ));
        assert!(matches!(
            answer(&ctx, &session, "[1,2]").await,
            Answer::Reply(v) if v["error"]["kind"] == "invalid"
        ));
        let subscribed = answer(
            &ctx,
            &session,
            &json!({ "id": 1, "token": TOKEN, "command": "__subscribe" }).to_string(),
        )
        .await;
        assert!(matches!(subscribed, Answer::Subscribe(_)));
    }

    #[tokio::test]
    async fn a_stale_session_hangs_up() {
        let ctx = context();
        let generation = Arc::new(AtomicU64::new(0));
        let session = Session {
            mode: Mode::Full,
            token: Arc::from(TOKEN),
            generation: (Arc::clone(&generation), 0),
        };
        generation.fetch_add(1, Ordering::SeqCst);
        let closed = answer(
            &ctx,
            &session,
            &json!({ "id": 1, "token": TOKEN, "command": "list_accounts" }).to_string(),
        )
        .await;
        assert!(matches!(closed, Answer::Close(v) if v["error"]["kind"] == "auth"));
    }

    #[test]
    fn modes_and_names() {
        assert_eq!(Mode::parse("read"), Mode::Read);
        assert_eq!(Mode::parse("full"), Mode::Full);
        assert_eq!(Mode::parse("anything"), Mode::Off);
        assert_eq!(pipe_name("Don Juan"), r"\\.\pipe\mocky-mail-donjuan");
        assert_eq!(pipe_name(r"..\evil"), r"\\.\pipe\mocky-mail-..evil");
        assert_eq!(pipe_name(""), r"\\.\pipe\mocky-mail-user");
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("", ""));
        let config: Value =
            serde_json::from_str(&mcp_config("/usr/bin/mocky-mail")).unwrap_or_default();
        assert_eq!(
            config["mcpServers"]["mocky-mail"]["command"],
            "/usr/bin/mocky-mail"
        );
        assert_eq!(config["mcpServers"]["mocky-mail"]["args"], json!(["mcp"]));
    }

    #[test]
    fn the_secret_is_made_once_and_kept_private() {
        let dir = std::env::temp_dir().join(format!("mm-automation-{}", crate::state::now()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = ensure_token(&dir).unwrap_or_default();
        assert_eq!(first.len(), 64);
        assert_eq!(ensure_token(&dir).ok(), Some(first.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(token_path(&dir))
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or_default();
            assert_eq!(mode, 0o600);
        }
        write_token(&token_path(&dir), &new_token().unwrap_or_default()).unwrap_or(());
        assert_ne!(ensure_token(&dir).ok(), Some(first));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real endpoint: a request over the pipe (Windows) or socket.
    #[cfg(any(windows, all(unix, not(any(target_os = "android", target_os = "ios")))))]
    #[tokio::test]
    async fn answers_over_the_local_endpoint() {
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
        let ctx = context();
        let session = Session::fixed(Mode::Read, TOKEN);
        let dir = std::env::temp_dir().join(format!("mm-ep-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let endpoint = if cfg!(windows) {
            pipe_name(&format!(
                "test-{}-{}",
                std::process::id(),
                crate::state::now()
            ))
        } else {
            dir.join(SOCKET_FILE).to_string_lossy().into_owned()
        };
        let listener = transport::Listener::bind(&endpoint).unwrap_or_else(|_| unreachable!());
        let server = tokio::spawn(transport::serve(ctx.clone(), listener, session));
        #[cfg(windows)]
        let stream = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(&endpoint)
            .unwrap_or_else(|_| unreachable!());
        #[cfg(unix)]
        let stream = tokio::net::UnixStream::connect(&endpoint)
            .await
            .unwrap_or_else(|_| unreachable!());
        let (read, mut write) = tokio::io::split(stream);
        let request = json!({ "id": 7, "token": TOKEN, "command": "list_accounts" });
        write
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap_or(());
        let mut line = String::new();
        BufReader::new(read).read_line(&mut line).await.unwrap_or(0);
        let reply: Value = serde_json::from_str(&line).unwrap_or_default();
        assert_eq!(reply, json!({ "id": 7, "ok": [] }));
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
