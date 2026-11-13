//! The mermdraw core, run as a subprocess. Every read and write of a diagram goes through its `--json` commands
//! (mermdraw's docs/ADDONS.md), so this addon never reads or writes markdown itself.

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// How long a JSON command may take before it is killed.
const TIMEOUT: Duration = Duration::from_secs(60);
/// The version of mermdraw's addon contract this addon speaks; every JSON output carries it as `contract`.
const CONTRACT: u64 = 1;
/// How long `mermdraw FILE:LINE` may take to print the editor URL.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

pub struct Core {
    /// `$MERMDRAW` (set when this runs as `mermdraw mcp`), else `mermdraw` on PATH
    pub bin: PathBuf,
    /// the folder the tools may touch, canonical; relative paths in tool calls start here. Err: why there is none,
    /// which every tool call answers with
    pub root: Result<PathBuf, String>,
}

#[derive(Debug, PartialEq)]
pub enum CoreError {
    /// the file changed since the version the caller read; `version` is its version now
    Conflict {
        message: String,
        version: String,
    },
    Failed(String),
}

impl CoreError {
    pub fn message(&self) -> String {
        match self {
            CoreError::Conflict { version, .. } => format!(
                "Nothing was written: the file changed since you read it (its version is now {version}). Call read_diagram \
                 again, redo your change on the current source, and write it with the new version."
            ),
            CoreError::Failed(m) => m.clone(),
        }
    }
}

/// What a path argument must name.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Want {
    /// a markdown (.md, .markdown) or mermaid (.mmd, .mermaid) file
    Doc,
    /// a document or a folder
    DocOrDir,
}

pub fn is_doc(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [".md", ".markdown", ".mmd", ".mermaid"].iter().any(|e| p.ends_with(e))
}

impl Core {
    /// The project folder, or why there is none.
    pub fn root(&self) -> Result<&Path, String> {
        self.root.as_deref().map_err(Clone::clone)
    }

    /// Runs `mermdraw ARGS… --json` in the root, with `stdin` as its input, and parses what it prints.
    pub async fn json<T: DeserializeOwned>(&self, args: &[&str], stdin: Option<&str>) -> Result<T, CoreError> {
        let root = self.root().map_err(CoreError::Failed)?;
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .arg("--json")
            .current_dir(root)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| self.spawn_error(e))?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            let input = input.to_string();
            // written alongside the wait, so a large source can't deadlock against mermdraw's output
            tokio::spawn(async move {
                let _ = pipe.write_all(input.as_bytes()).await;
            });
        }
        let out = tokio::time::timeout(TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| CoreError::Failed(format!("mermdraw {} did not finish within {}s", args.join(" "), TIMEOUT.as_secs())))?
            .map_err(|e| CoreError::Failed(format!("could not run {}: {e}", self.bin.display())))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) else {
            if stderr.contains("unknown option --json") {
                return Err(CoreError::Failed(format!(
                    "{} is too old: mermdraw-mcp needs mermdraw 0.2 or later for its --json commands; update mermdraw",
                    self.bin.display()
                )));
            }
            let why = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
            return Err(CoreError::Failed(format!("mermdraw {} failed: {why}", args.join(" "))));
        };
        match value.get("contract").and_then(Value::as_u64) {
            Some(CONTRACT) => {}
            Some(theirs) => {
                let update = if theirs > CONTRACT { "mermdraw-mcp (mermdraw addons add mcp)" } else { "mermdraw" };
                return Err(CoreError::Failed(format!(
                    "{} speaks version {theirs} of the addon contract and this mermdraw-mcp speaks {CONTRACT}: update {update}",
                    self.bin.display()
                )));
            }
            None => {
                return Err(CoreError::Failed(format!(
                    "{} is too old: mermdraw-mcp needs mermdraw 0.2 or later (its JSON has no addon contract version)",
                    self.bin.display()
                )));
            }
        }
        if let Some(err) = value.get("error") {
            let text = |k: &str| err.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            return Err(if text("code") == "conflict" {
                CoreError::Conflict { message: text("message"), version: text("version") }
            } else {
                CoreError::Failed(text("message"))
            });
        }
        if !out.status.success() {
            return Err(CoreError::Failed(format!("mermdraw {} exited with {}", args.join(" "), out.status)));
        }
        serde_json::from_value(value).map_err(|e| CoreError::Failed(format!("unexpected output from mermdraw {}: {e}", args.join(" "))))
    }

    /// Runs `mermdraw FILE:LINE`, which opens the diagram on the editor already serving the project, or becomes that
    /// editor. Returns the editor URL, and the process id of an editor it started, which keeps running on its own.
    pub async fn open(&self, file: &str, line: usize, browser: bool) -> Result<(String, Option<u32>), String> {
        let root = self.root()?;
        let mut cmd = std::process::Command::new(&self.bin);
        cmd.arg(format!("{file}:{line}")).current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if !browser {
            cmd.arg("--no-open");
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // a started editor outlives this server and the client that started it
        }
        let mut child = Command::from(cmd).spawn().map_err(|e| self.spawn_error(e).message())?;
        let mut out = BufReader::new(child.stdout.take().expect("piped")).lines();
        let mut reused = false;
        // `mermdraw VERSION  root DIR` or `… already running for DIR`, then `  → URL`
        let url = tokio::time::timeout(OPEN_TIMEOUT, async {
            while let Ok(Some(l)) = out.next_line().await {
                reused |= l.contains("already running");
                if let Some((_, rest)) = l.split_once("→ ") {
                    return rest.split_whitespace().next().map(str::to_string);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        let mut stderr = child.stderr.take().expect("piped");
        let Some(url) = url else {
            let _ = child.start_kill();
            let mut why = String::new();
            let _ = tokio::time::timeout(Duration::from_secs(2), stderr.read_to_string(&mut why)).await;
            let _ = child.wait().await;
            let why = why.trim().trim_start_matches("mermdraw: ");
            return Err(format!("mermdraw could not open {file}: {}", if why.is_empty() { "it printed no editor URL" } else { why }));
        };
        let pid = if reused { None } else { child.id() };
        // keep the editor's output flowing (to this server's log) and reap it when it exits
        tokio::spawn(async move {
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::stderr()).await;
            while let Ok(Some(_)) = out.next_line().await {}
            let _ = child.wait().await;
        });
        Ok((url, pid))
    }

    fn spawn_error(&self, e: io::Error) -> CoreError {
        if e.kind() == io::ErrorKind::NotFound {
            CoreError::Failed(format!(
                "cannot find mermdraw ({}): install it (https://github.com/berttoli/mermdraw#install), or set MERMDRAW to its path",
                self.bin.display()
            ))
        } else {
            CoreError::Failed(format!("could not run {}: {e}", self.bin.display()))
        }
    }

    /// The path to pass mermdraw for a tool's `path` argument, relative to the root. Refuses paths outside the root,
    /// whether by `..`, by an absolute path elsewhere or through a symlink, and anything but markdown and mermaid
    /// files (and folders, for `Want::DocOrDir`).
    pub fn resolve(&self, path: &str, want: Want) -> Result<String, String> {
        let root = self.root()?;
        let path = path.trim();
        if path.is_empty() {
            return Err("give a path, relative to the project folder".into());
        }
        let given = Path::new(path);
        let joined = if given.is_absolute() { given.to_path_buf() } else { root.join(given) };
        let mut full = PathBuf::new();
        for c in joined.components() {
            match c {
                Component::CurDir => {}
                Component::ParentDir => {
                    full.pop();
                }
                other => full.push(other),
            }
        }
        let outside = || format!("{path} is outside the project folder {}", root.display());
        if !full.starts_with(root) {
            return Err(outside());
        }
        // the deepest part of the path that exists must not lead out of the root through a symlink
        let existing = full.ancestors().find(|p| p.exists()).unwrap_or(root);
        if !existing.canonicalize().map(|c| c.starts_with(root)).unwrap_or(false) {
            return Err(outside());
        }
        let rel: Vec<String> =
            full.strip_prefix(root).unwrap().components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let rel = if rel.is_empty() { ".".to_string() } else { rel.join("/") };
        let is_dir = full.is_dir();
        match want {
            Want::DocOrDir if is_dir => Ok(rel),
            _ if is_doc(&rel) && !is_dir => Ok(rel),
            Want::Doc => Err(format!("{path} is not a markdown (.md) or mermaid (.mmd) file")),
            Want::DocOrDir => Err(format!("{path} is not a folder or a markdown (.md) or mermaid (.mmd) file")),
        }
    }
}

/// The folder the tools work in when no `--root` is given: the git repository around `dir`, else `dir` itself. Never
/// the home folder, a folder above it or the top of the disk, where desktop clients often start servers: that would hand
/// agents every markdown file on the machine. Err says why there is no project and what to do.
pub fn project_root(dir: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    let dir = dir.canonicalize().map_err(|e| format!("No project folder: the current folder {}: {e}", dir.display()))?;
    let root = dir.ancestors().find(|d| d.join(".git").exists()).unwrap_or(&dir);
    let home = home.map(|h| h.canonicalize().unwrap_or_else(|_| h.to_path_buf()));
    let what = if root.parent().is_none() {
        "the top of the disk"
    } else if home.as_deref() == Some(root) {
        "your home folder"
    } else if home.as_deref().is_some_and(|h| h.starts_with(root)) {
        "a folder above your home folder"
    } else {
        return Ok(root.to_path_buf());
    };
    let started = if root == dir {
        format!("{}, {what}", dir.display())
    } else {
        format!("{}, in the git repository {}, {what}", dir.display(), root.display())
    };
    Err(format!(
        "No project folder: mermdraw-mcp was started in {started}, and won't let agents reach every markdown file there. \
         Add `--root /path/to/project` to its arguments in the MCP client's settings, or start the client in the project."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mermdraw-mcp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("docs")).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn paths_stay_inside_the_root() {
        let root = scratch("resolve");
        fs::write(root.join("docs/a.md"), "# a\n").unwrap();
        fs::write(root.join(".env"), "SECRET=1\n").unwrap();
        let core = Core { bin: "mermdraw".into(), root: Ok(root.clone()) };
        assert_eq!(core.resolve("docs/a.md", Want::Doc), Ok("docs/a.md".into()));
        assert_eq!(core.resolve("./docs/../docs/a.md", Want::Doc), Ok("docs/a.md".into()));
        assert_eq!(core.resolve(&root.join("docs/a.md").to_string_lossy(), Want::Doc), Ok("docs/a.md".into()));
        assert_eq!(core.resolve("docs/new/b.MD", Want::Doc), Ok("docs/new/b.MD".into())); // may not exist yet
        assert_eq!(core.resolve(".", Want::DocOrDir), Ok(".".into()));
        assert_eq!(core.resolve("docs", Want::DocOrDir), Ok("docs".into()));
        assert!(core.resolve("docs", Want::Doc).is_err());
        assert!(core.resolve(".env", Want::Doc).is_err());
        assert!(core.resolve("", Want::Doc).is_err());
        assert!(core.resolve("../outside.md", Want::Doc).unwrap_err().contains("outside the project folder"));
        assert!(core.resolve("docs/../../outside.md", Want::Doc).is_err());
        assert!(core.resolve("/etc/hosts.md", Want::Doc).is_err());
        #[cfg(unix)]
        {
            let elsewhere = scratch("elsewhere");
            std::os::unix::fs::symlink(&elsewhere, root.join("docs/link")).unwrap();
            assert!(core.resolve("docs/link/x.md", Want::Doc).is_err());
            fs::remove_dir_all(&elsewhere).unwrap();
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_root_is_the_git_repository() {
        let root = scratch("root");
        fs::create_dir_all(root.join(".git")).unwrap();
        assert_eq!(project_root(&root.join("docs"), None).unwrap(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn never_the_home_folder_or_the_top_of_the_disk() {
        let home = scratch("home");
        let refused = |dir: &Path| project_root(dir, Some(&home)).unwrap_err();
        // a folder inside the home folder is a project, in a git repository or not
        assert_eq!(project_root(&home.join("docs"), Some(&home)).unwrap(), home.join("docs"));
        let e = refused(&home);
        assert!(e.contains(&format!("started in {}, your home folder", home.display())), "{e}");
        assert!(e.contains("--root"), "it says what to do: {e}");
        assert!(refused(home.parent().unwrap()).contains("a folder above your home folder"));
        let top = home.ancestors().last().unwrap();
        assert!(project_root(top, None).unwrap_err().contains("the top of the disk"));
        // nor does a git repository of the whole home folder (dotfiles) make it one
        fs::create_dir_all(home.join(".git")).unwrap();
        let e = refused(&home.join("docs"));
        assert!(e.contains(&format!("in the git repository {}, your home folder", home.display())), "{e}");
        fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn without_a_project_every_call_says_why() {
        let why = "No project folder: add --root".to_string();
        let core = Core { bin: "mermdraw".into(), root: Err(why.clone()) };
        assert_eq!(core.resolve("docs/a.md", Want::Doc), Err(why.clone()));
        assert_eq!(core.resolve(".", Want::DocOrDir), Err(why.clone()));
        assert_eq!(core.json::<Value>(&["ls", "."], None).await.unwrap_err(), CoreError::Failed(why.clone()));
        assert_eq!(core.open("docs/a.md", 1, false).await.unwrap_err(), why);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reads_core_output_and_errors() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("json");
        let fake = root.join("fake-mermdraw");
        let script = r#"#!/bin/sh
case "$1" in
  ok) echo '{"mermdraw":"0.2.0","contract":1,"value":7}' ;;
  conflict) echo '{"mermdraw":"0.2.0","contract":1,"error":{"code":"conflict","message":"a.md changed","version":"9-9"}}'; exit 3 ;;
  fail) echo '{"mermdraw":"0.2.0","contract":1,"error":{"code":"error","message":"a.md has no diagram #4"}}'; exit 1 ;;
  old) echo "mermdraw: unknown option --json (try --help)" >&2; exit 1 ;;
  newer) echo '{"mermdraw":"0.9.0","contract":2,"value":7}' ;;
  older) echo '{"mermdraw":"0.2.0","contract":0,"value":7}' ;;
  nocontract) echo '{"mermdraw":"0.2.0-dev","value":7}' ;;
  echo) cat ;;
esac
"#;
        fs::write(&fake, script).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let core = Core { bin: fake, root: Ok(root.clone()) };
        let v: Value = core.json(&["ok"], None).await.unwrap();
        assert_eq!(v["value"], 7);
        let e = core.json::<Value>(&["conflict"], None).await.unwrap_err();
        assert_eq!(e, CoreError::Conflict { message: "a.md changed".into(), version: "9-9".into() });
        assert!(e.message().contains("Call read_diagram again"));
        assert_eq!(core.json::<Value>(&["fail"], None).await.unwrap_err(), CoreError::Failed("a.md has no diagram #4".into()));
        assert!(core.json::<Value>(&["old"], None).await.unwrap_err().message().contains("needs mermdraw 0.2 or later"));
        let newer = core.json::<Value>(&["newer"], None).await.unwrap_err().message();
        assert!(newer.contains("version 2 of the addon contract") && newer.contains("update mermdraw-mcp"), "{newer}");
        assert!(core.json::<Value>(&["older"], None).await.unwrap_err().message().ends_with("update mermdraw"));
        assert!(core.json::<Value>(&["nocontract"], None).await.unwrap_err().message().contains("needs mermdraw 0.2 or later"));
        let big = format!("{{\"contract\":1,\"x\":\"{}\"}}", "a".repeat(1 << 20)); // larger than a pipe buffer
        let v: Value = core.json(&["echo"], Some(&big)).await.unwrap();
        assert_eq!(v["x"].as_str().unwrap().len(), 1 << 20);
        let missing = Core { bin: root.join("nope"), root: Ok(root.clone()) };
        assert!(missing.json::<Value>(&["ok"], None).await.unwrap_err().message().contains("cannot find mermdraw"));
        fs::remove_dir_all(&root).unwrap();
    }
}
