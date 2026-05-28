use std::sync::Arc;

use crate::config::Config;
use crate::db::database::Database;

pub async fn start_acp_server(
    _db: Arc<Database>,
    _config: &Config,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("ACP server not yet implemented (Phase 3)");
    // Phase 3: Implement axum-based ACP REST server
    // Endpoints:
    // GET /ping
    // GET /agents
    // GET /agents/session-manager
    // GET /sessions/{session_id}
    // GET /resources/{resource_id}
    Ok(())
}
