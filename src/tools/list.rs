use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::database::Database;
use crate::db::sessions::{list_sessions, SessionFilter};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSessionsParams {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub project_path: Option<String>,
    pub agent: Option<String>,
    pub tag: Option<String>,
    pub since: Option<i64>,
    pub source: Option<String>,
}

pub async fn list_sessions_tool(
    db: &Database,
    params: ListSessionsParams,
) -> Result<CallToolResult, ErrorData> {
    let filter = SessionFilter {
        limit: params.limit,
        offset: params.offset,
        project_path: params.project_path,
        agent: params.agent,
        tag: params.tag,
        since: params.since,
        source: params.source,
    };

    let (sessions, total) = list_sessions(db, filter)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let result = serde_json::json!({
        "sessions": sessions,
        "total": total,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}
