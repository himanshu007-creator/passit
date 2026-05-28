use rmcp::ErrorData;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::db::database::Database;
use crate::db::messages::get_messages_by_session;
use crate::db::sessions::get_session;
use crate::tools::load::{format_briefing, format_compact, format_handoff, format_transcript};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConvertSessionParams {
    pub session_id: String,
    /// Output format: "briefing" (default — structured extraction of goal, completed,
    /// decisions, pending items, files, and latest turn), "compact" (compressed handoff
    /// with short labels and no decorative elements), "handoff" (full handoff summary
    /// with last 3 exchanges), "transcript" (full raw replay), "messages" (structured JSON),
    /// "openai" (OpenAI messages array), "anthropic" (Anthropic messages format).
    pub format: Option<String>,
    pub from_turn: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ConvertSessionResult {
    pub session_id: String,
    pub format: String,
    pub output: serde_json::Value,
    pub token_estimate: usize,
    pub tokens_saved: Option<i64>,
    pub compression_pct: Option<i64>,
}

pub async fn convert_session(
    db: &Database,
    params: ConvertSessionParams,
) -> Result<CallToolResult, ErrorData> {
    let session = get_session(db, &params.session_id)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        .ok_or_else(|| {
            ErrorData::internal_error(format!("Session not found: {}", params.session_id), None)
        })?;

    let mut messages = get_messages_by_session(db, &params.session_id)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

    if let Some(from) = params.from_turn {
        messages.retain(|m| m.turn_index >= from);
    }

    let format = params.format.as_deref().unwrap_or("briefing");
    let full_transcript = format_transcript(&session, &messages);
    let full_tokens = full_transcript.len() / 4;

    let (output, tokens_saved, compression_pct) = match format {
        "openai" => {
            let arr: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "role": m.role,
                        "content": m.content,
                    })
                })
                .collect();
            (serde_json::Value::Array(arr), None, None)
        }
        "anthropic" => {
            let arr: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    let role = if m.role == "assistant" {
                        "assistant"
                    } else {
                        "user"
                    };
                    serde_json::json!({
                        "role": role,
                        "content": [{"type": "text", "text": m.content}],
                    })
                })
                .collect();
            (serde_json::Value::Array(arr), None, None)
        }
        "messages" => {
            let arr: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "turn_index": m.turn_index,
                        "role": m.role,
                        "content": m.content,
                        "agent_id": m.agent_id,
                        "model": m.model,
                    })
                })
                .collect();
            (serde_json::Value::Array(arr), None, None)
        }
        "handoff" => {
            let handoff = format_handoff(
                &session,
                &messages,
                0,
                0,
                Some(db),
                &params.session_id,
                2000,
            );
            let handoff_tokens = handoff.len() / 4;
            let saved = full_tokens.saturating_sub(handoff_tokens) as i64;
            let pct = if full_tokens > 0 {
                (saved as f64 / full_tokens as f64 * 100.0) as i64
            } else {
                0
            };
            (
                serde_json::json!({"handoff": handoff, "tokens_saved": saved, "compression_pct": pct}),
                Some(saved),
                Some(pct),
            )
        }
        "compact" => {
            let compact = format_compact(
                &session,
                &messages,
                0,
                0,
                Some(db),
                &params.session_id,
                2000,
            );
            let compact_tokens = compact.len() / 4;
            let saved = full_tokens.saturating_sub(compact_tokens) as i64;
            let pct = if full_tokens > 0 {
                (saved as f64 / full_tokens as f64 * 100.0) as i64
            } else {
                0
            };
            (
                serde_json::json!({"compact": compact, "tokens_saved": saved, "compression_pct": pct}),
                Some(saved),
                Some(pct),
            )
        }
        _ => {
            // briefing (default)
            let briefing = format_briefing(
                &session,
                &messages,
                Some(db),
                &params.session_id,
                2000,
                1000,
            );
            let briefing_tokens = briefing.len() / 4;
            let saved = full_tokens.saturating_sub(briefing_tokens) as i64;
            let pct = if full_tokens > 0 {
                (saved as f64 / full_tokens as f64 * 100.0) as i64
            } else {
                0
            };
            (
                serde_json::json!({"briefing": briefing, "tokens_saved": saved, "compression_pct": pct}),
                Some(saved),
                Some(pct),
            )
        }
    };

    let text_estimate = match &output {
        serde_json::Value::String(s) => s.len(),
        _ => serde_json::to_string(&output).unwrap_or_default().len(),
    };

    let result = ConvertSessionResult {
        session_id: params.session_id,
        format: format.to_string(),
        output,
        token_estimate: text_estimate / 4,
        tokens_saved,
        compression_pct,
    };

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}
