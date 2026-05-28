use crate::db::database::{Database, StorageConnector};
use crate::db::messages::{NewMessage, add_message};
use crate::db::sessions::{CreateSessionParams, chrono_now, create_session};

pub mod claude;
pub mod gemini;
pub mod opencode;

#[derive(Debug, Clone)]
pub struct ImportedMessage {
    pub role: String,
    pub content: String,
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    #[allow(dead_code)]
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ImportedSession {
    pub original_id: String,
    pub source_agent: String,
    pub display_prefix: String,
    pub title: String,
    pub messages: Vec<ImportedMessage>,
    pub project_path: Option<String>,
    #[allow(dead_code)]
    pub created_at: i64,
}

#[derive(Debug, Default)]
pub struct ScannerSummary {
    pub total_sessions: usize,
    pub total_messages: usize,
    pub scanners: Vec<ScannerResult>,
}

#[derive(Debug)]
pub struct ScannerResult {
    pub name: &'static str,
    pub sessions_found: usize,
    pub sessions_imported: usize,
    pub messages_imported: usize,
}

pub trait HistoryScanner: Send + Sync {
    fn name(&self) -> &'static str;
    #[allow(dead_code)]
    fn display_prefix(&self) -> &'static str;
    fn detect(&self) -> bool;
    fn scan(&self) -> Result<Vec<ImportedSession>, String>;
}

pub fn run_history_scanners(db: &Database) -> ScannerSummary {
    let scanners: Vec<Box<dyn HistoryScanner>> = vec![
        Box::new(claude::ClaudeCodeScanner),
        Box::new(gemini::GeminiCliScanner),
        Box::new(opencode::OpenCodeScanner),
    ];

    let mut summary = ScannerSummary::default();

    for scanner in scanners {
        if !scanner.detect() {
            summary.scanners.push(ScannerResult {
                name: scanner.name(),
                sessions_found: 0,
                sessions_imported: 0,
                messages_imported: 0,
            });
            continue;
        }

        let sessions = match scanner.scan() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[history] {} scanner error: {}", scanner.name(), e);
                summary.scanners.push(ScannerResult {
                    name: scanner.name(),
                    sessions_found: 0,
                    sessions_imported: 0,
                    messages_imported: 0,
                });
                continue;
            }
        };

        let found = sessions.len();
        let mut imported = 0usize;
        let mut messages = 0usize;

        for session in sessions {
            if is_already_imported(db, &session.source_agent, &session.original_id) {
                continue;
            }
            match import_into_db(db, &session) {
                Ok(msg_count) => {
                    imported += 1;
                    messages += msg_count;
                }
                Err(e) => {
                    tracing::warn!(
                        "[history] failed to import {} session {}: {}",
                        scanner.name(),
                        session.original_id,
                        e
                    );
                }
            }
        }

        summary.scanners.push(ScannerResult {
            name: scanner.name(),
            sessions_found: found,
            sessions_imported: imported,
            messages_imported: messages,
        });
        summary.total_sessions += imported;
        summary.total_messages += messages;
    }

    if summary.total_sessions > 0 {
        tracing::info!(
            "[history] imported {} sessions ({} messages) from {} source(s)",
            summary.total_sessions,
            summary.total_messages,
            summary
                .scanners
                .iter()
                .filter(|s| s.sessions_imported > 0)
                .count()
        );
    }

    summary
}

fn is_already_imported(db: &Database, source: &str, original_id: &str) -> bool {
    let conn = match db.conn().lock() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let count: Result<i64, _> = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE json_extract(metadata, '$.source') = ?1 AND json_extract(metadata, '$.original_id') = ?2",
        rusqlite::params![source, original_id],
        |r| r.get(0),
    );
    count.map(|c| c > 0).unwrap_or(false)
}

fn import_into_db(db: &Database, session: &ImportedSession) -> Result<usize, String> {
    let metadata = serde_json::json!({
        "source": session.source_agent,
        "original_id": session.original_id,
        "imported_at": chrono_now(),
    });

    let tags = vec![format!("imported:{}", session.source_agent)];

    let created = create_session(
        db,
        CreateSessionParams {
            title: format!("[{}] {}", session.display_prefix, session.title),
            agent_origin: session.source_agent.to_string(),
            project_path: session.project_path.clone(),
            tags,
            metadata,
        },
    )
    .map_err(|e| format!("create_session: {}", e))?;

    for msg in &session.messages {
        add_message(
            db,
            NewMessage {
                session_id: created.id.clone(),
                role: msg.role.clone(),
                content: msg.content.clone(),
                content_type: None,
                agent_id: msg.agent_id.clone(),
                model: msg.model.clone(),
                tokens_in: msg.tokens_in,
                tokens_out: msg.tokens_out,
                metadata: None,
            },
        )
        .map_err(|e| format!("add_message: {}", e))?;
    }

    Ok(session.messages.len())
}
