# rust-graphify

Rust port of [graphify](https://github.com/safishamsi/graphify) — a knowledge graph builder for code and documents.

Deterministic AST extraction via tree-sitter, community detection, interactive visualization, and MCP server for AI editor integration.

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
```

Output is saved to `my-project/graphify-out/`:
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
| `graphify update <path>` | Re-extract code only (AST, no LLM) |
| `graphify cluster-only <path>` | Rerun clustering on existing graph |
| `graphify watch <path>` | Auto-rebuild on file changes |
| `graphify add <url>` | Fetch URL and add to corpus |
| `graphify serve` | Start MCP stdio server for AI editors |
| `graphify install [platform]` | Install skill to AI editor |
| `graphify benchmark` | Measure token reduction ratio |
| `graphify hook [install\|uninstall\|status]` | Manage git hooks |
| `graphify workspace init` | Create workspace config for multi-project setup |
| `graphify workspace run` | Build + merge all projects in workspace |

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
# Start the server
graphify serve --graph graphify-out/graph.json
```

Available tools: `query_graph`, `get_node`, `god_nodes`, `graph_stats`

### Claude Code integration

```bash
graphify install claude
```

### Cursor / VS Code

```bash
graphify install cursor
graphify install vscode
```

## Obsidian Integration

graphify exports Obsidian-compatible markdown vaults with wikilinks, YAML frontmatter, community overviews, Dataview queries, and bridge node detection.

### Single Project

```bash
graphify run ./my-project
# Vault is generated at ./my-project/graphify-out/
# Open graphify-out/ as an Obsidian vault
```

### Multi-Project Workspace

Use the workspace feature to automatically build, merge, and export multiple projects into a unified graph and Obsidian vault:

```bash
cd ~/codes

# Auto-detect git projects and create config
graphify workspace init

# Edit the generated config
cat graphify-workspace.yaml
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
# Build all projects, merge into unified graph + vault
graphify workspace run

# Override vault path from CLI
graphify workspace run --vault ~/my-vault
```

This produces:
- `graphify-workspace/graph.json` — unified graph across all projects
- `graphify-workspace/graph.html` — unified visualization
- `~/obsidian-vault/dev-knowledge/` — unified Obsidian vault

Cross-project connections are automatically detected: if `frontend` and `backend` both define a `Logger` class, they get linked with `shared_across_projects` edges.

### AI Editor + Obsidian Workflow

```
Obsidian Vault (browse/explore)
  ← graphify run (generates static .md files with wikilinks)

AI Editor (Claude Code, Cursor, etc.)
  ← graphify serve (MCP stdio, interactive queries over graph.json)
```

- **Obsidian** — visual exploration, community maps, Dataview queries
- **MCP server** — AI-assisted conversational queries from your editor
- Both read from the same `graphify-out/` directory. No separate setup needed.

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

Supports WAV directly, other formats via ffmpeg. YouTube download via yt-dlp.

Environment variables:
- `GRAPHIFY_WHISPER_MODEL` — model size: `tiny`, `base`, `small`, `medium`, `large` (default: `base`)
- `GRAPHIFY_WHISPER_MODEL_PATH` — explicit path to `.bin` file
- `GRAPHIFY_WHISPER_PROMPT` — override domain prompt

## Feature Flags

| Feature | Description | Dependency |
|---------|-------------|------------|
| `extract` | AST extraction (default) | tree-sitter |
| `lang-*` | Per-language parsers | tree-sitter-{lang} |
| `all-languages` | All 14 languages | |
| `fetch` | URL fetching for `add` command | reqwest |
| `watch` | File system monitoring | notify |
| `mcp` | Async MCP server | tokio |
| `svg` | SVG graph export | plotters |
| `video` | Audio/video transcription | whisper-rs, hound |
| `parallel` | Parallel file extraction | rayon |
| `all` | Everything except `video` | |

## Architecture

```
CLI (main.rs)
  ├─ detect/     File discovery, classification, .graphifyignore
  ├─ extract/    AST extraction via tree-sitter (14 languages)
  ├─ graph/      petgraph wrapper, build, diff
  ├─ cluster/    Louvain community detection
  ├─ analyze/    God nodes, surprising connections, questions
  ├─ export/     JSON, HTML, Obsidian, GraphML, Canvas, Cypher, Wiki
  ├─ report      GRAPH_REPORT.md generation
  ├─ serve/      MCP stdio server, BFS/DFS traversal
  ├─ cache       SHA256 per-file extraction cache
  ├─ security/   URL validation, SSRF prevention, label sanitization
  ├─ ingest/     URL fetching, content normalization
  ├─ watch       File system monitoring, auto-rebuild
  ├─ hooks       Git post-commit/post-checkout hooks
  ├─ transcribe  whisper.cpp audio transcription
  └─ benchmark   Token reduction measurement
```

## License

MIT
