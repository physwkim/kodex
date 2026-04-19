# kodex

AI knowledge graph that learns across sessions. Builds a persistent, queryable graph from your codebase and accumulates knowledge as you work — patterns, decisions, conventions, domain concepts — so the next session starts where the last one left off.

Tree-sitter AST extraction, Louvain community detection, HDF5 storage, Obsidian vault integration, MCP server for AI editors. Single Rust binary.

## Install

```bash
cargo build --release
cargo build --release --features all          # all optional features
cargo build --release --features "lang-python,lang-go"  # specific languages
```

## Quick Start

```bash
kodex run ./my-project
kodex query "how does authentication work"
kodex path "Client" "Database"
kodex explain "AuthService"
```

## Output Structure

```
my-project/kodex-out/
├── kodex.h5            ← primary storage (HDF5, fast partial I/O)
├── graph.json           ← JSON compat
├── graph.html           ← interactive visualization (vis.js)
├── GRAPH_REPORT.md      ← analysis report
├── cache/               ← extraction cache
└── vault/               ← Obsidian opens HERE
    ├── .obsidian/
    ├── _KNOWLEDGE_INDEX.md   ← compact knowledge summary (read this first)
    ├── _KNOWLEDGE_*.md       ← AI-accumulated knowledge
    ├── _INSIGHT_*.md         ← AI-discovered insights
    ├── _COMMUNITY_*.md       ← community overviews
    ├── AuthService.md        ← code node notes
    └── GRAPH_REPORT.md
```

Data files (`kodex.h5`, `graph.json`, `graph.html`) stay in `kodex-out/`. Obsidian vault lives in `kodex-out/vault/` — no binary files visible in Obsidian.

## Commands

| Command | Description |
|---------|-------------|
| `kodex run <path>` | Full pipeline: detect → extract → build → cluster → analyze → export |
| `kodex query "<question>"` | BFS/DFS search over the graph |
| `kodex path "<source>" "<target>"` | Shortest path between two nodes |
| `kodex explain "<node>"` | Show node details and neighbors |
| `kodex update <path>` | Re-extract code only (AST, no LLM cost) |
| `kodex cluster-only <path>` | Rerun clustering on existing graph |
| `kodex watch <path> [--vault <path>]` | Auto-rebuild on code changes + vault sync |
| `kodex add <url>` | Fetch URL and add to corpus |
| `kodex serve` | Start MCP stdio server |
| `kodex install [platform]` | Install skill to AI editor |
| `kodex benchmark` | Measure token reduction ratio |
| `kodex hook [install\|uninstall\|status]` | Manage git hooks |
| `kodex workspace init` | Create multi-project workspace config |
| `kodex workspace run [--vault <path>]` | Build + merge all workspace projects |

## AI Knowledge Accumulation

AI agents accumulate knowledge across sessions using the vault as persistent storage. No database — `.md` files are the database.

### How It Works

```
Session 1: Claude analyzes code
  → discovers "Repository pattern for DB access"
  → writes vault/_KNOWLEDGE_Repository_Pattern.md (confidence: 60%)

Session 2: Claude reads _KNOWLEDGE_INDEX.md at session start
  → knows about Repository pattern from session 1
  → discovers same pattern in another module
  → updates file: observations 2, confidence 68%

Session 10: pattern observed repeatedly
  → confidence 89%, 8 observations → established knowledge
```

### Setup

Add to your project's `CLAUDE.md`:

```markdown
## Kodex Knowledge System

### Session start
1. Read `kodex-out/vault/_KNOWLEDGE_INDEX.md` — one file, compact summary
2. Read `kodex-out/GRAPH_REPORT.md` — project structure
3. Only read individual `_KNOWLEDGE_*.md` when you need details

### During work
"Would telling the next session this help?" → save it.
Write to `kodex-out/vault/_KNOWLEDGE_<title>.md`.
If same file exists, increment observations and raise confidence.
Don't touch _KNOWLEDGE_INDEX.md (auto-generated).

See KNOWLEDGE_SYSTEM.md for full rules.
```

### Knowledge Types

| Type | When to Save | Example |
|------|-------------|---------|
| `architecture` | System structure discovered | "3-layer: API → Service → Repository" |
| `pattern` | Design pattern found | "Observer for event handling" |
| `decision` | Design choice with reasoning | "JWT for stateless microservices" |
| `convention` | Code convention discovered | "All errors wrapped in AppError" |
| `coupling` | Module dependency found | "auth changes require session changes" |
| `domain` | Business concept understood | "Trade states: pending → filled → cancelled" |
| `preference` | User working style | "Prefers functional style over OOP" |
| `bug_pattern` | Recurring bug type | "Off-by-one in pagination" |
| `tech_debt` | Improvement opportunity | "Legacy auth middleware needs rewrite" |
| `ops` | Deploy/infra knowledge | "Staging uses different DB credentials" |
| `performance` | Bottleneck found | "N+1 query in user listing endpoint" |
| `lesson` | Mistake learned from | "Don't mock the DB in integration tests" |

Any type not listed — invent one. AI decides what's worth saving.

### Confidence Growth

Repeated observations increase confidence asymptotically:

```
Obs 1: 0.60 → Obs 2: 0.68 → Obs 3: 0.74 → Obs 5: 0.83 → Obs 10: 0.93
```

### Token Optimization

Claude reads **one file** (`_KNOWLEDGE_INDEX.md`, ~500 tokens) instead of all knowledge files (~6000+ tokens). Details are read on-demand.

## Storage

HDF5 is the default storage format via [rust-hdf5](https://crates.io/crates/rust-hdf5) (pure Rust, no C dependency).

| | HDF5 (default) | JSON (compat) |
|---|---|---|
| File | `kodex.h5` | `graph.json` |
| 10K node load | ~5ms | ~100ms |
| Add 1 node | dataset append | full rewrite |
| Concurrent access | SWMR supported | no locking |
| File size (10K nodes) | ~1MB | ~5MB |
| Partial read | per-dataset | full parse |

Both are generated on every `kodex run`. The system prefers `.h5` when available.

## Obsidian Integration

Vault lives at `kodex-out/vault/` — clean, no binary files.

Features:
- YAML frontmatter with source_file, type, community, tags
- `[[wikilinks]]` between connected nodes
- `_COMMUNITY_*.md` with cohesion scores, Dataview queries, bridge nodes
- `.obsidian/graph.json` with community color groups
- Vault is the source of truth — edits sync back to graph

### Single Project

```bash
kodex run ./my-project
# Open my-project/kodex-out/vault/ in Obsidian
```

### Live Sync

```bash
kodex watch ./my-project --vault ./my-project/kodex-out/vault
# Code changes → auto-rebuild graph + vault
# Obsidian edits → sync back to graph
```

### Multi-Project Workspace

```bash
cd ~/codes
kodex workspace init    # auto-detect git projects
```

```yaml
# kodex-workspace.yaml
projects:
  - ./frontend
  - ./backend
  - ./shared-lib

output: ./kodex-workspace
vault: ~/obsidian-vault/dev-knowledge
```

```bash
kodex workspace run
```

**What workspace does:**
1. Builds each project's graph independently
2. Merges into unified graph (cross-project edges for shared names)
3. Runs community detection on merged graph
4. Exports unified vault to configured path
5. **Collects** `_KNOWLEDGE_*.md` from each project into unified vault (tagged with origin)
6. **Distributes** cross-project knowledge back to each project's `kodex-out/vault/`

Result: Claude working on `frontend` can read knowledge discovered in `backend`.

### Workflow

```
Code → kodex run → kodex-out/
                      ├── kodex.h5 (data)
                      └── vault/ (Obsidian)
                            ├── _KNOWLEDGE_INDEX.md ← Claude reads this
                            ├── _KNOWLEDGE_*.md     ← Claude writes these
                            └── *.md                ← code graph nodes

AI Editor → kodex serve (MCP) → query, explain, learn, recall
                ↓
    vault/_KNOWLEDGE_*.md updated → next session reads → better context
```

## MCP Server

```bash
kodex serve
```

| Tool | Description |
|------|-------------|
| `query_graph` | BFS/DFS traversal from keyword-matching nodes |
| `get_node` | Fetch node details by label |
| `god_nodes` | List most-connected entities |
| `graph_stats` | Node/edge/community counts |
| `learn` | Store/reinforce knowledge (auto confidence growth) |
| `recall` | Query accumulated knowledge by keyword or type |
| `knowledge_context` | Compact knowledge summary for session start |
| `save_insight` | Link multiple nodes with a named pattern |
| `save_note` | Free-text memo with related nodes |
| `add_edge` | Add relationship between nodes |

### AI Editor Setup

```bash
kodex install claude    # Claude Code
kodex install cursor    # Cursor
kodex install vscode    # VS Code Copilot
kodex install codex     # Codex
kodex install kiro      # Kiro
```

## Supported Languages

Python, JavaScript, TypeScript, Go, Rust, Java, C, C++, Ruby, C#, Scala, PHP, Swift, Lua

## Export Formats

| Format | File | Use Case |
|--------|------|----------|
| HDF5 | `kodex.h5` | Primary storage, fast queries |
| JSON | `graph.json` | Compat, external tools |
| HTML | `graph.html` | Interactive vis.js visualization |
| Obsidian | `vault/*.md` | Knowledge management |
| Canvas | `graph.canvas` | Obsidian infinite canvas |
| GraphML | `graph.graphml` | Gephi, yEd |
| Neo4j | `import.cypher` | Graph database import |
| Wiki | `wiki/*.md` | Wikipedia-style articles |
| Report | `GRAPH_REPORT.md` | Human-readable analysis |

## Transcription

whisper.cpp (native C++, no Python):

```bash
cargo build --release --features video
mkdir -p ~/.cache/whisper
curl -L -o ~/.cache/whisper/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

| Variable | Description | Default |
|----------|-------------|---------|
| `KODEX_WHISPER_MODEL` | Model size | `base` |
| `KODEX_WHISPER_MODEL_PATH` | Explicit model path | auto-detect |
| `KODEX_WHISPER_PROMPT` | Override domain prompt | from god nodes |

## Feature Flags

| Feature | Description | Dependency |
|---------|-------------|------------|
| `extract` | AST extraction (default) | tree-sitter |
| `lang-*` | Per-language parsers | tree-sitter-{lang} |
| `all-languages` | All 14 supported languages | |
| `fetch` | URL fetching | reqwest |
| `watch` | File monitoring | notify |
| `mcp` | Async MCP server | tokio |
| `svg` | SVG export | plotters |
| `video` | Audio transcription | whisper-rs, hound |
| `parallel` | Parallel extraction | rayon |
| `all` | Everything except `video` | |

Note: HDF5 (`rust-hdf5`) is always included — not feature-gated.

## Architecture

```
CLI (main.rs, clap)
  ├─ detect/      File discovery, .kodexignore, sensitive file filtering
  ├─ extract/     AST extraction via tree-sitter (14 supported languages)
  ├─ graph/       petgraph wrapper (EngramGraph), build, merge, diff
  ├─ cluster/     Louvain community detection with modularity optimization
  ├─ analyze/     God nodes, surprising connections, questions
  ├─ storage      HDF5 read/write (primary format)
  ├─ export/      JSON, HTML, Obsidian, GraphML, Canvas, Cypher, Wiki
  ├─ report       GRAPH_REPORT.md generation
  ├─ serve/       MCP stdio server (JSON-RPC 2.0)
  ├─ workspace    Multi-project build, merge, knowledge sync
  ├─ vault        Vault-native graph loading
  ├─ knowledge    AI insight/note persistence
  ├─ learn        Knowledge accumulation with confidence growth
  ├─ cache        SHA256 per-file extraction cache
  ├─ security/    URL validation, SSRF prevention
  ├─ ingest/      URL fetching, content normalization
  ├─ watch        File monitoring, auto-rebuild, vault sync
  ├─ hooks        Git post-commit/post-checkout hooks
  ├─ transcribe   whisper.cpp transcription
  ├─ install      AI editor skill installation
  └─ benchmark    Token reduction measurement
```

## Pipeline

```
detect → extract → build → cluster → analyze → export
                                                  ├── kodex.h5 (HDF5 primary)
                                                  ├── graph.json (compat)
                                                  ├── graph.html (visualization)
                                                  ├── GRAPH_REPORT.md
                                                  └── vault/ (Obsidian)
                                                       ├── _KNOWLEDGE_INDEX.md
                                                       ├── _KNOWLEDGE_*.md
                                                       ├── _COMMUNITY_*.md
                                                       └── *.md (node notes)
```

## License

MIT
