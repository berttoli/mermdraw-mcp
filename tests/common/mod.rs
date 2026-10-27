//! Drives the built mermdraw-mcp binary over stdio, the way an MCP client does.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::Duration;

pub const MODERN: &str = "2026-07-28";
const WAIT: Duration = Duration::from_secs(20);

pub struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next_id: u64,
}

impl Mcp {
    /// Starts the server in `root`, using `core` as mermdraw (a path that doesn't exist when the test needs none).
    pub fn start(root: &Path, core: &Path) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mermdraw-mcp"))
            .current_dir(root)
            .env("MERMDRAW", core)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start mermdraw-mcp");
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Mcp { stdin: child.stdin.take(), child, lines, next_id: 1 }
    }

    pub fn send(&mut self, msg: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    /// The next message the server writes. Every line on stdout must be one JSON-RPC 2.0 message.
    pub fn recv(&mut self) -> Value {
        let line = match self.lines.recv_timeout(WAIT) {
            Ok(l) => l,
            Err(RecvTimeoutError::Timeout) => panic!("no message from mermdraw-mcp within {WAIT:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("mermdraw-mcp closed stdout"),
        };
        let msg: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"));
        assert_eq!(msg["jsonrpc"], "2.0", "not a JSON-RPC 2.0 message: {line}");
        msg
    }

    /// Sends a request and returns its response.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let msg = self.recv();
            if msg["id"] == json!(id) {
                return msg;
            }
            assert!(msg.get("id").is_none(), "response to a request that wasn't sent: {msg}");
        }
    }

    /// A 2026-07-28 request: the protocol version, client and capabilities ride in `_meta`.
    pub fn modern(&mut self, method: &str, mut params: Value) -> Value {
        params["_meta"] = meta(MODERN);
        self.request(method, params)
    }

    /// Calls a tool the 2026-07-28 way and returns the result (panics on a JSON-RPC error).
    pub fn call(&mut self, tool: &str, args: Value) -> Value {
        let resp = self.modern("tools/call", json!({"name": tool, "arguments": args}));
        assert!(resp.get("error").is_none(), "{tool}: {resp}");
        resp["result"].clone()
    }

    /// The legacy handshake: initialize, then the initialized notification.
    pub fn initialize(&mut self, version: &str) -> Value {
        let resp = self.request(
            "initialize",
            json!({"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "legacy-test", "version": "1.0"}}),
        );
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        resp
    }

    /// Closes stdin and waits for the server to exit; true when it exits within a few seconds.
    pub fn close(mut self) -> bool {
        drop(self.stdin.take());
        for _ in 0..50 {
            if self.child.try_wait().unwrap().is_some() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn meta(version: &str) -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientInfo": {"name": "mermdraw-mcp-tests", "version": "1.0"},
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// The structured result of a successful tool call.
pub fn ok(result: &Value) -> Value {
    assert_eq!(result["isError"], json!(false), "tool failed: {result}");
    assert_eq!(result["resultType"], "complete");
    let structured = result["structuredContent"].clone();
    // the same data as JSON text, for clients that only read `content`
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, structured);
    structured
}

/// The message of a tool execution error.
pub fn tool_error(result: &Value) -> String {
    assert_eq!(result["isError"], json!(true), "expected a tool error: {result}");
    result["content"][0]["text"].as_str().unwrap().to_string()
}

/// A fresh project folder (a git repository) for one test.
pub fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mermdraw-mcp-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    dir.canonicalize().unwrap()
}
