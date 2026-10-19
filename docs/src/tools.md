# Tools

moodle-mcp exposes 22 tools: 20 curated ones that mirror how you navigate
Moodle by hand are always available, and a two-tool escape hatch —
`list_ws_functions` + `call_ws_function` — reaches everything else your
token allows once you enable it with `MOODLE_ENABLE_WS_FUNCTION=true`.
There is no 1:1 dump of the ~350 raw Moodle web-service functions — the
curated set covers the student journey; the escape hatch covers the rest.

Every tool returns structured JSON in `structuredContent`. Document tools
(`get_course_overview`, `get_assignment`) additionally return markdown in
`text`. Nulls are stripped recursively, because Moodle 3.7 omits many
fields that 4.x/5.x include.

## Read vs. Write

Each tool is either read-only or a write. Write tools act **as the token
owner** — the server's instructions tell the agent to confirm with you
before calling them, and the tool descriptions repeat it. Uploads land in
your private draft area, not with your teacher, until you explicitly
finalize a submission.

## Identity & Courses

| Tool                  | Signature     | Read/Write | Purpose                                |
| --------------------- | ------------- | :--------: | -------------------------------------- |
| `whoami`              | `()`          |    read    | Token owner identity + site/release    |
| `list_courses`        | `(search?)`   |    read    | Enrolled courses; optional name filter |
| `get_course_overview` | `(course_id)` |    read    | Whole-course markdown map              |

`get_course_overview` merges five Moodle endpoints into a single markdown
document: sections, files, assignments, quizzes, forums, and grades — every
ID and file URL embedded, so one call gives the agent (and you) the whole
course map.

## Assignments

| Tool                | Signature                                     | Read/Write | Purpose                                                                  |
| ------------------- | --------------------------------------------- | :--------: | ------------------------------------------------------------------------ |
| `get_assignment`    | `(assignment_id)`                             |    read    | Full document: times, state, description, attachments, submission, grade |
| `submit_assignment` | `(assignment_id, text?, item_id?, finalize?)` |   write    | The single write: online text and/or draft files, optionally finalizing  |

## Forums

| Tool                      | Signature                      | Read/Write | Purpose                                           |
| ------------------------- | ------------------------------ | :--------: | ------------------------------------------------- |
| `get_forum_discussions`   | `(forum_id)`                   |    read    | Thread list with opening-post previews            |
| `get_discussion_posts`    | `(discussion_id)`              |    read    | Whole thread as markdown, attachments included    |
| `create_forum_discussion` | `(forum_id, subject, message)` |   write    | Start a thread                                    |
| `reply_to_discussion`     | `(post_id, subject, message)`  |   write    | Reply (to the opening post = reply to the thread) |

## Files

| Tool            | Signature          | Read/Write | Purpose                                                            |
| --------------- | ------------------ | :--------: | ------------------------------------------------------------------ |
| `fetch_file`    | `(url)`            |    read    | PDF/DOCX/PPTX/XLSX/ODT/RTF/EPUB/CSV → markdown; 100k-character cap |
| `download_file` | `(url, save_to)`   |    read    | Raw bytes → local path (metadata only)                             |
| `upload_file`   | `(path, item_id?)` |   write    | Local file → draft area                                            |

File tools take and return **local file paths**, never base64. Same-site
URLs only, unless `MOODLE_ALLOW_EXTERNAL_FETCH=1`.

## Grades, Schedule & Messaging

| Tool                | Signature                             | Read/Write | Purpose                                          |
| ------------------- | ------------------------------------- | :--------: | ------------------------------------------------ |
| `get_grades`        | `(course_id?)`                        |    read    | Cross-course summary, or every grade item of one |
| `get_schedule`      | `(time_from?, time_to?, course_ids?)` |    read    | Merged timeline (default: next 30 days)          |
| `get_notifications` | `(read?, limit?)`                     |    read    | Recent notifications                             |
| `get_conversations` | `(limit?)`                            |    read    | Conversations with member ids                    |
| `send_message`      | `(user_id, text)`                     |   write    | Direct message                                   |
| `get_quiz_attempts` | `(quiz_id)`                           |    read    | Attempt records (read-only)                      |
| `get_user`          | `(field, value)`                      |    read    | Profile lookup                                   |
| `search_users`      | `(search?, limit?)`                   |    read    | User search                                      |

## Escape Hatch

| Tool                | Signature               | Read/Write | Purpose                                                              |
| ------------------- | ----------------------- | :--------: | -------------------------------------------------------------------- |
| `call_ws_function`  | `(wsfunction, params?)` | read/write | Call any enabled web-service function (PHP bracket notation handled) |
| `list_ws_functions` | `(query?)`              |    read    | Discover callable web-service functions by keyword                   |

`call_ws_function` is an intentional overwriting surface. It is hidden
unless `MOODLE_ENABLE_WS_FUNCTION=true` opts in, and `MOODLE_READ_ONLY`
hides it again regardless.

## ID Flow

Tools chain through IDs: each level's output embeds the IDs for the next.

| ID              | Produced by                              | Consumed by                                        |
| --------------- | ---------------------------------------- | -------------------------------------------------- |
| `course_id`     | `list_courses`, `get_course_overview`    | overview, grades, schedule, assignments            |
| `forum_id`      | `get_course_overview`                    | `get_forum_discussions`, `create_forum_discussion` |
| `discussion_id` | `get_forum_discussions`                  | `get_discussion_posts`                             |
| `post_id`       | `get_discussion_posts`                   | `reply_to_discussion`                              |
| `assignment_id` | `get_course_overview`, `get_assignment`  | `get_assignment`, `submit_assignment`              |
| `item_id`       | `upload_file`                            | `submit_assignment`                                |
| `user_id`       | conversations, notifications, `get_user` | `send_message`, `get_user`                         |

## What Is Intentionally Not Provided

Quiz-taking. `get_quiz_attempts` reads attempt records, but nothing answers
questions for you. This is a design decision, not a gap.

## Version Differences

moodle-mcp targets Moodle 3.7 → 5.x. Differences between versions (function
names, pagination, response shapes) are absorbed by the server, not by your
prompts — the same tool call works across all supported versions.
