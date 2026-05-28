use rmcp::ErrorData;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::database::Database;
use crate::db::messages::copy_messages;
use crate::db::sessions::{CreateSessionParams, create_session, get_session};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BranchSessionParams {
    pub session_id: String,
    pub from_turn: i64,
    pub branch_title: Option<String>,
}

pub async fn branch_session(
    db: &Database,
    params: BranchSessionParams,
) -> Result<CallToolResult, ErrorData> {
    let source = get_session(db, &params.session_id)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        .ok_or_else(|| {
            ErrorData::internal_error(format!("Session not found: {}", params.session_id), None)
        })?;

    let title = params
        .branch_title
        .unwrap_or_else(|| format!("{} (branch @ turn {})", source.title, params.from_turn));

    let new_session = create_session(
        db,
        CreateSessionParams {
            title,
            agent_origin: source.agent_origin.clone(),
            project_path: source.project_path.clone(),
            tags: source.tags.clone(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let copied = copy_messages(db, &params.session_id, &new_session.id, params.from_turn)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let result = serde_json::json!({
        "new_session_id": new_session.id,
        "branch_title": new_session.title,
        "messages_copied": copied,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}
