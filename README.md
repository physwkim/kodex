# kodex

AI knowledge graph that learns across sessions. Builds a persistent, queryable graph from your codebase and accumulates knowledge as you work — so the next session starts where the last one left off.

Single Rust binary. HDF5 storage. Tree-sitter AST extraction. MCP server for AI editors.

## Install

```bash
cargo install --path .
```

## Quick Start

```bash
kodex run .                             # build knowledge graph
kodex query "how does auth work"        # search
kodex explain "AuthService"             # node details
kodex serve                             # start MCP server for AI editors
kodex install claude                    # register MCP in Claude Code
```

## Architecture

```
~/.kodex/                              ← global home
├── registry.json                      ← all registered projects
└── workspace.h5                       ← unified knowledge across all projects

~/codes/my-project/
├── kodex-out/
│   ├── kodex.h5        ← project graph + knowledge (source of truth)
│   ├── graph.html      ← interactive visualization
│   ├── GRAPH_REPORT.md ← analysis report
│   ├── cache/          ← extraction cache
│   └── vault/          ← Obsidian vault (on-demand)
│       ├── .obsidian/
│       ├── _COMMUNITY_*.md
│       └── *.md
└── CLAUDE.md           ← AI instructions
```

### Data Flow

```
kodex run
  ├─ detect → extract → build → cluster → analyze
  ├─ kodex.h5          (code graph: nodes, edges, communities)
  ├─ graph.html        (visualization)
  ├─ vault/*.md        (Obsidian export)
  └─ ~/.kodex/         (auto-register project + sync knowledge)

kodex serve (MCP, stdin/stdout)
  ├─ read:   kodex.h5 + ~/.kodex/workspace.h5
  ├─ write:  kodex.h5 /knowledge/ group
  └─ tools:  query_graph, learn, recall, explain, ...

Claude session
  → kodex serve auto-started by MCP config
  → knowledge_context → reads project + global knowledge
  → work, discover patterns
  → learn → writes to kodex.h5
  → next session reads it back
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
    ├── titles, types, descriptions, related, tags (vlen strings)
    ├── confidence   (f64)
    └── observations (u32)

~/.kodex/workspace.h5
└── /knowledge/      ← merged from all projects
```

## Commands

| Command | Description |
|---------|-------------|
| `kodex run <path>` | Full pipeline + auto-register in global workspace |
| `kodex query "<question>"` | BFS/DFS search over the graph |
| `kodex path "<source>" "<target>"` | Shortest path between two nodes |
| `kodex explain "<node>"` | Show node details and neighbors |
| `kodex update <path>` | Re-extract code only (AST, no LLM cost) |
| `kodex cluster-only <path>` | Rerun clustering on existing graph |
| `kodex watch <path>` | Auto-rebuild on code changes |
| `kodex serve` | Start MCP stdio server |
| `kodex install <platform>` | Register MCP server + install skill |
| `kodex list` | Show all registered projects |
| `kodex benchmark` | Measure token reduction ratio |
| `kodex add <url>` | Fetch URL and add to corpus |
| `kodex hook [install\|uninstall\|status]` | Git hooks |
| `kodex workspace init` | Create multi-project config |
| `kodex workspace run` | Build + merge all workspace projects |

## AI Knowledge System

### How It Works

```
Session 1: Claude analyzes code
  → MCP learn("Repository Pattern", "All DB access via *Repo classes")
  → kodex.h5 /knowledge/ (confidence: 60%)
  → auto-synced to ~/.kodex/workspace.h5

Session 2: Claude starts
  → MCP knowledge_context() → reads from kodex.h5 + workspace.h5
  → "Repository Pattern (60%, 1 obs)" — already knows
  → finds same pattern again → learn() → confidence 68%, 2 obs

Session 10:
  → confidence 89%, 8 observations → established knowledge

Different project:
  → MCP recall("repository") → finds it via workspace.h5
  → knowledge crosses project boundaries automatically
```

### Setup

```bash
kodex install claude    # registers MCP server in .claude/settings.json
```

This auto-adds to `.claude/settings.json`:
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

Claude Code starts `kodex serve` automatically. No manual server management.

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

Custom types allowed. AI decides what's worth saving.

### Confidence Growth

```
Obs 1: 0.60 → Obs 2: 0.68 → Obs 3: 0.74 → Obs 5: 0.83 → Obs 10: 0.93
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `query_graph` | BFS/DFS traversal from matching nodes |
| `get_node` | Node details by label |
| `god_nodes` | Most-connected entities |
| `graph_stats` | Node/edge/community counts |
| `learn` | Store/reinforce knowledge in kodex.h5 |
| `recall` | Search knowledge (local + global workspace) |
| `knowledge_context` | Compact summary for session start |
| `save_insight` | Link nodes with a named pattern |
| `save_note` | Free-text memo with related nodes |
| `add_edge` | Add relationship between nodes |

## Global Workspace

Every `kodex run` auto-registers the project in `~/.kodex/registry.json` and syncs knowledge to `~/.kodex/workspace.h5`.

```bash
kodex list
# Registered projects (3):
#   ✓ my-api: /Users/me/codes/my-api
#   ✓ my-frontend: /Users/me/codes/my-frontend
#   ✓ shared-lib: /Users/me/codes/shared-lib
# Workspace: /Users/me/.kodex/workspace.h5
```

- `learn` in any project → synced to workspace.h5
- `recall` in any project → searches local h5 + workspace.h5
- Knowledge crosses project boundaries automatically

## Obsidian

Vault is generated during `kodex run` at `kodex-out/vault/`. Open in Obsidian for visual exploration with wikilinks, community maps, and Dataview queries.

## Supported Languages

Python, JavaScript, TypeScript, Go, Rust, Java, C, C++, Ruby, C#, Scala, PHP, Swift, Lua

## Feature Flags

| Feature | Description | Dependency |
|---------|-------------|------------|
| `extract` | AST extraction (default) | tree-sitter |
| `lang-*` | Per-language parsers | tree-sitter-{lang} |
| `all-languages` | All 14 languages | |
| `fetch` | URL fetching | reqwest |
| `watch` | File monitoring | notify |
| `video` | Audio transcription | whisper-rs, hound |
| `parallel` | Parallel extraction | rayon |
| `all` | Everything except `video` | |

HDF5 via [rust-hdf5](https://crates.io/crates/rust-hdf5) is always included (pure Rust, no C).

## License

MIT
