# Changelog

Every release gets a section in the repository's
[CHANGELOG.md](https://github.com/G36maid/moodle-mcp/blob/main/CHANGELOG.md)
([Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, semantic
versioning). Pushing a `v*` tag builds the prebuilt binaries for every
platform and publishes the crate to crates.io, so a release is one tag
from start to finish.

This page summarizes the highlights of past releases; the changelog in the
repository is the complete record.

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
