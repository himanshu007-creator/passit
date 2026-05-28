use std::path::{Path, PathBuf};

use crate::history::{HistoryScanner, ImportedMessage, ImportedSession};

pub struct ClaudeCodeScanner;

impl HistoryScanner for ClaudeCodeScanner {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn display_prefix(&self) -> &'static str {
        "CLAUDE"
    }

    fn detect(&self) -> bool {
        let dir = dirs_data_dir();
        dir.exists()
    }

    fn scan(&self) -> Result<Vec<ImportedSession>, String> {
        let root = dirs_data_dir();
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
            let dir_path = entry.path();
            if !dir_path.is_dir() {
                continue;
            }

            let project_dir_name = dir_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let project_path = decode_claude_project_dir(&project_dir_name);

            let jsonl_files = collect_jsonl_files(&dir_path);

            for jsonl_path in &jsonl_files {
                let (info, messages) = match parse_claude_jsonl(jsonl_path) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!(
                            "[history/claude] failed to parse {}: {}",
                            jsonl_path.display(),
                            e
                        );
                        continue;
                    }
                };
                if messages.is_empty() {
                    continue;
                }
                sessions.push(ImportedSession {
                    original_id: info.session_id,
                    source_agent: "claude-code".to_string(),
                    display_prefix: "CLAUDE".to_string(),
                    title: info.title,
                    messages,
                    project_path: Some(project_path.clone()),
                    created_at: info.created_at,
                });
            }
        }

        Ok(sessions)
    }
}

fn collect_jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") && path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

struct ClaudeSessionInfo {
    session_id: String,
    title: String,
    created_at: i64,
}

fn parse_claude_jsonl(path: &Path) -> Result<(ClaudeSessionInfo, Vec<ImportedMessage>), String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let mut messages: Vec<ImportedMessage> = Vec::new();
    let mut title = String::new();
    let mut session_id = String::new();
    let mut created_at: i64 = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let json: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("json: {} in {}", e, path.display()))?;

        let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

        if session_id.is_empty()
            && let Some(sid) = json.get("sessionId").and_then(|s| s.as_str())
        {
            session_id = sid.to_string();
        }

        if let Some(ts) = json
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_iso_timestamp_opt)
            && (created_at == 0 || ts < created_at)
        {
            created_at = ts;
        }

        match msg_type {
            "ai-title" => {
                if let Some(t) = json.get("aiTitle").and_then(|t| t.as_str()) {
                    title = t.to_string();
                }
            }
            "user" => {
                if let Some(content_val) = extract_user_text(&json) {
                    messages.push(ImportedMessage {
                        role: "user".to_string(),
                        content: content_val,
                        agent_id: None,
                        model: None,
                        tokens_in: None,
                        tokens_out: None,
                        created_at: parse_claude_timestamp(&json),
                    });
                }
            }
            "assistant" => {
                if let Some(content_val) = extract_assistant_text(&json) {
                    let model = json
                        .get("message")
                        .and_then(|m| m.get("model"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());

                    messages.push(ImportedMessage {
                        role: "assistant".to_string(),
                        content: content_val,
                        agent_id: None,
                        model,
                        tokens_in: None,
                        tokens_out: None,
                        created_at: parse_claude_timestamp(&json),
                    });
                }
            }
            _ => {}
        }
    }

    if session_id.is_empty() {
        session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
    }

    if messages.is_empty() && !title.is_empty() {
        messages.push(ImportedMessage {
            role: "assistant".to_string(),
            content: format!("Title: {}", title),
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            created_at: None,
        });
    }

    Ok((
        ClaudeSessionInfo {
            session_id,
            title,
            created_at,
        },
        messages,
    ))
}

fn extract_user_text(json: &serde_json::Value) -> Option<String> {
    let content = json.get("message")?.get("content")?;
    match content {
        serde_json::Value::String(s) => {
            let s = s.trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        }
        serde_json::Value::Array(arr) => {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    let t = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if t == "tool_result" || t == "tool_use" {
                        return None;
                    }
                    if let Some(text) = item.get("text").and_then(|s| s.as_str()) {
                        return Some(text.to_string());
                    }
                    if let Some(input) = item.get("input") {
                        return serde_json::to_string(input).ok();
                    }
                    None
                })
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

fn extract_assistant_text(json: &serde_json::Value) -> Option<String> {
    let content = json.get("message")?.get("content")?.as_array()?;
    let texts: Vec<String> = content
        .iter()
        .filter_map(|item| {
            let t = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if t == "thinking" {
                return None;
            }
            if t == "tool_use" {
                let name = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                let input_str = item
                    .get("input")
                    .and_then(|i| serde_json::to_string(i).ok())
                    .unwrap_or_default();
                if input_str.len() > 500 {
                    let truncated: String = input_str.chars().take(500).collect();
                    return Some(format!("[Tool: {}]\n{}", name, truncated));
                }
                return Some(format!("[Tool: {}]\n{}", name, input_str));
            }
            item.get("text")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .collect();

    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

fn parse_claude_timestamp(json: &serde_json::Value) -> Option<i64> {
    let ts = json.get("timestamp").and_then(|t| t.as_str())?;
    parse_iso_timestamp_opt(ts)
}

fn parse_iso_timestamp_opt(iso: &str) -> Option<i64> {
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

fn decode_claude_project_dir(dir_name: &str) -> String {
    if dir_name.starts_with('-') && !dir_name.is_empty() {
        let without_prefix = &dir_name[1..];
        let decoded = without_prefix.replace("--", "/").replace('-', "/");
        format!("/{}", decoded)
    } else {
        String::new()
    }
}

fn dirs_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/himanshu.".to_string());
    let mut d = PathBuf::from(&home);
    d.push(".claude/projects");
    d
}
