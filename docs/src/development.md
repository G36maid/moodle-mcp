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

The server is small enough to read in one sitting — six modules carry
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
- `src/sso/` holds the per-school SSO drivers — one self-contained file
  per school whose login cannot go through `login/token.php` (SSO
  portals: the password belongs to the identity provider). See the next
  section.
- `tests/` mirrors the suite CI runs: `tests/client_integration.rs`
  mocks the REST endpoint with wiremock, and `tests/mcp_protocol.rs`
  runs the real server over an in-memory duplex transport — protocol
  behavior is tested without a socket or a Moodle. Live version checks
  (a new Moodle major, say) are deliberate manual acts, not CI: spin up
  [`moodlehq/moodle-docker`](https://github.com/moodlehq/moodle-docker)
  with a shallow clone of the tag, then know the three non-obvious
  switches — the REST protocol config key is `webserviceprotocols`
  (no underscore), a fresh install leaves the mobile service row
  `enabled=0` until the admin UI (or SQL on `m_external_services`)
  flips it, and `admin/cli/create_user.php` no longer exists (use SQL
  or the admin account). 5.2 was verified this way without code
  changes.

## Adding a school's SSO driver

`src/sso/` exists because some schools federate login through a portal
(OIDC/SAML): the password belongs to the identity provider, so the
credentials mode answers `invalidlogin` no matter what. Each school gets
one self-contained file — `src/sso/ntust.rs` is the reference — holding
the metadata (host, Moodle version, identity provider, the date the flow
was verified live), the host match, and the whole replayed chain.
`src/sso/mod.rs` carries the shared launch-flow helpers (`launch.php`
URL building, `moodlemobile://` payload decoding, token validation) and
the host-matched dispatch; `auth` consults it before any request moves,
so schools without a driver simply use the credentials mode.

Adding a school is one file plus one arm:

1. Copy `src/sso/ntust.rs` as the template and keep the metadata block
   honest — a flow verified live on a date, or not verified at all.
2. Adjust the replayed chain to the school's portal. Every step's error
   message names what broke, and the password never crosses plain http
   (the loopback carve-out matches `MOODLE_ALLOW_HTTP`).
3. Add the `matches(host)` arm to `school_for` in `src/sso/mod.rs`.
4. Cover the whole chain with an offline wiremock test (see the one in
   `ntust.rs`), then verify once against the live portal before opening
   the PR — responsibly: authorized accounts, a handful of requests,
   and weaknesses you notice go to the school, not to this repository
   (see [Security & Authentication](security.md#testing-schools-responsibly)).

Never probe or guess: `school_for` is an exact host match, so an
unknown school fails with guidance instead of leaking credentials to
the wrong form. If a second school turns out to share the same flow
shape, extract the shared steps into a family module then — one member
is not a family.

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
   creates the GitHub release from the changelog section, publishes the
   crate to crates.io via trusted publishing, and publishes the server
   entry to the official MCP registry (OIDC — the registry verifies the
   `mcp-name` marker in the crates.io README, and `server.json`'s version
   must match the tag).

## What's Next

- [About](./about.md) — the project's goals and design principles
- [Tools](./tools.md) — what the tools do and how ids flow
- [Changelog](./changelog.md) — what each release changed
- [GitHub issues](https://github.com/G36maid/moodle-mcp/issues) — bug
  reports and feature ideas
