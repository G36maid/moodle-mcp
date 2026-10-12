# moodle-mcp

**A Moodle LMS → MCP (Model Context Protocol) bridge, written in Rust.**

Expose your Moodle site's Web Services to any MCP client (pi, Claude Code,
Codex, Claude Desktop, …) as a set of typed, student-first tools. Ships as a
single static binary over the `stdio` transport.

Built on the official Rust MCP SDK [`rmcp`](https://crates.io/crates/rmcp)
3.5 and [`reqwest`](https://crates.io/crates/reqwest). Design rationale and
NTNU live-validation notes: [`docs/design-decisions.md`](docs/design-decisions.md)
and [`docs/design-review.md`](docs/design-review.md).

## Tools (22)

### Entry & identity
| Tool | Purpose |
|---|---|
| `whoami()` | token owner identity + connected site/release |
| `list_courses(search?)` | enrolled courses; optional name filter |

### Course map (the core)
| Tool | Purpose |
|---|---|
| `get_course_overview(course_id)` | **one markdown document** describing the whole course: assignments, quizzes, forums, graded items and every section with file URLs — 5 WS calls merged server-side |

### Assignments
| Tool | Purpose |
|---|---|
| `get_assignment(assignment_id)` | **markdown document**: times, draft/attempt state, full description, attachments (URLs), your submission (text/files), grade and teacher feedback |
| `submit_assignment(assignment_id, text?, item_id?, finalize?)` | the single write: online text and/or draft files, optionally finalizing for grading |

### Forums (top-down navigation)
| Tool | Purpose |
|---|---|
| `get_forum_discussions(forum_id)` | thread list with **preview** of each opening post |
| `get_discussion_posts(discussion_id)` | whole thread: posts (converted to markdown) + attachments |
| `create_forum_discussion(forum_id, subject, message)` | start a thread |
| `reply_to_discussion(post_id, subject, message)` | reply to a post (replying to the opening post = replying to the thread) |

### Files
| Tool | Purpose |
|---|---|
| `fetch_file(url)` | PDF/DOCX/PPTX/XLSX/ODT/RTF/EPUB/CSV → **markdown** (anydoc), 100k-char cap |
| `download_file(url, save_to)` | raw bytes to a local path (metadata only) |
| `upload_file(path, item_id?)` | local file → draft area; returns `item_id` |

### Timeline, grades, messaging, quizzes
| Tool | Purpose |
|---|---|
| `get_schedule(time_from?, time_to?, course_ids?)` | merged timeline: due dates + quiz closes + calendar events (defaults to the next 30 days) |
| `get_grades(course_id?)` | omit `course_id`: cross-course summary; with it: every grade item of the course |
| `get_notifications(read?, limit?)` | recent notifications |
| `get_conversations(limit?)` | conversations with member ids (for `send_message`) |
| `send_message(user_id, text)` | direct message |
| `get_quiz_attempts(quiz_id)` | attempt records (read-only; quiz-taking is intentionally not provided) |
| `get_user(field, value)` | profile lookup by id/username/email/idnumber |
| `search_users(search?, limit?)` | user search (site permission permitting) |

### Escape hatch
| Tool | Purpose |
|---|---|
| `call_ws_function(wsfunction, params?)` | call any WS function your token allows (params flattened to PHP bracket notation) |
| `list_functions(query?)` | discover callable functions by keyword |

✍️ marks write operations. Tool errors carry actionable English hints so the
agent can self-correct (`isError: true`), and unknown function names get
"Did you mean: …?" suggestions.

## Output contract

- Every tool returns **dual-channel** results: `structuredContent` (JSON, for
  codemode scripts) plus `text` (compact JSON for direct model reading).
- `get_course_overview` and `get_assignment` return **markdown documents** in
  `text` (they ARE documents) with machine-readable `structuredContent`.
- Nulls are stripped; timestamps are ISO-8601; HTML fields are converted to
  markdown (tables survive); documents larger than 100k chars are truncated
  with a pointer to `download_file`.

## Moodle version compatibility

Validated live against **Moodle 3.7** (NTNU) and designed for 4.x/5.x.
Version differences are absorbed dynamically from `site_info.functions`:

| Feature | Modern (4.x/5.x) | Moodle 3.7 fallback |
|---|---|---|
| Forum discussions | `mod_forum_get_discussions` | `mod_forum_get_forum_discussions` (no `sortby`, wrapped response) |
| Discussion pagination | `page`/`perpage` on posts | rejected on 3.7 → auto-retry without |
| Notifications | `core_message_get_notifications` | `core_message_get_messages?type=notifications` |
| User search | `core_user_get_users` | `core_message_message_search_users` |
| Calendar `timefrom`/`timeto` | passed to API | never relied on — filtering is client-side |

## Setup

### 1. Get a token

Use the built-in CLI (same binary):

```bash
moodle-mcp token --url https://moodle3.ntnu.edu.tw --username U
# Password: (hidden prompt)
# → export MOODLE_URL='https://moodle3.ntnu.edu.tw' MOODLE_TOKEN='…'
```

This calls `/login/token.php` (service `moodle_mobile_app`, verified on NTNU).
For SSO accounts without a password:

```bash
moodle-mcp token --url https://<site> --open
# opens the Security keys page (/user/managetoken.php) — copy the token by hand
```

Site-side requirements: Web services enabled + REST protocol + the
`moodle_mobile_app` service allowed for the user (standard Moodle setup; see
`docs/design-decisions.md` §Setup notes in research docs).

### 2. Run

```bash
cargo build --release
# binary: target/release/moodle-mcp (~15 MB, no runtime deps)
```

Environment: `MOODLE_URL`, `MOODLE_TOKEN`, optional `MOODLE_TIMEOUT_SECS`
(default 60), `RUST_LOG` (logs go to **stderr** — stdout is the MCP stream).

### 3. Register with your MCP client

pi (project-local):

```bash
mkdir -p .pi && cat > .pi/mcp.json <<'EOF'
{
  "mcpServers": {
    "moodle": {
      "command": "/path/to/moodle-mcp",
      "env": { "MOODLE_URL": "https://…", "MOODLE_TOKEN": "…" }
    }
  }
}
EOF
```

Claude Code:

```bash
claude mcp add moodle --env MOODLE_URL=https://… --env MOODLE_TOKEN=… -- /path/to/moodle-mcp
```

## Development

```bash
cargo test                 # 25 tests: unit + wiremock integration + MCP protocol
cargo clippy --all-targets
cargo fmt -- --check
cargo build --release
```

Architecture, per-tool behavior decisions, the Moodle privacy model observed
on NTNU, and the version-compatibility matrix live in
[`docs/design-decisions.md`](docs/design-decisions.md). ID flow reference:

| ID | Produced by | Consumed by |
|---|---|---|
| course_id | list_courses / overview | overview / grades / schedule / assignments |
| forum_id | get_course_overview | get_forum_discussions / create_forum_discussion |
| discussion_id | get_forum_discussions | get_discussion_posts |
| post_id | get_discussion_posts | reply_to_discussion |
| assignment_id | get_course_overview / get_assignment | get_assignment / submit_assignment |
| item_id (draft area) | upload_file | submit_assignment |
| user_id | conversations members / notifications / get_user | send_message / get_user |

## Security notes

- Always use **HTTPS** — the token is sent on every request.
- Prefer a dedicated account with a least-privilege external service.
- Moodle core has no WS rate limiting; put throttling at your reverse proxy
  for multi-tenant deployments.
- Write tools act **as the token owner** — the server instructions tell the
  agent to confirm intent before submitting or sending.

## License

MIT
