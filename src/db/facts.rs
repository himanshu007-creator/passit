use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::database::{Database, StorageConnector};
use crate::db::messages::Message;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum FactType {
    Goal,
    Decision,
    Completed,
    FileTouched,
    StatusSummary,
}

impl FactType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FactType::Goal => "goal",
            FactType::Decision => "decision",
            FactType::Completed => "completed",
            FactType::FileTouched => "file_touched",
            FactType::StatusSummary => "status_summary",
        }
    }

    pub fn max_per_session(&self) -> i64 {
        match self {
            FactType::Goal => 1,
            FactType::Decision => 5,
            FactType::Completed => 8,
            FactType::FileTouched => 10,
            FactType::StatusSummary => 1,
        }
    }
}

impl std::fmt::Display for FactType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for FactType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "goal" => Ok(FactType::Goal),
            "decision" => Ok(FactType::Decision),
            "completed" => Ok(FactType::Completed),
            "file_touched" => Ok(FactType::FileTouched),
            "status_summary" => Ok(FactType::StatusSummary),
            _ => Err(format!("unknown fact type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFact {
    pub id: String,
    pub session_id: String,
    pub fact_type: FactType,
    pub content: String,
    pub turn_index: i64,
    pub created_at: i64,
}

pub struct NewFact {
    pub session_id: String,
    pub fact_type: FactType,
    pub content: String,
    pub turn_index: i64,
}

#[derive(Debug)]
pub struct ExtractedFact {
    pub fact_type: FactType,
    pub content: String,
}

// ── CRUD ──

pub fn add_fact(db: &Database, fact: NewFact) -> Result<SessionFact, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let id = format!("fact_{}", ulid::Ulid::new());
    let now = chrono_now();

    conn.execute(
        "INSERT INTO session_facts (id, session_id, fact_type, content, turn_index, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            fact.session_id,
            fact.fact_type.as_str(),
            fact.content,
            fact.turn_index,
            now
        ],
    )?;

    Ok(SessionFact {
        id,
        session_id: fact.session_id,
        fact_type: fact.fact_type,
        content: fact.content,
        turn_index: fact.turn_index,
        created_at: now,
    })
}

#[allow(dead_code)]
pub fn get_facts(db: &Database, session_id: &str) -> Result<Vec<SessionFact>, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let mut stmt = conn.prepare(
        "SELECT id, session_id, fact_type, content, turn_index, created_at
         FROM session_facts WHERE session_id = ?1 ORDER BY turn_index ASC",
    )?;

    let rows = stmt.query_map(params![session_id], |row| {
        let ft: String = row.get(2)?;
        Ok(SessionFact {
            id: row.get(0)?,
            session_id: row.get(1)?,
            fact_type: ft.parse().unwrap_or(FactType::Goal),
            content: row.get(3)?,
            turn_index: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn get_facts_by_type(
    db: &Database,
    session_id: &str,
    fact_type: FactType,
) -> Result<Vec<SessionFact>, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let mut stmt = conn.prepare(
        "SELECT id, session_id, fact_type, content, turn_index, created_at
         FROM session_facts WHERE session_id = ?1 AND fact_type = ?2 ORDER BY turn_index ASC",
    )?;

    let rows = stmt.query_map(params![session_id, fact_type.as_str()], |row| {
        let ft: String = row.get(2)?;
        Ok(SessionFact {
            id: row.get(0)?,
            session_id: row.get(1)?,
            fact_type: ft.parse().unwrap_or(FactType::Goal),
            content: row.get(3)?,
            turn_index: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

#[allow(dead_code)]
pub fn fact_count_by_type(
    db: &Database,
    session_id: &str,
    fact_type: FactType,
) -> Result<i64, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.query_row(
        "SELECT COUNT(*) FROM session_facts WHERE session_id = ?1 AND fact_type = ?2",
        params![session_id, fact_type.as_str()],
        |r| r.get(0),
    )
}

pub fn prune_facts(
    db: &Database,
    session_id: &str,
    fact_type: FactType,
    max_count: i64,
) -> Result<(), rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.execute(
        "DELETE FROM session_facts WHERE id IN (
            SELECT id FROM session_facts
            WHERE session_id = ?1 AND fact_type = ?2
            ORDER BY turn_index ASC
            LIMIT MAX(0, (SELECT COUNT(*) FROM session_facts WHERE session_id = ?1 AND fact_type = ?2) - ?3)
        )",
        params![session_id, fact_type.as_str(), max_count],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn delete_facts(db: &Database, session_id: &str) -> Result<(), rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.execute(
        "DELETE FROM session_facts WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

pub fn upsert_goal(
    db: &Database,
    session_id: &str,
    content: &str,
    turn_index: i64,
) -> Result<SessionFact, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let now = chrono_now();

    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM session_facts WHERE session_id = ?1 AND fact_type = 'goal' LIMIT 1",
            params![session_id],
            |r| r.get(0),
        )
        .ok();

    if let Some(existing_id) = existing {
        conn.execute(
            "UPDATE session_facts SET content = ?1, turn_index = ?2, created_at = ?3 WHERE id = ?4",
            params![content, turn_index, now, existing_id],
        )?;
        Ok(SessionFact {
            id: existing_id,
            session_id: session_id.to_string(),
            fact_type: FactType::Goal,
            content: content.to_string(),
            turn_index,
            created_at: now,
        })
    } else {
        let id = format!("fact_{}", ulid::Ulid::new());
        conn.execute(
            "INSERT INTO session_facts (id, session_id, fact_type, content, turn_index, created_at)
             VALUES (?1, ?2, 'goal', ?3, ?4, ?5)",
            params![id, session_id, content, turn_index, now],
        )?;
        Ok(SessionFact {
            id,
            session_id: session_id.to_string(),
            fact_type: FactType::Goal,
            content: content.to_string(),
            turn_index,
            created_at: now,
        })
    }
}

fn is_duplicate_fact(
    db: &Database,
    session_id: &str,
    content: &str,
) -> Result<bool, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let normalized = content.trim().to_lowercase();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_facts WHERE session_id = ?1 AND LOWER(TRIM(content)) = ?2",
        params![session_id, normalized],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

// ── Write-time extraction from single message ──

const DECISION_PATTERNS: &[&str] = &[
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

const COMPLETED_PATTERNS: &[&str] = &[
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

const GOAL_REDIRECT_PATTERNS: &[&str] = &[
    "actually",
    "let's change direction",
    "instead let's",
    "on second thought",
    "new plan",
    "let me clarify",
    "what i really want",
    "the actual goal",
];

pub fn extract_facts_from_message(msg: &Message, is_first_user: bool) -> Vec<ExtractedFact> {
    let mut facts: Vec<ExtractedFact> = Vec::new();

    match msg.role.as_str() {
        "user" => {
            let text = msg.content.trim();
            if text.is_empty() {
                return facts;
            }
            if is_first_user {
                let truncated: String = text.chars().take(200).collect();
                let content = if text.len() > 200 {
                    format!("{}...", truncated)
                } else {
                    truncated
                };
                facts.push(ExtractedFact {
                    fact_type: FactType::Goal,
                    content,
                });
            } else {
                // Check for goal redirect
                let lower = text.to_lowercase();
                if GOAL_REDIRECT_PATTERNS.iter().any(|p| lower.contains(p)) && text.len() > 20 {
                    let truncated: String = text.chars().take(200).collect();
                    let content = if text.len() > 200 {
                        format!("{}...", truncated)
                    } else {
                        truncated
                    };
                    facts.push(ExtractedFact {
                        fact_type: FactType::Goal,
                        content,
                    });
                }
            }
        }
        "assistant" => {
            let lower = msg.content.to_lowercase();

            // Decisions
            for pattern in DECISION_PATTERNS {
                if let Some(idx) = lower.find(pattern) {
                    let before = &msg.content[..idx];
                    let sent_start = before.rfind(['.', '!', '?']).map(|p| p + 1).unwrap_or(0);
                    let after = &msg.content[idx..];
                    let rel_end = after
                        .find(['.', '!', '?'])
                        .map(|p| p + 1)
                        .unwrap_or_else(|| after.len().min(150));
                    let sentence = msg.content[sent_start..idx + rel_end].trim().to_string();
                    if !sentence.is_empty() {
                        facts.push(ExtractedFact {
                            fact_type: FactType::Decision,
                            content: sentence,
                        });
                    }
                    break;
                }
            }

            // Completed
            let first_line = msg.content.lines().next().unwrap_or("").trim().to_string();
            if first_line.len() > 10
                && first_line.len() < 120
                && !first_line.starts_with('`')
                && !first_line.starts_with("```")
            {
                let keyword_hit = COMPLETED_PATTERNS.iter().any(|k| lower.contains(k));
                if keyword_hit {
                    let item = first_line.trim_end_matches(&['.', '!', ';'][..]);
                    facts.push(ExtractedFact {
                        fact_type: FactType::Completed,
                        content: item.to_string(),
                    });
                }
            }

            // Files touched (in messages too)
            for segment in msg.content.split('`') {
                let path = segment.trim();
                if path.is_empty() || path.len() > 200 || path.contains('\n') {
                    continue;
                }
                let is_path = (path.contains('/') || path.starts_with('~'))
                    && (path.contains('.') || path.starts_with('/'));
                let is_code_marker =
                    path.starts_with("```") || path == "//" || path.starts_with('\\');
                if is_path && !is_code_marker {
                    facts.push(ExtractedFact {
                        fact_type: FactType::FileTouched,
                        content: path.to_string(),
                    });
                }
            }
        }
        "tool" => {
            // Files touched in tool results
            for segment in msg.content.split('`') {
                let path = segment.trim();
                if path.is_empty() || path.len() > 200 || path.contains('\n') {
                    continue;
                }
                let is_path = (path.contains('/') || path.starts_with('~'))
                    && (path.contains('.') || path.starts_with('/'));
                let is_code_marker =
                    path.starts_with("```") || path == "//" || path.starts_with('\\');
                if is_path && !is_code_marker {
                    facts.push(ExtractedFact {
                        fact_type: FactType::FileTouched,
                        content: path.to_string(),
                    });
                }
            }
        }
        _ => {}
    }

    facts
}

/// Non-fatal wrapper: extract facts from message and store them.
/// Logs errors internally, never returns them — designed to never block the write.
pub fn try_extract_and_store(db: &Database, msg: &Message, is_first_user: bool) {
    let facts = extract_facts_from_message(msg, is_first_user);
    for fact in facts {
        match fact.fact_type {
            FactType::Goal => {
                if let Err(e) = upsert_goal(db, &msg.session_id, &fact.content, msg.turn_index) {
                    tracing::warn!("failed to upsert goal fact: {e}");
                }
            }
            _ => {
                match is_duplicate_fact(db, &msg.session_id, &fact.content) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(e) => {
                        tracing::warn!("failed to check duplicate fact: {e}");
                        continue;
                    }
                }
                let nf = NewFact {
                    session_id: msg.session_id.clone(),
                    fact_type: fact.fact_type,
                    content: fact.content,
                    turn_index: msg.turn_index,
                };
                if let Err(e) = add_fact(db, nf) {
                    tracing::warn!("failed to store fact: {e}");
                    continue;
                }
                if let Err(e) = prune_facts(
                    db,
                    &msg.session_id,
                    fact.fact_type,
                    fact.fact_type.max_per_session(),
                ) {
                    tracing::warn!("failed to prune facts: {e}");
                }
            }
        }
    }
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::database::Database;
    use crate::db::sessions::{CreateSessionParams, create_session};

    fn create_test_session(db: &Database) -> String {
        let session = create_session(
            db,
            CreateSessionParams {
                title: "Fact Test".into(),
                agent_origin: "test".into(),
                project_path: None,
                tags: vec![],
                metadata: serde_json::Value::Object(serde_json::Map::new()),
            },
        )
        .unwrap();
        session.id
    }

    #[test]
    fn test_add_and_get_facts() {
        let db = Database::open_in_memory().unwrap();
        let sid = create_test_session(&db);

        let f = add_fact(
            &db,
            NewFact {
                session_id: sid.clone(),
                fact_type: FactType::Decision,
                content: "Use Rust for the backend".into(),
                turn_index: 3,
            },
        )
        .unwrap();
        assert!(f.id.starts_with("fact_"));

        let facts = get_facts(&db, &sid).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].content, "Use Rust for the backend");
    }

    #[test]
    fn test_get_facts_by_type() {
        let db = Database::open_in_memory().unwrap();
        let sid = create_test_session(&db);

        add_fact(
            &db,
            NewFact {
                session_id: sid.clone(),
                fact_type: FactType::Decision,
                content: "d1".into(),
                turn_index: 1,
            },
        )
        .unwrap();
        add_fact(
            &db,
            NewFact {
                session_id: sid.clone(),
                fact_type: FactType::Completed,
                content: "c1".into(),
                turn_index: 2,
            },
        )
        .unwrap();
        add_fact(
            &db,
            NewFact {
                session_id: sid.clone(),
                fact_type: FactType::Decision,
                content: "d2".into(),
                turn_index: 3,
            },
        )
        .unwrap();

        let decisions = get_facts_by_type(&db, &sid, FactType::Decision).unwrap();
        assert_eq!(decisions.len(), 2);
        let completed = get_facts_by_type(&db, &sid, FactType::Completed).unwrap();
        assert_eq!(completed.len(), 1);
    }

    #[test]
    fn test_prune_facts() {
        let db = Database::open_in_memory().unwrap();
        let sid = create_test_session(&db);

        for i in 0..10 {
            add_fact(
                &db,
                NewFact {
                    session_id: sid.clone(),
                    fact_type: FactType::FileTouched,
                    content: format!("/path/file{}.rs", i),
                    turn_index: i as i64,
                },
            )
            .unwrap();
        }

        prune_facts(&db, &sid, FactType::FileTouched, 5).unwrap();
        let remaining = get_facts_by_type(&db, &sid, FactType::FileTouched).unwrap();
        assert_eq!(remaining.len(), 5, "expected 5 after prune");
        // Should keep newest, so check last turn_index
        assert_eq!(remaining.last().unwrap().turn_index, 9);
    }

    #[test]
    fn test_delete_facts() {
        let db = Database::open_in_memory().unwrap();
        let sid = create_test_session(&db);

        add_fact(
            &db,
            NewFact {
                session_id: sid.clone(),
                fact_type: FactType::Goal,
                content: "test".into(),
                turn_index: 0,
            },
        )
        .unwrap();
        delete_facts(&db, &sid).unwrap();

        let facts = get_facts(&db, &sid).unwrap();
        assert!(facts.is_empty());
    }

    #[test]
    fn test_upsert_goal_creates() {
        let db = Database::open_in_memory().unwrap();
        let sid = create_test_session(&db);

        let f = upsert_goal(&db, &sid, "Build a web app", 0).unwrap();
        assert_eq!(f.fact_type, FactType::Goal);
        assert_eq!(f.content, "Build a web app");

        let facts = get_facts_by_type(&db, &sid, FactType::Goal).unwrap();
        assert_eq!(facts.len(), 1);
    }

    #[test]
    fn test_upsert_goal_updates() {
        let db = Database::open_in_memory().unwrap();
        let sid = create_test_session(&db);

        upsert_goal(&db, &sid, "Build a web app", 0).unwrap();
        upsert_goal(&db, &sid, "Build a mobile app instead", 5).unwrap();

        let facts = get_facts_by_type(&db, &sid, FactType::Goal).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].content, "Build a mobile app instead");
        assert_eq!(facts[0].turn_index, 5);
    }

    #[test]
    fn test_fact_type_display_and_parse() {
        assert_eq!(FactType::Goal.as_str(), "goal");
        assert_eq!(FactType::Decision.as_str(), "decision");

        assert_eq!("goal".parse::<FactType>().unwrap(), FactType::Goal);
        assert_eq!(
            "status_summary".parse::<FactType>().unwrap(),
            FactType::StatusSummary
        );
        assert!("unknown".parse::<FactType>().is_err());
    }

    #[test]
    fn test_cascade_delete() {
        let db = Database::open_in_memory().unwrap();
        let sid = create_test_session(&db);

        add_fact(
            &db,
            NewFact {
                session_id: sid.clone(),
                fact_type: FactType::Goal,
                content: "g".into(),
                turn_index: 0,
            },
        )
        .unwrap();
        add_fact(
            &db,
            NewFact {
                session_id: sid.clone(),
                fact_type: FactType::Decision,
                content: "d".into(),
                turn_index: 1,
            },
        )
        .unwrap();

        // Delete the session — facts should cascade
        let conn = db.conn().lock().expect("poisoned lock");
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![sid])
            .unwrap();
        drop(conn);

        let facts = get_facts(&db, &sid).unwrap();
        assert!(facts.is_empty());
    }

    #[test]
    fn test_goal_max_per_session() {
        assert_eq!(FactType::Goal.max_per_session(), 1);
        assert_eq!(FactType::Decision.max_per_session(), 5);
        assert_eq!(FactType::Completed.max_per_session(), 8);
        assert_eq!(FactType::FileTouched.max_per_session(), 10);
    }

    // ── extraction tests ──

    fn make_msg(role: &str, content: &str, turn: i64) -> Message {
        Message {
            id: format!("msg_{}", ulid::Ulid::new()),
            session_id: "ses_test".into(),
            turn_index: turn,
            role: role.into(),
            content: content.into(),
            content_type: "text/plain".into(),
            agent_id: None,
            model: None,
            tokens_in: 0,
            tokens_out: 0,
            created_at: 0,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    #[test]
    fn test_extract_goal_from_first_user_message() {
        let msg = make_msg("user", "We need to build a CRUD app", 0);
        let facts = extract_facts_from_message(&msg, true);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].fact_type, FactType::Goal);
        assert_eq!(facts[0].content, "We need to build a CRUD app");
    }

    #[test]
    fn test_no_goal_from_nonfirst_user_message() {
        let msg = make_msg("user", "Let's add authentication", 5);
        let facts = extract_facts_from_message(&msg, false);
        // No redirect patterns in this message
        assert!(facts.is_empty());
    }

    #[test]
    fn test_goal_redirect_pattern() {
        let msg = make_msg(
            "user",
            "Actually, let's change direction and use Python instead",
            5,
        );
        let facts = extract_facts_from_message(&msg, false);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].fact_type, FactType::Goal);
    }

    #[test]
    fn test_extract_decision() {
        let msg = make_msg("assistant", "We decided to use SQLite for the database.", 3);
        let facts = extract_facts_from_message(&msg, false);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].fact_type, FactType::Decision);
        assert!(facts[0].content.contains("SQLite"));
    }

    #[test]
    fn test_extract_completed() {
        let msg = make_msg(
            "assistant",
            "Implemented the user authentication module.\n\nNext we need to...",
            8,
        );
        let facts = extract_facts_from_message(&msg, false);
        let completed: Vec<_> = facts
            .iter()
            .filter(|f| f.fact_type == FactType::Completed)
            .collect();
        assert_eq!(completed.len(), 1);
        assert_eq!(
            completed[0].content,
            "Implemented the user authentication module"
        );
    }

    #[test]
    fn test_extract_file_touched() {
        let msg = make_msg("assistant", "Check `src/main.rs` for the entry point", 4);
        let facts = extract_facts_from_message(&msg, false);
        let files: Vec<_> = facts
            .iter()
            .filter(|f| f.fact_type == FactType::FileTouched)
            .collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "src/main.rs");
    }

    #[test]
    fn test_extract_file_from_tool() {
        let msg = make_msg("tool", "Found in `src/db/database.rs`", 5);
        let facts = extract_facts_from_message(&msg, false);
        let files: Vec<_> = facts
            .iter()
            .filter(|f| f.fact_type == FactType::FileTouched)
            .collect();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_extract_skips_empty_content() {
        let msg = make_msg("user", "", 0);
        let facts = extract_facts_from_message(&msg, true);
        assert!(facts.is_empty());
    }

    #[test]
    fn test_extract_no_decision_on_user_message() {
        let msg = make_msg("user", "We decided to use Rust", 1);
        let facts = extract_facts_from_message(&msg, false);
        assert!(!facts.iter().any(|f| f.fact_type == FactType::Decision));
    }
}
