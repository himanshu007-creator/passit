use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::database::{Database, StorageConnector};
use crate::db::transfers::{count_transfers, recent_transfers, total_tokens_saved};

const PASSIT_LOGO: &str = r"
  ▓▓▓▓   ▓▓▓   ▓▓▓▓  ▓▓▓▓ ▓▓▓ ▓▓▓▓▓
  ▓   ▓ ▓   ▓ ▓     ▓      ▓    ▓
  ▓▓▓▓  ▓▓▓▓▓  ▓▓▓   ▓▓▓   ▓    ▓
  ▓     ▓   ▓     ▓     ▓  ▓    ▓
  ▓     ▓   ▓ ▓▓▓▓  ▓▓▓▓  ▓▓▓   ▓
";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SummaryParams {
    #[allow(dead_code)]
    pub verbose: Option<bool>,
}

pub async fn summary_tool(db: &Database) -> Result<CallToolResult, ErrorData> {
    // ── transfer stats (acquire own lock) ──
    let transfer_count = count_transfers(db).unwrap_or(0);
    let tokens_saved_total = total_tokens_saved(db).unwrap_or(0);

    // ── session totals & by-source (scoped lock) ──
    let (total, total_msgs, by_source) = {
        let conn = db.conn().lock().expect("poisoned lock");

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let total_msgs: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let mut src_stmt = conn
            .prepare(
                "SELECT COALESCE(json_extract(metadata, '$.source'), agent_origin) as source, COUNT(*) as cnt
                 FROM sessions GROUP BY source ORDER BY cnt DESC",
            )
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let by_source: Vec<(String, i64)> = src_stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .filter_map(|r| r.ok())
            .collect();

        (total, total_msgs, by_source)
    };

    // ── db size ──
    let db_path = db.path().to_string_lossy().to_string();
    let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    let size_str = if db_size > 1_000_000 {
        format!("{:.1} MB", db_size as f64 / 1_000_000.0)
    } else if db_size > 1_000 {
        format!("{:.1} KB", db_size as f64 / 1_000.0)
    } else {
        format!("{} B", db_size)
    };

    // ── build the formatted block ──
    let mut out = String::new();
    out.push_str(&format!(
        "passit {} — cross-agent conversation hub\n",
        env!("CARGO_PKG_VERSION")
    ));
    out.push_str(PASSIT_LOGO);
    out.push('\n');

    // ── header banner ──
    out.push_str(&format!(
        "  ┌─ Sessions: {}  ─  Messages: {}  ─  DB: {}  ─  Transfers: {}  ─  Tokens saved: {} ─┐\n",
        total, total_msgs, size_str, transfer_count, tokens_saved_total
    ));
    out.push('\n');

    // ── per-source bar chart ──
    if by_source.is_empty() {
        out.push_str("  No sessions yet — run `scan` to discover conversations.\n");
    } else {
        let max_cnt = by_source.iter().map(|(_, c)| *c).max().unwrap_or(1) as f64;
        out.push_str("  ┌─ By Source ─────────────────────────────────────────────┐\n");
        for (source, count) in &by_source {
            let bar_len = ((*count as f64 / max_cnt) * 30.0).ceil() as usize;
            let bar = "▓".repeat(bar_len);
            out.push_str(&format!("  │ {:12} {:5}  {}\n", source, count, bar));
        }
        out.push_str("  └──────────────────────────────────────────────────────────┘\n");
    }

    // ── recent sessions ──
    let recent_rows: Vec<(String, String, String, i64, i64)> = {
        let conn = db.conn().lock().expect("poisoned lock");
        let mut recent_stmt = conn
            .prepare(
                "SELECT s.id, s.title, s.agent_origin,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                        s.updated_at
                 FROM sessions s ORDER BY s.updated_at DESC LIMIT 5",
            )
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let rows = recent_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .filter_map(|r| r.ok())
            .collect::<Vec<(String, String, String, i64, i64)>>();
        rows
    };
    if !recent_rows.is_empty() {
        out.push_str("\n  ┌─ Recent Sessions ──────────────────────────────────────┐\n");
        for (i, (id, title, _origin, msg_cnt, updated)) in recent_rows.iter().enumerate() {
            let age = fmt_age(*updated);
            let short_id: String = id.chars().take(12).collect();
            let truncated: String = title.chars().take(48).collect();
            out.push_str(&format!(
                "  │ {}. {} ({} — {} msgs, {})\n",
                i + 1,
                truncated,
                short_id,
                    msg_cnt,
                    age
                ));
            }
            out.push_str("  └──────────────────────────────────────────────────────────┘\n");
        }

    // ── recent transfers ──
    if transfer_count > 0 {
        let recent = recent_transfers(db, 3).unwrap_or_default();
        if !recent.is_empty() {
            out.push_str("\n  ┌─ Recent Transfers ─────────────────────────────────────┐\n");
            for ev in &recent {
                let age = fmt_age(ev.transferred_at);
                let sid: String = ev.session_id.chars().take(12).collect();
                out.push_str(&format!(
                    "  │ {} {} → {} ({})\n",
                    sid, ev.from_agent, ev.to_agent, age
                ));
            }
            out.push_str("  └──────────────────────────────────────────────────────────┘\n");
        }
    }

    out.push_str("\n  passit is ready. Run `summary` for this view.\n");

    Ok(CallToolResult::success(vec![Content::text(out)]))
}

fn fmt_age(ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let diff_secs = (now - ms).max(0) / 1000;
    if diff_secs < 60 {
        format!("{}s", diff_secs)
    } else if diff_secs < 3600 {
        format!("{}m", diff_secs / 60)
    } else if diff_secs < 86400 {
        format!("{}h", diff_secs / 3600)
    } else {
        format!("{}d", diff_secs / 86400)
    }
}
