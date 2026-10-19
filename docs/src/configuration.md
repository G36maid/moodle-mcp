# Configuration

moodle-mcp is configured entirely through environment variables set on the
server process — there is no config file. Every MCP client lets you set
environment variables in its server definition.

## Required Variables

| Variable       | Description                                                        |
| -------------- | ------------------------------------------------------------------ |
| `MOODLE_URL`   | Site base URL, no trailing slash                                   |
| `MOODLE_TOKEN` | Web service token (see [Security & Authentication](./security.md)) |

## All Variables

| Variable                      | Required | Default                | Description                                                                                                                  |
| ----------------------------- | :------: | ---------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `MOODLE_URL`                  |    ✔    | —                      | Site base URL, no trailing slash                                                                                             |
| `MOODLE_TOKEN`                |    ✔    | —                      | Web service token                                                                                                            |
| `MOODLE_TIMEOUT_SECS`         |    —     | `60`                   | Per-request timeout; also bounds the startup timezone lookup                                                                 |
| `MOODLE_TIMEZONE`             |    —     | auto / `UTC`           | IANA name (`Asia/Taipei`) for rendering timestamps. The default detects the zone Moodle uses for you and falls back to `UTC` |
| `MOODLE_ALLOW_EXTERNAL_FETCH` |    —     | off                    | Lets `fetch_file`/`download_file` fetch URLs outside the configured site (off: only same-site Moodle file URLs)              |
| `MOODLE_ALLOW_HTTP`           |    —     | off                    | `true` allows a plain-http `MOODLE_URL` — otherwise http is rejected to protect the token. Loopback hosts are always exempt  |
| `MOODLE_READ_ONLY`            |    —     | off                    | `true` hides all write/upload tools from the tool list — read-only LLM access                                                |
| `MOODLE_ENABLE_WS_FUNCTION`   |    —     | off                    | `true` exposes the `call_ws_function` escape hatch — any web-service function your token allows, including writes            |
| `MOODLE_DISABLE_TOOLS`        |    —     | —                      | Comma-separated tool names to hide on top (e.g. `send_message,upload_file`); unknown names warn at startup                   |
| `MOODLE_LOG_LEVEL`            |    —     | `info`                 | Log verbosity: `error`/`warn`/`info`/`debug`/`trace`. `trace` logs response-body snippets — avoid on shared machines         |
| `MOODLE_UA`                   |    —     | `moodle-mcp/{version}` | User-Agent override for institutional-filtering edge cases                                                                   |
| `RUST_LOG`                    |    —     | `info`                 | Advanced log filter override (logs go to stderr; stdout is the MCP stream)                                                   |

Boolean switches accept `true`/`false` — also `1`/`0`, `yes`/`no`,
`on`/`off`, case-insensitive. Any other value fails at startup with the
correct spelling, so a typo can never silently disable a safety switch.

## Read-Only Mode

Set `MOODLE_READ_ONLY=true` to hide every write and upload tool
(`create_forum_discussion`, `reply_to_discussion`, `submit_assignment`,
`send_message`, `upload_file`, `call_ws_function`). The agent can still read
everything your token can see, but nothing you say can make it post.

To hide individual tools while keeping the rest writable, list them in
`MOODLE_DISABLE_TOOLS`:

```sh
MOODLE_DISABLE_TOOLS=send_message,upload_file
```

> **Tip:** For a shared or classroom machine, `MOODLE_READ_ONLY=true` plus
> `MOODLE_LOG_LEVEL=warn` keeps the agent useful and the logs quiet.

## The Escape Hatch

`call_ws_function` can invoke any of the ~350 web-service functions your
token allows — including writes — so the escape hatch pair is not on the
default tool list. Opt in when the 20 curated tools are not enough:

```sh
MOODLE_ENABLE_WS_FUNCTION=true
```

Every call still passes through your client's confirmation for destructive
tools, and `MOODLE_READ_ONLY` hides `call_ws_function` again regardless of
this setting. The opt-in covers `list_ws_functions` too — a discovery tool
whose calls cannot be executed would just dead-end the agent — but being
read-only, it survives when `MOODLE_READ_ONLY` and this setting are both
on.

## Timeouts and Timezones

`MOODLE_TIMEOUT_SECS` bounds each API request and the startup timezone
lookup. A value that is not a number of seconds (or is `0`) fails at
startup — the same rule the boolean switches follow, so a typo can never
silently shorten every timeout to nothing. File downloads stream to disk
and use a read timeout instead of a total timeout, so large files finish
while stalled connections still die.

By default timestamps render in the timezone Moodle itself uses for you:
the site's forced timezone, then your profile timezone if the site shows
it, then the site default (the site-level values need Moodle 4.2+), and
`UTC` otherwise. Set `MOODLE_TIMEZONE` to an IANA name to override
detection.

## Example: Read-Only Server for Claude Code

```json
{
  "mcpServers": {
    "moodle": {
      "command": "/path/to/moodle-mcp",
      "env": {
        "MOODLE_URL": "https://moodle.example.com",
        "MOODLE_TOKEN": "your-token",
        "MOODLE_READ_ONLY": "true"
      }
    }
  }
}
```

## Example: Codex

`~/.codex/config.toml` — one file shared by the Codex CLI, the ChatGPT
desktop app, and the IDE extension — or `.codex/config.toml` for a single
trusted project:

```toml
[mcp_servers.moodle]
command = "/path/to/moodle-mcp"
env = { "MOODLE_URL" = "https://moodle.example.com", "MOODLE_TOKEN" = "your-token" }
```

## Example: OpenCode

`opencode.json` (project) or `~/.config/opencode/opencode.json` (global):

```json
{
  "mcp": {
    "moodle": {
      "type": "local",
      "command": ["/path/to/moodle-mcp"],
      "environment": {
        "MOODLE_URL": "https://moodle.example.com",
        "MOODLE_TOKEN": "your-token"
      }
    }
  }
}
```

## Example: pi

`.pi/mcp.json` (project) or `~/.pi/agent/mcp.json` (global):

```json
{
  "mcpServers": {
    "moodle": {
      "command": "/path/to/moodle-mcp",
      "env": {
        "MOODLE_URL": "https://moodle.example.com",
        "MOODLE_TOKEN": "your-token"
      }
    }
  }
}
```

Or register it from the terminal:

```sh
pi mcp add moodle \
  --env MOODLE_URL=https://moodle.example.com \
  --env MOODLE_TOKEN=your-token \
  -- /path/to/moodle-mcp
```

The command writes the user-level config by default; add `-l` to write the
project file instead. Run `/reload` inside pi after editing either file
outside a session.

> **Note:** `.pi/mcp.json` is gitignored by convention because it holds a
> real token — never commit tokens.
