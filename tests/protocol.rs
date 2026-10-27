//! The MCP protocol over stdio, in both eras: 2026-07-28 (per-request `_meta`, server/discover) and the legacy
//! `initialize` handshake (2025-11-25 and earlier). None of this needs mermdraw itself.

mod common;
use common::*;
use serde_json::{Value, json};

const TOOLS: &[&str] = &["add_diagram", "list_diagrams", "open_diagram", "read_diagram", "search_icons", "write_diagram"];

fn no_core(root: &std::path::Path) -> std::path::PathBuf {
    root.join("no-mermdraw-here")
}

fn tool_names(result: &Value) -> Vec<String> {
    result["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn discover_describes_the_server() {
    let root = project("discover");
    let mut mcp = Mcp::start(&root, &no_core(&root));
    let r = mcp.modern("server/discover", json!({}));
    let res = &r["result"];
    assert_eq!(res["resultType"], "complete");
    let versions: Vec<&str> = res["supportedVersions"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    for v in ["2026-07-28", "2025-11-25", "2025-06-18"] {
        assert!(versions.contains(&v), "{v} missing from {versions:?}");
    }
    assert!(res["capabilities"]["tools"].is_object());
    assert_eq!(res["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "mermdraw-mcp");
    assert_eq!(res["_meta"]["io.modelcontextprotocol/serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(res["instructions"].as_str().unwrap().contains("read_diagram"));
    assert!(res["ttlMs"].as_u64().is_some());
    assert_eq!(res["cacheScope"], "public");
    assert!(mcp.close(), "the server should exit when stdin closes");
}

#[test]
fn modern_tools_list_has_every_tool_with_schemas_and_cache_hints() {
    let root = project("modern-list");
    let mut mcp = Mcp::start(&root, &no_core(&root));
    let first = mcp.modern("tools/list", json!({}))["result"].clone();
    assert_eq!(first["resultType"], "complete");
    assert!(first["ttlMs"].as_u64().unwrap() > 0);
    assert_eq!(first["cacheScope"], "public");
    assert_eq!(tool_names(&first), TOOLS);
    for tool in first["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert_eq!(tool["outputSchema"]["type"], "object", "{name} has structured output");
        assert!(tool["description"].as_str().unwrap().len() > 40, "{name} needs a description");
        assert!(tool["annotations"]["readOnlyHint"].is_boolean(), "{name}");
    }
    let write = first["tools"].as_array().unwrap().iter().find(|t| t["name"] == "write_diagram").unwrap();
    let required: Vec<&str> = write["inputSchema"]["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(required, ["file", "index", "source", "version"]);
    // deterministic order, so clients can cache it
    assert_eq!(mcp.modern("tools/list", json!({}))["result"]["tools"], first["tools"]);
}

#[test]
fn unsupported_versions_get_the_supported_list() {
    let root = project("unsupported");
    let mut mcp = Mcp::start(&root, &no_core(&root));
    for method in ["tools/list", "server/discover"] {
        let r = mcp.request(method, json!({"_meta": meta("1900-01-01")}));
        assert_eq!(r["error"]["code"], -32022, "{method}: {r}");
        assert_eq!(r["error"]["data"]["requested"], "1900-01-01");
        let supported = r["error"]["data"]["supported"].as_array().unwrap();
        assert!(supported.contains(&json!("2026-07-28")) && supported.contains(&json!("2025-11-25")));
    }
    // the server still answers a supported version afterwards
    assert_eq!(tool_names(&mcp.modern("tools/list", json!({}))["result"]), TOOLS);
}

#[test]
fn requests_without_the_required_meta_are_invalid_params() {
    let root = project("no-meta");
    let mut mcp = Mcp::start(&root, &no_core(&root));
    let r = mcp.request("tools/list", json!({}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = mcp.request("tools/list", json!({"_meta": {"io.modelcontextprotocol/protocolVersion": MODERN}}));
    assert_eq!(r["error"]["code"], -32602, "clientCapabilities is required too: {r}");
}

#[test]
fn modern_errors() {
    let root = project("modern-errors");
    let mut mcp = Mcp::start(&root, &no_core(&root));
    let r = mcp.modern("tools/call", json!({"name": "no_such_tool", "arguments": {}}));
    assert_eq!(r["error"]["code"], -32602, "an unknown tool is a protocol error: {r}");
    let r = mcp.modern("no/such/method", json!({}));
    assert_eq!(r["error"]["code"], -32601, "{r}");
    // tool execution errors are results the model can act on
    let r = mcp.call("read_diagram", json!({"file": "../../etc/hosts.md"}));
    assert!(tool_error(&r).contains("outside the project folder"));
    assert_eq!(r["resultType"], "complete");
    assert!(tool_error(&mcp.call("read_diagram", json!({}))).contains("file"));
    assert!(tool_error(&mcp.call("read_diagram", json!({"file": "notes.txt"}))).contains("not a markdown"));
    // with mermdraw missing, the tool says how to fix it
    std::fs::write(root.join("docs/a.md"), "# a\n").unwrap();
    assert!(tool_error(&mcp.call("read_diagram", json!({"file": "docs/a.md"}))).contains("cannot find mermdraw"));
    // notifications get no answer: the next message is the ping's response
    mcp.send(&json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 99}}));
    assert!(mcp.modern("server/discover", json!({}))["result"].is_object());
}

#[test]
fn legacy_initialize_handshake() {
    let root = project("legacy");
    let mut mcp = Mcp::start(&root, &no_core(&root));
    let init = mcp.initialize("2025-11-25");
    let res = &init["result"];
    assert_eq!(res["protocolVersion"], "2025-11-25");
    assert_eq!(res["serverInfo"]["name"], "mermdraw-mcp");
    assert!(res["capabilities"]["tools"].is_object());
    assert!(res["instructions"].is_string());
    let list = mcp.request("tools/list", json!({}))["result"].clone();
    assert_eq!(tool_names(&list), TOOLS);
    assert!(list.get("ttlMs").is_none() && list.get("cacheScope").is_none(), "no 2026-07-28 caching hints for legacy clients");
    let r = mcp.request("tools/call", json!({"name": "read_diagram", "arguments": {"file": "../x.md"}}));
    assert!(tool_error(&r["result"]).contains("outside the project folder"));
    assert_eq!(mcp.request("tools/call", json!({"name": "nope", "arguments": {}}))["error"]["code"], -32602);
    assert_eq!(mcp.request("ping", json!({}))["result"], json!({}));
    assert!(mcp.close());
}

#[test]
fn legacy_versions_are_negotiated() {
    for (asked, agreed) in
        [("2025-06-18", "2025-06-18"), ("2024-11-05", "2024-11-05"), ("2099-01-01", "2025-11-25"), (MODERN, "2025-11-25")]
    {
        let root = project(&format!("negotiate-{asked}"));
        let mut mcp = Mcp::start(&root, &no_core(&root));
        let r = mcp.initialize(asked);
        // an unknown version, or one without a handshake, gets the newest version that has one
        assert_eq!(r["result"]["protocolVersion"], agreed, "asked {asked}: {r}");
    }
}

#[test]
fn discover_probe_then_legacy_fallback() {
    // a dual-era client probes with server/discover first; that must not lock the server into either era
    let root = project("probe");
    let mut mcp = Mcp::start(&root, &no_core(&root));
    assert!(mcp.modern("server/discover", json!({}))["result"]["supportedVersions"].is_array());
    assert_eq!(mcp.initialize("2025-11-25")["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(tool_names(&mcp.request("tools/list", json!({}))["result"]), TOOLS);
}

#[test]
fn version_and_help_flags() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mermdraw-mcp")).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("mermdraw-mcp {}", env!("CARGO_PKG_VERSION")));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mermdraw-mcp")).arg("--help").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("claude mcp add mermdraw -- mermdraw mcp"));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mermdraw-mcp")).arg("--bogus").output().unwrap();
    assert!(!out.status.success() && out.stdout.is_empty());
}
