use std::fmt;

use rmcp::ErrorData;
use rmcp::model::{CallToolResult, Content};
use rmcp::service::{ElicitationMode, Peer, RoleServer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db::database::Database;
use crate::db::facts::{FactType, get_facts_by_type};
use crate::db::messages::{Message, get_messages_by_session};
use crate::db::sessions::{Session, get_session, increment_load_count};
use crate::db::transfers::log_transfer as log_transfer_event;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Output format for the session")]
pub enum HandoffFormat {
    #[serde(rename = "handoff")]
    #[schemars(description = "Smart Handoff — compressed summary with last exchanges")]
    Handoff,
    #[serde(rename = "briefing")]
    #[schemars(description = "Structured Briefing — goal, decisions, completed, files")]
    Briefing,
    #[serde(rename = "compact")]
    #[schemars(description = "Compact — minimal labels, no decoration")]
    Compact,
    #[serde(rename = "transcript")]
    #[schemars(description = "Full Transcript — complete replay")]
    Transcript,
    #[serde(rename = "messages")]
    #[schemars(description = "Structured JSON — machine-readable messages array")]
    Messages,
}

impl fmt::Display for HandoffFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handoff => write!(f, "handoff"),
            Self::Briefing => write!(f, "briefing"),
            Self::Compact => write!(f, "compact"),
            Self::Transcript => write!(f, "transcript"),
            Self::Messages => write!(f, "messages"),
        }
    }
}

/// User-facing elicit picker type for format selection.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FormatChoice {
    pub format: HandoffFormat,
}

rmcp::elicit_safe!(FormatChoice);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoadSessionParams {
    pub session_id: String,
    /// Output format: "handoff" (default — compact summary with last exchanges),
    /// "briefing" (structured extraction of goal, completed, decisions, pending, files),
    /// "compact" (compressed handoff with short labels, no decoration),
    /// "transcript" (full replay), or "messages" (structured JSON array).
    /// When omitted, the MCP client shows an interactive picker.
    pub format: Option<String>,
    pub max_content_length: Option<usize>,
    pub from_turn: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct LoadSessionResult {
    pub session: serde_json::Value,
    pub transcript: Option<String>,
    pub messages: Option<Vec<serde_json::Value>>,
    pub token_estimate: usize,
    pub truncated: bool,
    pub instruction: String,
    pub tokens_saved: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
pub async fn load_session(
    db: &Database,
    params: LoadSessionParams,
    agent_id: &str,
    verbatim_budget: usize,
    _summary_budget: usize,
    _anchor_budget: usize,
    _llm_summary_enabled: bool,
    peer: Option<Peer<RoleServer>>,
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

    // ── elicit path: when format is omitted, use MCP Elicitation picker ──
    let chosen_format: String = if let Some(ref f) = params.format {
        f.clone()
    } else if let Some(peer) = peer {
        let has_form = peer
            .supported_elicitation_modes()
            .contains(&ElicitationMode::Form);
        if has_form {
            match peer
                .elicit::<FormatChoice>("How would you like to load this session?")
                .await
            {
                Ok(Some(choice)) => choice.format.to_string(),
                Ok(None) => {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "No format selected. Use `format` parameter to specify one.",
                    )]));
                }
                Err(rmcp::service::ElicitationError::UserDeclined)
                | Err(rmcp::service::ElicitationError::UserCancelled) => {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "Session load cancelled.",
                    )]));
                }
                Err(e) => {
                    return Err(ErrorData::internal_error(
                        format!("Elicitation error: {e}"),
                        None,
                    ));
                }
            }
        } else {
            // Client doesn't support elicitation — default to handoff
            "handoff".to_string()
        }
    } else {
        // No peer available (test/CLI context) — default to handoff
        "handoff".to_string()
    };

    // Content is being delivered — count the load
    let _ = increment_load_count(db, &params.session_id, agent_id);

    // Clear re-fetchable tool results before formatting
    clear_tool_results(&mut messages);

    let total_length: usize = messages.iter().map(|m| m.content.len()).sum();
    let max_len = params.max_content_length.unwrap_or(usize::MAX);
    let truncated = total_length > max_len;

    let messages = if total_length > max_len {
        truncate_messages(messages, max_len)
    } else {
        messages
    };

    let session_json = serde_json::json!({
        "id": session.id,
        "title": session.title,
        "agent_origin": session.agent_origin,
        "project_path": session.project_path,
        "message_count": messages.len(),
        "created_at": session.created_at,
        "tags": session.tags,
    });

    if chosen_format == "messages" {
        let msg_json: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "turn_index": m.turn_index,
                    "role": m.role,
                    "content": m.content,
                    "agent_id": m.agent_id,
                    "model": m.model,
                    "created_at": m.created_at,
                })
            })
            .collect();

        let result = LoadSessionResult {
            session: session_json,
            transcript: None,
            messages: Some(msg_json),
            token_estimate: 0,
            truncated,
            instruction:
                "Use these messages to restore context. Continue the conversation naturally."
                    .to_string(),
            tokens_saved: None,
        };

        return Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
        )]));
    }

    if chosen_format == "transcript" {
        let transcript = format_transcript(&session, &messages);
        let token_estimate = transcript.len() / 4;

        let result = LoadSessionResult {
            session: session_json,
            transcript: Some(transcript),
            messages: None,
            token_estimate,
            truncated,
            instruction: "Full transcript replay. Use this to review the conversation history."
                .to_string(),
            tokens_saved: None,
        };

        return Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
        )]));
    }

    if chosen_format == "briefing" {
        let briefing = format_briefing(
            &session,
            &messages,
            Some(db),
            &params.session_id,
            verbatim_budget,
            _summary_budget,
        );
        let token_estimate = briefing.len() / 4;

        let result = LoadSessionResult {
            session: session_json,
            transcript: Some(briefing),
            messages: None,
            token_estimate,
            truncated,
            instruction: "Respond directly to the LATEST message. Do not re-analyze history."
                .to_string(),
            tokens_saved: None,
        };

        return Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
        )]));
    }

    if chosen_format == "compact" {
        let full_tokens = format_transcript(&session, &messages).len() / 4;
        let dummy = format_compact(
            &session,
            &messages,
            0,
            0,
            Some(db),
            &params.session_id,
            verbatim_budget,
        );
        let compact_tokens = dummy.len() / 4;
        let tokens_saved = full_tokens.saturating_sub(compact_tokens) as i64;
        let pct = if full_tokens > 0 {
            (tokens_saved as f64 / full_tokens as f64 * 100.0) as i64
        } else {
            0
        };

        let compact = format_compact(
            &session,
            &messages,
            tokens_saved,
            pct,
            Some(db),
            &params.session_id,
            verbatim_budget,
        );
        let compact_len = compact.len() / 4;

        let result = LoadSessionResult {
            session: session_json,
            transcript: Some(compact),
            messages: None,
            token_estimate: compact_len,
            truncated,
            instruction: "[CONTINUE] Respond to last turn directly. No re-analysis, no greeting."
                .to_string(),
            tokens_saved: Some(tokens_saved),
        };

        return Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
        )]));
    }

    // Default: handoff format
    let full_transcript = format_transcript(&session, &messages);
    let full_tokens = full_transcript.len() / 4;

    // Compute tokens_saved first (needed for format_handoff)
    let dummy_handoff = format_handoff(
        &session,
        &messages,
        0,
        0,
        Some(db),
        &params.session_id,
        verbatim_budget,
    );
    let handoff_tokens = dummy_handoff.len() / 4;
    let tokens_saved = full_tokens.saturating_sub(handoff_tokens) as i64;
    let pct = if full_tokens > 0 {
        (tokens_saved as f64 / full_tokens as f64 * 100.0) as i64
    } else {
        0
    };

    let transcript = format_handoff(
        &session,
        &messages,
        tokens_saved,
        pct,
        Some(db),
        &params.session_id,
        verbatim_budget,
    );
    let token_estimate = transcript.len() / 4;

    if session.agent_origin != agent_id {
        let _ = log_transfer_event(
            db,
            &params.session_id,
            &session.agent_origin,
            agent_id,
            tokens_saved,
        );
    }

    let instruction = format!(
        "HANDOFF: Cross-agent session continuation.\n\
         Tokens saved vs full replay: {} ({}% compression).\n\
         The user's most recent message (shown in LAST EXCHANGE) is their active query.\n\
         Do NOT re-analyze the full history, do NOT ask what they want to do next.\n\
         Your first output must be a direct, natural continuation of the work mid-sentence.",
        tokens_saved, pct,
    );

    let result = LoadSessionResult {
        session: session_json,
        transcript: Some(transcript),
        messages: None,
        token_estimate,
        truncated,
        instruction,
        tokens_saved: Some(tokens_saved),
    };

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}

fn budget_split(messages: &[Message], verbatim_budget: usize) -> (&[Message], &[Message]) {
    if messages.is_empty() {
        return (&[], &[]);
    }
    let mut tokens = 0usize;
    let mut split_at = messages.len();
    for (i, m) in messages.iter().enumerate().rev() {
        tokens += (m.content.len() / 4).max(1);
        if tokens > verbatim_budget {
            split_at = i + 1;
            break;
        }
    }
    messages.split_at(split_at)
}

fn load_fallback_goal(db: &Database, session_id: &str, messages: &[Message]) -> String {
    if let Ok(facts) = get_facts_by_type(db, session_id, FactType::Goal)
        && let Some(fact) = facts.last()
    {
        return fact.content.clone();
    }
    extract_goal(messages)
}

pub(crate) fn format_handoff(
    session: &Session,
    messages: &[Message],
    tokens_saved: i64,
    compression_pct: i64,
    db: Option<&Database>,
    session_id: &str,
    verbatim_budget: usize,
) -> String {
    let total = messages.len();
    let last_turn = messages.last().map(|m| m.turn_index).unwrap_or(0);
    let origin = &session.agent_origin;
    let project = session.project_path.as_deref().unwrap_or("N/A");

    let mut out = String::new();

    // ── banner ──
    out.push_str(&format!(
        "\
╔══════════════════════════════════════════════════════════╗
║                     SESSION HANDOFF                      ║
╠══════════════════════════════════════════════════════════╣
║ Session: {}
║ Origin:  {}
║ Project: {}
║ Turns:   {}  (last turn #{})
╚══════════════════════════════════════════════════════════╝\n\n",
        session.title, origin, project, total, last_turn
    ));

    // ── budget-based split ──
    let (early, context) = budget_split(messages, verbatim_budget);

    if !early.is_empty() {
        let goal = match db {
            Some(d) => load_fallback_goal(d, session_id, early),
            None => extract_goal(early),
        };
        out.push_str("│ GOAL\n");
        out.push_str(&format!("│ {}\n\n", goal));

        let last_early_turn = early.last().map(|m| m.turn_index).unwrap_or(0);
        let early_types = summarize_turns(early);
        out.push_str(&format!(
            "│ EARLIER WORK (turns 1–{})\n│ {} turns: {}\n\n",
            last_early_turn,
            early.len(),
            early_types
        ));
    }

    // ── last exchanges in full ──
    if !context.is_empty() {
        out.push_str(&format!(
            "── LAST EXCHANGE (turns {}–{}) ──\n\n",
            context.first().map(|m| m.turn_index).unwrap_or(0),
            context.last().map(|m| m.turn_index).unwrap_or(0),
        ));
        for m in context {
            let agent_label = m
                .agent_id
                .as_ref()
                .map(|a| format!(" ({})", a))
                .unwrap_or_default();
            let model_label = m
                .model
                .as_ref()
                .map(|m| format!(" [{}]", m))
                .unwrap_or_default();
            out.push_str(&format!(
                "[Turn {} | {}{}{}]\n{}\n\n",
                m.turn_index, m.role, agent_label, model_label, m.content
            ));
        }
    }

    // ── instruction block ──
    out.push_str(&format!(
        "\
── INSTRUCTIONS ──
HANDOFF: Cross-agent session continuation.
Tokens saved vs full replay: {} ({}% compression).
The user's most recent message (shown in LAST EXCHANGE) is their active query.
Do NOT re-analyze the full history, do NOT ask what they want to do next.
Your first output must be a direct, natural continuation of the work mid-sentence.

── END HANDOFF ──
",
        tokens_saved, compression_pct
    ));

    out
}

fn extract_goal(messages: &[Message]) -> String {
    let mut best: Option<String> = None;
    let mut best_score: usize = 0;

    let goal_keywords = [
        "we need to",
        "i want to",
        "goal",
        "task",
        "objective",
        "create",
        "build",
        "implement",
        "make",
        "solve",
        "figure out",
        "fix",
        "add",
        "change",
        "refactor",
        "the idea is",
        "we should",
        "we're going to",
        "purpose",
    ];

    for m in messages {
        if m.role != "user" {
            continue;
        }
        let text = m.content.trim();
        if text.is_empty() || text.len() < 20 {
            continue;
        }

        let lower = text.to_lowercase();
        let keyword_score = goal_keywords.iter().filter(|k| lower.contains(*k)).count();
        let length_score = text.len().min(200) / 10;

        // Prefer messages with goal keywords and reasonable length
        let score = keyword_score * 50 + length_score;

        if score > best_score {
            best_score = score;
            let truncated: String = text.chars().take(200).collect();
            if text.len() > 200 {
                best = Some(format!("{}...", truncated));
            } else {
                best = Some(truncated.to_string());
            }
        }
    }

    best.unwrap_or_else(|| {
        // Fallback: first non-empty user message
        for m in messages {
            if m.role == "user" {
                let t = m.content.trim();
                if !t.is_empty() {
                    let truncated: String = t.chars().take(100).collect();
                    if t.len() > 100 {
                        return format!("{}...", truncated);
                    }
                    return truncated.to_string();
                }
            }
        }
        "Resume previous work session".to_string()
    })
}

fn summarize_turns(messages: &[Message]) -> String {
    let user_count = messages.iter().filter(|m| m.role == "user").count();
    let assistant_count = messages.iter().filter(|m| m.role == "assistant").count();
    let tool_count = messages.iter().filter(|m| m.role == "tool").count();

    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("{} user", user_count));
    parts.push(format!("{} assistant", assistant_count));
    if tool_count > 0 {
        parts.push(format!("{} tool results", tool_count));
    }

    let first_user = messages.iter().find(|m| m.role == "user").map(|m| {
        let t: String = m.content.chars().take(60).collect();
        t
    });

    if let Some(start) = first_user {
        format!("{} — started with \"{}\"", parts.join(", "), start)
    } else {
        parts.join(", ")
    }
}

pub(crate) fn format_briefing(
    session: &Session,
    messages: &[Message],
    db: Option<&Database>,
    session_id: &str,
    verbatim_budget: usize,
    _summary_budget: usize,
) -> String {
    let total = messages.len();
    let last_user = messages.iter().rev().find(|m| m.role == "user");
    let pending = last_user.map(|m| m.content.as_str()).unwrap_or("");
    let project = session.project_path.as_deref().unwrap_or("N/A");

    let mut out = String::new();
    out.push_str(&format!(
        "══ BRIEFING ══  {}  |  {}  |  {} turns  |  {}\n\n",
        session.title, session.agent_origin, total, project
    ));

    // Split: middle (anchor + compressed) and verbatim tail
    let (early, verbatim_msgs) = budget_split(messages, verbatim_budget);

    // ── SESSION FACTS (ANCHOR) ──
    out.push_str("── SESSION FACTS (ANCHOR) ──\n\n");

    let goal = match db {
        Some(d) => load_fallback_goal(d, session_id, messages),
        None => extract_goal(messages),
    };
    out.push_str(&format!("GOAL:       {}\n", goal));

    let decisions: Vec<String> = match db {
        Some(d) => {
            if let Ok(facts) = get_facts_by_type(d, session_id, FactType::Decision) {
                facts.into_iter().map(|f| f.content).collect()
            } else {
                extract_decisions(messages)
            }
        }
        None => extract_decisions(messages),
    };
    if !decisions.is_empty() {
        out.push_str("DECISIONS:");
        for d in &decisions {
            out.push_str(&format!("\n  \u{2022} {}", d));
        }
        out.push('\n');
    }

    let completed: Vec<String> = match db {
        Some(d) => {
            if let Ok(facts) = get_facts_by_type(d, session_id, FactType::Completed) {
                facts.into_iter().map(|f| f.content).collect()
            } else {
                extract_completed(messages)
            }
        }
        None => extract_completed(messages),
    };
    if !completed.is_empty() {
        out.push_str("COMPLETED:");
        for c in &completed {
            out.push_str(&format!("\n  \u{2713} {}", c));
        }
        out.push('\n');
    }

    let files: Vec<String> = match db {
        Some(d) => {
            if let Ok(facts) = get_facts_by_type(d, session_id, FactType::FileTouched) {
                facts.into_iter().map(|f| f.content).collect()
            } else {
                extract_files(messages)
            }
        }
        None => extract_files(messages),
    };
    if !files.is_empty() {
        out.push_str("FILES:");
        for f in &files {
            out.push_str(&format!("\n  \u{2192} {}", f));
        }
        out.push('\n');
    }

    out.push('\n');

    // ── CONTEXT (COMPRESSED MIDDLE) ──
    if total > verbatim_msgs.len() {
        let summary = summarize_turns(early);
        let last_early_turn = early.last().map(|m| m.turn_index).unwrap_or(0);
        let end_label = if last_early_turn > 0 {
            format!(" (turns 0–{})", last_early_turn)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "── CONTEXT (COMPRESSED MIDDLE{}) ──\n\n{}\n\n",
            end_label, summary
        ));
    }

    // ── LATEST (VERBATIM TAIL) ──
    if !verbatim_msgs.is_empty() {
        out.push_str(&format!(
            "── LATEST (turns {}–{}) ──\n\n",
            verbatim_msgs.first().map(|m| m.turn_index).unwrap_or(0),
            verbatim_msgs.last().map(|m| m.turn_index).unwrap_or(0),
        ));
        for m in verbatim_msgs {
            let role_char = match m.role.as_str() {
                "user" => "U",
                "assistant" => "A",
                "tool" => "T",
                _ => &m.role,
            };
            out.push_str(&format!(
                "[{}#{}] {}\n\n",
                role_char, m.turn_index, m.content
            ));
        }
    }

    // ── PENDING ──
    out.push_str("── PENDING ──\n\n");
    if !pending.is_empty() {
        out.push_str(&format!("{}\n\n", pending));
    } else {
        out.push_str("(awaiting input)\n\n");
    }

    out.push_str("CONTINUE: Respond to PENDING. No re-analysis. No greeting.\n");

    out
}

pub(crate) fn format_compact(
    session: &Session,
    messages: &[Message],
    tokens_saved: i64,
    compression_pct: i64,
    db: Option<&Database>,
    session_id: &str,
    verbatim_budget: usize,
) -> String {
    let total = messages.len();
    let last_turn = messages.last().map(|m| m.turn_index).unwrap_or(0);
    let origin = &session.agent_origin;
    let project = session.project_path.as_deref().unwrap_or("N/A");

    let mut out = String::new();

    // ── one-line header ──
    out.push_str(&format!(
        "[HANDOFF] {} | {} | {} | {} turns (#{})\n\n",
        session.title, origin, project, total, last_turn
    ));

    // ── budget-based split ──
    let (early, context) = budget_split(messages, verbatim_budget);

    if !early.is_empty() {
        let goal = match db {
            Some(d) => load_fallback_goal(d, session_id, early),
            None => extract_goal(early),
        };
        out.push_str(&format!("GOAL: {}\n", goal));
        let early_types = summarize_turns(early);
        out.push_str(&format!("EARLY: {}\n\n", early_types));
    }

    // ── last exchanges with short labels ──
    if !context.is_empty() {
        out.push_str(&format!("── LAST {} TURNS ──\n\n", context.len()));
        for m in context {
            let role_char = match m.role.as_str() {
                "user" => "U",
                "assistant" => "A",
                "tool" => "T",
                _ => &m.role,
            };
            out.push_str(&format!(
                "[{}#{}] {}\n\n",
                role_char, m.turn_index, m.content
            ));
        }
    }

    // ── one-line instruction ──
    out.push_str(&format!(
        "[CONTINUE] {} tokens saved ({}% compression). Respond to last turn directly. No re-analysis, no greeting.\n",
        tokens_saved, compression_pct
    ));

    out
}

fn extract_completed(messages: &[Message]) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let done_indicators = [
        "done",
        "completed",
        "finished",
        "works now",
        "all set",
        "success",
        "succeeded",
        "working",
        "fixed",
        "resolved",
        "implemented",
        "added",
        "created",
        "built",
        "set up",
    ];

    for m in messages {
        if m.role != "assistant" {
            continue;
        }
        let lower = m.content.to_lowercase();
        let first_line = m.content.lines().next().unwrap_or("").trim().to_string();
        if first_line.len() > 10 && first_line.len() < 120 {
            let keyword_hit = done_indicators.iter().any(|k| lower.contains(k));
            if keyword_hit && !first_line.starts_with('`') && !first_line.starts_with("```") {
                let item = first_line.trim_end_matches(&['.', '!', ';'][..]);
                if !items.contains(&item.to_string()) {
                    items.push(item.to_string());
                }
            }
        }
    }

    items.truncate(8);
    items
}

fn extract_decisions(messages: &[Message]) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let decision_patterns = [
        "we decided",
        "i decided",
        "we chose",
        "i chose",
        "going with",
        "let's use",
        "we'll use",
        "we'll go with",
        "opted for",
        "settled on",
        "went with",
        "selected",
        "we picked",
        "the choice is",
        "choosing",
    ];

    for m in messages {
        if m.role != "assistant" {
            continue;
        }
        let lower = m.content.to_lowercase();
        for pattern in &decision_patterns {
            if let Some(idx) = lower.find(pattern) {
                // Extract the sentence containing the decision
                let before = &m.content[..idx];
                let sent_start = before.rfind(['.', '!', '?']).map(|p| p + 1).unwrap_or(0);
                let after = &m.content[idx..];
                let rel_end = after
                    .find(['.', '!', '?'])
                    .map(|p| p + 1)
                    .unwrap_or_else(|| after.len().min(150));
                let sentence = m.content[sent_start..idx + rel_end].trim().to_string();
                if !items.contains(&sentence) {
                    items.push(sentence);
                }
                break;
            }
        }
    }

    items.truncate(5);
    items
}

fn extract_files(messages: &[Message]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();

    for m in messages {
        if m.role != "assistant" && m.role != "tool" {
            continue;
        }
        for segment in m.content.split('`') {
            let path = segment.trim();
            if path.is_empty() || path.len() > 200 || path.contains('\n') {
                continue;
            }
            // Likely a file path if it contains / and has an extension or is a known path
            let is_path = (path.contains('/') || path.starts_with("~"))
                && (path.contains('.') || path.starts_with('/'));
            let is_code_marker = path.starts_with("```") || path == "//" || path.starts_with("\\");
            if is_path && !is_code_marker && !files.contains(&path.to_string()) {
                files.push(path.to_string());
            }
        }
    }

    files.truncate(10);
    files
}

// ── Tool result clearing (Phase 3) ──

/// Replace re-fetchable tool results with a stub.
/// Called before any formatting to reduce token overhead.
pub(crate) fn clear_tool_results(messages: &mut [Message]) {
    let indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "tool")
        .map(|(i, _)| i)
        .collect();

    for idx in indices {
        if messages[idx].content.len() < 200 {
            continue;
        }
        if is_content_quoted_in_later_messages(messages, idx) {
            continue;
        }
        if is_refetchable(&messages[idx].content) {
            messages[idx].content = "[tool result cleared — re-fetchable]".to_string();
        }
    }
}

fn is_refetchable(content: &str) -> bool {
    let markers = ["cat ", "ls ", "glob ", "read_file", "grep "];
    if markers.iter().any(|p| content.contains(p)) {
        return true;
    }
    content.len() > 2000 && looks_like_file_dump(content)
}

fn looks_like_file_dump(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() < 10 {
        return false;
    }
    let code_keywords = [
        "fn ",
        "pub ",
        "let ",
        "const ",
        "use ",
        "import ",
        "def ",
        "class ",
        "function ",
        "interface ",
        "struct ",
        "enum ",
        "impl ",
        "return ",
        "if ",
        "else ",
        "for ",
        "while ",
        "match ",
        "case ",
    ];
    let code_lines = lines
        .iter()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("//")
                || t.starts_with("/*")
                || t.starts_with('*')
                || t.starts_with('#')
                || t.starts_with("```")
                || t.starts_with("  ")
                || t.starts_with('\t')
                || code_keywords.iter().any(|k| t.starts_with(k))
                || t.contains(" = ")
                || t.contains("():")
                || t.contains(" => ")
                || t.ends_with(';')
                || t.ends_with(" {")
                || t.ends_with('}')
                || t.ends_with(",)")
        })
        .count();
    code_lines as f64 / lines.len() as f64 > 0.3
}

/// Check if any portion of a tool result appears quoted in later user/assistant messages.
fn is_content_quoted_in_later_messages(messages: &[Message], tool_idx: usize) -> bool {
    let content = &messages[tool_idx].content;
    let lines: Vec<&str> = content.lines().collect();
    for later in messages.iter().skip(tool_idx + 1) {
        if later.role != "user" && later.role != "assistant" {
            continue;
        }
        for line in lines.iter().take(5) {
            let trimmed = line.trim();
            if trimmed.len() > 20 && later.content.contains(trimmed) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn format_transcript(session: &Session, messages: &[Message]) -> String {
    let header = format!(
        "=== Session: {} ===\nOrigin: {}\nProject: {}\nDate: {}\nMessages: {}\n\n",
        session.title,
        session.agent_origin,
        session.project_path.as_deref().unwrap_or("N/A"),
        format_time(session.created_at),
        messages.len(),
    );

    let turns: Vec<String> = messages
        .iter()
        .map(|m| {
            let agent_label = m
                .agent_id
                .as_ref()
                .map(|a| format!(" ({})", a))
                .unwrap_or_default();
            let model_label = m
                .model
                .as_ref()
                .map(|m| format!(" [{}]", m))
                .unwrap_or_default();
            format!(
                "[Turn {} | {}{}{}]\n{}",
                m.turn_index, m.role, agent_label, model_label, m.content
            )
        })
        .collect();

    header + &turns.join("\n\n---\n\n")
}

fn truncate_messages(messages: Vec<Message>, max_length: usize) -> Vec<Message> {
    let mut result: Vec<Message> = Vec::new();
    let mut cumulative = 0usize;

    for m in messages.into_iter().rev() {
        let content_len = m.content.len();
        if cumulative + content_len <= max_length {
            cumulative += content_len;
            result.push(m);
        } else {
            let remaining = max_length.saturating_sub(cumulative);
            if remaining > 50 {
                let truncated_content = format!(
                    "{}...",
                    &m.content[..remaining.min(content_len).saturating_sub(3)]
                );
                result.push(Message {
                    content: truncated_content,
                    ..m
                });
            }
            break;
        }
    }

    result.reverse();
    result
}

fn format_time(ms: i64) -> String {
    let duration = std::time::Duration::from_millis(ms as u64);
    let secs = duration.as_secs();
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    format!("{}d {:02}:{:02} UTC", days, hours, minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_format_display() {
        assert_eq!(HandoffFormat::Handoff.to_string(), "handoff");
        assert_eq!(HandoffFormat::Briefing.to_string(), "briefing");
        assert_eq!(HandoffFormat::Compact.to_string(), "compact");
        assert_eq!(HandoffFormat::Transcript.to_string(), "transcript");
        assert_eq!(HandoffFormat::Messages.to_string(), "messages");
    }

    #[test]
    fn handoff_format_serde() {
        for (variant, expected) in [
            (HandoffFormat::Handoff, "\"handoff\""),
            (HandoffFormat::Briefing, "\"briefing\""),
            (HandoffFormat::Compact, "\"compact\""),
            (HandoffFormat::Transcript, "\"transcript\""),
            (HandoffFormat::Messages, "\"messages\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected, "serialize {:?}", variant);
            let deserialized: HandoffFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant, "deserialize {:?}", variant);
        }
    }

    #[test]
    fn format_choice_serde() {
        let choice = FormatChoice {
            format: HandoffFormat::Briefing,
        };
        let json = serde_json::to_string(&choice).unwrap();
        assert_eq!(json, r#"{"format":"briefing"}"#);

        let deserialized: FormatChoice = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.format.to_string(), "briefing");
    }

    #[test]
    fn format_choice_is_elicit_safe() {
        fn assert_elicit_safe<T: rmcp::service::ElicitationSafe>() {}
        assert_elicit_safe::<FormatChoice>();
    }
}
