# rust-graphify

Rust port of [graphify](https://github.com/safishamsi/graphify) — a knowledge graph builder for code and documents.

Deterministic AST extraction via tree-sitter, community detection, interactive visualization, and MCP server for AI editor integration. Single binary, no Python runtime required.

## Install

```bash
cargo build --release
cargo build --release --features all          # all features
cargo build --release --features "lang-python,lang-go"  # specific languages
```

## Quick Start

```bash
graphify run ./my-project
graphify query "how does authentication work"
graphify path "Client" "Database"
graphify explain "AuthService"
```

Output in `my-project/graphify-out/`:
- `graph.json` — queryable knowledge graph
- `graph.html` — interactive visualization (vis.js)
- `GRAPH_REPORT.md` — analysis report

## Commands

| Command | Description |
|---------|-------------|
| `graphify run <path>` | Full pipeline: detect → extract → build → cluster → analyze → export |
| `graphify query "<question>"` | BFS/DFS search over the graph |
| `graphify path "<source>" "<target>"` | Shortest path between two nodes |
| `graphify explain "<node>"` | Show node details and neighbors |
| `graphify update <path>` | Re-extract code only (AST, no LLM cost) |
| `graphify cluster-only <path>` | Rerun clustering on existing graph |
| `graphify watch <path> [--vault <path>]` | Auto-rebuild on code changes, optionally sync vault |
| `graphify add <url>` | Fetch URL and add to corpus |
| `graphify serve` | Start MCP stdio server |
| `graphify install [platform]` | Install skill to AI editor |
| `graphify benchmark` | Measure token reduction ratio |
| `graphify hook [install\|uninstall\|status]` | Manage git hooks |
| `graphify workspace init` | Create multi-project workspace config |
| `graphify workspace run [--vault <path>]` | Build + merge all workspace projects |

## AI Knowledge Accumulation

graphify enables AI agents (Claude, Cursor, etc.) to **accumulate knowledge across sessions** using the Obsidian vault as persistent storage.

### How It Works

```
Session 1: Claude analyzes code
  → discovers "Repository pattern used for DB access"
  → writes graphify-out/_KNOWLEDGE_Repository_Pattern.md (confidence: 60%)

Session 2: Claude reads _KNOWLEDGE_*.md files at session start
  → knows about Repository pattern from session 1
  → discovers same pattern in another module
  → updates existing file: observations 2, confidence 68%

Session 10: pattern observed repeatedly
  → confidence 89%, 8 observations
  → Claude treats this as established knowledge
```

### Setup

Add this to your project's `CLAUDE.md` (or copy `CLAUDE.md.example`):

```markdown
## Graphify Knowledge System

### Session start
1. Read `graphify-out/_KNOWLEDGE_*.md` — accumulated knowledge from previous sessions
2. Read `graphify-out/GRAPH_REPORT.md` — project structure overview

### During work
When you discover patterns, decisions, conventions, or domain concepts,
save them immediately as `graphify-out/_KNOWLEDGE_<title>.md`.
If the same knowledge file already exists, update observations and confidence.

See KNOWLEDGE_SYSTEM.md for detailed rules.
```

### Knowledge Types

| Type | When to Save | Example |
|------|-------------|---------|
| `pattern` | Architectural pattern found | "Repository pattern for all DB access" |
| `decision` | Design choice with reasoning | "JWT chosen for stateless microservices" |
| `convention` | Code convention discovered | "All errors wrapped in AppError" |
| `coupling` | Module dependency found | "auth changes require session changes" |
| `preference` | User preference learned | "Prefers functional style over OOP" |
| `bug_pattern` | Recurring bug type | "Off-by-one errors in pagination" |
| `domain` | Domain concept understood | "Trade states: pending → filled → cancelled" |

### Knowledge File Format

```markdown
---
type: knowledge
knowledge_type: pattern
confidence: 0.68
observations: 2
first_seen: 1713500000
last_seen: 1713600000
tags: [architecture, data-access]
related_nodes: [user_repo, order_repo, product_repo]
---

# Repository Pattern

All database access goes through *Repo classes that implement a common interface.

---

Session 2: Confirmed ProductRepo also follows this pattern.

## Related

[[user_repo]] [[order_repo]] [[product_repo]]
```

### Confidence Growth

Confidence increases asymptotically with each observation:

```
Observation 1: 0.60
Observation 2: 0.68
Observation 3: 0.74
Observation 5: 0.83
Observation 10: 0.93
```

Formula: `new_confidence = 1.0 - (1.0 - current) * 0.8`

### MCP Tools for Knowledge

When using `graphify serve`, these tools are available:

| Tool | Description |
|------|-------------|
| `learn` | Store or reinforce a knowledge item |
| `recall` | Query knowledge by keyword or type |
| `knowledge_context` | Get compact summary for session bootstrap |
| `save_insight` | Store a pattern/concept linking multiple nodes |
| `save_note` | Store free-text memo with related nodes |
| `add_edge` | Add a single relationship between nodes |

### Vault as Source of Truth

Knowledge files live in `graphify-out/` as plain `.md` files:

```
graphify-out/
├── _KNOWLEDGE_Repository_Pattern.md    ← AI-accumulated knowledge
├── _KNOWLEDGE_JWT_Auth_Decision.md
├── _KNOWLEDGE_Error_Convention.md
├── _INSIGHT_Auth_Facade.md             ← AI-discovered insights
├── _NOTE_Refactoring_Plan.md           ← AI memos
├── _COMMUNITY_Auth.md                  ← auto-generated community overview
├── AuthService.md                      ← auto-generated node note
├── GRAPH_REPORT.md                     ← auto-generated report
├── graph.json                          ← cache (auto-regenerated from vault)
└── graph.html                          ← visualization
```

- **No database needed** — `.md` files are the database
- **Git-trackable** — knowledge history in version control
- **Obsidian-browsable** — open vault in Obsidian for visual exploration
- **AI-readable** — Claude reads `.md` files at session start

## Obsidian Integration

graphify exports Obsidian-compatible markdown vaults with:

- YAML frontmatter (source_file, type, community, tags)
- `[[wikilinks]]` between connected nodes
- `_COMMUNITY_*.md` overview notes with cohesion scores
- Dataview queries (`TABLE source_file FROM #community/...`)
- Bridge node detection (nodes connecting multiple communities)

### Single Project

```bash
graphify run ./my-project
# Open ./my-project/graphify-out/ as an Obsidian vault
```

### Live Sync

```bash
# Code changes → auto-rebuild graph + vault
graphify watch ./my-project --vault ./my-project/graphify-out

# Edits in Obsidian (add/remove [[wikilinks]]) sync back to graph.json
```

### Multi-Project Workspace

```bash
cd ~/codes
graphify workspace init        # auto-detect git projects, create config
```

```yaml
# graphify-workspace.yaml
projects:
  - ./frontend
  - ./backend
  - ./shared-lib

output: ./graphify-workspace
vault: ~/obsidian-vault/dev-knowledge
```

```bash
graphify workspace run                     # build + merge all projects
graphify workspace run --vault ~/my-vault  # override vault path
```

Cross-project connections are automatically detected. Community detection runs on the merged graph, so clusters can span project boundaries.

### Workflow

```
Code → graphify run → Obsidian Vault (browse, explore, edit)
                  ↓
           graph.json (cache)
                  ↓
     AI Editor → graphify serve (MCP) → query, explain, learn
                  ↓
         _KNOWLEDGE_*.md (accumulated knowledge)
                  ↓
         Next session → Claude reads → better context
```

## MCP Server

```bash
graphify serve --graph graphify-out/graph.json
```

| Tool | Description |
|------|-------------|
| `query_graph` | BFS/DFS traversal from keyword-matching nodes |
| `get_node` | Fetch node details by label |
| `god_nodes` | List most-connected entities |
| `graph_stats` | Node/edge/community counts |
| `learn` | Store/reinforce knowledge |
| `recall` | Query accumulated knowledge |
| `knowledge_context` | Compact summary for session bootstrap |
| `save_insight` | Link multiple nodes with a named pattern |
| `save_note` | Free-text memo with related nodes |
| `add_edge` | Add relationship between nodes |

### AI Editor Setup

```bash
graphify install claude    # Claude Code
graphify install cursor    # Cursor
graphify install vscode    # VS Code Copilot
graphify install codex     # Codex
graphify install kiro      # Kiro
```

## Supported Languages

Python, JavaScript, TypeScript, Go, Rust, Java, C, C++, Ruby, C#, Kotlin, Scala, PHP, Swift, Lua

## Export Formats

| Format | File | Use Case |
|--------|------|----------|
| JSON | `graph.json` | Programmatic queries, MCP server |
| HTML | `graph.html` | Interactive browser visualization (vis.js) |
| Obsidian | `*.md` | Knowledge management with wikilinks |
| Canvas | `graph.canvas` | Obsidian infinite canvas |
| GraphML | `graph.graphml` | Gephi, yEd desktop visualization |
| Neo4j | `import.cypher` | Graph database import |
| Wiki | `wiki/*.md` | Wikipedia-style articles per community |
| Report | `GRAPH_REPORT.md` | Human-readable analysis |

## Transcription

Transcribe audio/video using whisper.cpp (native C++, no Python):

```bash
cargo build --release --features video

mkdir -p ~/.cache/whisper
curl -L -o ~/.cache/whisper/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

| Environment Variable | Description | Default |
|---------------------|-------------|---------|
| `GRAPHIFY_WHISPER_MODEL` | Model size | `base` |
| `GRAPHIFY_WHISPER_MODEL_PATH` | Explicit model path | auto-detect |
| `GRAPHIFY_WHISPER_PROMPT` | Override domain prompt | from god nodes |

## Feature Flags

| Feature | Description | Dependency |
|---------|-------------|------------|
| `extract` | AST extraction (default) | tree-sitter |
| `lang-*` | Per-language parsers | tree-sitter-{lang} |
| `all-languages` | All 14 languages | |
| `fetch` | URL fetching | reqwest |
| `watch` | File system monitoring | notify |
| `mcp` | Async MCP server | tokio |
| `svg` | SVG graph export | plotters |
| `video` | Audio/video transcription | whisper-rs, hound |
| `parallel` | Parallel file extraction | rayon |
| `all` | Everything except `video` | |

## Architecture

```
CLI (main.rs, clap)
  ├─ detect/      File discovery, classification, .graphifyignore
  ├─ extract/     AST extraction via tree-sitter (14 languages)
  ├─ graph/       petgraph wrapper, build, merge, diff
  ├─ cluster/     Louvain community detection
  ├─ analyze/     God nodes, surprising connections, questions
  ├─ export/      JSON, HTML, Obsidian, GraphML, Canvas, Cypher, Wiki
  ├─ report       GRAPH_REPORT.md generation
  ├─ serve/       MCP stdio server (JSON-RPC 2.0)
  ├─ workspace    Multi-project build + merge
  ├─ vault        Vault-native graph loading (vault = source of truth)
  ├─ knowledge    AI insight/note persistence
  ├─ learn        Knowledge accumulation across sessions
  ├─ cache        SHA256 per-file extraction cache
  ├─ security/    URL validation, SSRF prevention
  ├─ ingest/      URL fetching, content normalization
  ├─ watch        File monitoring, auto-rebuild, vault sync
  ├─ hooks        Git hooks
  ├─ transcribe   whisper.cpp transcription
  ├─ install      AI editor skill installation
  └─ benchmark    Token reduction measurement
```

## Pipeline

```
detect → extract → build → cluster → analyze → export
                                                  ├─ graph.json (cache)
                                                  ├─ graph.html
                                                  ├─ GRAPH_REPORT.md
                                                  ├─ Obsidian vault (.md)
                                                  └─ _KNOWLEDGE_*.md (AI knowledge)
```

## License

MIT
