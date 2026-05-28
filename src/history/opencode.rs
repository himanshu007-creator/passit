use std::path::{Path, PathBuf};

use crate::history::{HistoryScanner, ImportedMessage, ImportedSession};

pub struct OpenCodeScanner;

impl HistoryScanner for OpenCodeScanner {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn display_prefix(&self) -> &'static str {
        "OPENCODE"
    }

    fn detect(&self) -> bool {
        let config = config_dir();
        config.join("opencode.json").exists()
    }

    fn scan(&self) -> Result<Vec<ImportedSession>, String> {
        // OpenCode stores sessions under a data directory.
        // Try the XDG data home first, then fall back to ~/.opencode/data.
        let data_dir = data_dir_candidates().into_iter().find(|d| d.exists());

        let data_dir = match data_dir {
            Some(d) => d,
            None => return Ok(vec![]),
        };

        let sessions_dir = data_dir.join("sessions");
        if !sessions_dir.exists() {
            return Ok(vec![]);
        }

        let mut sessions = Vec::new();

        // Walk session directories: sessions/{projectHash}/{sessionId}.json
        let mut project_dirs: Vec<_> = match std::fs::read_dir(&sessions_dir) {
            Ok(e) => e.filter_map(|e| e.ok()).collect(),
            Err(_) => return Ok(vec![]),
        };
        project_dirs.sort_by_key(|e| e.path());

        for project_entry in project_dirs {
            let project_path = project_entry.path();
            if !project_path.is_dir() {
                continue;
            }

            let mut session_files: Vec<_> = match std::fs::read_dir(&project_path) {
                Ok(e) => e.filter_map(|e| e.ok()).collect(),
                Err(_) => continue,
            };
            session_files.sort_by_key(|e| e.path());

            for session_entry in session_files {
                let session_file = session_entry.path();
                if session_file.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }

                let session = match parse_opencode_session(&session_file) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!(
                            "[history/opencode] failed to parse {}: {}",
                            session_file.display(),
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

fn parse_opencode_session(path: &std::path::Path) -> Result<ImportedSession, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("json: {}", e))?;

    let session_id = json
        .get("id")
        .or_else(|| json.get("sessionId"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("OpenCode Session")
        .to_string();

    let created_at = json
        .get("createdAt")
        .or_else(|| json.get("created_at"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let project_path = json
        .get("projectPath")
        .or_else(|| json.get("project_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Messages could be inline in the session or in a separate messages directory
    let messages = if let Some(msgs) = json.get("messages").and_then(|m| m.as_array()) {
        parse_opencode_messages(msgs)?
    } else if let Some(msg_dir_val) = json.get("messageDir").or_else(|| json.get("message_dir")) {
        let msg_dir = path
            .parent()
            .unwrap_or(PathBuf::new().as_path())
            .join(msg_dir_val.as_str().unwrap_or(""));
        read_opencode_message_files(&msg_dir)?
    } else if let Some(msg_ids) = json.get("messageIds").and_then(|m| m.as_array()) {
        let parent = path.parent().unwrap_or(Path::new("."));
        let mut messages = Vec::new();
        for id_val in msg_ids {
            if let Some(id) = id_val.as_str() {
                let msg_path = parent.join(format!("messages/{}.json", id));
                if msg_path.exists()
                    && let Ok(m) = read_opencode_message_file(&msg_path)
                {
                    messages.push(m);
                }
            }
        }
        messages
    } else {
        vec![]
    };

    Ok(ImportedSession {
        original_id: session_id,
        source_agent: "opencode".to_string(),
        display_prefix: "OPENCODE".to_string(),
        title,
        messages,
        project_path,
        created_at,
    })
}

fn parse_opencode_messages(arr: &[serde_json::Value]) -> Result<Vec<ImportedMessage>, String> {
    let mut messages = Vec::new();
    for item in arr {
        let role = item
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("user")
            .to_string();

        let content = item
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let model = item
            .get("model")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string());

        let tokens_in = item
            .get("tokensIn")
            .or_else(|| item.get("tokens_in"))
            .and_then(|v| v.as_i64());

        let tokens_out = item
            .get("tokensOut")
            .or_else(|| item.get("tokens_out"))
            .and_then(|v| v.as_i64());

        messages.push(ImportedMessage {
            role,
            content,
            agent_id: None,
            model,
            tokens_in,
            tokens_out,
            created_at: None,
        });
    }
    Ok(messages)
}

fn read_opencode_message_files(dir: &std::path::Path) -> Result<Vec<ImportedMessage>, String> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(e) => e.filter_map(|e| e.ok()).collect(),
        Err(_) => return Ok(vec![]),
    };
    files.sort_by_key(|e| e.path());

    let mut messages = Vec::new();
    for entry in files {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match read_opencode_message_file(&path) {
            Ok(m) => messages.push(m),
            Err(e) => eprintln!("[history/opencode] message parse error: {}", e),
        }
    }
    Ok(messages)
}

fn read_opencode_message_file(path: &std::path::Path) -> Result<ImportedMessage, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("json: {}", e))?;

    let role = json
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("user")
        .to_string();

    let content = json
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    let model = json
        .get("model")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string());

    let tokens_in = json
        .get("tokensIn")
        .or_else(|| json.get("tokens_in"))
        .and_then(|v| v.as_i64());

    let tokens_out = json
        .get("tokensOut")
        .or_else(|| json.get("tokens_out"))
        .and_then(|v| v.as_i64());

    Ok(ImportedMessage {
        role,
        content,
        agent_id: None,
        model,
        tokens_in,
        tokens_out,
        created_at: None,
    })
}

fn config_dir() -> PathBuf {
    if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
        let mut d = PathBuf::from(val);
        d.push("opencode");
        return d;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/himanshu.".to_string());
    let mut d = PathBuf::from(&home);
    d.push(".config/opencode");
    d
}

fn data_dir_candidates() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/himanshu.".to_string());
    let mut candidates = Vec::new();

    // XDG_DATA_HOME
    if let Ok(val) = std::env::var("XDG_DATA_HOME") {
        let mut d = PathBuf::from(val);
        d.push("opencode");
        candidates.push(d);
    }

    // ~/.local/share/opencode
    let mut d1 = PathBuf::from(&home);
    d1.push(".local/share/opencode");
    candidates.push(d1);

    // ~/.opencode/data
    let mut d2 = PathBuf::from(&home);
    d2.push(".opencode/data");
    candidates.push(d2);

    candidates
}
