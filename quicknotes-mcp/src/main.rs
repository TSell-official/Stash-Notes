//! `quicknotes-mcp` — a Model Context Protocol server over stdio.
//!
//! Any MCP client (Claude Desktop, Cursor, Qwen Code, custom agents) can add
//! this binary as a server and immediately gain tools to create, search,
//! list, read, edit and delete notes in the same SQLite + FTS5 database the
//! desktop app and CLI use.
//!
//! Run with `quicknotes-mcp` and point your MCP client's config at it.

use async_trait::async_trait;
use quicknotes_core as core;
use rusqlite::Connection;
use rust_mcp_sdk::{
    error::SdkResult,
    macros,
    mcp_server::{server_runtime, McpServerOptions, ServerHandler},
    schema::{
        schema_utils::CallToolError, CallToolRequestParams, CallToolResult, Implementation,
        ListToolsResult, ListToolsResultCacheScope, PaginatedRequestParams, RpcError,
        ServerCapabilities, ServerCapabilitiesTools, ServerResult,
    },
    McpServer, RequestContext, ServerDetails, StdioTransport, ToMcpServerHandler, TransportOptions,
};
use std::sync::{Arc, Mutex};

/// SQLite connections are `!Send`/`!Sync`. This server runs on a
/// single-threaded tokio runtime (`flavor = "current_thread"`), so the
/// connection is never actually shared across OS threads; this newtype
/// documents that invariant while satisfying the SDK's `Send + Sync`
/// handler bound.
struct SharedConn(Mutex<Connection>);

// SAFETY: only touched from the current-thread runtime; see above.
unsafe impl Send for SharedConn {}
unsafe impl Sync for SharedConn {}

impl SharedConn {
    fn new(conn: Connection) -> Self {
        Self(Mutex::new(conn))
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().expect("database mutex poisoned")
    }
}

fn io_err(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::other(msg.into())
}

/* ---------------------------- tool definitions ---------------------------- */

#[macros::mcp_tool(
    name = "note_create",
    description = "Create a new note with a title, body content, and optional comma-separated tags. Returns the new numeric note id."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, macros::JsonSchema)]
pub struct NoteCreateTool {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: String,
}

#[macros::mcp_tool(
    name = "note_search",
    description = "Full-text search across all notes using SQLite FTS5 (BM25 ranked). Returns matching notes with highlighted snippets."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, macros::JsonSchema)]
pub struct NoteSearchTool {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[macros::mcp_tool(
    name = "note_list",
    description = "List the most recently updated notes (pinned first), each with id, title, snippet and tags."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, macros::JsonSchema)]
pub struct NoteListTool {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[macros::mcp_tool(
    name = "note_get",
    description = "Fetch one full note by its numeric id, including complete body content."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, macros::JsonSchema)]
pub struct NoteGetTool {
    pub id: i64,
}

#[macros::mcp_tool(
    name = "note_update",
    description = "Update an existing note's title, content and/or tags (omitted fields keep their current value)."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, macros::JsonSchema)]
pub struct NoteUpdateTool {
    pub id: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
}

#[macros::mcp_tool(
    name = "note_delete",
    description = "Delete a note by its numeric id. This also removes it from the full-text index."
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, macros::JsonSchema)]
pub struct NoteDeleteTool {
    pub id: i64,
}

fn default_limit() -> i64 {
    20
}

/* ------------------------------ server logic ------------------------------ */

struct QuickNotesHandler {
    db: Arc<SharedConn>,
}

impl QuickNotesHandler {
    fn parse_args<T: serde::de::DeserializeOwned>(
        params: &CallToolRequestParams,
    ) -> Result<T, CallToolError> {
        serde_json::from_value(serde_json::Value::Object(
            params.arguments.clone().unwrap_or_default(),
        ))
        .map_err(|e| CallToolError::new(io_err(format!("invalid arguments: {e}"))))
    }

    fn text(s: impl Into<String>) -> ServerResult {
        ServerResult::from(CallToolResult::text_content(vec![s.into().into()]))
    }

    fn json<T: serde::Serialize>(v: &T) -> Result<ServerResult, CallToolError> {
        Ok(Self::text(
            serde_json::to_string_pretty(v)
                .map_err(|e| CallToolError::new(io_err(e.to_string())))?,
        ))
    }
}

#[async_trait]
impl ServerHandler for QuickNotesHandler {
    async fn handle_list_tools_request(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: &RequestContext,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            tools: vec![
                NoteCreateTool::tool(),
                NoteSearchTool::tool(),
                NoteListTool::tool(),
                NoteGetTool::tool(),
                NoteUpdateTool::tool(),
                NoteDeleteTool::tool(),
            ],
            cache_scope: ListToolsResultCacheScope::Private,
            result_type: "complete".to_string(),
            ttl_ms: 0,
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _context: &RequestContext,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ServerResult, CallToolError> {
        // The mutex is only held for the duration of one synchronous SQLite
        // call; there are no await points inside the lock.
        let conn = self.db.lock();
        let err = |e: anyhow::Error| CallToolError::new(io_err(e.to_string()));

        match params.name.as_str() {
            "note_create" => {
                let args: NoteCreateTool = Self::parse_args(&params)?;
                let id = core::create_note(&conn, &args.title, &args.content, &args.tags)
                    .map_err(err)?;
                Ok(Self::text(format!(r#"{{"ok":true,"id":{id}}}"#)))
            }
            "note_search" => {
                let args: NoteSearchTool = Self::parse_args(&params)?;
                let results = core::search_notes(&conn, &args.query, args.limit).map_err(err)?;
                Self::json(&results)
            }
            "note_list" => {
                let args: NoteListTool = Self::parse_args(&params)?;
                let results = core::list_notes(&conn, args.limit).map_err(err)?;
                Self::json(&results)
            }
            "note_get" => {
                let args: NoteGetTool = Self::parse_args(&params)?;
                match core::get_note(&conn, args.id).map_err(err)? {
                    Some(note) => Self::json(&note),
                    None => Err(CallToolError::new(io_err(format!(
                        "note {} not found",
                        args.id
                    )))),
                }
            }
            "note_update" => {
                let args: NoteUpdateTool = Self::parse_args(&params)?;
                let existing = core::get_note(&conn, args.id)
                    .map_err(err)?
                    .ok_or_else(|| {
                        CallToolError::new(io_err(format!("note {} not found", args.id)))
                    })?;
                core::update_note(
                    &conn,
                    args.id,
                    args.title.as_deref().unwrap_or(&existing.title),
                    args.content.as_deref().unwrap_or(&existing.content),
                    args.tags.as_deref().unwrap_or(&existing.tags),
                )
                .map_err(err)?;
                Ok(Self::text(format!(r#"{{"ok":true,"id":{}}}"#, args.id)))
            }
            "note_delete" => {
                let args: NoteDeleteTool = Self::parse_args(&params)?;
                core::delete_note(&conn, args.id).map_err(err)?;
                Ok(Self::text(format!(
                    r#"{{"ok":true,"deleted":{}}}"#,
                    args.id
                )))
            }
            other => Err(CallToolError::unknown_tool(other)),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> SdkResult<()> {
    let db_path = core::default_db_path();
    let conn = core::open_db(&db_path).map_err(|e| {
        io_err(format!(
            "failed to open database {}: {e}",
            db_path.display()
        ))
    })?;

    let handler = QuickNotesHandler {
        db: Arc::new(SharedConn::new(conn)),
    };

    let server_details = ServerDetails {
        server_info: Implementation {
            name: "quicknotes-mcp".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            title: Some("QuickNotes".into()),
            description: Some(
                "Local-first sticky notes with FTS5 search, exposed to AI agents".into(),
            ),
            icons: vec![],
            website_url: Some("https://github.com/TSell-official/quicknotes".into()),
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        instructions: Some(
            "Manage the user's local QuickNotes store. Tools: note_create, note_search, \
             note_list, note_get, note_update, note_delete. All operations hit a single \
             shared SQLite + FTS5 database also used by the desktop app and CLI."
                .into(),
        ),
        meta: None,
    };

    let transport = StdioTransport::new(TransportOptions::default())?;
    let server = server_runtime::create_server(McpServerOptions {
        transport,
        handler: handler.to_mcp_server_handler(),
        server_details,
        message_observer: None,
    });
    server.start().await
}
