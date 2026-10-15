# AGENTS.md

Rust MCP server (rmcp) exposing Moodle Web Services to LLM clients over stdio. Single crate, lib + thin bin. Targets Moodle 3.7 → 5.x — response-shape differences between versions must keep working.

## Commands

CI (`.github/workflows/ci.yml`) runs in this order — match it locally:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings   # -D warnings enforced in CI
cargo test
```

- Rust 1.88+ (edition 2024).
- All tests are offline: no live Moodle needed. Integration tests mock the REST endpoint with wiremock (`tests/client_integration.rs`); protocol tests run the real server over an in-memory duplex transport (`tests/mcp_protocol.rs`).
- Releases: pushing a `v*` tag builds multi-platform binaries (`.github/workflows/release.yml`).

## Architecture

- `src/main.rs` — thin bin: no args = serve MCP over stdio; `auth` subcommand (src/auth.rs) fetches a web-service token (interactive prompts; `--url`/`--username`/`--password` flags skip them).
- `src/client.rs` (`MoodleClient`) — all Moodle calls go through one endpoint: `POST {base}/webservice/rest/server.php`. JSON params are flattened to PHP bracket notation (`courseids[0]=2`). Two reqwest clients: API calls have a total timeout; file downloads have none (streaming) but a read timeout. Redirect policy is host-locked for site-originated redirects because the token is injected into `pluginfile.php` URLs — don't loosen it.
- `src/tools/` — rmcp requires all `#[tool]` methods to live in the single `#[tool_router]` impl block in `mod.rs`. When adding a tool: declare the `#[tool]` method in `mod.rs`, put business logic in the per-domain module (`assignments.rs`, `forums.rs`, `files.rs`, …).
- `src/output.rs` — dual-channel result policy: every tool returns structured JSON in `structuredContent`; document tools (`get_course_overview`, `get_assignment`) additionally put markdown in `text`. Nulls are stripped recursively (Moodle 3.7 omits many fields). Only markdown text is truncated (`MAX_DOC_CHARS`), never the JSON.

## Conventions

- **Never write to stdout** — it is the MCP JSON-RPC stream. Logs go to stderr via tracing (`RUST_LOG`).
- Write tools carry `WRITE operation — confirm with the user before calling` in their descriptions; preserve this when editing. Drafts are preferred over finalizing submissions.
- Tool annotations: read tools set `annotations(read_only_hint = true)`; additive write tools (create/reply/send/upload/download) set `destructive_hint = false`. Overwriting surfaces (`submit_assignment`, `call_ws_function`) intentionally keep the spec default (`destructive = true`). Preserve these when adding tools.
- Tool descriptions use structured prose (2026-10 style): sentence 1 = purpose + what it returns; then workflow/boundary sentences only where a real confusion pair or gate exists ("for X use Y"); then upstream id chaining ("…_id comes from …"). No section labels, no examples, no implementation mechanism.
- Quiz-taking is intentionally not provided — don't add it.
- Error messages are plain-language and suggest the fix; keep that style.
- Comments explain **why**, never narrate **what** the adjacent code does — version quirks, trade-offs, upstream Moodle source links. No step-by-step story comments or `// --- section ---` chapter markers; if a marker carries real why-content, demote it to one why-comment instead.
- `*mcp.json` (e.g. `.pi/mcp.json`) is gitignored because it holds a real `MOODLE_TOKEN` — never commit tokens.

## Env (runtime, not tests)

`MOODLE_URL`, `MOODLE_TOKEN` required; `MOODLE_TIMEOUT_SECS` (default 60; also bounds the startup timezone lookup), `MOODLE_TIMEZONE` (IANA name; default: the zone Moodle uses for the user, i.e. forced site zone, then profile timezone, then site default, else UTC; the site-level values need Moodle 4.2+), `MOODLE_READ_ONLY` (`1` hides write/upload/escape-hatch tools), `MOODLE_DISABLE_TOOLS` (comma-separated extra tools to hide), `MOODLE_ALLOW_HTTP` (`1` allows plain-http sites; loopback hosts are always exempt), `MOODLE_UA` (User-Agent override; default `moodle-mcp/{version}`), `MOODLE_ALLOW_EXTERNAL_FETCH` (`1` lets file tools fetch non-site URLs), `MOODLE_LOG_LEVEL` (`error|warn|info|debug|trace`; default info), `RUST_LOG` (advanced EnvFilter override).
