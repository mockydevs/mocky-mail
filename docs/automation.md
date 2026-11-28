# Scripts and AI tools: the command line and the MCP server

Mocky Mail on a computer can be used by scripts and AI assistants on the
same computer, through the `mocky-mail` command line and the MCP server
built into it (`mocky-mail mcp`). They talk to the running app; the app does
the work, with the accounts, rules and security settings it already has.

## Turning it on

Open **Settings › Automation** in Mocky Mail and choose:

- **Off** (the default): nothing can reach the app.
- **Read only**: programs can list and read mail, folders, contacts,
  calendars and settings, and search. They can't send, move, delete or
  change anything.
- **Full access**: programs can also send mail (through the Outbox, so Undo
  Send still applies), move, flag, delete, and change contacts, events,
  rules and settings.

Some things always need you at the computer, whatever the setting: file
pickers and save dialogs, signing in with Google or Microsoft in the
browser, opening links and files, printing, installing updates, and making a
new automation secret. A program can't change the automation setting
itself.

Settings › Automation also shows where the command line is, and a
ready-made MCP configuration to copy.

## The command line

The installers put `mocky-mail` next to the app:

| System | Where |
|---|---|
| Windows | `C:\Program Files\Mocky Mail\mocky-mail.exe` (the app's install folder) |
| macOS | `/Applications/Mocky Mail.app/Contents/MacOS/mocky-mail` |
| Linux (`.deb`, `.rpm`) | `/usr/bin/mocky-mail` |

```text
mocky-mail accounts                     the accounts
mocky-mail folders don@example.com      an account's folders (id or address)
mocky-mail list                         the newest mail in every inbox
mocky-mail list --folder <id> --unread --limit 50
mocky-mail search from:ann invoice      search every account
mocky-mail read <message-id>            show a message
mocky-mail send --from don@example.com --to ann@example.com \
    --subject "Site visit" --body "See you at 10." [--cc …] [--bcc …] [--html page.html]
mocky-mail commands                     every command the app allows now
mocky-mail call get_message '{"id":"<message-id>","allowRemote":false}'
mocky-mail mcp                          the MCP server (for AI tools)
```

`--json` prints any result as JSON. `call` runs any command of
[`docs/ipc.md`](ipc.md) with its camelCase arguments; `commands` lists them
with their arguments.

When the app isn't running, or automation is off, the command line says so
("Turn on Settings › Automation in Mocky Mail") and exits with status 1.

## MCP: Claude Desktop and other AI tools

The MCP server speaks the Model Context Protocol (version 2025-06-18) over
standard input and output. Its tools are the commands the app allows in its
current mode, each with a description and a JSON schema for its arguments;
errors from the app come back as tool errors the model can read.

**Claude Desktop**: Settings › Developer › Edit Config, then add (with the
path from Settings › Automation in Mocky Mail):

```json
{
  "mcpServers": {
    "mocky-mail": {
      "command": "C:\\Program Files\\Mocky Mail\\mocky-mail.exe",
      "args": ["mcp"]
    }
  }
}
```

On macOS the command is
`/Applications/Mocky Mail.app/Contents/MacOS/mocky-mail`; on Linux
`/usr/bin/mocky-mail`. Restart Claude Desktop; the Mocky Mail tools appear
while the app is running with automation on.

**Claude Code**: `claude mcp add mocky-mail -- /usr/bin/mocky-mail mcp` (with
your system's path).

**Other MCP clients** (Cursor, VS Code, …): add a stdio server whose command
is the `mocky-mail` program and whose argument is `mcp`.

## Security

- **Local only.** The app listens on a Windows named pipe
  (`\\.\pipe\mocky-mail-<user>`, which refuses connections from other
  computers) or on a Unix socket in its own data folder that only you can
  open (`automation.sock`, mode 0600). Nothing listens on the network.
- **A secret on every request.** Each request carries the 32-byte secret in
  `automation-token` in the app's data folder (`%APPDATA%\ke.co.mocky.mail`,
  `~/Library/Application Support/ke.co.mocky.mail`, or
  `~/.local/share/ke.co.mocky.mail`). Only your user can read it (mode 0600
  on macOS and Linux; your own profile folder on Windows). Anything that can
  read it can use the app as the mode allows, so treat it like a password.
  Making a new secret in Settings › Automation (`reset_automation_token`)
  replaces it; programs holding the old
  one stop working at once.
- **Read only or full access**, as described above, checked by the app for
  every request. Turning automation off closes the channel and every
  connection to it.
- On Windows the pipe is made only when no other program already holds its
  name, so another program can't stand in for the app's own; the command
  line can't check who answers the pipe, so a program already running as
  you could still pretend to be the app before it starts.

The `--endpoint` and `--token-file` options (or `MOCKY_MAIL_ENDPOINT` and
`MOCKY_MAIL_TOKEN_FILE`) point the command line elsewhere, for example at a
development build.

## The protocol

Programs other than `mocky-mail` can use the channel directly: one JSON
object per line, `{"id": 1, "token": "…", "command": "list_accounts", "args":
{}}`, answered by `{"id": 1, "ok": […]}` or `{"id": 1, "error": {"kind",
"message"}}`. See "Automation" in [`docs/ipc.md`](ipc.md).
