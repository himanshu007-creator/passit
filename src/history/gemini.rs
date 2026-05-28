use std::path::PathBuf;

use crate::history::{HistoryScanner, ImportedMessage, ImportedSession};

pub struct GeminiCliScanner;

impl HistoryScanner for GeminiCliScanner {
    fn name(&self) -> &'static str {
        "gemini-cli"
    }

    fn display_prefix(&self) -> &'static str {
        "GEMINI"
    }

    fn detect(&self) -> bool {
        let dir = gemini_data_dir();
        dir.join("tmp").exists()
    }

    fn scan(&self) -> Result<Vec<ImportedSession>, String> {
        let root = gemini_data_dir().join("tmp");
        if !root.exists() {
            return Ok(vec![]);
        }

        let mut sessions = Vec::new();
        let mut entries: Vec<_> = match std::fs::read_dir(&root) {
            Ok(e) => e.filter_map(|e| e.ok()).collect(),
            Err(_) => return Ok(vec![]),
        };
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let project_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let chats_dir = path.join("chats");
            if !chats_dir.exists() {
                continue;
            }

            let chat_files = collect_chat_files(&chats_dir);

            for chat_path in &chat_files {
                let session = match parse_gemini_chat(chat_path, &project_name) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!(
                            "[history/gemini] failed to parse {}: {}",
                            chat_path.display(),
                            e
                        );
                        continue;
                    }
                };
                sessions.push(session);
            }
        }

        Ok(sessions)
    }
}

fn collect_chat_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    // Direct .jsonl files in chats/
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") && path.is_file() {
                files.push(path);
            }
        }
    }

    // Nested session directories with subagent .jsonl files
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sp = sub.path();
                        if sp.extension().and_then(|e| e.to_str()) == Some("jsonl") && sp.is_file()
                        {
                            files.push(sp);
                        }
                    }
                }
            }
        }
    }

    files.sort();
    files
}

fn parse_gemini_chat(
    path: &std::path::Path,
    project_name: &str,
) -> Result<ImportedSession, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let lines: Vec<&str> = content.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();

    if lines.is_empty() {
        return Err("empty file".to_string());
    }

    // First line is the session header
    let header: serde_json::Value =
        serde_json::from_str(lines[0]).map_err(|e| format!("header json: {}", e))?;

    let session_id = header
        .get("sessionId")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown")
        .to_string();

    let start_time_str = header
        .get("startTime")
        .and_then(|s| s.as_str())
        .unwrap_or("");

    let created_at = parse_gemini_timestamp(start_time_str);

    let kind = header
        .get("kind")
        .and_then(|s| s.as_str())
        .unwrap_or("main");

    // For main sessions, generate a meaningful title from the first user message.
    // For subagents, just mark as such.
    let mut title = if kind == "subagent" {
        format!("[subagent] {}", project_name)
    } else {
        format!("Session in {}", project_name)
    };

    let mut messages: Vec<ImportedMessage> = Vec::new();

    for line in &lines[1..] {
        if line.starts_with("{\"$set\"") {
            continue;
        }

        let json: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("json: {} in {}", e, path.display()))?;

        let msg_type = json
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("");

        match msg_type {
            "user" => {
                if let Some(text) = extract_gemini_user_text(&json) {
                    // Use first user message as title if not already set
                    if title.starts_with("Session in ") || title.starts_with("[subagent]") {
                        let truncated: String = text.chars().take(80).collect();
                        title = if truncated.len() < text.len() {
                            format!("{}...", truncated)
                        } else {
                            truncated
                        };
                    }

                    messages.push(ImportedMessage {
                        role: "user".to_string(),
                        content: text,
                        agent_id: None,
                        model: None,
                        tokens_in: None,
                        tokens_out: None,
                        created_at: json
                            .get("timestamp")
                            .and_then(|t| t.as_str())
                            .and_then(parse_gemini_timestamp_opt),
                    });
                }
            }
            "gemini" => {
                if let Some(text) = extract_gemini_response_text(&json) {
                    let model = json
                        .get("model")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());

                    let tokens = json.get("tokens");
                    let tokens_in =
                        tokens.and_then(|t| t.get("input")).and_then(|v| v.as_i64());
                    let tokens_out =
                        tokens.and_then(|t| t.get("output")).and_then(|v| v.as_i64());

                    messages.push(ImportedMessage {
                        role: "assistant".to_string(),
                        content: text,
                        agent_id: None,
                        model,
                        tokens_in,
                        tokens_out,
                        created_at: json
                            .get("timestamp")
                            .and_then(|t| t.as_str())
                            .and_then(parse_gemini_timestamp_opt),
                    });
                }
            }
            _ => {}
        }
    }

    if messages.is_empty() {
        return Err("no messages found".to_string());
    }

    Ok(ImportedSession {
        original_id: format!("{}-{}", project_name, session_id),
        source_agent: "gemini-cli".to_string(),
        display_prefix: "GEMINI".to_string(),
        title,
        messages,
        project_path: None,
        created_at,
    })
}

fn extract_gemini_user_text(json: &serde_json::Value) -> Option<String> {
    let content = json.get("content")?;
    match content {
        serde_json::Value::Array(arr) => {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        serde_json::Value::String(s) => {
            let s = s.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        _ => None,
    }
}

fn extract_gemini_response_text(json: &serde_json::Value) -> Option<String> {
    let content = json.get("content")?;
    match content {
        serde_json::Value::String(s) => {
            let s = s.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        serde_json::Value::Array(arr) => {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        _ => None,
    }
}

fn parse_gemini_timestamp(iso: &str) -> i64 {
    parse_gemini_timestamp_opt(iso).unwrap_or(0)
}

fn parse_gemini_timestamp_opt(iso: &str) -> Option<i64> {
    let s = iso.trim_end_matches('Z');
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&format!("{}Z", s)) {
        Some(dt.timestamp_millis())
    } else if let Ok(dt) =
        chrono::NaiveDateTime::parse_from_str(&s.replace('Z', ""), "%Y-%m-%dT%H:%M:%S%.f")
    {
        Some(dt.and_utc().timestamp_millis())
    } else if let Ok(dt) =
        chrono::NaiveDateTime::parse_from_str(&s.replace('Z', ""), "%Y-%m-%dT%H:%M:%S")
    {
        Some(dt.and_utc().timestamp_millis())
    } else {
        None
    }
}

fn gemini_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/himanshu.".to_string());
    let mut d = PathBuf::from(&home);
    d.push(".gemini");
    d
}
