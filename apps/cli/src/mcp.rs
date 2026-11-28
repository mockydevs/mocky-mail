//! `mocky-mail mcp`: a Model Context Protocol server on standard input and
//! output (JSON-RPC 2.0, one message per line, protocol version
//! 2025-06-18). Its tools are the commands the app allows in its current
//! automation mode, with their argument schemas; calling one runs it in
//! the app over the automation channel. Errors from the app come back as
//! tool errors (`isError`), so the model can read them and try again.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};

use crate::channel::Channel;

/// The protocol version spoken.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// What the model is told about the server.
const INSTRUCTIONS: &str = "Mocky Mail is the user's email, calendar and contacts app, running on this computer. \
Each tool is one of the app's commands; arguments use the camelCase names in each tool's schema. \
Start with list_accounts and list_folders, then list_messages (query.filter \"all\", query.offset 0, query.limit up to 200; \
query.search takes the app's search words, e.g. from:ann subject:invoice) and get_message (allowRemote false). \
send_message puts a message in the Outbox, where the user can still undo it for a few seconds. \
What the tools can do depends on Settings › Automation in the app: Read only, or Full access.";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[allow(clippy::needless_pass_by_value, reason = "callers build it in place")]
fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// The tools, from the app's command list.
async fn tools<C: Channel>(channel: &mut C) -> Result<Vec<Value>, String> {
    let listed = channel
        .call("__commands", Value::Null)
        .await
        .map_err(|e| e.message)?;
    Ok(listed
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|command| {
            let writes = command.get("writes").and_then(Value::as_bool) != Some(false);
            json!({
                "name": command.get("name"),
                "title": command.get("name"),
                "description": command.get("description"),
                "inputSchema": command.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                "annotations": {
                    "readOnlyHint": !writes,
                    "destructiveHint": writes,
                    "openWorldHint": false,
                },
            })
        })
        .collect())
}

/// A tool's result: the value as JSON text (and as structured content).
fn tool_result(value: &Value) -> Value {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let structured = if value.is_object() {
        value.clone()
    } else {
        json!({ "result": value })
    };
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": structured,
        "isError": false,
    })
}

fn tool_error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// Answer one message; `None` for notifications (and answers sent to us).
pub async fn handle<C: Channel>(channel: &mut C, message: &Value) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error(
            &Value::Null,
            INVALID_REQUEST,
            "Expected a JSON-RPC message.",
        ));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        // A response to something we sent (we send nothing): ignore.
        return id
            .filter(|_| !object.contains_key("result") && !object.contains_key("error"))
            .map(|id| error(&id, INVALID_REQUEST, "Expected a method."));
    };
    // Notifications get no answer.
    let id = id?;
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => result(
            &id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "mocky-mail", "title": "Mocky Mail", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }),
        ),
        "ping" => result(&id, json!({})),
        "tools/list" => match tools(channel).await {
            Ok(tools) => result(&id, json!({ "tools": tools })),
            Err(message) => error(&id, INTERNAL_ERROR, &message),
        },
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(&id, INVALID_PARAMS, "tools/call needs a tool name."));
            };
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            if !(arguments.is_object() || arguments.is_null()) {
                return Some(error(
                    &id,
                    INVALID_PARAMS,
                    "Tool arguments must be an object.",
                ));
            }
            if name.starts_with("__") {
                return Some(result(
                    &id,
                    tool_error(&format!("There is no tool called {name}.")),
                ));
            }
            match channel.call(name, arguments).await {
                Ok(value) => result(&id, tool_result(&value)),
                Err(failure) => result(&id, tool_error(&failure.message)),
            }
        }
        "resources/list" => result(&id, json!({ "resources": [] })),
        "prompts/list" => result(&id, json!({ "prompts": [] })),
        _ => error(&id, METHOD_NOT_FOUND, &format!("Unknown method {method}.")),
    })
}

/// Answer one line of input.
pub async fn handle_line<C: Channel>(channel: &mut C, line: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(line) {
        Ok(message) => handle(channel, &message).await,
        Err(_) => Some(error(
            &Value::Null,
            PARSE_ERROR,
            "Couldn't read that as JSON.",
        )),
    }
}

/// Serve until the input ends.
pub async fn serve<C: Channel, R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut channel: C,
    input: R,
    mut output: W,
) {
    let mut lines = BufReader::new(input).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = handle_line(&mut channel, &line).await {
            let mut bytes = serde_json::to_vec(&answer).unwrap_or_default();
            bytes.push(b'\n');
            if output.write_all(&bytes).await.is_err() || output.flush().await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::tests::Fake;

    async fn ask(fake: &mut Fake, message: Value) -> Value {
        handle(fake, &message).await.unwrap()
    }

    #[tokio::test]
    async fn initialize_says_what_it_speaks() {
        let mut fake = Fake::new();
        let answer = ask(
            &mut fake,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" }
            } }),
        )
        .await;
        assert_eq!(answer["id"], 1);
        assert_eq!(answer["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(answer["result"]["capabilities"]["tools"].is_object());
        assert_eq!(answer["result"]["serverInfo"]["name"], "mocky-mail");
        // The initialized notification gets no answer.
        assert!(
            handle(
                &mut fake,
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
            )
            .await
            .is_none()
        );
        assert_eq!(
            ask(
                &mut fake,
                json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" })
            )
            .await["result"],
            json!({})
        );
    }

    #[tokio::test]
    async fn tools_are_the_commands_the_app_allows() {
        let mut fake = Fake::new();
        let answer = ask(
            &mut fake,
            json!({ "jsonrpc": "2.0", "id": "t", "method": "tools/list" }),
        )
        .await;
        let tools = answer["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "list_accounts");
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
        assert_eq!(tools[1]["name"], "send_message");
        assert_eq!(tools[1]["inputSchema"]["required"], json!(["draft"]));
        assert_eq!(tools[1]["annotations"]["readOnlyHint"], false);
        fake.full = false;
        let answer = ask(
            &mut fake,
            json!({ "jsonrpc": "2.0", "id": "t", "method": "tools/list" }),
        )
        .await;
        assert_eq!(answer["result"]["tools"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn calls_run_in_the_app_and_errors_are_tool_errors() {
        let mut fake = Fake::new();
        let answer = ask(
            &mut fake,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_accounts", "arguments": {} } }),
        )
        .await;
        assert_eq!(answer["result"]["isError"], false);
        assert_eq!(
            answer["result"]["structuredContent"]["result"][0]["id"],
            "a1"
        );
        assert!(
            answer["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("don@mocky.co.ke")
        );
        assert_eq!(
            fake.calls.last().map(|c| c.0.as_str()),
            Some("list_accounts")
        );

        fake.full = false;
        let refused = ask(
            &mut fake,
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "send_message", "arguments": { "draft": {} } } }),
        )
        .await;
        assert_eq!(refused["result"]["isError"], true);
        assert!(
            refused["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("read only")
        );

        let unknown = ask(
            &mut fake,
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "launch_rockets" } }),
        )
        .await;
        assert_eq!(unknown["result"]["isError"], true);
        // The channel's own requests aren't tools.
        let internal = ask(
            &mut fake,
            json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": { "name": "__subscribe" } }),
        )
        .await;
        assert_eq!(internal["result"]["isError"], true);
    }

    #[tokio::test]
    async fn protocol_mistakes_are_json_rpc_errors() {
        let mut fake = Fake::new();
        assert_eq!(
            handle_line(&mut fake, "{").await.unwrap()["error"]["code"],
            PARSE_ERROR
        );
        assert_eq!(
            ask(
                &mut fake,
                json!({ "jsonrpc": "2.0", "id": 1, "method": "sampling/whatever" })
            )
            .await["error"]["code"],
            METHOD_NOT_FOUND
        );
        assert_eq!(
            ask(
                &mut fake,
                json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {} })
            )
            .await["error"]["code"],
            INVALID_PARAMS
        );
        assert_eq!(
            ask(&mut fake, json!([1])).await["error"]["code"],
            INVALID_REQUEST
        );
    }

    #[tokio::test]
    async fn serves_lines_over_streams() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n"
        );
        let mut output = Vec::new();
        serve(Fake::new(), input.as_bytes(), &mut output).await;
        let lines: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[1]["id"], 2);
        assert!(lines[1]["result"]["tools"].is_array());
    }
}
