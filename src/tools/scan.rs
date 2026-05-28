use rmcp::ErrorData;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db::database::Database;
use crate::history;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScanHistoryParams {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScanHistoryResult {
    pub total_imported: usize,
    pub total_messages: usize,
    pub scanners: Vec<ScannerResultInfo>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScannerResultInfo {
    pub name: String,
    pub sessions_found: usize,
    pub sessions_imported: usize,
    pub messages_imported: usize,
}

pub async fn scan_history_tool(
    db: &Database,
    _params: ScanHistoryParams,
) -> Result<CallToolResult, ErrorData> {
    let summary = history::run_history_scanners(db);

    let result = ScanHistoryResult {
        total_imported: summary.total_sessions,
        total_messages: summary.total_messages,
        scanners: summary
            .scanners
            .into_iter()
            .map(|s| ScannerResultInfo {
                name: s.name.to_string(),
                sessions_found: s.sessions_found,
                sessions_imported: s.sessions_imported,
                messages_imported: s.messages_imported,
            })
            .collect(),
    };

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()),
    )]))
}
