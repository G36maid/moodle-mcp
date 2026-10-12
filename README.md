# moodle-mcp

**A Moodle LMS → MCP (Model Context Protocol) bridge, written in Rust.**

Expose your Moodle site's Web Services to any MCP client (Claude Code, Codex,
Claude Desktop, …) as a set of typed, student-first tools. Ships as a single
static binary over the `stdio` transport.

Built on the official Rust MCP SDK
[`rmcp`](https://crates.io/crates/rmcp) 3.5 and
[`reqwest`](https://crates.io/crates/reqwest). Moodle-side research and
ecosystem survey behind the design decisions: see
[`docs/research.md`](docs/research.md).

## Tools (27)

| Category | Tools |
|---|---|
| Site | `site_info` — identity/connection check + list of functions your token may call |
| Courses | `list_courses` · `get_course_contents` · `search_courses` |
| Assignments | `get_assignments` · `get_assignment_status` · `submit_assignment_text` · `submit_assignment_files` · `submit_assignment_for_grading` |
| Grades | `get_grades` |
| Calendar | `get_calendar_events` · `get_upcoming_deadlines` (facade: assignments due within N days) |
| Forums | `get_forums` · `get_forum_discussions` · `get_discussion_posts` · `create_forum_discussion` ✍️ · `reply_to_discussion` ✍️ |
| Messaging | `get_notifications` · `get_conversations` · `send_message` ✍️ |
| Users | `search_users` · `get_user` |
| Quizzes | `get_quizzes` · `get_quiz_attempts` |
| Files | `download_file` (auto token injection) · `upload_file` ✍️ |
| Escape hatch | `call_ws_function` — call *any* function enabled for your token, with free-form JSON params (validated against `site_info.functions`) |

✍️ = write operation. Output is compact, LLM-oriented JSON (HTML stripped,
dates ISO-8601, oversized responses capped). Moodle API failures surface as
tool-level errors (`isError: true`) with actionable hints so the model can
self-correct.

## Setup

### 1. Moodle side (one-time, admin)

1. **Site administration → Advanced features** → enable *Web services*.
2. **Site administration → Server → Web services → Manage protocols** → enable *REST*.
3. Use the built-in **`moodle_mobile_app`** service (fastest path), or create a
   *custom external service* with only the functions you need (recommended for
   least privilege), e.g.:
   `core_webservice_get_site_info`, `core_enrol_get_users_courses`,
   `core_course_get_contents`, `core_course_search_courses`,
   `mod_assign_get_assignments`, `mod_assign_get_submission_status`,
   `mod_assign_save_submission`, `mod_assign_submit_for_grading`,
   `gradereport_user_get_grade_items`, `core_calendar_get_calendar_events`,
   `mod_forum_get_forums_by_courses`, `mod_forum_get_discussions`,
   `mod_forum_get_discussion_posts`, `mod_forum_add_discussion`,
   `mod_forum_add_discussion_post`, `core_message_get_notifications`,
   `core_message_get_conversations`, `core_message_send_instant_messages`,
   `core_user_get_users`, `core_user_get_users_by_field`,
   `mod_quiz_get_quizzes_by_courses`, `mod_quiz_get_user_attempts`.
4. Authorise the user to the service (if *Authorised users only*), and tick
   *can download/upload files* for file tools.

### 2. Get a token

- As a user: `POST https://<site>/login/token.php?username=…&password=…&service=moodle_mobile_app`
- Or admin: **Manage tokens → Add** (supports IP restriction & expiry).

### 3. Run

```bash
# build
cargo build --release
# binary: target/release/moodle-mcp  (~9 MB, no runtime deps)
```

Environment:

| Var | Required | Meaning |
|---|---|---|
| `MOODLE_URL` | ✔ | e.g. `https://moodle.example.com` (no trailing slash) |
| `MOODLE_TOKEN` | ✔ | web service token |
| `MOODLE_TIMEOUT_SECS` | – | request timeout (default `60`) |
| `RUST_LOG` | – | tracing filter (default `info`); logs go to **stderr** |

### 4. Register with your MCP client

Claude Code:

```bash
claude mcp add moodle --env MOODLE_URL=https://moodle.example.com --env MOODLE_TOKEN=<token> -- /path/to/moodle-mcp
```

Generic `mcpServers` JSON (Claude Desktop, Codex, …):

```json
{
  "mcpServers": {
    "moodle": {
      "command": "/path/to/moodle-mcp",
      "env": {
        "MOODLE_URL": "https://moodle.example.com",
        "MOODLE_TOKEN": "<token>"
      }
    }
  }
}
```

## Development

```bash
cargo test            # 18 tests: unit + wiremock integration + MCP protocol smoke
cargo clippy --all-targets
cargo build --release
```

Architecture:

```
src/
├── main.rs          entry: tracing(stderr) → config → serve stdio
├── lib.rs           crate root
├── config.rs        env configuration
├── error.rs         MoodleError (untagged exception mapping) + tool-error bridge
├── client.rs        MoodleClient: form-urlencoded + PHP bracket notation,
│                    exception detection (HTTP 200 + {exception,…}),
│                    pluginfile.php download, upload.php multipart
├── util.rs          HTML strip, ISO dates, truncation, draft item ids
└── tools/
    ├── mod.rs       #[tool_router] surface — 27 tools + site-info cache
    ├── site.rs courses.rs assignments.rs grades.rs calendar.rs forums.rs
    ├── messaging.rs users.rs quizzes.rs files.rs raw.rs
```

Design notes (from `docs/research.md`):

- Moodle's REST endpoint is **RPC-style** (`wsfunction=`), not RESTful;
  params are form-urlencoded in PHP bracket notation (`courseids[0]=2`).
- API errors arrive with **HTTP 200** and an exception payload — the client
  detects `{exception, errorcode, message}` and converts to tool-level errors.
- Tool list is intentionally built from functions that exist in the
  official WS API (several existing moodle-mcp projects call non-existent
  functions; typed Rust + verified function list avoids that class of bug).
- `call_ws_function` keeps full coverage without hardcoding every Moodle
  function, while validating against `site_info.functions`.

## Moodle version compatibility

Validated live against **Moodle 3.7** (NTNU) and designed for 4.x/5.x.
Version differences are handled dynamically from `site_info.functions`:

| Feature | Modern (4.x/5.x) | Moodle 3.7 fallback |
|---|---|---|
| Forum discussions | `mod_forum_get_discussions` | `mod_forum_get_forum_discussions` (no `sortby`, response wrapped in `{discussions}`) |
| Discussion pagination | `page`/`perpage` on posts | `mod_forum_get_discussion_posts` rejects them → auto-retry without |
| Notifications | `core_message_get_notifications` | `core_message_get_messages?type=notifications` |
| User search | `core_user_get_users` | `core_message_message_search_users` (may still be permission-limited) |
| Calendar `timefrom`/`timeto` | passed to API | silently ignored by 3.7 → filtered client-side |
| Conversation shape | `lastmessage`/`timemodified`/`subtype` | `messages[]`/`type` |

`pick_function()` chooses the right WS function per call from the token's
enabled list, so no rebuild is needed when switching sites.

## Security notes

- Always use **HTTPS** — the token is sent on every request.
- Prefer a dedicated account with a least-privilege external service.
- Moodle core has no WS rate limiting; put throttling at your reverse proxy
  if the server is multi-tenant.
- Write tools act **as the token owner**: your MCP client should confirm
  before submitting assignments or sending messages (the server instructions
  say exactly that).

## License

MIT
