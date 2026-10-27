//! End to end: the built mermdraw-mcp over stdio, against a real mermdraw (0.2 or later) in a temporary project.
//!
//! mermdraw is taken from $MERMDRAW_CORE, else from a sibling checkout (../mermdraw/target/release/mermdraw). Without
//! one these tests are skipped, unless MERMDRAW_E2E_REQUIRED is set (as in CI), which makes that a failure.

mod common;
use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ARCH: &str = "# Shop\n\nSome prose that must survive.\n\n## Checkout\n\n```mermaid\nflowchart LR\n    web[Web] --> api[API]\n    \
%% mermdraw:layout web:0,0 api:200,0\n```\n\n## Login\n\n```mermaid\nsequenceDiagram\n  A->>B: hi\n```\n\nTrailing text.\n";

fn core() -> Option<PathBuf> {
    // made absolute: the server runs it from the test project, not from here
    let given = std::env::var_os("MERMDRAW_CORE").filter(|v| !v.is_empty()).map(PathBuf::from);
    let found = given.map(|p| p.canonicalize().unwrap_or(p)).or_else(|| {
        let sibling = Path::new(env!("CARGO_MANIFEST_DIR")).join("../mermdraw/target/release/mermdraw");
        sibling.canonicalize().ok()
    });
    let usable =
        found.filter(|bin| Command::new(bin).args(["ls", "--json", env!("CARGO_MANIFEST_DIR")]).output().is_ok_and(|o| o.status.success()));
    if usable.is_none() {
        let msg = "no mermdraw 0.2+ found: set MERMDRAW_CORE to a mermdraw binary with the --json commands";
        assert!(std::env::var_os("MERMDRAW_E2E_REQUIRED").is_none_or(|v| v.is_empty()), "{msg}");
        eprintln!("skipping: {msg}");
    }
    usable
}

/// Stops a process the test started when it goes out of scope, so a failing assertion can't leave an editor running.
enum Stop {
    Child(std::process::Child),
    #[cfg_attr(not(unix), allow(dead_code))]
    Pid(String),
}

impl Drop for Stop {
    fn drop(&mut self) {
        match self {
            Stop::Child(c) => {
                let _ = c.kill();
                let _ = c.wait();
            }
            Stop::Pid(pid) => {
                let _ = Command::new("kill").arg(pid.as_str()).status();
            }
        }
    }
}

fn setup(name: &str) -> Option<(PathBuf, Mcp)> {
    let core = core()?;
    let root = project(name);
    fs::write(root.join("docs/arch.md"), ARCH).unwrap();
    fs::write(root.join("docs/notes.md"), "# Notes\n\nNo diagrams here.\n").unwrap();
    let mcp = Mcp::start(&root, &core);
    Some((root, mcp))
}

#[test]
fn list_read_write_add() {
    let Some((root, mut mcp)) = setup("rw") else { return };
    let arch = root.join("docs/arch.md");

    let listing = ok(&mcp.call("list_diagrams", json!({})));
    let files = listing["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "only files with diagrams: {listing}");
    assert_eq!(files[0]["file"], "docs/arch.md");
    assert_eq!(files[0]["title"], "Shop");
    let d = &files[0]["diagrams"];
    assert_eq!((d[0]["index"].clone(), d[0]["kind"].clone(), d[0]["title"].clone()), (json!(1), json!("flowchart"), json!("Checkout")));
    assert_eq!((d[0]["start_line"].clone(), d[0]["layout"].clone()), (json!(7), json!(true)));
    assert_eq!((d[1]["kind"].clone(), d[1]["layout"].clone()), (json!("sequenceDiagram"), json!(false)));
    assert_eq!(ok(&mcp.call("list_diagrams", json!({"path": "docs/arch.md"})))["files"][0]["diagrams"].as_array().unwrap().len(), 2);

    let read = ok(&mcp.call("read_diagram", json!({"file": "docs/arch.md"})));
    assert_eq!(read["source"], "flowchart LR\n    web[Web] --> api[API]");
    assert_eq!(read["layout"], true);
    let version = read["version"].as_str().unwrap().to_string();
    assert_eq!(ok(&mcp.call("read_diagram", json!({"file": "docs/arch.md", "index": 2})))["source"], "sequenceDiagram\n  A->>B: hi");

    // write keeps the canvas positions and every other line of the file
    let source = "flowchart LR\n    web[Web] --> api[API]\n    api --> db[(Postgres)]";
    let w = ok(&mcp.call("write_diagram", json!({"file": "docs/arch.md", "index": 1, "source": source, "version": version})));
    assert_eq!((w["index"].clone(), w["layout"].clone(), w["created"].clone()), (json!(1), json!("kept"), json!(false)));
    let text = fs::read_to_string(&arch).unwrap();
    assert_eq!(
        text,
        ARCH.replace("    web[Web] --> api[API]\n", "    web[Web] --> api[API]\n    api --> db[(Postgres)]\n"),
        "only the diagram's lines changed, and its layout line is still there"
    );
    assert_eq!(ok(&mcp.call("read_diagram", json!({"file": "docs/arch.md"})))["version"], w["version"]);

    // a stale version is a tool error and leaves the file alone
    let stale = mcp.call("write_diagram", json!({"file": "docs/arch.md", "index": 1, "source": "flowchart LR\n  x", "version": version}));
    let msg = tool_error(&stale);
    assert!(msg.contains("Nothing was written") && msg.contains("read_diagram again"), "{msg}");
    assert!(msg.contains(w["version"].as_str().unwrap()), "the error gives the current version: {msg}");
    assert_eq!(fs::read_to_string(&arch).unwrap(), text);
    // ...as does a change made behind the agent's back
    fs::write(&arch, format!("{text}\nEdited in vim.\n")).unwrap();
    let r = mcp.call("write_diagram", json!({"file": "docs/arch.md", "index": 1, "source": "flowchart LR\n  x", "version": w["version"]}));
    assert!(tool_error(&r).contains("changed since you read it"));
    assert!(fs::read_to_string(&arch).unwrap().ends_with("Edited in vim.\n"));

    // other refusals
    let v = ok(&mcp.call("read_diagram", json!({"file": "docs/arch.md"})))["version"].clone();
    let fence = mcp.call("write_diagram", json!({"file": "docs/arch.md", "index": 1, "source": "graph TD\n```", "version": v}));
    assert!(tool_error(&fence).contains("fences"));
    assert!(tool_error(&mcp.call("read_diagram", json!({"file": "docs/arch.md", "index": 9}))).contains("no diagram #9"));
    assert!(tool_error(&mcp.call("read_diagram", json!({"file": "docs/missing.md"}))).contains("missing.md"));
    let outside = mcp.call("write_diagram", json!({"file": "../evil.md", "index": 1, "source": "graph TD", "version": v}));
    assert!(tool_error(&outside).contains("outside the project folder"));
    assert!(!root.parent().unwrap().join("evil.md").exists());

    // add creates the file and its folder, then appends
    let a =
        ok(&mcp
            .call("add_diagram", json!({"file": "docs/flows/login.md", "source": "sequenceDiagram\n  U->>S: log in", "title": "Login"})));
    assert_eq!((a["index"].clone(), a["created"].clone()), (json!(1), json!(true)));
    let b = ok(&mcp.call("add_diagram", json!({"file": "docs/flows/login.md", "source": "graph TD\n  a --> b"})));
    assert_eq!((b["index"].clone(), b["created"].clone()), (json!(2), json!(false)));
    let text = fs::read_to_string(root.join("docs/flows/login.md")).unwrap();
    assert!(text.starts_with("# login\n\n## Login\n\n```mermaid\nsequenceDiagram\n  U->>S: log in\n```\n"), "{text}");
    assert_eq!(ok(&mcp.call("list_diagrams", json!({"path": "docs/flows"})))["files"][0]["diagrams"].as_array().unwrap().len(), 2);

    // the legacy era reaches the same tools
    drop(mcp);
    let mut legacy = Mcp::start(&root, &core().unwrap());
    legacy.initialize("2025-06-18");
    let r = legacy.request("tools/call", json!({"name": "read_diagram", "arguments": {"file": "docs/arch.md", "index": 2}}));
    assert_eq!(r["result"]["structuredContent"]["source"], "sequenceDiagram\n  A->>B: hi");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn search_icons() {
    let Some((root, mut mcp)) = setup("icons") else { return };
    let none = ok(&mcp.call("search_icons", json!({"query": "zz-postgres"})));
    assert_eq!(none["icons"], json!([]));
    fs::create_dir_all(root.join(".mermdraw/icons")).unwrap();
    let pack = r#"{"prefix":"zz-test","icons":{"postgresql":{"body":""},"postgresql-wordmark":{"body":""},"redis":{"body":""}}}"#;
    fs::write(root.join(".mermdraw/icons/zz-test.json"), pack).unwrap();
    let found = ok(&mcp.call("search_icons", json!({"query": "zz-test postgres", "limit": 1})));
    assert_eq!(found["icons"][0]["icon"], "zz-test:postgresql");
    assert_eq!(found["icons"][0]["source"], "project");
    assert_eq!((found["icons"].as_array().unwrap().len(), found["total"].clone()), (1, json!(2)));
    assert!(found.get("hint").is_none());
    assert!(tool_error(&mcp.call("search_icons", json!({"query": " "}))).contains("query"));
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn open_diagram_reuses_or_starts_the_editor() {
    use std::io::{BufRead, BufReader, Read, Write};
    let Some((root, mut mcp)) = setup("open") else { return };
    let core = core().unwrap();

    // an editor already serving the project is reused
    let mut child = Command::new(&core)
        .arg(&root)
        .arg("--no-open")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let editor = Stop::Child(child);
    let mut banner = String::new();
    while !banner.contains("→ ") {
        assert!(out.read_line(&mut banner).unwrap() > 0, "editor exited: {banner}");
    }
    let o = ok(&mcp.call("open_diagram", json!({"file": "docs/arch.md", "index": 2, "open_browser": false})));
    drop(editor);
    assert_eq!(o["server"], "reused", "{o}");
    assert!(o.get("pid").is_none());
    assert_eq!(o["line"], 15);
    let url = o["url"].as_str().unwrap();
    assert!(url.starts_with("http://127.0.0.1:") && url.ends_with("/?file=docs/arch.md#d=2"), "{url}");
    assert!(banner.contains(url.split('?').next().unwrap()), "the URL is the running editor's: {banner} vs {url}");

    // with none running, one is started in the background and keeps running
    let o = ok(&mcp.call("open_diagram", json!({"file": "docs/arch.md", "line": 1, "open_browser": false})));
    assert_eq!(o["server"], "started", "{o}");
    let started = Stop::Pid(o["pid"].as_u64().unwrap().to_string());
    let port: u16 = o["url"].as_str().unwrap().trim_start_matches("http://127.0.0.1:").split('/').next().unwrap().parse().unwrap();
    assert!(o["url"].as_str().unwrap().ends_with("#d=1"));
    drop(mcp); // the editor outlives the MCP server
    let info = (|| {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
        write!(s, "GET /api/info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").ok()?;
        let mut body = String::new();
        s.read_to_string(&mut body).ok()?;
        Some(body)
    })();
    drop(started);
    let info = info.expect("the started editor answers");
    assert!(info.contains(&format!("\"root\":\"{}\"", root.display())), "{info}");

    let mut mcp = Mcp::start(&root, &core);
    assert!(tool_error(&mcp.call("open_diagram", json!({"file": "docs/nothere.md", "open_browser": false}))).contains("does not exist"));
    assert!(
        tool_error(&mcp.call("open_diagram", json!({"file": "docs/arch.md", "index": 7, "open_browser": false}))).contains("no diagram #7")
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn every_stdout_line_is_a_protocol_message() {
    // Mcp::recv asserts this for every line; drive each tool once, failures included, to cover their output paths
    let Some((root, mut mcp)) = setup("stdout") else { return };
    let calls: Vec<(&str, Value)> = vec![
        ("list_diagrams", json!({"path": "docs"})),
        ("read_diagram", json!({"file": "docs/arch.md"})),
        ("write_diagram", json!({"file": "docs/arch.md", "index": 1, "source": "graph TD", "version": "x"})),
        ("add_diagram", json!({"file": "docs/notes.md", "source": "graph TD\n  n"})),
        ("search_icons", json!({"query": "rust"})),
    ];
    for (tool, args) in calls {
        let r = mcp.call(tool, args);
        assert!(r["content"][0]["text"].is_string(), "{tool}: {r}");
    }
    assert!(mcp.close());
    fs::remove_dir_all(&root).unwrap();
}
