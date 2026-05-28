use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::database::Database;
use crate::db::messages::get_messages_by_session;
use crate::db::sessions::get_session;
use crate::tools::load::format_transcript;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportSessionParams {
    pub session_id: String,
    pub format: Option<String>,
}

pub async fn export_session(
    db: &Database,
    params: ExportSessionParams,
) -> Result<CallToolResult, ErrorData> {
    let session = get_session(db, &params.session_id)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        .ok_or_else(|| ErrorData::internal_error(format!("Session not found: {}", params.session_id), None))?;

    let messages = get_messages_by_session(db, &params.session_id)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let fmt = params.format.as_deref().unwrap_or("json");
    let short_id = &params.session_id[..params.session_id.len().min(12)];

    if fmt == "markdown" {
        let transcript = format_transcript(&session, &messages);
        let result = serde_json::json!({
            "content": transcript,
            "format": "markdown",
            "filename": format!("session-{}.md", short_id),
        });
        return Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
        )]));
    }

    let export = serde_json::json!({
        "version": "1.0",
        "exported_at": format_time_ms(std::time::SystemTime::now()),
        "session": {
            "id": session.id,
            "title": session.title,
            "agent_origin": session.agent_origin,
            "project_path": session.project_path,
            "created_at": session.created_at,
            "updated_at": session.updated_at,
            "tags": session.tags,
        },
        "messages": messages.iter().map(|m| serde_json::json!({
            "turn_index": m.turn_index,
            "role": m.role,
            "content": m.content,
            "agent_id": m.agent_id,
            "model": m.model,
            "created_at": m.created_at,
        })).collect::<Vec<_>>(),
    });

    let result = serde_json::json!({
        "content": serde_json::to_string_pretty(&export).unwrap_or_else(|_| "{}".to_string()),
        "format": "json",
        "filename": format!("session-{}.json", short_id),
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}

fn format_time_ms(time: std::time::SystemTime) -> String {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}
