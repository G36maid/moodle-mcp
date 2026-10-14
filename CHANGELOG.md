# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning
follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- `get_course_overview` missed the content teachers put in section
  summaries or module descriptions instead of modules; that content is
  included now (#13)
- `get_course_overview` surfaces section and module availability
  restrictions and hidden states — a restricted item is no longer listed
  as if freely available

## [0.1.3] - 2026-10-14

### Changed

- `download_file` no longer creates missing `save_to` parent directories
  (introduced in 0.1.2). Following the explicit-action convention of the
  official filesystem server, the download fails with an error naming the
  missing directory and suggesting to create it first or save to an existing
  one — no directory structure is created on the agent's behalf (#1)

## [0.1.2] - 2026-10-14

### Fixed

- `download_file` creates missing `save_to` parent directories instead of
  failing with `No such file or directory`, and rejects a blank `save_to` up
  front with a message that suggests the fix (#1)
- `get_notifications` defaults to returning both read and unread
  notifications — the previous default silently returned only already-read
  ones (#2)
- `get_assignment` prefers `gradefordisplay` for the grade and hides Moodle's
  `-1.00000` no-grade sentinel, so gradebook grades display instead of a
  bogus -1 (#3)
- `search_users` sends the parameters `core_message_message_search_users`
  actually requires and merges `contacts`/`noncontacts` into one list; blank
  queries are rejected client-side instead of hanging until timeout (#4)
- `download_file` and `fetch_file` rewrite `pluginfile.php` URLs into
  token-authenticated ones and detect login-page or error-JSON responses, so
  an HTML login page is no longer saved as the file (#5)
- `get_discussion_posts` reads attachment URLs from the `url` field,
  restoring attachment links on Moodle 4.5 and later (#6)
- parallel `fetch_file` calls can no longer return another file's content —
  the Moodle fetch ticket is now derived from an atomic counter (#7)
- `upload_file` surfaces `upload.php` per-file error records instead of
  reporting success when the upload was rejected (#9)
- online-text submissions and teacher feedback comments render as markdown
  instead of raw `<p>`/`<span>`/`<br>` tags (#12)

### Added

- `MOODLE_READ_ONLY=1` hides all write/upload tools for cautious setups;
  `MOODLE_DISABLE_TOOLS` hides additional tools by name, warning on unknown
  names at startup (#10)
- `MOODLE_TIMEZONE` overrides the timezone used to render timestamps; the
  default auto-detects the Moodle user's timezone preference and falls back
  to UTC (#8)
- `whoami` reports the active timezone, whether read-only mode is on, and the
  server's own version
- `limit: 0` on `get_notifications`, `get_conversations` and `search_users`
  now means the default (20) instead of a single item
- published on crates.io — `cargo install moodle-mcp`

### Changed

- tool descriptions restructured as concise prose with MCP capability
  annotations (read-only / non-destructive hints)
- all localized READMEs document every environment variable, including the
  two new ones

## [0.1.1] - 2026-10-13

### Added

- interactive `auth` subcommand (`moodle-mcp auth`) with masked password
  prompts; `--url`/`--username`/`--password` flags skip the prompts
- README translations for zh-TW, zh-CN, ja, es, de, fr and pt-BR

## [0.1.0] - 2026-10-13

Initial release: 22 MCP tools covering courses, assignments, forums, files,
messaging, notifications and users, plus a `call_ws_function` +
`list_functions` escape hatch reaching every web-service function the token
can call. Verified against Moodle 3.7 and designed for 4.x/5.x.

[0.1.3]: https://github.com/G36maid/moodle-mcp/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/G36maid/moodle-mcp/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/G36maid/moodle-mcp/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/G36maid/moodle-mcp/releases/tag/v0.1.0
