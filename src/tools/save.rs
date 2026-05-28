use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db::database::Database;
use crate::db::facts::try_extract_and_store;
use crate::db::messages::{add_message, get_message_count, NewMessage};
use crate::db::sessions::{
    create_session, get_session, update_session_timestamp, update_session_title,
    generate_title, CreateSessionParams,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveSessionTurnParams {
    pub session_id: Option<String>,
    pub role: String,
    pub content: String,
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub project_path: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct SaveSessionTurnResult {
    pub session_id: String,
    pub message_id: String,
    pub turn_index: i64,
    pub session_title: String,
    pub is_new_session: bool,
}

pub async fn save_session_turn(
    db: &Database,
    params: SaveSessionTurnParams,
    default_agent_id: &str,
) -> Result<CallToolResult, ErrorData> {
    let session_id;
    let mut is_new = false;

    match params.session_id {
        Some(ref sid) => {
            let existing = get_session(db, sid)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            match existing {
                Some(_) => {
                    session_id = sid.clone();
                    update_session_timestamp(db, &session_id)
                        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

                    if params.role == "user"
                        && get_message_count(db, &session_id).unwrap_or(0) == 0
                    {
                        let title = generate_title(
                            &params.content,
                            params.agent_id.as_deref().unwrap_or(default_agent_id),
                        );
                        update_session_title(db, &session_id, &title)
                            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                    }
                }
                None => {
                    return Err(ErrorData::internal_error(format!("Session not found: {}", sid), None));
                }
            }
        }
        None => {
            let agent_origin = params.agent_id.clone().unwrap_or_else(|| default_agent_id.to_string());
            let project_path = params.project_path.clone().or_else(|| {
                std::env::current_dir().ok().and_then(|p| p.to_str().map(|s| s.to_string()))
            });
            let title = generate_title(&params.content, &agent_origin);
            let tags = params.tags.clone().unwrap_or_default();
            let session = create_session(
                db,
                CreateSessionParams {
                    title: title.clone(),
                    agent_origin,
                    project_path,
                    tags,
                    metadata: serde_json::Value::Object(serde_json::Map::new()),
                },
            )
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            session_id = session.id;
            is_new = true;
        }
    }

    let is_first_user = params.role == "user"
        && get_message_count(db, &session_id).unwrap_or(0) == 0;

    let msg = add_message(
        db,
        NewMessage {
            session_id: session_id.clone(),
            role: params.role.clone(),
            content: params.content.clone(),
            content_type: None,
            agent_id: params.agent_id.clone(),
            model: params.model.clone(),
            tokens_in: params.tokens_in,
            tokens_out: params.tokens_out,
            metadata: None,
        },
    )
    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    // Non-fatal write-time fact extraction
    try_extract_and_store(db, &msg, is_first_user);

    let session = get_session(db, &session_id)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        .ok_or_else(|| ErrorData::internal_error("Session not found after creation", None))?;

    let result = SaveSessionTurnResult {
        session_id,
        message_id: msg.id,
        turn_index: msg.turn_index,
        session_title: session.title,
        is_new_session: is_new,
    };

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}
