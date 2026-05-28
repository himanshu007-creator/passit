use rusqlite::params;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::db::database::{Database, StorageConnector};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub agent_origin: String,
    pub project_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub tags: Vec<String>,
    pub metadata: serde_json::Value,
    pub times_loaded: i64,
    pub last_loaded_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub agent_origin: String,
    pub project_path: Option<String>,
    pub message_count: i64,
    pub last_message_preview: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateSessionParams {
    pub title: String,
    pub agent_origin: String,
    pub project_path: Option<String>,
    pub tags: Vec<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct SessionFilter {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub project_path: Option<String>,
    pub agent: Option<String>,
    pub tag: Option<String>,
    pub since: Option<i64>,
    pub source: Option<String>,
}

impl Default for SessionFilter {
    fn default() -> Self {
        Self {
            limit: Some(20),
            offset: Some(0),
            project_path: None,
            agent: None,
            tag: None,
            since: None,
            source: None,
        }
    }
}

pub fn create_session(db: &Database, params: CreateSessionParams) -> Result<Session, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let id = format!("ses_{}", Ulid::new());
    let now = chrono_now();

    conn.execute(
        "INSERT INTO sessions (id, title, agent_origin, project_path, created_at, updated_at, tags, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            params.title,
            params.agent_origin,
            params.project_path,
            now,
            now,
            serde_json::to_string(&params.tags).unwrap_or_else(|_| "[]".to_string()),
            serde_json::to_string(&params.metadata).unwrap_or_else(|_| "{}".to_string()),
        ],
    )?;

    Ok(Session {
        id,
        title: params.title,
        agent_origin: params.agent_origin,
        project_path: params.project_path,
        created_at: now,
        updated_at: now,
        tags: params.tags,
        metadata: params.metadata,
        times_loaded: 0,
        last_loaded_by: None,
    })
}

pub fn get_session(db: &Database, id: &str) -> Result<Option<Session>, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let mut stmt = conn.prepare(
        "SELECT id, title, agent_origin, project_path, created_at, updated_at, tags, metadata,
                COALESCE(times_loaded, 0), last_loaded_by
         FROM sessions WHERE id = ?1",
    )?;

    let mut rows = stmt.query(params![id])?;
    match rows.next()? {
        Some(row) => Ok(Some(Session {
            id: row.get(0)?,
            title: row.get(1)?,
            agent_origin: row.get(2)?,
            project_path: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            tags: parse_json_array(row.get::<_, String>(6)?),
            metadata: parse_json_value(row.get::<_, String>(7)?),
            times_loaded: row.get(8)?,
            last_loaded_by: row.get(9)?,
        })),
        None => Ok(None),
    }
}

pub fn list_sessions(db: &Database, filter: SessionFilter) -> Result<(Vec<SessionSummary>, i64), rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let mut where_clauses: Vec<String> = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref proj) = filter.project_path {
        where_clauses.push("s.project_path = ?".to_string());
        param_values.push(Box::new(proj.clone()));
    }
    if let Some(ref agent) = filter.agent {
        where_clauses.push("s.agent_origin = ?".to_string());
        param_values.push(Box::new(agent.clone()));
    }
    if let Some(ref tag) = filter.tag {
        where_clauses.push("s.tags LIKE ?".to_string());
        param_values.push(Box::new(format!("%\"{}\"%", tag)));
    }
    if let Some(since) = filter.since {
        where_clauses.push("s.updated_at >= ?".to_string());
        param_values.push(Box::new(since));
    }
    if let Some(ref source) = filter.source {
        where_clauses.push("json_extract(s.metadata, '$.source') = ?".to_string());
        param_values.push(Box::new(source.clone()));
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) FROM sessions s {}", where_sql);
    let total: i64 = conn.query_row(&count_sql, rusqlite::params_from_iter(&param_values), |r| r.get(0))?;

    let limit = filter.limit.unwrap_or(20);
    let offset = filter.offset.unwrap_or(0);
    param_values.push(Box::new(limit as i64));
    param_values.push(Box::new(offset as i64));

    let query_sql = format!(
        "SELECT s.id, s.title, s.agent_origin, s.project_path, s.created_at, s.updated_at, s.tags,
                (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as message_count,
                (SELECT m.content FROM messages m WHERE m.session_id = s.id ORDER BY m.turn_index DESC LIMIT 1) as last_message
         FROM sessions s {}
         ORDER BY s.updated_at DESC
         LIMIT ? OFFSET ?",
        where_sql
    );

    let mut stmt = conn.prepare(&query_sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(&param_values), |row| {
        let preview: Option<String> = row.get(8)?;
        Ok(SessionSummary {
            id: row.get(0)?,
            title: row.get(1)?,
            agent_origin: row.get(2)?,
            project_path: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            tags: parse_json_array(row.get::<_, String>(6)?),
            message_count: row.get(7)?,
            last_message_preview: preview.map(|p| p.chars().take(120).collect()),
        })
    })?;

    let sessions: Vec<SessionSummary> = rows.filter_map(|r| r.ok()).collect();
    Ok((sessions, total))
}

pub fn update_session_timestamp(db: &Database, id: &str) -> Result<(), rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.execute(
        "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
        params![chrono_now(), id],
    )?;
    Ok(())
}

pub fn increment_load_count(db: &Database, id: &str, loaded_by: &str) -> Result<(), rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.execute(
        "UPDATE sessions SET times_loaded = COALESCE(times_loaded, 0) + 1, last_loaded_by = ?1, updated_at = ?2 WHERE id = ?3",
        params![loaded_by, chrono_now(), id],
    )?;
    Ok(())
}

pub fn update_session_title(db: &Database, id: &str, title: &str) -> Result<(), rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.execute(
        "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, chrono_now(), id],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn delete_session(db: &Database, id: &str) -> Result<(), rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis() as i64
}

fn parse_json_array(s: String) -> Vec<String> {
    serde_json::from_str(&s).unwrap_or_default()
}

fn parse_json_value(s: String) -> serde_json::Value {
    serde_json::from_str(&s).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::database::Database;

    fn create_test_session(db: &Database) -> Session {
        create_session(
            db,
            CreateSessionParams {
                title: "Test Session".into(),
                agent_origin: "test-agent".into(),
                project_path: Some("/test/project".into()),
                tags: vec!["rust".into(), "test".into()],
                metadata: serde_json::json!({"key": "value"}),
            },
        )
        .unwrap()
    }

    #[test]
    fn test_create_and_get_session() {
        let db = Database::open_in_memory().unwrap();
        let session = create_test_session(&db);

        assert!(session.id.starts_with("ses_"));
        assert_eq!(session.title, "Test Session");
        assert_eq!(session.agent_origin, "test-agent");
        assert_eq!(session.project_path, Some("/test/project".to_string()));
        assert_eq!(session.tags, vec!["rust", "test"]);
        assert_eq!(session.metadata, serde_json::json!({"key": "value"}));

        let fetched = get_session(&db, &session.id).unwrap().unwrap();
        assert_eq!(fetched.id, session.id);
        assert_eq!(fetched.title, session.title);
    }

    #[test]
    fn test_get_nonexistent_session() {
        let db = Database::open_in_memory().unwrap();
        let result = get_session(&db, "ses_nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_list_sessions_empty() {
        let db = Database::open_in_memory().unwrap();
        let (sessions, total) = list_sessions(&db, SessionFilter::default()).unwrap();
        assert_eq!(total, 0);
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_list_sessions_with_multiple() {
        let db = Database::open_in_memory().unwrap();
        let _s1 = create_test_session(&db);
        let _s2 = create_test_session(&db);

        let (sessions, total) = list_sessions(&db, SessionFilter::default()).unwrap();
        assert_eq!(total, 2);
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_list_sessions_filter_by_agent() {
        let db = Database::open_in_memory().unwrap();
        create_test_session(&db);
        create_session(
            &db,
            CreateSessionParams {
                title: "Other".into(),
                agent_origin: "other-agent".into(),
                project_path: None,
                tags: vec![],
                metadata: serde_json::Value::Object(serde_json::Map::new()),
            },
        )
        .unwrap();

        let (sessions, total) = list_sessions(
            &db,
            SessionFilter {
                agent: Some("test-agent".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(total, 1);
        assert_eq!(sessions[0].agent_origin, "test-agent");
    }

    #[test]
    fn test_list_sessions_filter_by_project() {
        let db = Database::open_in_memory().unwrap();
        create_test_session(&db);
        create_session(
            &db,
            CreateSessionParams {
                title: "No Project".into(),
                agent_origin: "agent".into(),
                project_path: None,
                tags: vec![],
                metadata: serde_json::Value::Object(serde_json::Map::new()),
            },
        )
        .unwrap();

        let (_sessions, total) = list_sessions(
            &db,
            SessionFilter {
                project_path: Some("/test/project".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(total, 1);
    }

    #[test]
    fn test_list_sessions_pagination() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..5 {
            create_session(
                &db,
                CreateSessionParams {
                    title: format!("Session {}", i),
                    agent_origin: "agent".into(),
                    project_path: None,
                    tags: vec![],
                    metadata: serde_json::Value::Object(serde_json::Map::new()),
                },
            )
            .unwrap();
        }

        let (sessions, total) = list_sessions(
            &db,
            SessionFilter {
                limit: Some(2),
                offset: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(total, 5);
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_update_session_title() {
        let db = Database::open_in_memory().unwrap();
        let session = create_test_session(&db);
        update_session_title(&db, &session.id, "Updated Title").unwrap();

        let fetched = get_session(&db, &session.id).unwrap().unwrap();
        assert_eq!(fetched.title, "Updated Title");
    }

    #[test]
    fn test_update_session_timestamp() {
        let db = Database::open_in_memory().unwrap();
        let session = create_test_session(&db);
        std::thread::sleep(std::time::Duration::from_millis(10));
        update_session_timestamp(&db, &session.id).unwrap();

        let fetched = get_session(&db, &session.id).unwrap().unwrap();
        assert!(fetched.updated_at > session.updated_at);
    }

    #[test]
    fn test_delete_session() {
        let db = Database::open_in_memory().unwrap();
        let session = create_test_session(&db);
        delete_session(&db, &session.id).unwrap();
        assert!(get_session(&db, &session.id).unwrap().is_none());
    }

    #[test]
    fn test_generate_title() {
        let title = generate_title("Hello world", "test-agent");
        assert_eq!(title, "[test-agent] Hello world");

        let long = generate_title(&"a".repeat(100), "test-agent");
        assert_eq!(long.len(), 73); // [test-agent] aaaaaa... (13 + 57 + 3 = 73)

        let empty = generate_title("", "test-agent");
        assert_eq!(empty, "[test-agent] Session");
    }

    #[test]
    fn test_create_duplicate_id_not_allowed() {
        let db = Database::open_in_memory().unwrap();
        let session = create_test_session(&db);

        // Manually try to insert with same ID
        let conn = db.conn().lock().expect("poisoned lock");
        let result = conn.execute(
            "INSERT INTO sessions (id, title, agent_origin, created_at, updated_at, tags, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![session.id, "Duplicate", "agent", 0i64, 0i64, "[]", "{}"],
        );
        assert!(result.is_err());
    }
}

pub fn generate_title(content: &str, agent_origin: &str) -> String {
    let first_line = content.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        format!("[{}] Session", agent_origin)
    } else {
        let truncated = if first_line.len() > 60 {
            format!("{}...", &first_line[..57])
        } else {
            first_line.to_string()
        };
        format!("[{}] {}", agent_origin, truncated)
    }
}
