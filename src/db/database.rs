use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Trait for pluggable storage backends.
/// Implementations provide a thread-safe SQLite connection.
pub trait StorageConnector: Send + Sync {
    fn conn(&self) -> &Mutex<Connection>;
}

pub struct Database {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl StorageConnector for Database {
    fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }
}

impl Database {
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        let db = Database {
            conn: Mutex::new(conn),
            db_path: path.to_path_buf(),
        };
        db.run_migrations()?;
        Ok(db)
    }

    #[allow(dead_code)]
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Database {
            conn: Mutex::new(conn),
            db_path: PathBuf::from(":memory:"),
        };
        db.run_migrations()?;
        Ok(db)
    }

    fn run_migrations(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn().lock().expect("poisoned lock on database");
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id            TEXT PRIMARY KEY,
                title         TEXT NOT NULL,
                agent_origin  TEXT NOT NULL,
                project_path  TEXT,
                created_at    INTEGER NOT NULL,
                updated_at    INTEGER NOT NULL,
                tags          TEXT DEFAULT '[]',
                metadata      TEXT DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS messages (
                id            TEXT PRIMARY KEY,
                session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                turn_index    INTEGER NOT NULL,
                role          TEXT NOT NULL CHECK(role IN ('user','assistant','system','tool')),
                content       TEXT NOT NULL,
                content_type  TEXT DEFAULT 'text/plain',
                agent_id      TEXT,
                model         TEXT,
                tokens_in     INTEGER DEFAULT 0,
                tokens_out    INTEGER DEFAULT 0,
                created_at    INTEGER NOT NULL,
                metadata      TEXT DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS transfer_events (
                id             TEXT PRIMARY KEY,
                session_id     TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                from_agent     TEXT NOT NULL,
                to_agent       TEXT NOT NULL,
                transferred_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id);
            CREATE INDEX IF NOT EXISTS idx_messages_turn_index ON messages(session_id, turn_index);
            CREATE INDEX IF NOT EXISTS idx_sessions_updated_at ON sessions(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_path);
            CREATE INDEX IF NOT EXISTS idx_transfer_events_session ON transfer_events(session_id);
            CREATE INDEX IF NOT EXISTS idx_transfer_events_at ON transfer_events(transferred_at DESC);
            ",
        )?;

        // session_facts table (write-time extracted facts for compaction)
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS session_facts (
                id            TEXT PRIMARY KEY,
                session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                fact_type     TEXT NOT NULL,
                content       TEXT NOT NULL,
                turn_index    INTEGER NOT NULL,
                created_at    INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_session_facts_session ON session_facts(session_id);
            CREATE INDEX IF NOT EXISTS idx_session_facts_type ON session_facts(session_id, fact_type);
            ",
        )?;

        // Tolerant column additions (ok if already exist)
        let _ = conn.execute_batch("ALTER TABLE sessions ADD COLUMN times_loaded INTEGER DEFAULT 0");
        let _ = conn.execute_batch("ALTER TABLE sessions ADD COLUMN last_loaded_by TEXT");
        let _ = conn.execute_batch("ALTER TABLE transfer_events ADD COLUMN tokens_saved INTEGER DEFAULT 0");

        Ok(())
    }
}

/// Storage factory: pass a file path or `":memory:"`.
#[allow(dead_code)]
pub fn create_storage(path: &str) -> Result<Database, rusqlite::Error> {
    if path == ":memory:" {
        Database::open_in_memory()
    } else {
        Database::open(Path::new(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_open_in_memory() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn().lock().expect("poisoned lock");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_factory_memory() {
        let db = create_storage(":memory:").unwrap();
        let conn = db.conn().lock().expect("poisoned lock");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_open_file_based() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        let db = Database::open(&path).unwrap();
        let conn = db.conn().lock().expect("poisoned lock");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert!(path.exists());
    }

    #[test]
    fn test_wal_mode_enabled() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal_test.db");
        let db = Database::open(&path).unwrap();
        let conn = db.conn().lock().expect("poisoned lock");
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn test_foreign_keys_enabled() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn().lock().expect("poisoned lock");
        let enabled: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(enabled, 1);
    }

    #[test]
    fn test_create_parent_directories() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("dirs").join("test.db");
        let db = Database::open(&nested).unwrap();
        let conn = db.conn().lock().expect("poisoned lock");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert!(nested.exists());
    }

    #[test]
    fn test_storage_connector_trait() {
        fn use_trait(_s: &impl StorageConnector) {}
        let db = Database::open_in_memory().unwrap();
        use_trait(&db);
    }

    #[test]
    fn test_reopen_persists_data() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("persist.db");
        {
            let db = Database::open(&path).unwrap();
            let conn = db.conn().lock().expect("poisoned lock");
            conn.execute(
                "INSERT INTO sessions (id, title, agent_origin, created_at, updated_at, tags, metadata)
                 VALUES ('ses_test', 'Test', 'agent', 0, 0, '[]', '{}')",
                [],
            )
            .unwrap();
        }
        {
            let db = Database::open(&path).unwrap();
            let conn = db.conn().lock().expect("poisoned lock");
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 1);
        }
    }
}
