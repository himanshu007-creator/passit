use rusqlite::params;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::db::database::{Database, StorageConnector};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub turn_index: i64,
    pub role: String,
    pub content: String,
    pub content_type: String,
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub created_at: i64,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub content_type: Option<String>,
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub session_id: String,
    pub session_title: String,
    pub turn_index: i64,
    pub role: String,
    pub content_snippet: String,
}

pub fn add_message(db: &Database, msg: NewMessage) -> Result<Message, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let id = format!("msg_{}", Ulid::new());
    let now = chrono_now();

    let turn_index: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(turn_index), -1) FROM messages WHERE session_id = ?1",
            params![msg.session_id],
            |r| r.get(0),
        )
        .unwrap_or(-1)
        + 1;

    let content_type = msg
        .content_type
        .clone()
        .unwrap_or_else(|| "text/plain".to_string());
    let metadata = msg
        .metadata
        .clone()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    conn.execute(
        "INSERT INTO messages (id, session_id, turn_index, role, content, content_type, agent_id, model, tokens_in, tokens_out, created_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            msg.session_id,
            turn_index,
            msg.role,
            msg.content,
            content_type,
            msg.agent_id,
            msg.model,
            msg.tokens_in.unwrap_or(0),
            msg.tokens_out.unwrap_or(0),
            now,
            serde_json::to_string(&metadata)
                .unwrap_or_else(|_| "{}".to_string()),
        ],
    )?;

    Ok(Message {
        id,
        session_id: msg.session_id,
        turn_index,
        role: msg.role,
        content: msg.content,
        content_type,
        agent_id: msg.agent_id,
        model: msg.model,
        tokens_in: msg.tokens_in.unwrap_or(0),
        tokens_out: msg.tokens_out.unwrap_or(0),
        created_at: now,
        metadata,
    })
}

pub fn get_messages_by_session(
    db: &Database,
    session_id: &str,
) -> Result<Vec<Message>, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let mut stmt = conn.prepare(
        "SELECT id, session_id, turn_index, role, content, content_type, agent_id, model,
                tokens_in, tokens_out, created_at, metadata
         FROM messages WHERE session_id = ?1 ORDER BY turn_index ASC",
    )?;

    let rows = stmt.query_map(params![session_id], |row| {
        Ok(Message {
            id: row.get(0)?,
            session_id: row.get(1)?,
            turn_index: row.get(2)?,
            role: row.get(3)?,
            content: row.get(4)?,
            content_type: row.get(5)?,
            agent_id: row.get(6)?,
            model: row.get(7)?,
            tokens_in: row.get(8)?,
            tokens_out: row.get(9)?,
            created_at: row.get(10)?,
            metadata: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
        })
    })?;

    rows.collect()
}

pub fn search_messages(
    db: &Database,
    query: &str,
    limit: u32,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let pattern = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT m.session_id, s.title, m.turn_index, m.role, substr(m.content, 1, 200)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         WHERE m.content LIKE ?1
         ORDER BY m.created_at DESC
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![pattern, limit], |row| {
        Ok(SearchHit {
            session_id: row.get(0)?,
            session_title: row.get(1)?,
            turn_index: row.get(2)?,
            role: row.get(3)?,
            content_snippet: row.get(4)?,
        })
    })?;

    rows.collect()
}

pub fn get_message_count(db: &Database, session_id: &str) -> Result<i64, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
        params![session_id],
        |r| r.get(0),
    )
}

#[allow(clippy::type_complexity)]
pub fn copy_messages(
    db: &Database,
    source_session_id: &str,
    target_session_id: &str,
    from_turn: i64,
) -> Result<i64, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let mut stmt = conn.prepare(
        "SELECT role, content, content_type, agent_id, model, tokens_in, tokens_out, metadata
         FROM messages WHERE session_id = ?1 AND turn_index >= ?2 ORDER BY turn_index ASC",
    )?;

    let messages: Vec<(
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
        String,
    )> = stmt
        .query_map(params![source_session_id, from_turn], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get::<_, String>(7)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let now = chrono_now();
    let mut count = 0;
    for (role, content, content_type, agent_id, model, tokens_in, tokens_out, metadata) in &messages
    {
        let id = format!("msg_{}", Ulid::new());
        let next_turn: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(turn_index), -1) FROM messages WHERE session_id = ?1",
                params![target_session_id],
                |r| r.get(0),
            )
            .unwrap_or(-1)
            + 1;

        conn.execute(
            "INSERT INTO messages (id, session_id, turn_index, role, content, content_type, agent_id, model, tokens_in, tokens_out, created_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                target_session_id,
                next_turn,
                role,
                content,
                content_type,
                agent_id,
                model,
                tokens_in,
                tokens_out,
                now,
                metadata,
            ],
        )?;
        count += 1;
    }

    Ok(count)
}

pub use crate::db::sessions::chrono_now;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::database::Database;
    use crate::db::sessions::{CreateSessionParams, create_session};

    #[test]
    fn test_add_and_get_messages() {
        let db = Database::open_in_memory().unwrap();
        let session = create_session(
            &db,
            CreateSessionParams {
                title: "Test".into(),
                agent_origin: "test".into(),
                project_path: None,
                tags: vec![],
                metadata: serde_json::Value::Object(serde_json::Map::new()),
            },
        )
        .unwrap();

        let msg1 = add_message(
            &db,
            NewMessage {
                session_id: session.id.clone(),
                role: "user".into(),
                content: "Hello".into(),
                content_type: None,
                agent_id: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                metadata: None,
            },
        )
        .unwrap();
        assert_eq!(msg1.turn_index, 0);

        let msg2 = add_message(
            &db,
            NewMessage {
                session_id: session.id.clone(),
                role: "assistant".into(),
                content: "Hi there".into(),
                content_type: None,
                agent_id: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                metadata: None,
            },
        )
        .unwrap();
        assert_eq!(msg2.turn_index, 1);

        let msgs = get_messages_by_session(&db, &session.id).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "Hello");
        assert_eq!(msgs[1].content, "Hi there");
    }

    #[test]
    fn test_search_messages() {
        let db = Database::open_in_memory().unwrap();
        let session = create_session(
            &db,
            CreateSessionParams {
                title: "Search Test".into(),
                agent_origin: "test".into(),
                project_path: None,
                tags: vec![],
                metadata: serde_json::Value::Object(serde_json::Map::new()),
            },
        )
        .unwrap();

        add_message(
            &db,
            NewMessage {
                session_id: session.id.clone(),
                role: "user".into(),
                content: "How do I implement retry logic?".into(),
                content_type: None,
                agent_id: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                metadata: None,
            },
        )
        .unwrap();

        add_message(
            &db,
            NewMessage {
                session_id: session.id.clone(),
                role: "assistant".into(),
                content: "Here's a retry implementation with exponential backoff".into(),
                content_type: None,
                agent_id: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                metadata: None,
            },
        )
        .unwrap();

        let results = search_messages(&db, "retry", 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_message_count() {
        let db = Database::open_in_memory().unwrap();
        let session = create_session(
            &db,
            CreateSessionParams {
                title: "Count Test".into(),
                agent_origin: "test".into(),
                project_path: None,
                tags: vec![],
                metadata: serde_json::Value::Object(serde_json::Map::new()),
            },
        )
        .unwrap();

        assert_eq!(get_message_count(&db, &session.id).unwrap(), 0);

        add_message(
            &db,
            NewMessage {
                session_id: session.id.clone(),
                role: "user".into(),
                content: "Hi".into(),
                content_type: None,
                agent_id: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                metadata: None,
            },
        )
        .unwrap();

        assert_eq!(get_message_count(&db, &session.id).unwrap(), 1);
    }
}
