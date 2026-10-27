# mermdraw-mcp

An [MCP](https://modelcontextprotocol.io) server that lets coding agents (Claude Code, Codex, Cursor, Zed, …) work
with the mermaid diagrams in your markdown through [mermdraw](https://github.com/berttoli/mermdraw). Agents can list,
read, rewrite and add diagrams, find logos for them, and open one in mermdraw's visual editor for you to look at or
rearrange.

It's a mermdraw addon: `mermdraw mcp` runs it. Every read and write goes through mermdraw's own `--json` commands, so
mermdraw stays the only program that edits your markdown. That gives the agent the same guarantees as the editor:

- **Only the named diagram changes.** The surrounding prose, the other diagrams, line endings and indentation stay
  exactly as they were.
- **Canvas positions survive.** The `%% mermdraw:layout` line holding the positions you arranged on the canvas is
  kept out of what the agent reads and is carried over when it writes.
- **No lost edits.** A write names the version of the file it was based on. If you (or the editor, or git) changed
  the file since, the write is refused and the agent is told to read it again.
- **Writes stay inside the project folder**, and only to markdown and mermaid files.

## Install

You need mermdraw 0.2 or later ([install](https://github.com/berttoli/mermdraw#install)). Then:

```bash
mermdraw addons add mcp
```

This downloads the release for your platform from this repository and checks it against the release's
`SHA256SUMS.txt`. When the GitHub CLI is installed, it also checks the build-provenance attestation. Check that it
runs:

```bash
mermdraw mcp --version
```

Or build it with Rust and put `mermdraw-mcp` on your `PATH`, where `mermdraw mcp` finds it too:

```bash
cargo install --git https://github.com/berttoli/mermdraw-mcp
```

## Setup

The server speaks MCP over stdio. Both the 2026-07-28 revision and the earlier `initialize`-based revisions
(2025-11-25 back to 2024-11-05) are supported, so current and older clients both work. It works on the project it is
started in: the git repository around its working folder, or the folder given with `--root`.

**Claude Code**

```bash
claude mcp add mermdraw -- mermdraw mcp
```

Add `-s project` to write it to the repository's `.mcp.json` so teammates get it too.

**Codex**

```bash
codex mcp add mermdraw -- mermdraw mcp
```

or in `~/.codex/config.toml`:

```toml
[mcp_servers.mermdraw]
command = "mermdraw"
args = ["mcp"]
```

**Cursor**: `.cursor/mcp.json` in the project (or `~/.cursor/mcp.json` for every project):

```json
{
  "mcpServers": {
    "mermdraw": {
      "type": "stdio",
      "command": "mermdraw",
      "args": ["mcp", "--root", "${workspaceFolder}"]
    }
  }
}
```

**Zed**: in `settings.json` (*zed: open settings file*):

```json
{
  "context_servers": {
    "mermdraw": {
      "command": "mermdraw",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

If a client starts the server outside your project, add `"--root", "/path/to/project"` to its arguments. If the
client can't find `mermdraw`, use its full path (`which mermdraw`).

## Tools

| Tool | What it does |
| --- | --- |
| `list_diagrams` | The diagrams in a markdown file, or in every markdown file under a folder (default: the project): index, type, title, lines, and each file's version |
| `read_diagram` | One diagram's mermaid source (without fences or the layout line) and the file's version |
| `write_diagram` | Replaces a diagram with a new source, given the version it read. Keeps the canvas positions; a stale version is refused |
| `add_diagram` | Appends a diagram to a markdown file, under a `## title` heading if given; creates the file if needed |
| `search_icons` | Finds logos in the installed icon packs (`mermdraw icons add logos devicon`), e.g. `postgres` → `logos:postgresql` |
| `open_diagram` | Opens a diagram in mermdraw's editor in your browser, reusing the editor already running for the project or starting one in the background |

All tools return structured results (with output schemas) plus the same JSON as text. Failures the agent can act on
come back as tool errors (`isError: true`) whose message says what to do: a stale version, a missing diagram, a path
outside the project. Only protocol messages go to stdout; logs go to stderr.

Try asking your agent: *"Add a sequence diagram of the login flow to docs/auth.md, then open it."* Or: *"In
docs/architecture.md, add a Redis cache between the API and Postgres, with logos."*

## How it finds mermdraw

`mermdraw mcp` sets `MERMDRAW` to the mermdraw that started it, and the server runs that binary. When you start
`mermdraw-mcp` directly, it runs `mermdraw` from `PATH`, or `$MERMDRAW` if you set it. It speaks version 1 of
mermdraw's [addon contract](https://github.com/berttoli/mermdraw/blob/main/docs/ADDONS.md) and checks the `contract`
number of every answer. If mermdraw speaks another version, the tool error says whether to update mermdraw or
mermdraw-mcp.

## Develop

Rust (pinned in `rust-toolchain.toml`); the end-to-end tests also need a mermdraw 0.2+ binary.

```bash
cargo test                                          # unit + protocol tests (both eras, errors, unknown versions)
MERMDRAW_CORE=../mermdraw/target/release/mermdraw cargo test   # plus end-to-end tests against real mermdraw
cargo build --release && tests/smoke.sh             # the release binary over stdio
tests/install.sh ../mermdraw/target/release/mermdraw  # mermdraw addons add mcp from a fake local release, then mermdraw mcp
```

Without `MERMDRAW_CORE`, the end-to-end tests use `../mermdraw/target/release/mermdraw` when that exists and are
skipped otherwise. CI builds mermdraw from its `main` branch and requires them.

Releases work like mermdraw's ([RELEASING.md](https://github.com/berttoli/mermdraw/blob/main/docs/RELEASING.md)):
push a `vX.Y.Z` tag, and `.github/workflows/release.yml` builds, verifies and attests `mermdraw-mcp-TARGET.tar.gz`
(`.zip` on Windows) for five targets, plus `SHA256SUMS.txt`, in a draft release. These are the asset names
`mermdraw addons add mcp` downloads. `scripts/release.sh` is mermdraw's own release script, copied unchanged, so
each step also runs locally.

The server is built on [rmcp](https://github.com/modelcontextprotocol/rust-sdk), the official Rust MCP SDK.

## License

[MIT](LICENSE) © berttoli
