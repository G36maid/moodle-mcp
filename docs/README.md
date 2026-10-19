# moodle-mcp Docs

Documentation for [moodle-mcp](https://github.com/G36maid/moodle-mcp), a
Model Context Protocol server that lets LLM agents use a real Moodle LMS.

This follows the Zed docs conventions: **mdBook** for structure and
**Prettier** (80-char line width) for formatting.

To preview the docs locally, install the latest mdBook and mermaid plugin
and serve:

```sh
cargo install mdbook mdbook-mermaid
mdbook serve docs
```

Then open http://localhost:3000.

Before committing, format and verify with Prettier:

```sh
cd docs && pnpm dlx prettier@3.5.0 . --write
cd docs && pnpm dlx prettier@3.5.0 . --check
```

Conventions for editing these pages live in [AGENTS.md](AGENTS.md).
