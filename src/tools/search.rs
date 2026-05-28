use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::database::Database;
use crate::db::messages::search_messages;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchSessionsParams {
    pub query: String,
    pub limit: Option<u32>,
}

pub async fn search_sessions(
    db: &Database,
    params: SearchSessionsParams,
) -> Result<CallToolResult, ErrorData> {
    let limit = params.limit.unwrap_or(20);

    let results = search_messages(db, &params.query, limit)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let result = serde_json::json!({
        "results": results,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}
