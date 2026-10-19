# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning
follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-10-19

### Added

- auth follows whole-site redirects — schools that migrate hosts
  (moodle.example.edu → moodle3.example.edu) answer the old origin with a
  bare 303, and every later request would land on the homepage instead of
  the page asked for
- token acquisition failures name the fix: every `login/token.php` error
  code now maps to a plain-language hint (web services disabled, missing
  create-token capability, IP-restricted mobile access, SSO-only
  accounts…)
- a landing page and a documentation book, published at
  <https://moodle.ntnu.cc> (mdBook sources in `docs/`, built and deployed
  by CI on every push)
- file tools accept only same-site Moodle URLs — external links belong to the
  agent's own web tooling (`MOODLE_ALLOW_EXTERNAL_FETCH=true` opts out)
- `MOODLE_ENABLE_WS_FUNCTION` opts in to the `call_ws_function` escape hatch
  (hidden by default — a write-capable raw call is a conscious choice;
  `MOODLE_READ_ONLY` hides it regardless)
- `MOODLE_ALLOW_HTTP=true` / `auth --allow-http` allow plain-http sites (local
  test instances on loopback hosts need no flags)
- `MOODLE_UA` overrides the User-Agent on both HTTP clients (default stays
  `moodle-mcp/{version}`)
- `MOODLE_LOG_LEVEL` sets the log level (`error|warn|info|debug|trace`,
  default `info`); `RUST_LOG` remains the advanced escape hatch
- a `download_file` `overwrite` parameter (`false` by default) for
  intentional re-downloads
- a `cargo-audit` CI job failing the build on RustSec advisory matches
- security-review hardening: redirects that originate from the Moodle
  site may never drop to plain http (the token-bearing chain), an SSO
  driver's callback POST gets the same https guard as the password POST,
  the `export` line `auth` prints is quote-safe against hostile tokens,
  and `MOODLE_TIMEOUT_SECS` typos fail at startup instead of silently
  becoming the default
- per-school SSO drivers under `src/sso/` (one file, one PR): auth
  dispatches by host — a school with a maintained driver replays its own
  login chain over plain HTTP (no browser, no token paste) and the token
  is validated before it is shown; every other school uses the
  credentials mode (verified live against NTUST's IdentityServer portal)

### Removed

- the `auth --open` flag: the Security-keys-page flow did not work at any
  surveyed school (the page was blocked by the web server or empty), and
  both verified schools are covered without a browser — NTNU by the
  credentials mode, NTUST by the SSO driver

### Changed

- `list_functions` is renamed to `list_ws_functions` — it pairs with
  `call_ws_function` and says what it lists (web-service functions)
- the whole escape hatch pair (`list_ws_functions`, `call_ws_function`) is
  out of the default tool surface: the server
  now lists 20 tools until `MOODLE_ENABLE_WS_FUNCTION=true` opts in
- boolean env switches (`MOODLE_READ_ONLY`, `MOODLE_ALLOW_HTTP`,
  `MOODLE_ALLOW_EXTERNAL_FETCH`, `MOODLE_ENABLE_WS_FUNCTION`) accept
  `true`/`false` (also `1`/`0`, `yes`/`no`, `on`/`off`, case-insensitive)
  and reject any other value at startup instead of silently falling back —
  a typo'd `MOODLE_READ_ONLY=ture` used to disable the safety switch
- `MOODLE_URL` without a scheme now defaults to `https://`, and plain
  `http://` is rejected unless the host is loopback or the escape hatch is
  set — the token (and on `auth`, the password) would otherwise travel in
  cleartext

### Fixed

- timestamps render in the timezone Moodle itself uses for the user: the
  site's forced timezone, then the user's profile timezone if the site
  shows it, then the site default (the site-level values need Moodle 4.2+).
  The 0.1.2 detection read a user preference, but Moodle keeps the timezone
  in the user record, so without `MOODLE_TIMEZONE` stock sites fell back to
  UTC (#8, fix by @thc1006)
- `fetch_file` no longer writes a predictable temp file for document
  conversion (a symlink/TOCTOU hazard on multi-user machines) — documents
  are converted fully in memory, with one less disk round-trip

## [0.1.4] - 2026-10-15

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
