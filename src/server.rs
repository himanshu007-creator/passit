use std::sync::Arc;

use rmcp::handler::server::router::Router;
use rmcp::handler::server::router::tool::ToolRoute;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo, Tool};
use rmcp::service::ServiceExt;
use rmcp::transport::stdio;
use rmcp::ErrorData;
use serde::de::DeserializeOwned;

use crate::config::Config;
use crate::db::database::{Database, StorageConnector};
use crate::tools;

pub struct SessionManager {
    pub db: Arc<Database>,
    pub config: Config,
}

fn make_tool<T: schemars::JsonSchema>(name: &'static str, description: &'static str) -> Tool {
    let value = serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default();
    let map = value.as_object().cloned().unwrap_or_default();
    Tool::new(name, description, map)
}

fn parse_params<T: DeserializeOwned>(args: Option<rmcp::model::JsonObject>) -> Result<T, ErrorData> {
    let value = args.map(serde_json::Value::Object).unwrap_or(serde_json::json!({}));
    serde_json::from_value(value)
        .map_err(|e| ErrorData::invalid_params(e.to_string(), None))
}

fn build_router(handler: SessionManager) -> Router<SessionManager> {
    Router::new(handler)
        .with_tool(save_tool())
        .with_tool(drop_tool())
        .with_tool(list_tool())
        .with_tool(grab_tool())
        .with_tool(push_tool())
        .with_tool(search_tool())
        .with_tool(scan_tool())
        .with_tool(fork_tool())
        .with_tool(status_tool())
        .with_tool(summary_tool())
        .with_tool(trim_tool())
        .with_tool(convert_tool())
}

fn save_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::save::SaveSessionTurnParams>(
            "save",
            "Save a conversation turn to resume later. Called automatically after every message \
             exchange to persist the conversation. If session_id is omitted, a new session is \
             auto-created.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let agent_id = ctx.service.config.agent_id.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::save::SaveSessionTurnParams>(args)?;
                tools::save::save_session_turn(&db, params, &agent_id).await
            })
        },
    )
}

fn drop_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::load::LoadSessionParams>(
            "drop",
            "Load a saved conversation and return a HANDOFF summary so the user can continue \
              seamlessly where they left off. Default format produces a compact handoff block with \
              the goal, a compressed summary of earlier turns, the last 3 exchanges verbatim, and \
              a strong continuation instruction. Use this when the user says 'resume where I left \
              off', 'continue that conversation', 'find and continue my last session', 'pick up \
              from where I was', or wants to cross-load a conversation from another agent. \
              Use format='briefing' for structured extraction, format='compact' for compressed handoff, \
              format='transcript' for a full raw replay, or 'messages' for structured JSON. \
              When format is omitted, an interactive picker is shown so the user can choose \
              the output style.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let agent_id = ctx.service.config.agent_id.clone();
            let config = ctx.service.config.clone();
            let args = ctx.arguments;
            let peer = ctx.request_context.peer.clone();
            Box::pin(async move {
                let params = parse_params::<tools::load::LoadSessionParams>(args)?;
                tools::load::load_session(
                    &db, params, &agent_id,
                    config.verbatim_budget,
                    config.summary_budget,
                    config.anchor_budget,
                    config.llm_summary_enabled,
                    Some(peer),
                ).await
            })
        },
    )
}

fn list_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::list::ListSessionsParams>(
            "list",
            "List all saved conversations, ordered by most recently updated. \
             Use this when the user says 'show my conversations', 'what sessions do I have', \
             'list my saved chats', or wants to browse available sessions. \
             Filter by source (claude, opencode, gemini), agent, project, or tag.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::list::ListSessionsParams>(args)?;
                tools::list::list_sessions_tool(&db, params).await
            })
        },
    )
}

fn grab_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::import_tool::ImportSessionParams>(
            "grab",
            "Import a conversation from outside — from another agent export, a past JSON backup, \
             or any external source. Use this when the user says 'grab this conversation', \
             'import my old chat', 'pull this from a file', or wants to restore a conversation \
             that was exported earlier.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::import_tool::ImportSessionParams>(args)?;
                tools::import_tool::import_session(&db, params).await
            })
        },
    )
}

fn push_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::export_tool::ExportSessionParams>(
            "push",
            "Export a conversation to share with another agent or tool. \
             Use this when the user says 'send this to claude', 'share with gemini', \
             'export for ChatGPT', 'push this conversation out'. \
             Supports JSON (full data) and markdown (readable transcript) formats.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::export_tool::ExportSessionParams>(args)?;
                tools::export_tool::export_session(&db, params).await
            })
        },
    )
}

fn search_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::search::SearchSessionsParams>(
            "search",
            "Search across all saved conversations. \
             Use this when the user says 'find where we talked about X', \
             'search my conversations for Y', 'what did we decide about Z'. \
             Returns matching messages with their session context.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::search::SearchSessionsParams>(args)?;
                tools::search::search_sessions(&db, params).await
            })
        },
    )
}

fn scan_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::scan::ScanHistoryParams>(
            "scan",
            "Scan all local agent history files — Claude Code (~/.claude/projects), \
             Gemini CLI (~/.gemini/tmp), OpenCode (~/.config/opencode) — and import \
             any conversations that haven't been saved yet. \
             Use this when the user says 'find my old conversations', 'import from claude', \
             'discover my past sessions', 'scan for conversations'.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::scan::ScanHistoryParams>(args)?;
                tools::scan::scan_history_tool(&db, params).await
            })
        },
    )
}

fn fork_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::branch::BranchSessionParams>(
            "fork",
            "Fork a conversation at a specific turn. Creates a new session with all messages \
             from the given turn onwards. \
             Use this when the user says 'fork from turn 5', 'branch this conversation', \
             'try a different path from this point', 'split at message 3'.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::branch::BranchSessionParams>(args)?;
                tools::branch::branch_session(&db, params).await
            })
        },
    )
}

fn status_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::status::StatusParams>(
            "status",
            "Show the current state of passit: total saved conversations, count by source \
             (Claude, Gemini, OpenCode), database size, and recent sessions. \
             Use this when the user says 'show status', 'what's in passit', \
             'how many conversations do I have', 'check passit'.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            Box::pin(async move {
                tools::status::status_tool(&db).await
            })
        },
    )
}

fn trim_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::trim::TrimSessionParams>(
            "trim",
            "Trim a conversation to fit within a token limit, keeping the most recent turns. \
             Use this when the user says 'trim this conversation', 'shorten it for context', \
             'fit in token limit', 'keep only the last few messages'.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::trim::TrimSessionParams>(args)?;
                tools::trim::trim_session(&db, params).await
            })
        },
    )
}

fn summary_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::summary::SummaryParams>(
            "summary",
            "Show a rich dashboard overview of passit: session count by source with \
             visual bar charts, total messages, database size, transfer statistics, \
             recent sessions timeline, and the PASSIT ASCII logo. Use this when the \
             user wants a visual summary or status board.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            Box::pin(async move {
                tools::summary::summary_tool(&db).await
            })
        },
    )
}

fn convert_tool() -> ToolRoute<SessionManager> {
    ToolRoute::new_dyn(
        make_tool::<tools::convert::ConvertSessionParams>(
            "convert",
            "Convert a conversation between formats: 'briefing' (default — structured \
              extraction of goal, completed, decisions, pending items, and files), 'compact' \
              (compressed handoff with short labels, no decoration), 'handoff' (full handoff \
              with last 3 exchanges), 'transcript' (full replay), 'messages' (structured JSON), \
              'openai' (OpenAI messages array), 'anthropic' (Anthropic messages format). \
              Use this when the user says 'convert to x format', 'export as briefing', \
              'get a compact summary'.",
        ),
        |ctx: ToolCallContext<'_, SessionManager>| {
            let db = ctx.service.db.clone();
            let args = ctx.arguments;
            Box::pin(async move {
                let params = parse_params::<tools::convert::ConvertSessionParams>(args)?;
                tools::convert::convert_session(&db, params).await
            })
        },
    )
}

impl rmcp::ServerHandler for SessionManager {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::new("passit", env!("CARGO_PKG_VERSION")))
    }
}

pub async fn run_mcp_server(config: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let db = Arc::new(Database::open(&config.db_path)?);
    let db_path = config.db_path.clone();
    let handler = SessionManager { db: db.clone(), config };
    let router = build_router(handler);

    let logo = r"
  ▓▓▓▓   ▓▓▓   ▓▓▓▓  ▓▓▓▓ ▓▓▓ ▓▓▓▓▓
  ▓   ▓ ▓   ▓ ▓     ▓      ▓    ▓
  ▓▓▓▓  ▓▓▓▓▓  ▓▓▓   ▓▓▓   ▓    ▓
  ▓     ▓   ▓     ▓     ▓  ▓    ▓
  ▓     ▓   ▓ ▓▓▓▓  ▓▓▓▓  ▓▓▓   ▓
";
    tracing::info!("{}", logo);

    // Background scan: import agent histories without blocking MCP startup.
    // Conversations become available as they're imported.
    let scan_db = db.clone();
    std::thread::spawn(move || {
        let summary = crate::history::run_history_scanners(&scan_db);
        if summary.total_sessions > 0 {
            tracing::info!(
                "passit: imported {} sessions ({} messages)",
                summary.total_sessions,
                summary.total_messages
            );
        } else {
            tracing::info!("passit: no new sessions to import");
        }
    });

    let session_count = db.conn().lock().ok()
        .and_then(|c| c.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0)).ok())
        .unwrap_or(0);
    tracing::info!(
        "passit {} ready | {} sessions | {}",
        env!("CARGO_PKG_VERSION"),
        session_count,
        db_path.display(),
    );

    let running = router.serve(stdio()).await?;
    let _ = running.waiting().await;
    Ok(())
}
