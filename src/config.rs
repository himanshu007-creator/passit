use std::path::PathBuf;

fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());

    let new_path = PathBuf::from(&home).join(".passit").join("sessions.db");

    // Auto-migrate from ~/.usm/sessions.db on first use
    let old_path = PathBuf::from(&home).join(".usm").join("sessions.db");
    if !new_path.exists() && old_path.exists() {
        if let Some(parent) = new_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::copy(&old_path, &new_path) {
            tracing::warn!("failed to migrate sessions from ~/.usm: {e}");
        } else {
            tracing::info!("migrated existing sessions from ~/.usm to ~/.passit");
        }
    }

    new_path
}

#[derive(Clone, Debug)]
pub struct Config {
    pub db_path: PathBuf,
    pub agent_id: String,
    pub enable_acp: bool,
    pub acp_port: u16,
    #[allow(dead_code)]
    pub acp_bind: String,
    #[allow(dead_code)]
    pub log_level: String,
    #[allow(dead_code)]
    pub max_sessions: u32,
    /// Token budget for verbatim tail of latest exchanges (Phase 2/4).
    pub verbatim_budget: usize,
    /// Token budget for compressed middle section (Phase 2/4).
    pub summary_budget: usize,
    /// Token budget for anchor facts section (Phase 2/4).
    pub anchor_budget: usize,
    /// Whether LLM abstractive summary is available as a load option.
    pub llm_summary_enabled: bool,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            db_path: std::env::var("PASSIT_DB_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| default_db_path()),
            agent_id: std::env::var("PASSIT_AGENT_ID").unwrap_or_else(|_| "opencode".to_string()),
            enable_acp: std::env::var("PASSIT_ENABLE_ACP").is_ok_and(|v| v == "true"),
            acp_port: std::env::var("PASSIT_ACP_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(7700),
            acp_bind: std::env::var("PASSIT_ACP_BIND").unwrap_or_else(|_| "127.0.0.1".to_string()),
            log_level: std::env::var("PASSIT_LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
            max_sessions: std::env::var("PASSIT_MAX_SESSIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(500),
            verbatim_budget: std::env::var("PASSIT_VERBATIM_BUDGET")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000),
            summary_budget: std::env::var("PASSIT_SUMMARY_BUDGET")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
            anchor_budget: std::env::var("PASSIT_ANCHOR_BUDGET")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            llm_summary_enabled: std::env::var("PASSIT_LLM_SUMMARY")
                .is_ok_and(|v| v == "true"),
        }
    }
}
