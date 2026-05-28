use rmcp::ErrorData;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::db::database::{Database, StorageConnector};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusParams {}

#[derive(Debug, Serialize)]
pub struct StatusResult {
    pub total_sessions: usize,
    pub by_source: Vec<SourceCount>,
    pub db_size_bytes: u64,
    pub recent_sessions: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct SourceCount {
    pub source: String,
    pub count: usize,
}

pub async fn status_tool(db: &Database) -> Result<CallToolResult, ErrorData> {
    let conn = db.conn().lock().expect("poisoned lock");

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let mut stmt = conn
        .prepare(
            "SELECT COALESCE(json_extract(metadata, '$.source'), agent_origin) as source, COUNT(*) as cnt
             FROM sessions GROUP BY source ORDER BY cnt DESC",
        )
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let by_source: Vec<SourceCount> = stmt
        .query_map([], |row| {
            Ok(SourceCount {
                source: row.get(0)?,
                count: row.get::<_, i64>(1)? as usize,
            })
        })
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        .filter_map(|r| r.ok())
        .collect();

    let db_path = db.path().to_string_lossy().to_string();
    let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

    let mut recent_stmt = conn
        .prepare(
            "SELECT s.id, s.title, s.agent_origin, s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count
             FROM sessions s ORDER BY s.updated_at DESC LIMIT 5",
        )
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let recent_sessions: Vec<serde_json::Value> = recent_stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "title": row.get::<_, String>(1)?,
                "agent_origin": row.get::<_, String>(2)?,
                "created_at": row.get::<_, i64>(3)?,
                "updated_at": row.get::<_, i64>(4)?,
                "message_count": row.get::<_, i64>(5)?,
            }))
        })
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        .filter_map(|r| r.ok())
        .collect();

    let result = StatusResult {
        total_sessions: total as usize,
        by_source,
        db_size_bytes: db_size,
        recent_sessions,
    };

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}
