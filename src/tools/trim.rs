use rmcp::ErrorData;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::db::database::Database;
use crate::db::messages::get_messages_by_session;
use crate::db::sessions::get_session;
use crate::tools::load::format_transcript;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TrimSessionParams {
    pub session_id: String,
    pub max_tokens: usize,
    pub from_start: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TrimSessionResult {
    pub session_id: String,
    pub original_messages: usize,
    pub kept_messages: usize,
    pub transcript: String,
    pub token_estimate: usize,
}

pub async fn trim_session(
    db: &Database,
    params: TrimSessionParams,
) -> Result<CallToolResult, ErrorData> {
    let session = get_session(db, &params.session_id)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        .ok_or_else(|| {
            ErrorData::internal_error(format!("Session not found: {}", params.session_id), None)
        })?;

    let messages = get_messages_by_session(db, &params.session_id)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    let original_count = messages.len();
    let max_chars = params.max_tokens * 4;

    if messages.iter().map(|m| m.content.len()).sum::<usize>() <= max_chars {
        let transcript = format_transcript(&session, &messages);
        let token_estimate = transcript.len() / 4;
        let result = TrimSessionResult {
            session_id: params.session_id,
            original_messages: original_count,
            kept_messages: original_count,
            transcript,
            token_estimate,
        };
        return Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
        )]));
    }

    let keep_from_start = params.from_start.unwrap_or(false);
    let trimmed: Vec<_> = if keep_from_start {
        let mut acc = 0usize;
        messages
            .into_iter()
            .take_while(|m| {
                let next = acc + m.content.len();
                let keep = next <= max_chars;
                acc = next;
                keep
            })
            .collect()
    } else {
        let mut acc = 0usize;
        let mut msgs: Vec<_> = messages
            .into_iter()
            .rev()
            .take_while(|m| {
                let next = acc + m.content.len();
                let keep = next <= max_chars;
                acc = next;
                keep
            })
            .collect();
        msgs.reverse();
        msgs
    };

    let transcript = format_transcript(&session, &trimmed);
    let token_estimate = transcript.len() / 4;
    let result = TrimSessionResult {
        session_id: params.session_id,
        original_messages: original_count,
        kept_messages: trimmed.len(),
        transcript,
        token_estimate,
    };

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}
