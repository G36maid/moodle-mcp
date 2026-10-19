# Development

moodle-mcp is a single Rust crate (library plus a thin binary) built with
Rust 1.88 or later. The whole development loop runs offline: the test suite
fakes the Moodle REST endpoint, so you never need a live Moodle site to
hack on it.

Clone, build, and run the checks that CI runs:

```sh
git clone https://github.com/G36maid/moodle-mcp
cd moodle-mcp
cargo build
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Bird's-eye view of the codebase

The server is small enough to read in one sitting — five modules carry
everything:

- `src/main.rs` is a thin binary: no arguments serves MCP over stdio, and
  everything else is delegated. The `auth` subcommand lives in
  `src/auth.rs` and fetches a web-service token with interactive prompts;
  `--url`/`--username`/`--password` flags skip them for scripting.
- `src/client.rs` is the only place that talks to Moodle. Every call goes
  through one endpoint, `POST {base}/webservice/rest/server.php`, with
  parameters flattened to PHP bracket notation (`courseids[0]=2`). Two
  `reqwest` clients exist by design: API calls carry a total timeout,
  while file downloads stream without one (a read timeout kills stalled
  connections). Redirects are host-locked because the token gets
  injected into `pluginfile.php` URLs — don't loosen that.
- `src/tools/` holds one module per domain (`assignments.rs`,
  `forums.rs`, `files.rs`, …). rmcp requires every `#[tool]` method to
  live in the single `#[tool_router]` impl block in `mod.rs`: declare
  the method there, put the business logic in the domain module.
- `src/output.rs` implements the dual-channel result policy: every tool
  returns structured JSON in `structuredContent`, and document tools add
  a markdown rendering in `text`. Nulls are stripped recursively because
  Moodle 3.7 omits many fields. Only markdown is truncated
  (`MAX_DOC_CHARS`), never the JSON.
- `tests/` mirrors the suite CI runs: `tests/client_integration.rs`
  mocks the REST endpoint with wiremock, and `tests/mcp_protocol.rs`
  runs the real server over an in-memory duplex transport — protocol
  behavior is tested without a socket or a Moodle.

## Rules the codebase enforces

These come from the root
[AGENTS.md](https://github.com/G36maid/moodle-mcp/blob/main/AGENTS.md)
and are review-gated, not optional:

- **Never write to stdout.** stdout is the MCP JSON-RPC stream; logs go
  to stderr via `tracing` (`RUST_LOG` for advanced filtering).
- **Write tools must say so.** Their descriptions carry "WRITE operation
  — confirm with the user before calling", and drafts are preferred over
  finalizing submissions. Quiz-taking is intentionally not provided —
  don't add it.
- **Annotations have a policy.** Read tools set `read_only_hint = true`;
  additive write tools set `destructive_hint = false`; overwriting
  surfaces (`submit_assignment`, `call_ws_function`) keep the spec
  default. Preserve these when adding tools.
- **Tool descriptions are structured prose** — purpose first, workflow
  and boundary sentences only where a real confusion pair exists, then
  upstream id chaining. No section labels, no examples.
- **Comments explain why, never what** — version quirks, trade-offs,
  upstream Moodle source links. No step-by-step stories.

## Release process

1. Add a section to `CHANGELOG.md` — the release job extracts the
   tagged version's section verbatim for the GitHub release notes and
   fails if it is missing.
2. Bump `version` in `Cargo.toml` to match the tag; a check in the
   release job enforces this.
3. Push the `v*` tag: the workflow builds multi-platform binaries,
   creates the GitHub release from the changelog section, and publishes
   the crate to crates.io via trusted publishing.

## What's Next

- [About](./about.md) — the project's goals and design principles
- [Tools](./tools.md) — what the tools do and how ids flow
- [Changelog](./changelog.md) — what each release changed
- [GitHub issues](https://github.com/G36maid/moodle-mcp/issues) — bug
  reports and feature ideas
