#!/usr/bin/env bash
# Smoke test of a built mermdraw-mcp binary over stdio, without mermdraw: both protocol eras answer, stdout carries
# only protocol messages, and the server exits when stdin closes.
# usage: tests/smoke.sh [path/to/mermdraw-mcp]      (default: target/release/mermdraw-mcp)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/target/release/mermdraw-mcp}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}
expect() { # DESC ACTUAL SUBSTRING
  [[ "$2" == *"$3"* ]] || fail "$1: expected '$3' in: ${2:0:600}"
  echo "ok   $1"
}
only_json() { # DESC OUTPUT: every line is a JSON-RPC message
  local line
  while IFS= read -r line; do
    [[ "$line" == '{"jsonrpc":"2.0",'* ]] || fail "$1: stdout has a line that is not a JSON-RPC message: ${line:0:200}"
  done <<<"$2"
  echo "ok   $1"
}

[[ -x "$BIN" ]] || fail "no executable at $BIN (build with: cargo build --release)"
mkdir -p "$WORK/project/.git"
cd "$WORK/project"
export MERMDRAW="$WORK/no-mermdraw" # these checks must not need it

meta='"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"smoke","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}'
out="$(printf '%s\n' \
  "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{$meta}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{$meta}}" \
  '{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"1900-01-01","io.modelcontextprotocol/clientCapabilities":{}}}}' |
  "$BIN" 2>"$WORK/err")"
expect "answers server/discover (2026-07-28)" "$out" '"supportedVersions":['
expect "names itself" "$out" '"io.modelcontextprotocol/serverInfo":{"name":"mermdraw-mcp"'
expect "lists its tools" "$out" '"name":"write_diagram"'
expect "rejects an unknown protocol version" "$out" '"code":-32022'
only_json "stdout carries only protocol messages" "$out"
expect "logs to stderr" "$(cat "$WORK/err")" "mermdraw-mcp "

out="$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' |
  "$BIN" 2>/dev/null)"
expect "answers the legacy initialize handshake" "$out" '"protocolVersion":"2025-11-25"'
expect "lists its tools to legacy clients" "$out" '"name":"open_diagram"'
only_json "stdout carries only protocol messages (legacy)" "$out"
echo "smoke test passed"
