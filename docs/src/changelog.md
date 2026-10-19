# Changelog

Every release gets a section in the repository's
[CHANGELOG.md](https://github.com/G36maid/moodle-mcp/blob/main/CHANGELOG.md)
([Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, semantic
versioning). Pushing a `v*` tag builds the prebuilt binaries for every
platform and publishes the crate to crates.io, so a release is one tag
from start to finish.

This page summarizes the highlights of past releases; the changelog in the
repository is the complete record.

## 0.2.0 — 2026-10-19

- per-school SSO drivers: `auth` logs into SSO-federated schools by
  replaying the school's own login chain over plain HTTP — no browser, no
  token paste (verified live against NTUST's IdentityServer portal);
  adding a school is one file and one PR under `src/sso/`
- the escape-hatch pair `call_ws_function` + `list_ws_functions` is now
  opt-in via `MOODLE_ENABLE_WS_FUNCTION=true`; `list_functions` was
  renamed to `list_ws_functions`
- https is enforced end to end: schemeless URLs default to https, plain
  http is rejected (loopback exempt, `MOODLE_ALLOW_HTTP` opts out), and
  redirects from the site never drop to plain http
- safety-relevant env values fail loudly on typos instead of silently
  defaulting — booleans and `MOODLE_TIMEOUT_SECS`
- token acquisition failures name the fix for every `login/token.php`
  error code, and `auth` follows schools that migrated to a new host
- a landing page and this documentation book, published at
  <https://moodle.ntnu.cc>
- security-review hardening: quote-safe `auth` output, an https guard on
  the SSO callback, fully in-memory document conversion (no temp files)

## 0.1.4 — 2026-10-15

- `get_course_overview` includes the content teachers put in section
  summaries and module descriptions (#13)
- section and module availability restrictions and hidden states are
  surfaced — a restricted item is no longer listed as freely available

## 0.1.3 — 2026-10-14

- `download_file` no longer creates missing parent directories; it fails
  with an error naming the missing directory and suggesting the fix (#1)

## 0.1.2 — 2026-10-14

- published on crates.io — `cargo install moodle-mcp`
- `MOODLE_READ_ONLY` and `MOODLE_DISABLE_TOOLS` for cautious setups (#10)
- `MOODLE_TIMEZONE` and timezone detection fixes (#8)
- a dozen fixes across downloads, uploads, notifications, grades, user
  search, and markdown rendering (#2, #3, #4, #5, #6, #7, #9, #12)

## 0.1.1 — 2026-10-13

- interactive `moodle-mcp auth` with masked password prompts; flag
  variants for scripting
- README translations (zh-TW, zh-CN, ja, es, de, fr, pt-BR)

## 0.1.0 — 2026-10-13

- initial release: 22 tools covering courses, assignments, forums, files,
  messaging, notifications, and users, plus a `call_ws_function` escape
  hatch; document conversion built in; verified against Moodle 3.7 and
  designed for 4.x/5.x

## Updating

- **Prebuilt binary** — download the latest archive from the
  [releases page](https://github.com/G36maid/moodle-mcp/releases/latest)
  and replace the binary (same steps as
  [Installation](./installation.md))
- **Cargo** — re-running the install command replaces the old binary:
  `cargo install moodle-mcp`
- **From source** — `git pull` then `cargo build --release`

The server logs its version at startup
(`starting moodle-mcp v0.1.4 for …`); a cargo install can be checked
without starting it via `cargo install --list | grep moodle-mcp`.
