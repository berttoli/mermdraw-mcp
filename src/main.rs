//! mermdraw-mcp — an MCP server that lets coding agents list, read, write and open the mermaid diagrams in a
//! project's markdown. It is a mermdraw addon (`mermdraw mcp`) and does every read and write through mermdraw's
//! `--json` commands. Only protocol messages go to stdout; logs go to stderr.

mod core;
mod server;

use rmcp::ServiceExt;
use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let mut root: Option<PathBuf> = None;
    let mut args = env::args_os().skip(1);
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("-h" | "--help") => return print_help(),
            Some("-V" | "--version") => return println!("mermdraw-mcp {VERSION}"),
            Some("--root") => root = Some(args.next().map(PathBuf::from).unwrap_or_else(|| die("--root needs a folder"))),
            _ => die(&format!("unknown argument {} (try --help)", a.to_string_lossy())),
        }
    }
    let root = match root {
        Some(dir) => dir.canonicalize().unwrap_or_else(|e| die(&format!("--root {}: {e}", dir.display()))),
        None => env::current_dir().and_then(|d| core::project_root(&d)).unwrap_or_else(|e| die(&format!("current folder: {e}"))),
    };
    let bin = env::var_os("MERMDRAW").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| "mermdraw".into());
    eprintln!("mermdraw-mcp {VERSION}: project {}, mermdraw at {}", root.display(), bin.display());
    if std::io::stdin().is_terminal() {
        eprintln!("mermdraw-mcp: this is an MCP server for coding agents; it reads JSON-RPC on stdin (see --help for setup)");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap_or_else(|e| die(&e.to_string()));
    runtime.block_on(async {
        let server = server::Mermdraw::new(core::Core { bin, root });
        match server.serve(rmcp::transport::stdio()).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            Err(e) => die(&e.to_string()),
        }
    });
    // tokio's stdin reader can sit in a blocking read; the client has gone, so don't wait for it
    process::exit(0);
}

fn print_help() {
    println!(
        "mermdraw-mcp {VERSION} — MCP server for the mermaid diagrams in your markdown, over stdio

USAGE
  mermdraw mcp [--root DIR]     as a mermdraw addon (mermdraw addons add mcp)
  mermdraw-mcp [--root DIR]     directly

  --root DIR    the project folder the tools may read and write (default: the git repository
                around the current folder, else the current folder)
  -h, --help    show this help
  -V, --version show version

TOOLS
  list_diagrams, read_diagram, write_diagram, add_diagram, search_icons, open_diagram

SETUP
  Claude Code   claude mcp add mermdraw -- mermdraw mcp
  Codex         codex mcp add mermdraw -- mermdraw mcp
  Cursor, Zed   see https://github.com/berttoli/mermdraw-mcp#setup

Reads and writes go through mermdraw's --json commands (mermdraw 0.2 or later): $MERMDRAW,
which `mermdraw mcp` sets, else mermdraw on PATH."
    );
}

fn die(msg: &str) -> ! {
    eprintln!("mermdraw-mcp: {msg}");
    process::exit(1)
}
