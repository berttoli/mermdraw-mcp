#!/usr/bin/env bash
# Installs a built mermdraw-mcp the way users will, `mermdraw addons add mcp`, from a fake GitHub release made with
# scripts/release.sh package, then runs it as `mermdraw mcp` against a project.
# usage: tests/install.sh path/to/mermdraw [path/to/mermdraw-mcp]   (default: target/release/mermdraw-mcp)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
abs() { echo "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; }
CORE="$(abs "${1:?usage: tests/install.sh path/to/mermdraw [path/to/mermdraw-mcp]}")"
BIN="$(abs "${2:-$ROOT/target/release/mermdraw-mcp}")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
export XDG_CONFIG_HOME="$WORK/config" # never touch the real addons folder

fail() {
  echo "FAIL: $*" >&2
  exit 1
}
expect() { # DESC ACTUAL SUBSTRING
  [[ "$2" == *"$3"* ]] || fail "$1: expected '$3' in: ${2:0:600}"
  echo "ok   $1"
}
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

[[ -x "$CORE" ]] || fail "no mermdraw at $CORE"
[[ -x "$BIN" ]] || fail "no mermdraw-mcp at $BIN (build with: cargo build --release)"

# what github.com/berttoli/mermdraw-mcp/releases/latest/download/ will serve; the binary runs here, so it stands in
# for every unix target
REL="$WORK/releases/mermdraw-mcp/releases/latest/download"
for t in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
  "$ROOT/scripts/release.sh" package "$t" "$BIN" "$REL" >/dev/null
done
(cd "$REL" && sha256 mermdraw-mcp-* >SHA256SUMS.txt)

# gh would look for an attestation of this local build on GitHub and find none: keep it off PATH
mkdir -p "$WORK/tools"
for t in curl tar gzip; do p="$(command -v "$t")" && ln -s "$p" "$WORK/tools/$t"; done
out="$(PATH="$WORK/tools" MERMDRAW_ADDONS_URL="file://$WORK/releases" "$CORE" addons add mcp)"
expect "mermdraw addons add mcp installs the release" "$out" "installed mcp → $XDG_CONFIG_HOME/mermdraw/addons/mermdraw-mcp"
expect "the archive matched SHA256SUMS.txt" "$out" "matches SHA256SUMS.txt"
expect "mermdraw addons ls lists it" "$("$CORE" addons ls)" "mcp"
expect "mermdraw mcp --version runs the installed addon" "$("$CORE" mcp --version)" "mermdraw-mcp $("$BIN" --version | cut -d' ' -f2)"

mkdir -p "$WORK/project/.git" "$WORK/project/docs"
printf '# Shop\n\n\140\140\140mermaid\nflowchart LR\n    web --> api\n\140\140\140\n' >"$WORK/project/docs/shop.md" # \140 is a backtick
meta='"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}'
out="$(cd "$WORK/project" && printf '%s\n' \
  "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{$meta}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"read_diagram\",\"arguments\":{\"file\":\"docs/shop.md\"},$meta}}" |
  env -u MERMDRAW "$CORE" mcp 2>"$WORK/err")"
expect "mermdraw mcp speaks MCP over its stdio" "$out" '"io.modelcontextprotocol/serverInfo":{"name":"mermdraw-mcp"'
expect "it reads diagrams through the mermdraw that started it" "$out" 'flowchart LR\\n    web --> api'
expect "it was given that mermdraw's path" "$(cat "$WORK/err")" "mermdraw at $CORE"
expect "mermdraw addons rm mcp removes it" "$("$CORE" addons rm mcp)" "removed"
echo "install test passed"
