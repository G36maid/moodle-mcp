# Installation

## Prebuilt Binaries

No Rust toolchain needed — grab the archive for your platform from the
[latest release](https://github.com/G36maid/moodle-mcp/releases/latest):

| Platform            | Archive                                                 |
| ------------------- | ------------------------------------------------------- |
| Linux x86_64        | `moodle-mcp-<version>-x86_64-unknown-linux-gnu.tar.gz`  |
| Linux arm64         | `moodle-mcp-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon | `moodle-mcp-<version>-aarch64-apple-darwin.tar.gz`      |
| macOS Intel         | `moodle-mcp-<version>-x86_64-apple-darwin.tar.gz`       |
| Windows x64         | `moodle-mcp-<version>-x86_64-pc-windows-msvc.zip`       |

```sh
tar xzf moodle-mcp-*-aarch64-apple-darwin.tar.gz
sudo mv moodle-mcp-*/moodle-mcp /usr/local/bin/   # or anywhere on PATH
```

```sh
# Windows (PowerShell): unpack, then add the folder to your PATH
Expand-Archive moodle-mcp-<version>-x86_64-pc-windows-msvc.zip
```

Each archive is around 9 MB and unpacks to a single self-contained binary
(about 22 MB) plus the README and license — there are no runtime
dependencies.

> **Tip:** If macOS refuses to run a downloaded binary ("cannot be
> verified"), clear the quarantine flag once:
> `xattr -d com.apple.quarantine /usr/local/bin/moodle-mcp`.

## One-Click Bundles (`.mcpb`)

Every release also ships one `.mcpb` bundle per platform — the MCP bundle
format for one-click installs. Clients that understand it (Claude Desktop,
Claude Code) install the binary without a toolchain and prompt for
`MOODLE_URL` and `MOODLE_TOKEN` during setup, so the token from
`moodle-mcp auth` is all you need.

Grab `moodle-mcp-<version>-<platform>.mcpb` from the
[releases page](https://github.com/G36maid/moodle-mcp/releases/latest) and
open it with your client.

## Cargo

```sh
cargo install moodle-mcp   # needs Rust 1.88+
```

## From Source

```sh
git clone https://github.com/G36maid/moodle-mcp
cd moodle-mcp
cargo build --release
# binary: target/release/moodle-mcp
```

## Verify the Install

`moodle-mcp` with no arguments starts the MCP stdio server — it blocks
waiting for a client, which is expected. To confirm the binary is on your
PATH and see its version, run it with an unknown subcommand:

```sh
moodle-mcp help
# moodle-mcp: unknown subcommand 'help'
# usage: moodle-mcp [serve] | moodle-mcp auth [--url <site> ...]
```

When the server starts it logs `starting moodle-mcp v0.1.4 for …` to stderr
(logs go to stderr; stdout is reserved for the MCP protocol stream).

## Next Steps

Continue with [Getting Started](./getting-started.md) to get a token and
connect your client. Release notes live on the
[Changelog](./changelog.md) page.
