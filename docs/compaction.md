# Compaction Architecture

Passit compresses multi-turn conversations into compact handoffs to save token budget on every transfer. Format selection is done via the MCP Elicitation protocol (`peer.elicit::<FormatChoice>()`) when no explicit `format` parameter is provided — see [`architecture.md`](architecture.md#elicit-protocol) for details.

## Three-Layer Output Format

Every handoff (e.g. `load`, `convert briefing/brief`) produces a three-layer document:

```
══ BRIEFING ══
── SESSION FACTS (ANCHOR) ──
GOAL:       [verbatim goal from facts table]
DECISIONS:  [list of decisions]
COMPLETED:  [list of completed items]
FILES:      [list of touched files]

── CONTEXT (COMPRESSED MIDDLE) ──
[summary of resolved/early turns, token-budget-allocated]

── LATEST (turns N–M) ──
[exact last exchanges within VERBATIM_BUDGET]

── PENDING ──
[exact last user message, standalone]

CONTINUE: Respond to PENDING. No re-analysis. No greeting.
```

The receiving agent reads PENDING (redundant with LATEST for agents that skip to the bottom) and responds directly, without re-analyzing history.

## Token Budgets

Three constants control the split, configured via env vars (defaults in parentheses):

| Variable | Default | Purpose |
|---|---|---|
| `PASSIT_VERBATIM_BUDGET` | 2000 | Token budget for the verbatim tail (LATEST). Messages are included newest→oldest until this budget is filled. |
| `PASSIT_SUMMARY_BUDGET` | 1000 | Token budget for the compressed middle summary. |
| `PASSIT_ANCHOR_BUDGET` | 300 | Token budget for the facts anchor section. |

Budget is calculated as `content.len() / 4` (rough character→token ratio).

`budget_split()` in `load.rs` walks messages from newest to oldest, accumulating estimated tokens. Once the verbatim budget is filled, remaining messages go into the middle layer.

## Tool Result Clearing

Before formatting any handoff, `clear_tool_results()` strips re-fetchable tool outputs:

- **Refetchable** if the message contains markers for: `cat`, `ls`, `glob`, `read_file`, `grep`, *or* is >2000 chars and `looks_like_file_dump()` (high density of `/`, `.`, `=`, `-`, `:` characters — typical of file contents).
- **Preserved** if under 200 characters (likely a short error/success message).
- **Preserved** if quoted in any later user or assistant message (avoid breaking context).

## Facts Table Schema

`session_facts` stores structured facts extracted at write-time:

```sql
CREATE TABLE IF NOT EXISTS session_facts (
    id         TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    fact_type  TEXT NOT NULL,
    content    TEXT,
    turn_index INTEGER,
    created_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_facts_session ON session_facts(session_id);
CREATE INDEX IF NOT EXISTS idx_facts_session_type ON session_facts(session_id, fact_type);
```

### Fact Types

| Variant | SQL Value | Source | Cap |
|---|---|---|---|
| `Goal` | `Goal` | First user message, or redirect patterns ("actually", "let's change direction") | 1 |
| `Decision` | `Decision` | Assistant messages matching "we decided", "going with", etc. | 5 |
| `Completed` | `Completed` | Assistant messages matching "done", "implemented", "finished", etc. | 8 |
| `FileTouched` | `FileTouched` | Backtick paths with `/` and file extension, from any role | 10 |
| `StatusSummary` | `StatusSummary` | Reserved for future LLM abstractive summary (Phase 5) | 1 |

Dedup is by `(session_id, LOWER(TRIM(content)))` before insert.

## Extraction Pipeline (Write-Time)

```
save_message()
  ├── add_message()  [always succeeds or fails independently]
  └── try_extract_and_store()  [non-fatal — logs on error, never blocks write]
        └── extract_facts_from_message()
              ├── User message at session start → Goal
              ├── User message matching redirect → upsert Goal
              ├── Assistant message → Decision / Completed / FileTouched patterns
              └── Tool message → FileTouched from backtick paths
```

Goals are upserted (one per session). Other facts are capped per type, deduplicated, then inserted.

This replaces the read-time heuristic extraction that was used before Phase 1 — the old fallback functions (`extract_decisions`, `extract_completed`, `extract_files`) still exist but are only used when no facts table is available.

## Pre-Existing Warnings

The following are known and acceptable, never introduced by this module:

- `acp_bind`, `log_level`, `max_sessions` in Config — reserved for future use
- `open_in_memory`, `create_storage` — unused in production, kept for tests
- `get_facts`, `fact_count_by_type`, `delete_facts` — public API surface, used externally
- `extract_pending` — replaced by inline last-user-message logic in Phase 4
- `created_at` in `ImportedMessage`/`ImportedSession` — schema field, serialized/deserialized
- `display_prefix` in `HistoryScanner` — trait API for future scanners
- `verbose` in `SummaryParams` — reserved for future detail levels
