# AGENTS.md

Rust MCP server (rmcp) exposing Moodle Web Services to LLM clients over stdio. Single crate, lib + thin bin. Targets Moodle 3.7 → 5.x — response-shape differences between versions must keep working.

Full documentation lives in `docs/` (mdBook): architecture tour, tests,
and the release process in
[`docs/src/development.md`](docs/src/development.md); every environment
variable in [`docs/src/configuration.md`](docs/src/configuration.md);
docs governance in [`docs/AGENTS.md`](docs/AGENTS.md).

## Commands

CI (`.github/workflows/ci.yml`) runs in this order — match it locally:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings   # -D warnings enforced in CI
cargo test
```

Rust 1.88+ (edition 2024). All tests are offline — no live Moodle needed.
Hooks: `prek install` wires commit/push gates (fmt, clippy at push,
changelog structure, hygiene) — keep them enabled when committing.
Releases: pushing a `v*` tag builds multi-platform binaries and publishes
to crates.io; a `CHANGELOG.md` section for the version is required.

## Rules that gate every change

- **Never write to stdout** — it is the MCP JSON-RPC stream. Logs go to stderr via tracing (`RUST_LOG`).
- rmcp requires all `#[tool]` methods to live in the single `#[tool_router]` impl block in `src/tools/mod.rs`; business logic goes in the per-domain module (`assignments.rs`, `forums.rs`, `files.rs`, …).
- Write tools carry `WRITE operation — confirm with the user before calling` in their descriptions; drafts are preferred over finalizing submissions. Quiz-taking is intentionally not provided — don't add it.
- Annotations: read tools set `read_only_hint = true`; additive writes set `destructive_hint = false`; `submit_assignment` / `call_ws_function` keep the spec default.
- Tool descriptions: structured prose — purpose first, then only real confusion pairs, then id chaining. No section labels, no examples.
- Error messages are plain-language and suggest the fix.
- Comments explain **why**, never narrate what — no story comments or section markers.
- `*mcp.json` (e.g. `.pi/mcp.json`) is gitignored because it holds a real `MOODLE_TOKEN` — never commit tokens. Local credentials and private notes live in `.secrets/` (gitignored).

Runtime configuration: [`docs/src/configuration.md`](docs/src/configuration.md).
