//! The MCP tools. Each one is a thin wrapper around a mermdraw `--json` command.

use crate::core::{Core, Want};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    CacheScope, DiscoverResult, Implementation, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, Json, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The tool list and server description never change while the server runs.
const LIST_TTL_MS: u64 = 60 * 60 * 1000;

const INSTRUCTIONS: &str = "mermdraw keeps mermaid diagrams in ```mermaid blocks inside markdown files and edits them on a visual \
canvas. Use these tools instead of editing those blocks with file tools: they keep the canvas positions people arranged in \
mermdraw, change only the diagram you name, and refuse to overwrite edits made since you read it. Typical flow: list_diagrams \
to find diagrams, read_diagram for the source and version, write_diagram with that version (or add_diagram for a new one), \
search_icons for logos, then open_diagram to show the result to the user. Paths are relative to the project folder.";

#[derive(Clone)]
pub struct Mermdraw {
    core: Arc<Core>,
    tool_router: ToolRouter<Self>,
}

// ---------------------------------------------------------------- what mermdraw's --json commands print

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Listing {
    /// Markdown and mermaid files with their diagrams (for a folder: only files that have diagrams)
    pub files: Vec<FileDiagrams>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FileDiagrams {
    /// Path relative to the project folder
    pub file: String,
    /// The file's version, for write_diagram
    pub version: String,
    /// The file's first `#` heading
    pub title: Option<String>,
    pub diagrams: Vec<DiagramInfo>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DiagramInfo {
    /// 1-based number of the diagram in its file
    pub index: usize,
    /// The diagram type keyword: flowchart, graph, sequenceDiagram, classDiagram, architecture-beta, …
    pub kind: String,
    /// The diagram's frontmatter title, or the heading above it
    pub title: Option<String>,
    /// 1-based line of the opening ``` fence
    pub start_line: usize,
    /// 1-based line of the closing ``` fence
    pub end_line: usize,
    /// Whether the diagram has canvas positions from the mermdraw editor (they are kept on write)
    pub layout: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Diagram {
    pub file: String,
    /// The file's version: pass it to write_diagram
    pub version: String,
    pub index: usize,
    pub kind: String,
    pub title: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    /// Whether the diagram has canvas positions; they are not part of `source` and are kept on write
    pub layout: bool,
    /// The mermaid source, without the ``` fences and without the `%% mermdraw:layout` line
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Written {
    pub file: String,
    /// The diagram's 1-based number in the file
    pub index: usize,
    /// The file's new version, for the next write
    pub version: String,
    /// "kept": the diagram's canvas positions were carried over; "input": the source brought its own; "none": it has none
    pub layout: String,
    /// Whether the file was created
    pub created: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct IconSearch {
    pub query: String,
    /// The installed icon packs
    pub packs: Vec<IconPack>,
    /// Matching icons, best first
    pub icons: Vec<Icon>,
    /// How many icons matched in all (`icons` holds at most `limit`)
    pub total: usize,
    /// What to do when no packs are installed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct IconPack {
    pub name: String,
    /// "project" (vendored into the repository's .mermdraw/icons) or "global"
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Icon {
    /// What to write in the diagram, e.g. "logos:postgresql"
    pub icon: String,
    pub pack: String,
    pub name: String,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Opened {
    /// The editor page showing the diagram
    pub url: String,
    pub file: String,
    pub line: usize,
    /// "reused": an editor was already running for this project; "started": this call started one in the background
    pub server: String,
    /// Process id of the editor this call started
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

// ---------------------------------------------------------------- tool arguments

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArgs {
    /// A markdown file, or a folder to search (default: the whole project)
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    /// The markdown (.md) or mermaid (.mmd) file
    pub file: String,
    /// 1-based diagram number, as list_diagrams gives it (default 1)
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteArgs {
    pub file: String,
    /// 1-based diagram number
    pub index: usize,
    /// The complete new mermaid source, without ``` fences and without a `%% mermdraw:layout` line
    pub source: String,
    /// The file version read_diagram (or list_diagrams) returned
    pub version: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddArgs {
    /// The markdown (.md) file to append to; created if it doesn't exist
    pub file: String,
    /// The mermaid source, without ``` fences
    pub source: String,
    /// A heading to put above the diagram (`## title`)
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Words to look for in icon names, e.g. "postgres", "aws lambda", "kubernetes"
    pub query: String,
    /// At most this many icons (default 20)
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenArgs {
    pub file: String,
    /// 1-based diagram number to show (default: the first)
    #[serde(default)]
    pub index: Option<usize>,
    /// Or a 1-based line: shows the diagram at or below it
    #[serde(default)]
    pub line: Option<usize>,
    /// Open it in the user's browser (default true); false only returns the URL
    #[serde(default)]
    pub open_browser: Option<bool>,
}

#[tool_router]
impl Mermdraw {
    pub fn new(core: Core) -> Self {
        Mermdraw { core: Arc::new(core), tool_router: Self::tool_router() }
    }

    #[tool(
        description = "List the mermaid diagrams in a markdown file, or in every markdown file under a folder (default: the whole \
                       project). Gives each file's version and, for each diagram, its 1-based index, type, title and fence lines.",
        annotations(title = "List diagrams", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_diagrams(&self, Parameters(args): Parameters<ListArgs>) -> Result<Json<Listing>, String> {
        let path = self.core.resolve(args.path.as_deref().unwrap_or("."), Want::DocOrDir)?;
        self.core.json(&["ls", &path], None).await.map(Json).map_err(|e| e.message())
    }

    #[tool(
        description = "Read one mermaid diagram from a markdown (.md) or mermaid (.mmd) file: its source, without the ``` fences, \
                       and the file's version to pass to write_diagram. Canvas positions arranged in the mermdraw editor are left \
                       out of the source (`layout` says whether there are any) and survive write_diagram on their own.",
        annotations(title = "Read a diagram", read_only_hint = true, open_world_hint = false)
    )]
    async fn read_diagram(&self, Parameters(args): Parameters<ReadArgs>) -> Result<Json<Diagram>, String> {
        let file = self.core.resolve(&args.file, Want::Doc)?;
        let n = args.index.unwrap_or(1).to_string();
        self.core.json(&["cat", &file, &n], None).await.map(Json).map_err(|e| e.message())
    }

    #[tool(
        description = "Replace diagram `index` of a file with a new mermaid source (the whole diagram, without ``` fences or a \
                       `%% mermdraw:layout` line). `version` must be the one read_diagram returned: if the file changed since, \
                       nothing is written and the error says so; read the diagram again and redo the change. Only that \
                       diagram's lines change; the surrounding markdown and the diagram's canvas positions are kept.",
        annotations(
            title = "Write a diagram",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn write_diagram(&self, Parameters(args): Parameters<WriteArgs>) -> Result<Json<Written>, String> {
        let file = self.core.resolve(&args.file, Want::Doc)?;
        if args.index == 0 {
            return Err("index is 1-based: the first diagram is 1".into());
        }
        let n = args.index.to_string();
        self.core.json(&["write", &file, &n, "--base", &args.version], Some(&args.source)).await.map(Json).map_err(|e| e.message())
    }

    #[tool(
        description = "Append a new mermaid diagram to a markdown file, under a `## title` heading when one is given. Creates the \
                       file and its folders if needed. Returns the new diagram's index and the file's version.",
        annotations(
            title = "Add a diagram",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_diagram(&self, Parameters(args): Parameters<AddArgs>) -> Result<Json<Written>, String> {
        let file = self.core.resolve(&args.file, Want::Doc)?;
        let mut cmd = vec!["write", file.as_str(), "--append"];
        if let Some(t) = args.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            cmd.extend(["--title", t]);
        }
        self.core.json(&cmd, Some(&args.source)).await.map(Json).map_err(|e| e.message())
    }

    #[tool(
        description = "Find technology and product icons (logos) in the icon packs installed for mermdraw, e.g. \"postgres\", \
                       \"aws lambda\", \"kubernetes\". Write a result's `icon` into a flowchart node as \
                       `db@{ icon: \"logos:postgresql\", label: \"Orders DB\" }`, or into an architecture-beta diagram as \
                       `service db(logos:postgresql)[Orders DB]`. If no packs are installed, the result says how to add some.",
        annotations(title = "Search icons", read_only_hint = true, open_world_hint = false)
    )]
    async fn search_icons(&self, Parameters(args): Parameters<SearchArgs>) -> Result<Json<IconSearch>, String> {
        if args.query.trim().is_empty() {
            return Err("give a query, e.g. \"postgres\"".into());
        }
        let limit = args.limit.unwrap_or(20).clamp(1, 200).to_string();
        let mut found: IconSearch =
            self.core.json(&["icons", "search", &args.query, "--limit", &limit], None).await.map_err(|e| e.message())?;
        if found.packs.is_empty() {
            found.hint = Some(
                "No icon packs are installed. Ask the user to run `mermdraw icons add logos devicon` (or add `--vendor` to keep \
                 them in the repository), then search again."
                    .into(),
            );
        }
        Ok(Json(found))
    }

    #[tool(
        description = "Show a diagram to the user in mermdraw's visual editor, in their browser. Reuses the mermdraw editor already \
                       running for this project, or starts one in the background. Returns the editor URL.",
        annotations(
            title = "Open in the editor",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn open_diagram(&self, Parameters(args): Parameters<OpenArgs>) -> Result<Json<Opened>, String> {
        let file = self.core.resolve(&args.file, Want::Doc)?;
        if !self.core.root.join(&file).is_file() {
            return Err(format!("{file} does not exist"));
        }
        let line = match (args.index, args.line) {
            (Some(index), _) => {
                let listing: Listing = self.core.json(&["ls", &file], None).await.map_err(|e| e.message())?;
                let diagrams = listing.files.into_iter().next().map(|f| f.diagrams).unwrap_or_default();
                let found = diagrams.iter().find(|d| d.index == index);
                found.map(|d| d.start_line).ok_or_else(|| format!("{file} has no diagram #{index} (it has {})", diagrams.len()))?
            }
            (None, Some(line)) => line.max(1),
            (None, None) => 1,
        };
        let (url, pid) = self.core.open(&file, line, args.open_browser.unwrap_or(true)).await?;
        let server = if pid.is_some() { "started" } else { "reused" }.to_string();
        Ok(Json(Opened { url, file, line, server, pid }))
    }
}

#[tool_handler]
impl ServerHandler for Mermdraw {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("mermdraw-mcp", env!("CARGO_PKG_VERSION")).with_title("mermdraw"))
            .with_instructions(INSTRUCTIONS)
    }

    async fn discover(&self, _context: RequestContext<RoleServer>) -> Result<DiscoverResult, ErrorData> {
        Ok(DiscoverResult::from_server_info(self.supported_protocol_versions().into_owned(), self.get_info())
            .with_ttl_ms(LIST_TTL_MS)
            .with_cache_scope(CacheScope::Public))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = ListToolsResult::with_all_items(self.tool_router.list_all());
        // caching hints exist from 2026-07-28 on; legacy clients get the plain list
        Ok(if context.protocol_version().is_some_and(|v| v >= ProtocolVersion::V_2026_07_28) {
            tools.with_ttl_ms(LIST_TTL_MS).with_cache_scope(CacheScope::Public)
        } else {
            tools
        })
    }
}
