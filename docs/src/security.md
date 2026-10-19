# Security & Authentication

## Get a Token

`moodle-mcp auth` obtains a Moodle web service token without touching
curl. It has two modes:

**Credentials mode (default).** Prompts for site, username, and password
(masked as you type), then POSTs to `/login/token.php` — the same flow the
official Moodle mobile app uses. Flags (`--url`, `--username`, `--password`)
skip the matching prompts, so scripts work non-interactively:

```sh
moodle-mcp auth
moodle-mcp auth --url https://moodle.example.com --username yourname
```

**SSO-federated schools.** auth dispatches by host: a school with a
maintained driver under the project's `src/sso/` gets its own login
portal replayed over plain HTTP with the same credentials (no browser;
the SSO password travels only to the school's portal). Schools without
a driver use the credentials mode — if that answers `invalidlogin`, ask
your Moodle admin for a token (Site administration → Server → Web
services → Manage tokens) or contribute a driver (one file, one PR).

Other flags: `--service` names the web service (default
`moodle_mobile_app`), `--output json` prints machine-readable output, and
`--allow-http` permits a plain-http site URL.

If the token request fails, the site is missing the REST protocol or the
`moodle_mobile_app` service for your account — ask your administrator.

## What the Server Does With the Token

The token travels with every request, so the server treats it as a live
credential:

- **Same-site injection only.** The token is injected into `pluginfile.php`
  URLs; redirects that leave the Moodle host are stopped. Fetching an
  external URL with your token is not possible unless you set
  `MOODLE_ALLOW_EXTERNAL_FETCH=1`.
- **Redaction.** The token never appears in tool output or errors — any
  `token=…` in a rendered error message becomes `token=[REDACTED]`.
- **No stdout leakage.** Logs go to stderr; stdout is the MCP protocol
  stream, so the token never mixes into it.

## Transport Security

Use **HTTPS**. A plain-http `MOODLE_URL` is rejected because the token
(and, on `auth`, the password) would travel in cleartext. Two exemptions:

- Loopback hosts (`localhost`, `127.0.0.1`) are always allowed, for local
  test instances.
- `MOODLE_ALLOW_HTTP=1` (or `auth --allow-http`) explicitly opts out, for
  isolated networks where you accept the risk.

## Write Safety

Write tools act **as the token owner**. Three layers keep that deliberate:

1. The server's instructions and each write tool's description tell the
   agent to confirm with you before calling.
2. Submissions prefer drafts: `submit_assignment` finalizes only when you
   ask; uploads go to your private draft area until then.
3. `MOODLE_READ_ONLY=true` removes the write tools from the tool list
   entirely — see [Configuration](./configuration.md).

Quiz-taking is intentionally not provided.

## Transfer Limits

| Direction         | Limit  | Notes                                                    |
| ----------------- | ------ | -------------------------------------------------------- |
| Inline fetch      | 100 MB | `fetch_file`, capped at 100k characters after conversion |
| Streamed download | 500 MB | `download_file` streams to disk, never buffered whole    |
| Upload            | 100 MB | `upload_file`                                            |

Downloads use a read timeout rather than a total timeout, so large files
finish while stalled connections still die.

## Token Hygiene

- Never commit the token. Keep it in the client's env block, and keep
  token-bearing config files out of version control. This repository
  gitignores every client's project config for exactly that reason:

  - **Claude Code** — `.mcp.json` (matched by the `*mcp.json` rule)
  - **Codex** — `.codex/` (the user-level `~/.codex/config.toml` already
    lives outside any repository)
  - **OpenCode** — `opencode.json` / `opencode.jsonc`
  - **pi** — `.pi/mcp.json`

  In other repositories, prefer each client's user-level (global) config
  for token-bearing servers, or ignore the project config the same way.

- The token carries your full student permissions. Use
  `MOODLE_READ_ONLY=true` when the agent does not need to write.
- To revoke a token, log in to Moodle and remove it on the Security keys
  page (`/user/managetoken.php`).
- Avoid `MOODLE_LOG_LEVEL=trace` on shared machines — it logs
  response-body snippets.

## Testing Schools Responsibly

Verifying an SSO driver means logging into a school's production portal
with a real account — treat the school's systems and reputation as part
of the job:

- **Authorized accounts only.** Verify with an account you own or one
  you are explicitly permitted to test; school infrastructure remains
  the school's property, and its acceptable-use policy applies.
- **A handful of requests is the norm.** A driver verification is a
  single-digit number of HTTP calls. Never scan, brute-force, stress,
  or probe beyond what the driver needs.
- **Weaknesses you notice belong to the school.** An unenforced
  captcha, a broken certificate chain, an open redirect — report it
  through the school's own channel (the computer center) instead of
  this repository. Keep such observations out of code, commit messages,
  docs and issue trackers; private local notes in the gitignored
  `.secrets/` directory are the place. The driver's documentation
  states what works, never what is broken.
