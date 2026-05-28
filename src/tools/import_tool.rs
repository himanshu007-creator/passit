use rmcp::ErrorData;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::database::{Database, StorageConnector};
use crate::db::messages::{NewMessage, add_message};
use crate::db::sessions::get_session;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportSessionParams {
    pub content: String,
    pub merge: Option<bool>,
}

pub async fn import_session(
    db: &Database,
    params: ImportSessionParams,
) -> Result<CallToolResult, ErrorData> {
    let imported: serde_json::Value = serde_json::from_str(&params.content)
        .map_err(|e| ErrorData::internal_error(format!("Invalid JSON: {}", e), None))?;

    let session_data = imported
        .get("session")
        .and_then(|s| s.as_object())
        .ok_or_else(|| ErrorData::internal_error("Missing 'session' object", None))?;

    let session_id = session_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorData::internal_error("Missing session.id", None))?;

    let messages = imported
        .get("messages")
        .and_then(|m| m.as_array())
        .ok_or_else(|| ErrorData::internal_error("Missing 'messages' array", None))?;

    let merge = params.merge.unwrap_or(false);
    let existing =
        get_session(db, session_id).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let was_merge = existing.is_some();

    if existing.is_none() {
        let title = session_data
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Imported Session");
        let agent_origin = session_data
            .get("agent_origin")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let project_path = session_data
            .get("project_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let tags: Vec<String> = session_data
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let created_at = session_data
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(now_ms);

        // Insert session with custom ID directly (create_session auto-generates IDs)
        let conn = db.conn().lock().expect("poisoned lock on database");
        conn.execute(
            "INSERT INTO sessions (id, title, agent_origin, project_path, created_at, updated_at, tags, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                session_id,
                title,
                agent_origin,
                project_path,
                created_at,
                created_at,
                serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string()),
                "{}",
            ],
        )
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    } else if !merge {
        return Err(ErrorData::internal_error(
            format!(
                "Session {} already exists. Set merge=true to merge messages.",
                session_id
            ),
            None,
        ));
    }

    let mut imported_count = 0i64;
    for msg in messages {
        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ErrorData::internal_error("Message missing 'role'", None))?;

        if !["user", "assistant", "system", "tool"].contains(&role) {
            continue;
        }

        let content = msg
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ErrorData::internal_error("Message missing 'content'", None))?;

        add_message(
            db,
            NewMessage {
                session_id: session_id.to_string(),
                role: role.to_string(),
                content: content.to_string(),
                content_type: msg
                    .get("content_type")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                agent_id: msg
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                model: msg.get("model").and_then(|v| v.as_str()).map(String::from),
                tokens_in: msg.get("tokens_in").and_then(|v| v.as_i64()),
                tokens_out: msg.get("tokens_out").and_then(|v| v.as_i64()),
                metadata: None,
            },
        )
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        imported_count += 1;
    }

    let result = serde_json::json!({
        "session_id": session_id,
        "imported_messages": imported_count,
        "was_merge": was_merge,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis() as i64
}
