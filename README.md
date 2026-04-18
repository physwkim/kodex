# rust-graphify

Rust port of [graphify](https://github.com/safishamsi/graphify) — a knowledge graph builder for code and documents.

Deterministic AST extraction via tree-sitter, community detection, interactive visualization, and MCP server for AI editor integration. Single binary, no Python runtime required.

## Install

```bash
# Build with default features (14 languages)
cargo build --release

# Build with all features
cargo build --release --features all

# Build with specific languages only
cargo build --release --features "lang-python,lang-go,lang-rust"
```

## Quick Start

```bash
# Build knowledge graph for a project
graphify run ./my-project

# Query the graph
graphify query "how does authentication work"

# Find shortest path between two concepts
graphify path "Client" "Database"

# Explain a node
graphify explain "AuthService"

# Auto-rebuild on file changes
graphify watch ./my-project
```

Output is saved to `my-project/graphify-out/`:

- `graph.json` — queryable knowledge graph
- `graph.html` — interactive visualization (vis.js)
- `GRAPH_REPORT.md` — analysis report with god nodes, communities, knowledge gaps

## Commands

| Command | Description |
|---------|-------------|
| `graphify run <path>` | Full pipeline: detect → extract → build → cluster → analyze → export |
| `graphify query "<question>"` | BFS/DFS search over the graph |
| `graphify path "<source>" "<target>"` | Shortest path between two nodes |
| `graphify explain "<node>"` | Show node details and neighbors |
| `graphify update <path>` | Re-extract code only (AST, no LLM cost) |
| `graphify cluster-only <path>` | Rerun clustering on existing graph |
| `graphify watch <path>` | Auto-rebuild on code file changes |
| `graphify add <url>` | Fetch URL and add to corpus |
| `graphify serve` | Start MCP stdio server for AI editors |
| `graphify install [platform]` | Install skill to AI editor |
| `graphify benchmark` | Measure token reduction ratio |
| `graphify hook [install\|uninstall\|status]` | Manage git hooks |
| `graphify workspace init` | Create workspace config for multi-project setup |
| `graphify workspace run [--vault <path>]` | Build + merge all projects in workspace |

## Supported Languages

Python, JavaScript, TypeScript, Go, Rust, Java, C, C++, Ruby, C#, Kotlin, Scala, PHP, Swift, Lua

Each language is feature-gated. Use `--features all-languages` for all, or pick specific ones like `--features lang-python`.

## Export Formats

| Format | File | Use Case |
|--------|------|----------|
| JSON | `graph.json` | Programmatic queries, MCP server |
| HTML | `graph.html` | Interactive browser visualization (vis.js) |
| Obsidian | `vault/*.md` | Knowledge management with wikilinks |
| Canvas | `graph.canvas` | Obsidian infinite canvas |
| GraphML | `graph.graphml` | Gephi, yEd desktop visualization |
| Neo4j | `import.cypher` | Graph database import |
| Wiki | `wiki/*.md` | Wikipedia-style articles per community |
| Report | `GRAPH_REPORT.md` | Human-readable analysis |

## MCP Server

graphify includes an MCP (Model Context Protocol) stdio server for AI editor integration.

```bash
graphify serve --graph graphify-out/graph.json
```

Available tools:

| Tool | Description |
|------|-------------|
| `query_graph` | BFS/DFS traversal from keyword-matching nodes |
| `get_node` | Fetch node details by label |
| `god_nodes` | List most-connected entities |
| `graph_stats` | Node/edge/community counts |

### AI Editor Setup

```bash
graphify install claude    # Claude Code
graphify install cursor    # Cursor
graphify install vscode    # VS Code Copilot
graphify install codex     # Codex
graphify install kiro      # Kiro
```

## Obsidian Integration

graphify exports Obsidian-compatible markdown vaults with:

- YAML frontmatter (source_file, type, community, tags)
- `[[wikilinks]]` between connected nodes
- `_COMMUNITY_*.md` overview notes with cohesion scores
- Dataview queries (`TABLE source_file FROM #community/...`)
- Bridge node detection (nodes connecting multiple communities)
- `.obsidian/graph.json` with community color groups

### Single Project

```bash
graphify run ./my-project
# Open ./my-project/graphify-out/ as an Obsidian vault
```

### Multi-Project Workspace

Automatically build, merge, and export multiple projects into a unified graph and Obsidian vault:

```bash
cd ~/codes

# Auto-detect git projects and create config
graphify workspace init
```

Edit the generated config:

```yaml
# graphify-workspace.yaml
projects:
  - ./frontend
  - ./backend
  - ./shared-lib

# Where to write merged graph.json, graph.html, report
output: ./graphify-workspace

# Where to write the unified Obsidian vault
vault: ~/obsidian-vault/dev-knowledge
```

```bash
# Build all projects, merge into unified graph + vault
graphify workspace run

# Override vault path from CLI
graphify workspace run --vault ~/my-vault
```

This produces:

- `graphify-workspace/graph.json` — unified graph across all projects
- `graphify-workspace/graph.html` — unified interactive visualization
- `graphify-workspace/GRAPH_REPORT.md` — unified analysis report
- `~/obsidian-vault/dev-knowledge/` — unified Obsidian vault with cross-project links

Cross-project connections are automatically detected: if `frontend` and `backend` both define a `Logger` class, they are linked with `shared_across_projects` edges. Community detection runs on the merged graph, so clusters can span project boundaries.

### AI Editor + Obsidian Workflow

```
Obsidian Vault (browse & explore)
  ← graphify run / graphify workspace run
  Generates static .md files with wikilinks, Dataview queries, community maps

AI Editor (Claude Code, Cursor, etc.)
  ← graphify serve (MCP stdio server)
  Interactive queries: "how does auth work?", "path from Client to Database"
```

Both read from the same `graphify-out/` directory. No separate setup needed.

## Transcription

Transcribe audio/video files using whisper.cpp (native C++, no Python dependency):

```bash
# Build with video feature
cargo build --release --features video

# Download a Whisper model (once)
mkdir -p ~/.cache/whisper
curl -L -o ~/.cache/whisper/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

Supports WAV directly, other formats via ffmpeg. YouTube/URL audio download via yt-dlp.

| Environment Variable | Description | Default |
|---------------------|-------------|---------|
| `GRAPHIFY_WHISPER_MODEL` | Model size: `tiny`, `base`, `small`, `medium`, `large` | `base` |
| `GRAPHIFY_WHISPER_MODEL_PATH` | Explicit path to `.bin` model file | auto-detect |
| `GRAPHIFY_WHISPER_PROMPT` | Override domain-aware prompt | from god nodes |

## Feature Flags

| Feature | Description | Dependency |
|---------|-------------|------------|
| `extract` | AST extraction (default on) | tree-sitter |
| `lang-*` | Per-language tree-sitter parsers | tree-sitter-{lang} |
| `all-languages` | All 14 languages | |
| `fetch` | URL fetching for `add` command | reqwest |
| `watch` | File system monitoring for `watch` command | notify |
| `mcp` | Async runtime for MCP server | tokio |
| `svg` | SVG graph export | plotters |
| `video` | Audio/video transcription | whisper-rs, hound |
| `parallel` | Parallel file extraction | rayon |
| `all` | Everything except `video` | |

## Architecture

```
CLI (main.rs, clap)
  ├─ detect/      File discovery, classification, .graphifyignore, sensitive file filtering
  ├─ extract/     AST extraction via tree-sitter (14 languages)
  │   └─ languages/   Per-language configs and import handlers
  ├─ graph/       petgraph wrapper (GraphifyGraph), build, merge, diff
  ├─ cluster/     Louvain community detection with modularity optimization
  ├─ analyze/     God nodes, surprising connections, suggested questions
  ├─ export/      JSON, HTML (vis.js), Obsidian, GraphML, Canvas, Cypher, Wiki
  ├─ report       GRAPH_REPORT.md generation
  ├─ serve/       MCP stdio server (JSON-RPC 2.0), BFS/DFS traversal
  ├─ workspace    Multi-project build, merge, unified vault export
  ├─ cache        SHA256 per-file extraction cache
  ├─ security/    URL validation, SSRF prevention, label sanitization
  ├─ ingest/      URL fetching, HTML→text, content normalization
  ├─ watch        File system monitoring, auto-rebuild on code changes
  ├─ hooks        Git post-commit / post-checkout hooks
  ├─ transcribe   whisper.cpp audio/video transcription
  ├─ install      AI editor skill installation (Claude, Cursor, VS Code, etc.)
  └─ benchmark    Token reduction measurement
```

## Pipeline

```
detect → extract → build → cluster → analyze → export
                                                  ├─ graph.json
                                                  ├─ graph.html
                                                  ├─ GRAPH_REPORT.md
                                                  └─ Obsidian vault
```

1. **Detect** — find code/doc/paper/image files, skip sensitive files, apply .graphifyignore
2. **Extract** — parse AST with tree-sitter, extract classes/functions/imports/calls, build call graph
3. **Build** — assemble nodes and edges into a petgraph directed graph
4. **Cluster** — Louvain community detection with modularity-based optimization
5. **Analyze** — identify god nodes, surprising cross-file connections, suggest questions
6. **Export** — write JSON, HTML visualization, Obsidian vault, report

## License

MIT
