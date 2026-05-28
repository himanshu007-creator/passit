use rusqlite::params;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::db::database::{Database, StorageConnector};
use crate::db::sessions::chrono_now;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferEvent {
    pub id: String,
    pub session_id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub transferred_at: i64,
    pub tokens_saved: i64,
}

pub fn log_transfer(
    db: &Database,
    session_id: &str,
    from_agent: &str,
    to_agent: &str,
    tokens_saved: i64,
) -> Result<TransferEvent, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let id = format!("trf_{}", Ulid::new());
    let now = chrono_now();

    conn.execute(
        "INSERT INTO transfer_events (id, session_id, from_agent, to_agent, transferred_at, tokens_saved)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, session_id, from_agent, to_agent, now, tokens_saved],
    )?;

    Ok(TransferEvent {
        id,
        session_id: session_id.to_string(),
        from_agent: from_agent.to_string(),
        to_agent: to_agent.to_string(),
        transferred_at: now,
        tokens_saved,
    })
}

pub fn total_tokens_saved(db: &Database) -> Result<i64, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.query_row(
        "SELECT COALESCE(SUM(tokens_saved), 0) FROM transfer_events",
        [],
        |r| r.get(0),
    )
}

pub fn count_transfers(db: &Database) -> Result<i64, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    conn.query_row("SELECT COUNT(*) FROM transfer_events", [], |r| r.get(0))
}

pub fn recent_transfers(db: &Database, limit: u32) -> Result<Vec<TransferEvent>, rusqlite::Error> {
    let conn = db.conn().lock().expect("poisoned lock on database");
    let mut stmt = conn.prepare(
        "SELECT id, session_id, from_agent, to_agent, transferred_at, COALESCE(tokens_saved, 0)
         FROM transfer_events
         ORDER BY transferred_at DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(TransferEvent {
            id: row.get(0)?,
            session_id: row.get(1)?,
            from_agent: row.get(2)?,
            to_agent: row.get(3)?,
            transferred_at: row.get(4)?,
            tokens_saved: row.get(5)?,
        })
    })?;

    let mut events: Vec<TransferEvent> = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}
