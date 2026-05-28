# Passit Architecture

Passit is a cross-agent conversation continuity server. It lets any AI agent (Claude, OpenCode, Gemini, etc.) persist conversation sessions and resume them later — possibly from a different agent.

## System Overview

```
┌─────────────────────────────────────────────────────────┐
│                    Host Platform                         │
│  (Claude Desktop, OpenCode CLI, Gemini, custom client)  │
└──────────────┬──────────────────────────────┬───────────┘
               │  stdio JSON-RPC (MCP)        │
               ▼                              ▼
┌─────────────────────────────────────────────────────────┐
│                    passit server                         │
│                                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐  │
│  │  Router  │  │   MCP    │  │    Tool Functions     │  │
│  │(server.rs)│──▶  Model  │──▶   (tools/*.rs)       │  │
│  │          │  │(ToolRoute)│  │  save / load / list   │  │
│  │          │  │          │  │  search / convert / …  │  │
│  └──────────┘  └──────────┘  └──────────────────────┘  │
│                                       │                 │
│                                       ▼                 │
│  ┌──────────────────────────────────────────────────┐  │
│  │                  SQLite Database                  │  │
│  │  sessions │ messages │ session_facts │ transfers │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### Layers

**Transport:** Stdio JSON-RPC (MCP protocol). The server reads JSON-RPC requests from stdin and writes responses to stdout. No network layer in production — the host agent spawns `passit` as a subprocess.

**Router (`server.rs`):** Builds an MCP router with one `ToolRoute` per tool. Each route has:
- A **schema** (auto-generated from the params struct via `schemars::JsonSchema`)
- A **name** (e.g., `save`, `drop`, `list`)
- A **description** (the tool's documentation, shown to the LLM)
- A **handler closure** (parses JSON args, calls the Rust function)

**Tool Functions (`tools/*.rs`):** Pure business logic. Each function accepts typed params, operates on the database, and returns an MCP `CallToolResult`. No MCP awareness beyond the entry signature.

**Database (`db/*.rs`):** SQLite via `rusqlite`. Stores sessions, messages, facts, and transfer events. WAL mode for concurrent reads.

## Session Lifecycle

```
CREATE ──▶ SAVE ──▶ SAVE ──▶ ... ──▶ LOAD ──▶ CONTINUE
  │                    ▲                  │
  │                    └── transfer ──────┘
  │                             
  ▼
BRANCH ──▶ new session from checkpoint
```

1. **Save** (`tools/save.rs`): Called after every message exchange. If `session_id` is omitted, auto-creates a new session. Adds the message to the database. Non-fatally extracts facts (goal, decisions, etc.) from the message content.

2. **Load** (`tools/load.rs`): Retrieves a session and formats it for continuation. Supports multiple output formats. When called without a format, returns **conversational elicit text** for interactive menu selection (see Elicit Protocol below). When called with a format, returns the formatted content directly.

3. **Transfer Logging** (`db/transfers.rs`): On each load, computes `tokens_saved = full_transcript_tokens - handoff_tokens` and logs the transfer event. The `summary` tool displays cumulative savings across all transfers.

4. **Branch** (`tools/branch.rs`): Copies a session from a given turn index, creating a new session with shared history. Useful for exploring alternatives.

## Elicit Protocol

The load tool uses MCP Elicitation (MCP 2025-06-18 spec) to present an interactive format picker to the user. When `format` is omitted from the `drop` tool parameters, the server initiates an elicit request via the MCP protocol, and the client renders a native form with a dropdown of available formats.

This is powered by `rmcp` 1.7.0's `Peer::elicit::<T>()` method, which uses the `elicitation` feature flag and the `elicit_safe!` macro to generate JSON schemas from Rust types. `ToolCallContext.request_context.peer` exposes an outbound `Peer<RoleServer>` reference that can initiate elicitation from within a tool handler.

### Flow

```
User: "load my session"
  → Agent calls drop({ session_id: "abc-123" })
  → Server calls peer.elicit::<FormatChoice>("How would you like to load this session?")
  ← MCP client renders interactive picker (native dropdown)
  → User selects format (e.g., "handoff")
  ← Returns session content in chosen format
  → Agent continues work
```

### FormatChoice type

The `FormatChoice` struct (defined in `tools/load.rs`) contains a single `format` field of type `HandoffFormat` enum:

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct FormatChoice {
    pub format: HandoffFormat,
}

rmcp::elicit_safe!(FormatChoice);
```

The `HandoffFormat` enum provides variants: `handoff`, `briefing`, `compact`, `transcript`, `messages`. MCP clients render each variant as a selectable option.

### Fallback behavior

- **Client supports elicitation**: A native picker is shown. If the user cancels or declines, the tool returns a cancellation message. If the client's elicitation mode isn't `form`, defaults to `handoff`.
- **No peer available** (test/CLI context): `load_session` with `format: None` and `peer: None` defaults to `handoff`.
- **Format provided explicitly**: Content is returned directly without any elicitation step.

### load_count semantics

`increment_load_count` is only called when content is actually delivered. When elicit is used, the count is incremented once the user's format choice resolves and content is prepared.

## Output Formats

| Format | Description | Tokens |
|---|---|---|
| `handoff` | Compact summary + last exchanges verbatim + CONTINUE directive | ~15-20% of full |
| `briefing` | Three-layer: ANCHOR facts → COMPRESSED MIDDLE → VERBATIM TAIL → PENDING | ~20-30% of full |
| `compact` | Minimal labels (`[U#N]`/`[A#N]`), no decorative banner, one-line instruction | ~15-20% of full |
| `llm_summary` | LLM-generated abstractive summary (requires `PASSIT_LLM_SUMMARY=true`) | ~5-10% of full |
| `transcript` | Full raw replay | 100% |
| `messages` | Structured JSON array of messages | 100% (structured) |

### Three-Layer Compaction (briefing format)

Every briefing handoff is structured as:

```
══ BRIEFING ══
── SESSION FACTS (ANCHOR) ──
GOAL:       [verbatim from session_facts table]
DECISIONS:  [capped at 5]
COMPLETED:  [capped at 8]
FILES:      [capped at 10]

── CONTEXT (COMPRESSED MIDDLE) ──
[summary of earlier turns — token-budget-constrained]

── LATEST (turns N-M) ──
[last exchanges within VERBATIM_BUDGET]

── PENDING ──
[last user message, standalone]

CONTINUE: Respond to PENDING. No re-analysis. No greeting.
```

### Token Budgets

Three configurable constants split the conversation into layers:

| Env Variable | Default | Function |
|---|---|---|
| `PASSIT_VERBATIM_BUDGET` | 2000 | Token budget for the verbatim tail (LATEST section). Messages included newest→oldest until budget is filled. |
| `PASSIT_SUMMARY_BUDGET` | 1000 | Reserved for compressed middle summary. |
| `PASSIT_ANCHOR_BUDGET` | 300 | Reserved for facts anchor. |

Budget is estimated as `content.len() / 4` (characters to token ratio).

### Tool Result Clearing

Before formatting any handoff, `clear_tool_results()` strips re-fetchable tool outputs:
- Tool messages ≥200 chars with `cat`/`ls`/`glob`/`read_file`/`grep` markers or >2000 chars with file-dump characteristics
- Preserved if <200 chars (likely short error/success), or if content is quoted in later user/assistant messages

## Facts Pipeline

### Schema

```sql
CREATE TABLE session_facts (
    id         TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    fact_type  TEXT NOT NULL,  -- Goal | Decision | Completed | FileTouched | StatusSummary
    content    TEXT,
    turn_index INTEGER,
    created_at INTEGER
);
```

### Extraction (Write-Time)

```
save_session_turn()
  ├── add_message()           -- always succeeds or fails independently
  └── try_extract_and_store() -- non-fatal, logs on error
        └── extract_facts_from_message()
              ├── first user message              → Goal (truncated 200 chars)
              ├── user redirect patterns          → upsert Goal
              ├── assistant "we decided" patterns → Decision (cap 5)
              ├── assistant "done/finished"       → Completed (cap 8)
              └── backtick paths with / + ext     → FileTouched (cap 10)
```

Facts are:
- **Deduplicated** by `(session_id, LOWER(TRIM(content)))` before insert
- **Capped** per type (1 goal, 5 decisions, 8 completed, 10 file_touched)
- **Goal is upserted** — only one goal per session, updated on redirect

### Consumption (Read-Time)

When formatting a handoff, facts are read from the `session_facts` table and placed into the ANCHOR layer. Old heuristic extraction functions (`extract_decisions`, `extract_completed`, etc.) serve as fallbacks when no facts table is available.

## Database Schema (Summary)

```
sessions
├── id TEXT PRIMARY KEY
├── title TEXT
├── agent_origin TEXT
├── project_path TEXT
├── tags TEXT (JSON array)
├── created_at / updated_at INTEGER
├── load_count / last_loaded_by TEXT
└── metadata TEXT (JSON)

messages
├── id TEXT PRIMARY KEY
├── session_id TEXT → sessions(id) ON DELETE CASCADE
├── turn_index INTEGER
├── role TEXT (user|assistant|tool)
├── content TEXT
├── content_type / agent_id / model TEXT
├── tokens_in / tokens_out INTEGER
├── created_at INTEGER
└── metadata TEXT (JSON)

session_facts
├── id TEXT PRIMARY KEY
├── session_id TEXT → sessions(id) ON DELETE CASCADE
├── fact_type TEXT
├── content TEXT
├── turn_index INTEGER
└── created_at INTEGER

transfer_events
├── id TEXT PRIMARY KEY
├── source_agent TEXT
├── target_agent TEXT
├── format TEXT
├── tokens_saved INTEGER
└── created_at INTEGER
```

## MCP Tool Reference

| Tool | Function | Params |
|---|---|---|---|
| `save` | `save_session_turn` | `session_id?`, `role`, `content` |
| `drop` | `load_session` | `session_id`, `format?` |
| `list` | `list_sessions_tool` | `limit?`, `offset?`, `project_path?` |
| `grab` | `grab_session` | `session_id` (auto-save from interrupt) |
| `push` | `push_session` | `source_session_id`, `content` |
| `search` | `search_sessions` | `query`, `limit?` |
| `scan` | `scan_tool` | pattern-based session search |
| `fork` | `branch_session` | `session_id`, `from_turn`, `branch_title?` |
| `status` | `status_tool` | `session_id` |
| `summary` | `summary_tool` | `verbose?` |
| `trim` | `trim_sessions` | `before?`, `keep?` |
| `convert` | `convert_session` | `session_id`, `format?`, `from_turn?` |
