# Changelog

## 0.1.0 — 2026-10-27

First version: an MCP server addon for [mermdraw](https://github.com/berttoli/mermdraw), run as `mermdraw mcp`
(needs mermdraw 0.2 or later).

- Tools: `list_diagrams`, `read_diagram`, `write_diagram`, `add_diagram`, `search_icons` and `open_diagram`. All
  of them run mermdraw's `--json` commands and never touch markdown themselves. Canvas positions are kept on write,
  a write based on a stale version is refused with a tool error, and paths are confined to the project folder.
- `read_diagram` gives each node's canvas position, and `write_diagram` takes `positions` for the nodes it adds or
  moves, so a node an agent adds sits next to the ones it connects to (needs mermdraw 0.3; with 0.2 the editor places
  new nodes, and passing positions says to update).
- MCP 2026-07-28 (per-request `_meta`, `server/discover`, caching hints) and the earlier `initialize` handshake
  (2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05) over stdio, built on rmcp.
- Speaks version 1 of mermdraw's addon contract and checks the `contract` field of every mermdraw answer.
- `--root DIR` picks the project folder (default: the git repository around the working folder, else that folder).
  Without it, the home folder, a folder above it and the top of the disk are never the project, so a client that
  starts the server there doesn't hand agents every markdown file on the machine; every tool asks for `--root` instead.
- Release archives for Linux (static, x86_64 and arm64), macOS (Apple silicon and Intel) and Windows x64, named the
  way `mermdraw addons add mcp` expects, with SHA-256 checksums and build-provenance attestations.
