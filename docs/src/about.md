# About

## What is moodle-mcp?

moodle-mcp is a [Model Context Protocol](#what-is-mcp) server, written in
Rust, that lets LLM agents use a real [Moodle](#what-is-moodle) LMS as the
token owner — the same way you use it: read courses, fetch assignment PDFs,
work, upload, submit a draft. One static binary, no runtime dependencies.

## What is MCP?

The [Model Context Protocol](https://modelcontextprotocol.io) (MCP) is an
open standard that defines how LLM applications talk to external tools and
data sources. An MCP client — Claude Code, Codex, OpenCode, pi, and other
agent harnesses — connects to MCP servers, each exposing a set of typed
tools the model can call. moodle-mcp is one such server: it speaks MCP over
stdio and translates those tool calls into Moodle web-service requests.

## What is Moodle?

[Moodle](https://moodle.org) is a free, open-source learning management
system (LMS) used by universities, schools, and workplaces worldwide to run
courses: assignment briefs and submissions, quizzes, forums, grades, and
everything else around teaching. Your institution's course website almost
certainly runs it. moodle-mcp drives that same platform through its
official web-service API.

## The project

The project started from a simple observation: a chatbot with no way into
your Moodle leaves every study task manual — five browser tabs to answer
"what's due this week?", PDFs re-uploaded by hand into a chat window,
forums and grades copy-pasted one page at a time. moodle-mcp gives the
agent typed tools instead, and gets out of the way.

## Design Principles

- **Human-shaped tools.** The tools mirror steps you'd actually take
  clicking through Moodle, not a 1:1 dump of ~350 raw web-service
  functions. An opt-in escape hatch (`call_ws_function`) still reaches
  everything else.
- **Agent-native.** Tools take and return local file paths instead of
  base64; results embed the IDs for the next tool; errors are
  plain-language and suggest the fix.
- **Version-adaptive.** Verified live against Moodle 3.7, 4.5 and 5.2
  (3.7 and 4.5 on live sites, 5.2 on a self-hosted instance); response
  differences between versions are absorbed by the server.
- **Safe by default.** Write tools require explicit confirmation, drafts
  are preferred over irreversible submissions, plain HTTP is rejected, and
  quiz-taking is intentionally not provided.

## Built With

| Crate                                       | Role                                  |
| ------------------------------------------- | ------------------------------------- |
| [`rmcp`](https://crates.io/crates/rmcp)     | Official Rust MCP SDK                 |
| [`anydoc`](https://crates.io/crates/anydoc) | Document conversion (PDF/DOCX/… → md) |
| [`htmd`](https://crates.io/crates/htmd)     | HTML → markdown conversion            |

## Links

- [GitHub repository](https://github.com/G36maid/moodle-mcp) — source,
  issues, pull requests
- [Releases](https://github.com/G36maid/moodle-mcp/releases) — prebuilt
  binaries for Linux, macOS, and Windows
- [crates.io](https://crates.io/crates/moodle-mcp) — `cargo install
moodle-mcp`
- [CHANGELOG](https://github.com/G36maid/moodle-mcp/blob/main/CHANGELOG.md)
  — release notes

## License

MIT — see [LICENSE](https://github.com/G36maid/moodle-mcp/blob/main/LICENSE).

## Contributing

Bug reports and pull requests are welcome on
[GitHub](https://github.com/G36maid/moodle-mcp). For development setup and
architecture notes, start with
[AGENTS.md](https://github.com/G36maid/moodle-mcp/blob/main/AGENTS.md) in
the repository root; the test suite runs fully offline.
