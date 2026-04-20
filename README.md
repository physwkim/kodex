# kodex

AI knowledge graph that learns across sessions. Builds a persistent, queryable graph from your codebase and accumulates knowledge as you work — so the next session starts where the last one left off.

Single Rust binary. HDF5 storage. Tree-sitter AST extraction. MCP server for AI editors.

## Install

```bash
cargo install --path .
kodex install claude        # register MCP server in Claude Code
```

## Quick Start

```bash
kodex run .                             # build knowledge graph
kodex query "how does auth work"        # search
kodex explain "AuthService"             # node details
```

No manual server management needed. Claude Code starts `kodex serve` automatically via MCP.

## Architecture

```
~/.kodex/                                    ← global home
├── kodex.sock                               ← actor Unix socket
├── registry.json                            ← registered projects
└── workspace.h5                             ← unified knowledge (all projects)

~/codes/my-project/kodex-out/
├── kodex.h5         ← project graph + knowledge (HDF5, source of truth)
├── graph.html       ← interactive visualization
├── GRAPH_REPORT.md  ← analysis report
├── cache/           ← extraction cache
└── vault/           ← Obsidian export (on-demand)
```

### Process Model

```
kodex actor (single daemon, auto-managed)
  ├─ owns all HDF5 files (no concurrent write conflicts)
  ├─ listens on ~/.kodex/kodex.sock
  ├─ auto-started by first kodex serve
  └─ auto-exits after 5 min idle

kodex serve (per Claude session, MCP stdio proxy)
  ├─ Claude ←stdin/stdout→ serve ←socket→ actor
  ├─ injects project_dir from CWD
  └─ exits when Claude session ends (stdin EOF)
```

```
Claude A → kodex serve → ┐
Claude B → kodex serve → ├─ kodex.sock → kodex actor → kodex.h5
Claude C → kodex serve → ┘                           → workspace.h5
```

All write requests go through one actor — no file locking needed.

### Data Flow

```
kodex run ./my-project
  ├─ detect → extract (tree-sitter) → build → cluster → analyze
  ├─ kodex.h5       (code graph + knowledge)
  ├─ graph.html     (visualization)
  ├─ vault/*.md     (Obsidian)
  └─ auto-register in ~/.kodex/registry.json
                    sync knowledge → ~/.kodex/workspace.h5

kodex serve (MCP)
  → actor reads/writes kodex.h5 per project
  → recall/knowledge_context searches project h5 + workspace.h5
  → learn() writes to project h5, syncs to workspace.h5
```

### HDF5 Structure

```
kodex.h5
├── /nodes/          ← code graph (vlen strings)
│   ├── id, label, file_type, source_file, confidence
│   └── community    (u32)
├── /edges/          ← relationships
│   ├── source, target, relation, confidence
│   └── weight       (f64)
└── /knowledge/      ← AI-accumulated knowledge
    ├── titles, types, descriptions, related, tags
    ├── confidence   (f64)
    └── observations (u32)
```

## Commands

| Command | Description |
|---------|-------------|
| `kodex run <path>` | Build graph + register in global workspace |
| `kodex query "<question>"` | BFS/DFS search |
| `kodex path "<src>" "<tgt>"` | Shortest path |
| `kodex explain "<node>"` | Node details + neighbors |
| `kodex update <path>` | Re-extract code (AST only) |
| `kodex cluster-only <path>` | Rerun clustering |
| `kodex watch <path>` | Auto-rebuild on changes |
| `kodex serve` | MCP stdio proxy (auto-starts actor) |
| `kodex install <platform>` | Register MCP + skill |
| `kodex list` | Show registered projects |
| `kodex benchmark` | Token reduction ratio |
| `kodex add <url>` | Fetch URL to corpus |
| `kodex hook [action]` | Git hooks |
| `kodex workspace init\|run` | Multi-project workspace |

## AI Knowledge System

### How It Works

```
Session 1 (project-a):
  Claude → MCP learn("Repository Pattern", ...) → actor → project-a/kodex.h5
  → auto-sync → ~/.kodex/workspace.h5

Session 2 (project-b):
  Claude → MCP knowledge_context() → actor reads project-b/kodex.h5 + workspace.h5
  → "Repository Pattern (60%, from project-a)" — cross-project knowledge
  → discovers same pattern → learn() → confidence grows

Session 10:
  → confidence 89%, 8 observations → established knowledge
  → available in all projects via workspace.h5
```

### Setup

```bash
kodex install claude
```

Adds to `.claude/settings.json`:
```json
{
  "mcpServers": {
    "kodex": {
      "command": "kodex",
      "args": ["serve"]
    }
  }
}
```

Also supported: `kodex install cursor`, `kodex install vscode`, `kodex install codex`, `kodex install kiro`

### Knowledge Types

| Type | Example |
|------|---------|
| `architecture` | "3-layer: API → Service → Repository" |
| `pattern` | "Observer for event handling" |
| `decision` | "JWT for stateless microservices" |
| `convention` | "All errors wrapped in AppError" |
| `coupling` | "auth changes require session changes" |
| `domain` | "Trade states: pending → filled → cancelled" |
| `preference` | "Prefers functional over OOP" |
| `bug_pattern` | "Off-by-one in pagination" |
| `tech_debt` | "Legacy auth needs rewrite" |
| `ops` | "Staging uses different DB creds" |
| `performance` | "N+1 query in user listing" |
| `lesson` | "Don't mock DB in integration tests" |

Custom types allowed.

### Confidence Growth

```
Obs 1: 0.60 → Obs 2: 0.68 → Obs 3: 0.74 → Obs 5: 0.83 → Obs 10: 0.93
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `query_graph` | BFS/DFS traversal |
| `get_node` | Node details |
| `god_nodes` | Most-connected entities |
| `graph_stats` | Counts |
| `learn` | Store/reinforce knowledge |
| `recall` | Search local + global knowledge |
| `knowledge_context` | Session bootstrap summary |
| `save_insight` | Link nodes with pattern |
| `save_note` | Free-text memo |
| `add_edge` | Add relationship |

## Global Workspace

Every `kodex run` auto-registers the project.

```bash
kodex list
# ✓ my-api: /Users/me/codes/my-api
# ✓ my-frontend: /Users/me/codes/my-frontend
# Workspace: /Users/me/.kodex/workspace.h5
```

Knowledge syncs automatically. Learn in project A, recall in project B.

## Supported Languages

Python, JavaScript, TypeScript, Go, Rust, Java, C, C++, Ruby, C#, Scala, PHP, Swift, Lua

## Feature Flags

| Feature | Description |
|---------|-------------|
| `extract` | AST extraction (default) |
| `lang-*` | Per-language parsers |
| `all-languages` | All 14 languages |
| `fetch` | URL fetching |
| `watch` | File monitoring |
| `video` | Audio transcription (whisper.cpp) |
| `parallel` | Parallel extraction (rayon) |
| `all` | Everything except `video` |

HDF5 via [rust-hdf5](https://crates.io/crates/rust-hdf5) always included (pure Rust).

## License

MIT
