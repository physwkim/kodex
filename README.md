# kodex

AI knowledge graph that learns across sessions. Accumulates patterns, decisions, conventions, and domain knowledge as you work — so the next session starts where the last one left off.

Inspired by [graphify](https://github.com/safishamsi/graphify). Built from scratch in Rust with HDF5 as the core storage engine.

## Why HDF5

kodex stores everything in a single `~/.kodex/kodex.h5` file powered by [rust-hdf5](https://crates.io/crates/rust-hdf5) (pure Rust, no C dependency).

| | kodex (HDF5) | JSON-based tools |
|---|---|---|
| **All projects** | Single file | One file per project |
| **10K nodes load** | ~5ms | ~100ms |
| **Add knowledge** | Group append | Full file rewrite |
| **Concurrent sessions** | Actor daemon | File conflicts |
| **Inspection** | h5py, HDFView, Silx | Text editor |
| **Structure** | Hierarchical groups | Flat |

```
~/.kodex/kodex.h5
├── /nodes/          ← code graph (vlen strings, h5py compatible)
│   ├── id, label, file_type, source_file, confidence
│   └── community    (u32)
├── /edges/          ← relationships
│   ├── source, target, relation, confidence
│   └── weight       (f64)
└── /knowledge/      ← AI-accumulated knowledge
    ├── titles, types, descriptions, related, tags (vlen strings)
    ├── confidence   (f64)
    └── observations (u32)
```

## Install

```bash
cargo install --path .
kodex install claude        # register MCP server
```

## Quick Start

```bash
kodex run .                             # analyze codebase → ~/.kodex/kodex.h5
kodex query "how does auth work"        # search
kodex explain "AuthService"             # node details
kodex list                              # registered projects
kodex forget --below 0.3                # clean low-confidence knowledge
```

## Architecture

```
~/.kodex/                              ← global home (single source of truth)
├── kodex.h5                           ← all projects + all knowledge
├── kodex.sock                         ← actor Unix socket
└── registry.json                      ← project paths

~/codes/my-project/kodex-out/          ← view files only (optional)
├── graph.html                         ← interactive visualization
└── GRAPH_REPORT.md                    ← analysis report
```

### Process Model

```
kodex actor (single daemon)
  ├─ owns ~/.kodex/kodex.h5 exclusively
  ├─ listens on ~/.kodex/kodex.sock
  ├─ auto-started by first kodex serve
  └─ auto-exits after 5 min idle

kodex serve (per Claude session, MCP stdio proxy)
  ├─ Claude ←stdin/stdout→ serve ←socket→ actor
  └─ exits when Claude session ends
```

```
Claude A → kodex serve → ┐
Claude B → kodex serve → ├─ kodex.sock → kodex actor → kodex.h5
Claude C → kodex serve → ┘
```

All writes go through one actor. No file locking, no conflicts.

### Data Flow

```
kodex run ./my-project
  ├─ detect → extract (tree-sitter) → build → cluster → analyze
  ├─ save to ~/.kodex/kodex.h5 (tagged with project name)
  ├─ register in ~/.kodex/registry.json
  └─ generate graph.html + GRAPH_REPORT.md in project dir

kodex serve (MCP, auto-started by Claude)
  ├─ learn → actor → kodex.h5 /knowledge/
  ├─ recall → actor → kodex.h5 (searches all projects)
  ├─ forget → actor → remove from kodex.h5
  └─ query_graph → actor → kodex.h5 /nodes/ + /edges/
```

## Commands

| Command | Description |
|---------|-------------|
| `kodex run <path>` | Analyze codebase → save to global h5 |
| `kodex query "<question>"` | BFS/DFS search |
| `kodex path "<src>" "<tgt>"` | Shortest path |
| `kodex explain "<node>"` | Node details + neighbors |
| `kodex update <path>` | Re-extract code (AST only) |
| `kodex serve` | MCP stdio proxy (auto-starts actor) |
| `kodex install <platform>` | Register MCP + skill |
| `kodex list` | Show registered projects |
| `kodex forget [--title\|--type\|--project\|--below]` | Delete knowledge |
| `kodex benchmark` | Token reduction ratio |
| `kodex watch <path>` | Auto-rebuild on changes |

## AI Knowledge System

### How It Works

```
Session 1 (project-a):
  Claude → MCP learn("Repository Pattern", "All DB via *Repo") → kodex.h5

Session 2 (project-b):
  Claude → MCP knowledge_context() → reads from kodex.h5
  → "Repository Pattern (60%, from project-a)" — cross-project
  → same pattern found → learn() → confidence 68%

Session 10:
  → confidence 89% → established knowledge
  → available everywhere

Wrong knowledge?
  Claude → MCP forget({"title": "Bad Pattern"}) → removed from kodex.h5
  or: kodex forget --title "Bad Pattern"
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

Claude Code auto-starts `kodex serve` → auto-starts `kodex actor`.

Also: `kodex install cursor`, `kodex install vscode`, `kodex install codex`, `kodex install kiro`

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

### Confidence

```
Obs 1: 0.60 → Obs 2: 0.68 → Obs 3: 0.74 → Obs 5: 0.83 → Obs 10: 0.93
```

### Forget

```bash
kodex forget --title "Wrong Pattern"    # by title
kodex forget --type bug_pattern         # by type
kodex forget --project old-api          # by project
kodex forget --below 0.3               # low confidence cleanup
```

MCP: `{"method": "forget", "params": {"title": "Wrong Pattern"}}`

## MCP Tools

| Tool | Description |
|------|-------------|
| `query_graph` | BFS/DFS traversal |
| `get_node` | Node details |
| `god_nodes` | Most-connected entities |
| `graph_stats` | Counts |
| `learn` | Store/reinforce knowledge |
| `recall` | Search knowledge (all projects) |
| `knowledge_context` | Session bootstrap summary |
| `forget` | Delete wrong/outdated knowledge |
| `save_insight` | Link nodes with pattern |
| `save_note` | Free-text memo |
| `add_edge` | Add relationship |

## Supported Languages

Python, JavaScript, TypeScript, Go, Rust, Java, C, C++, Ruby, C#, Scala, PHP, Swift, Lua

Tree-sitter AST extraction. Each language feature-gated.

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

HDF5 via [rust-hdf5](https://crates.io/crates/rust-hdf5) always included.

## License

MIT
