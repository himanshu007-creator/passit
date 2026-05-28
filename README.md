<p align="center">
  <img src="https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white" alt="Rust"/>
  <img src="https://img.shields.io/badge/license-MIT-blue.svg?style=for-the-badge" alt="MIT License"/>
  <img src="https://img.shields.io/badge/MCP-Protocol-7B2FF7?style=for-the-badge" alt="MCP"/>
  <br/>
  <img src="https://img.shields.io/github/v/release/himanshu007-creator/passit?style=flat-square" alt="Latest Release"/>
  <img src="https://img.shields.io/crates/v/passit?style=flat-square" alt="Crates.io"/>
  <img src="https://img.shields.io/badge/Smithery-Installed-7B2FF7?style=flat-square" alt="Smithery"/>
  <a href="https://github.com/himanshu007-creator/passit/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/himanshu007-creator/passit/ci.yml?style=flat-square&label=CI" alt="CI"/></a>
</p>

# passit

**Pass conversations between AI agents — grab, drop, and continue seamlessly.**

passit is a local-first [MCP](https://modelcontextprotocol.io) server that lets you move conversations between any AI coding agent. Stuck on credits in Claude Code? `passit_drop` it into OpenCode. Need to pick up where you left off in Gemini? `passit_push` it over. No more lost context.

```bash
cargo install passit
```

## Why?

AI coding agents are walled gardens. Claude Code can't see what OpenCode discussed. Cursor can't pick up from Gemini. If one agent hits its limit, there's no way to continue elsewhere without copy-pasting.

passit breaks those walls with a shared, persistent, local-first session bus.

- **grab ↔ drop** — grab a conversation from any agent, drop it into another. Continue exactly where you left off.
- **push** — send conversations to other tools and agents
- **scan** — automatically discover and import past conversations from Claude Code, Gemini CLI, OpenCode
- **search** — find anything you've ever discussed, across all agents
- **fork** — branch a conversation to try different approaches
- **trim** — fit conversations within token limits
- **convert** — format conversations for any provider (OpenAI, Anthropic, JSON, transcript)

## Quick Start

### 1. Install

**Via crates.io:**
```bash
cargo install passit
```

**Via Smithery (one-command into Claude Desktop, Cursor, etc.):**
```bash
npx smithery mcp add himanshu007-creator/passit --client claude
```

**Docker:**
```bash
docker run --rm ghcr.io/himanshu007-creator/passit:latest
```

### 2. Configure an agent

Each agent needs an MCP server entry pointing to `passit`.

**OpenCode** — `~/.config/opencode/opencode.json`:
```json
{
  "mcpServers": {
    "passit": {
      "command": "passit",
      "env": {
        "PASSIT_AGENT_ID": "opencode"
      }
    }
  }
}
```

**Claude Code** — `~/.claude/claude.json`:
```json
{
  "mcpServers": {
    "passit": {
      "command": "passit",
      "env": {
        "PASSIT_AGENT_ID": "claude-code"
      }
    }
  }
}
```

**Cursor** — `~/.cursor/mcp.json`:
```json
{
  "mcpServers": {
    "passit": {
      "command": "passit",
      "env": {
        "PASSIT_AGENT_ID": "cursor"
      }
    }
  }
}
```

### 3. Use it

Just talk naturally. The agent picks up what you mean:

> "grab my last claude conversation and continue it here"

> "what conversations do I have saved?"

> "find where we talked about the parser"

> "resume where I left off on opencode"

> "push this session to gemini"

## Tools

| Tool | What it does | Say this |
|------|-------------|---------|
| `save` | Auto-save every turn (called automatically) | — |
| `drop` | Load a conversation into current context | "resume where I left off", "continue that convo" |
| `list` | List all saved conversations | "show my conversations", "what do I have saved" |
| `scan` | Discover & import from agent histories | "find my old conversations", "import from claude" |
| `grab` | Import a conversation from outside | "grab this conversation", "import my old chat" |
| `push` | Export for another agent/tool | "send this to gemini", "export for chatgpt" |
| `search` | Full-text search across all conversations | "find where we talked about X" |
| `fork` | Branch from a specific turn | "fork from turn 5", "try a different path" |
| `summary` | Show passit state, per-source stats, transfers, and compression savings | "what's in passit", "show status", "summary" |
| `trim` | Fit conversation within token limits | "trim this for context" |
| `convert` | Convert between formats | "convert to openai format" |

## CLI

```bash
# Run as MCP server (stdio) — default
passit

# List recent sessions
passit list --limit 10

# Scan agent histories
passit scan

# Export a session
passit export <session-id>
```

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `PASSIT_DB_PATH` | `~/.passit/sessions.db` | SQLite database path |
| `PASSIT_AGENT_ID` | `opencode` | Agent identifier |
| `PASSIT_LOG_LEVEL` | `info` | Log level |

## How It Works

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│ Claude Code │     │   OpenCode   │     │    Gemini   │
│  (source)   │     │  (current)   │     │   (target)  │
└──────┬──────┘     └──────┬───────┘     └──────┬──────┘
       │                   │                    │
       └───────────────────┼────────────────────┘
                           │
                    ┌──────▼──────┐
                    │   passit    │
                    │  MCP Server │  ◄── SQLite
                    │             │       ~/.passit/sessions.db
                    └─────────────┘
                           ▲
                    ┌──────┴──────┐
                    │  Scanner    │  ◄── ~/.claude/projects/
                    │  (on boot)  │       ~/.gemini/tmp/
                    └─────────────┘       ~/.opencode/data/
```

## Development

```bash
git clone https://github.com/himanshu007-creator/passit.git
cd passit
cargo test
cargo build --release
```

## License

MIT — see [LICENSE](LICENSE).

---

Built with Rust and the MCP protocol by [himanshu007-creator](https://github.com/himanshu007-creator).
