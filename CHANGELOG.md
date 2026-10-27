# Changelog

## Unreleased

First version: an MCP server addon for [mermdraw](https://github.com/berttoli/mermdraw), run as `mermdraw mcp`
(needs mermdraw 0.2 or later).

- Tools: `list_diagrams`, `read_diagram`, `write_diagram`, `add_diagram`, `search_icons` and `open_diagram`. All
  of them run mermdraw's `--json` commands and never touch markdown themselves. Canvas positions are kept on write,
  a write based on a stale version is refused with a tool error, and paths are confined to the project folder.
- MCP 2026-07-28 (per-request `_meta`, `server/discover`, caching hints) and the earlier `initialize` handshake
  (2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05) over stdio, built on rmcp.
- Speaks version 1 of mermdraw's addon contract and checks the `contract` field of every mermdraw answer.
- `--root DIR` picks the project folder (default: the git repository around the working folder).
- Release archives for Linux (static, x86_64 and arm64), macOS (Apple silicon and Intel) and Windows x64, named the
  way `mermdraw addons add mcp` expects, with SHA-256 checksums and build-provenance attestations.
