# moodle-mcp

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![MCP](https://img.shields.io/badge/protocol-MCP%20stdio-6C5CE7)](https://modelcontextprotocol.io)
[![Release](https://github.com/G36maid/moodle-mcp/actions/workflows/release.yml/badge.svg)](https://github.com/G36maid/moodle-mcp/actions/workflows/release.yml)

A [Model Context Protocol](https://modelcontextprotocol.io) server that lets
LLM agents **use a real Moodle LMS** — read courses, fetch assignment PDFs,
write code, upload and submit — through a single static binary.

Built in Rust on the official MCP SDK ([`rmcp`](https://crates.io/crates/rmcp)).
Validated live against **Moodle 3.7 → 5.x**.

---

## ❌ Without moodle-mcp

An agent asked about your coursework can only guess:

- ❌ "I don't have access to your Moodle" — even with the token in hand
- ❌ Hallucinated API endpoints and field names
- ❌ Copy-pasting raw JSON by hand, guessing PHP array notation
- ❌ Reading assignment PDFs is impossible for a text-only pipeline

## ✅ With moodle-mcp

22 typed tools covering the full student journey. The agent figures out the
rest:

```txt
What's due in the next two weeks?
```
→ `get_schedule` returns a merged timeline of assignment due dates, quiz
closes and calendar events.

```txt
Read the HW1 assignment brief and summarize the requirements.
```
→ `get_assignment` finds it, `fetch_file` converts the attached PDF into
markdown (math notation included) — right in the agent's context.

```txt
Upload my solution and turn it in.
```
→ `upload_file` puts the file in the draft area, `submit_assignment` saves
and finalizes it.

## ✨ Features

- **22 tools** — courses, course map, assignments, submission, forums,
  files (fetch/convert/download/upload), grades, schedule, messaging,
  quiz attempts, plus a generic escape hatch (`call_ws_function` +
  `list_functions`) so nothing is out of reach
- **Course map in one call** — `get_course_overview` merges five Moodle
  endpoints into a single markdown document with every ID and file URL embedded
- **PDF → markdown** — assignment briefs become readable requirements via
  [anydoc](https://crates.io/crates/anydoc); HTML descriptions keep their
  tables via [htmd](https://crates.io/crates/htmd)
- **Files stay on disk** — `download_file`/`upload_file` work with local
  paths; binaries never pass through the conversation
- **Version-adaptive** — verified against Moodle 3.7 and designed for 4.x/5.x;
  differences (function names, pagination, response shapes) are absorbed
  automatically
- **Safe by default** — write tools require explicit confirmation; drafts are
  preferred over irreversible submissions; quiz-taking is intentionally not
  provided
- **Single static binary** — no runtime dependencies

## 📦 Install

### Build

```bash
git clone https://github.com/G36maid/moodle-mcp
cd moodle-mcp
cargo build --release
# binary: target/release/moodle-mcp (~15 MB, static)
```

### Get a token

Use the built-in CLI (same binary):

```bash
moodle-mcp token --url https://moodle.example.com --username yourname
# Password: (hidden prompt)
# → export MOODLE_URL='https://moodle.example.com' MOODLE_TOKEN='…'
```

With SSO (no password), open the Security keys page in a browser instead:

```bash
moodle-mcp token --url https://moodle.example.com --open
```

Site-side requirements: web services enabled, REST protocol, and the
`moodle_mobile_app` service allowed for your account (standard Moodle setup).

### Connect a client

**pi** — project-local `.pi/mcp.json`:

```json
{
  "mcpServers": {
    "moodle": {
      "command": "/path/to/moodle-mcp",
      "env": {
        "MOODLE_URL": "https://moodle.example.com",
        "MOODLE_TOKEN": "your-token"
      }
    }
  }
}
```

**Claude Code:**

```bash
claude mcp add moodle --env MOODLE_URL=https://moodle.example.com \
  --env MOODLE_TOKEN=your-token -- /path/to/moodle-mcp
```

**Claude Desktop / any `mcpServers` client:**

```json
{
  "mcpServers": {
    "moodle": {
      "command": "/path/to/moodle-mcp",
      "env": { "MOODLE_URL": "https://moodle.example.com", "MOODLE_TOKEN": "your-token" }
    }
  }
}
```

| Env var | Required | Default | Description |
|---|:---:|---|---|
| `MOODLE_URL` | ✔ | — | Site base URL, no trailing slash |
| `MOODLE_TOKEN` | ✔ | — | Web service token |
| `MOODLE_TIMEOUT_SECS` | — | `60` | Per-request timeout |
| `RUST_LOG` | — | `info` | Log filter (logs → stderr; stdout is the MCP stream) |

## 💬 Prompt examples

All of these work with no further setup — the agent discovers the tools from
their descriptions:

| You say | The agent does |
|---|---|
| *"What's due in the next two weeks?"* | `get_schedule` → merged timeline of due dates, quiz closes, events |
| *"What's the HW1 assignment about? Read the brief."* | `get_assignment` → `fetch_file` converts the attached PDF to markdown |
| *"How did I do on the Data Communication labs?"* | `list_courses` → `get_grades(course_id)` → items, feedback, weights |
| *"Which of my courses have graded items?"* | `get_grades()` → cross-course summary |
| *"Summarize every announcement in the Data Communication forum."* | `get_course_overview` → `get_forum_discussions` → `get_discussion_posts` |
| *"Upload my solution.pdf for Quiz1 and save it as a draft."* | `upload_file` → `submit_assignment(item_id, finalize=false)` |
| *"What's on the agenda for the Software Engineering course?"* | `get_course_overview` → sections, files, forums |
| *"Can I message my teacher about an extension?"* | `get_user` / `get_conversations` → draft → confirms → `send_message` |

## 🔄 How the agent works

The tools mirror how you navigate Moodle — each level's output embeds the IDs
for the next:

```text
list_courses ──→ get_course_overview(course_id)        one map: everything
                    │                │
                    │ sections       │ forum_id / assignment_id / quiz_id / grades
                    ▼                ▼
      get_forum_discussions    get_assignment (full document)
                │                    │
                ▼                    ▼
      get_discussion_posts      fetch_file / download_file
      (thread + attachments)    (read the brief / save the PDF)
                                     │
                                     ▼
                upload_file(path) → item_id
                                     │
                                     ▼
                submit_assignment(assignment_id, item_id, finalize?)
```

Everything not covered by the 21 dedicated tools is reachable through the
`call_ws_function` + `list_functions` escape hatch — full coverage of the
~350 functions a typical token can call.

## 🛠 Tool reference

### Identity & courses
| Tool | Signature | Purpose |
|---|---|---|
| `whoami` | `()` | Token owner identity + site/release |
| `list_courses` | `(search?)` | Enrolled courses; optional name filter |
| `get_course_overview` | `(course_id)` | Whole-course markdown map |

### Assignments
| Tool | Signature | Purpose |
|---|---|---|
| `get_assignment` | `(assignment_id)` | Full document: times, state, description, attachments, submission, grade & feedback |
| `submit_assignment` | `(assignment_id, text?, item_id?, finalize?)` | The single write: online text and/or draft files, optionally finalizing |

### Forums
| Tool | Signature | Purpose |
|---|---|---|
| `get_forum_discussions` | `(forum_id)` | Thread list with opening-post previews |
| `get_discussion_posts` | `(discussion_id)` | Whole thread, converted to markdown, attachments included |
| `create_forum_discussion` | `(forum_id, subject, message)` | Start a thread |
| `reply_to_discussion` | `(post_id, subject, message)` | Reply (to the opening post = reply to the thread) |

### Files
| Tool | Signature | Purpose |
|---|---|---|
| `fetch_file` | `(url)` | PDF/DOCX/PPTX/XLSX/ODT/RTF/EPUB/CSV → markdown (anydoc); 100k-char cap |
| `download_file` | `(url, save_to)` | Raw bytes → local path (metadata only) |
| `upload_file` | `(path, item_id?)` | Local file → draft area |

### Grades, schedule, messaging
| Tool | Signature | Purpose |
|---|---|---|
| `get_grades` | `(course_id?)` | Cross-course summary, or every grade item of one course |
| `get_schedule` | `(time_from?, time_to?, course_ids?)` | Merged timeline (default: next 30 days) |
| `get_notifications` | `(read?, limit?)` | Recent notifications |
| `get_conversations` | `(limit?)` | Conversations with member ids |
| `send_message` | `(user_id, text)` | Direct message |
| `get_quiz_attempts` | `(quiz_id)` | Attempt records (read-only) |
| `get_user` / `search_users` | `(field, value)` / `(search?, limit?)` | Profile lookup / user search |

### Escape hatch
| Tool | Signature | Purpose |
|---|---|---|
| `call_ws_function` | `(wsfunction, params?)` | Call any enabled WS function (PHP bracket notation handled) |
| `list_functions` | `(query?)` | Discover callable functions by keyword |

## 🔐 Security notes

- Use **HTTPS** — the token travels with every request.
- The token is a credential: never commit it, never send it to hosts other
  than your Moodle site (the server only injects it into same-site
  `pluginfile.php` URLs).
- Write tools act **as the token owner**. The server's instructions tell the
  agent to confirm with you before submitting or sending.
- Quiz-taking is intentionally **not** provided.

## 🧰 Development

```bash
cargo test                 # 25 tests: unit + wiremock integration + MCP protocol
cargo clippy --all-targets
cargo fmt -- --check
cargo build --release
```

ID flow between tools:

| ID | Produced by | Consumed by |
|---|---|---|
| course_id | list_courses / get_course_overview | overview / grades / schedule / assignments |
| forum_id | get_course_overview | get_forum_discussions / create_forum_discussion |
| discussion_id | get_forum_discussions | get_discussion_posts |
| post_id | get_discussion_posts | reply_to_discussion |
| assignment_id | get_course_overview / get_assignment | get_assignment / submit_assignment |
| item_id (draft) | upload_file | submit_assignment |
| user_id | conversations members / notifications / get_user | send_message / get_user |

## 📄 License

MIT — see [LICENSE](LICENSE).
