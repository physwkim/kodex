# kodex

AI knowledge graph that learns across sessions. Accumulates patterns, decisions, conventions, and domain knowledge as you work — so the next session starts where the last one left off.

Inspired by [graphify](https://github.com/safishamsi/graphify). Built from scratch in Rust with HDF5 as the core storage engine.

## Install

```bash
cargo install --path .
kodex install claude        # register MCP server in Claude Code
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
~/.kodex/                              ← single source of truth
├── kodex.h5                           ← all projects + all knowledge
├── kodex.sock                         ← actor Unix socket
└── registry.json                      ← project paths

~/codes/my-project/kodex-out/          ← view files (optional)
├── graph.html                         ← interactive visualization
└── GRAPH_REPORT.md                    ← analysis report
```

### HDF5 Structure

```
kodex.h5 (version 0.4.0)
├── /nodes/                  ← code entities
│   ├── uuid                 ← stable identity (survives renames/moves)
│   ├── id, label            ← current name
│   ├── fingerprint          ← matching key for re-extraction
│   ├── body_hash            ← SHA256 of normalized function/class body
│   ├── logical_key          ← human-readable (project/file.py::Class.method)
│   ├── file_type, source_file, source_location, confidence
│   └── community (u32)
├── /edges/                  ← code relationships
│   ├── source, target, relation, confidence
│   ├── source_file, source_location
│   └── weight (f64)
├── /knowledge/              ← AI-accumulated knowledge
│   ├── uuid                 ← knowledge identity
│   ├── titles, types, descriptions, tags
│   ├── scope               ← repo / project / module / file / node
│   ├── status              ← active / tentative / obsolete / needs_review
│   ├── source              ← human / inferred / imported / agent
│   ├── applies_when        ← condition string ("auth modification", etc.)
│   ├── supersedes / superseded_by  ← knowledge chain
│   ├── confidence (f64), observations (u32)
│   └── last_validated_at (u64)
└── /links/                  ← knowledge ↔ node or knowledge ↔ knowledge
    ├── knowledge_uuid       ← source (always a knowledge UUID)
    ├── node_uuid            ← target (node UUID or knowledge UUID)
    ├── relation             ← related_to, depends_on, leads_to, contradicts, ...
    └── target_type          ← "" (node) or "knowledge"
```

All data in vlen strings (h5py compatible). Powered by [rust-hdf5](https://crates.io/crates/rust-hdf5) (pure Rust, no C dependency).

### Process Model

```
kodex actor (single daemon, auto-managed)
  ├─ owns kodex.h5 exclusively
  ├─ handles concurrent sessions via thread-per-client
  ├─ auto-started by first kodex serve
  └─ auto-exits after 5 min idle

kodex serve (per Claude session, MCP stdio proxy)
  ├─ Claude ←stdin/stdout→ serve ←socket→ actor
  └─ exits when Claude session ends (stdin EOF)
```

```
Claude A → kodex serve → ┐
Claude B → kodex serve → ├─ kodex.sock → kodex actor → kodex.h5
Claude C → kodex serve → ┘
```

### Data Flow

```
kodex run ./my-project
  ├─ detect → extract (tree-sitter) → build → cluster → analyze
  ├─ merge into ~/.kodex/kodex.h5 (preserves other projects)
  ├─ assign stable UUIDs via fingerprint matching
  ├─ register in ~/.kodex/registry.json
  └─ generate graph.html + GRAPH_REPORT.md

kodex serve (MCP)
  ├─ learn → knowledge entry with UUID → kodex.h5
  ├─ learn(context_uuid=K1) → auto-chain: K1 →leads_to→ K2
  ├─ recall_for_task → ranked by relevance to current files/nodes
  ├─ thought_chain → trace reasoning: root → ... → leaf
  ├─ knowledge_graph → BFS multi-hop over knowledge network
  ├─ link_knowledge → connect knowledge ↔ knowledge
  └─ query_graph → BFS/DFS over code graph
```

## Stable Identity

Nodes and knowledge have separate UUIDs that survive code changes:

```
Session 1:
  authenticate() → node_uuid=N-abc → fingerprint=7f3a...
  Claude learns "JWT pattern" → knowledge_uuid=K-999
  Link: K-999 ↔ N-abc

Refactor: authenticate() → verify_token()

Session 2:
  verify_token() → fingerprint match → same node_uuid=N-abc
  Knowledge link K-999 ↔ N-abc still intact
```

Matching policy:
1. Exact fingerprint (includes body_hash) → reuse UUID
2. Score-based (file + line + type + label + body_hash) → reuse if ≥ 0.4
3. Below threshold → new UUID

## Version Migration

kodex.h5 auto-migrates when opened by a newer version:

```
v0.1.0 (no uuid/fingerprint)   → auto-generates on load
v0.2.0 (no knowledge uuid)     → auto-generates on load
v0.3.0 (no knowledge metadata) → defaults added on load
v0.4.0 (current)                → no migration needed
```

Old h5 files just work. No manual steps.

## Commands

| Command | Description |
|---------|-------------|
| `kodex run <path>` | Analyze + merge into global h5 |
| `kodex query "<question>"` | BFS/DFS search |
| `kodex path "<src>" "<tgt>"` | Shortest path |
| `kodex explain "<node>"` | Node details + neighbors |
| `kodex update <path>` | Re-extract code (AST only) |
| `kodex serve` | MCP stdio proxy (auto-starts actor) |
| `kodex install <platform>` | Register MCP + skill |
| `kodex list` | Show registered projects |
| `kodex forget [--title\|--type\|--project\|--below]` | Delete knowledge |
| `kodex import` | Import Claude Code memories into kodex |
| `kodex export` | Export kodex knowledge to Claude Code memories |
| `kodex benchmark` | Token reduction ratio |
| `kodex watch <path>` | Auto-rebuild on changes |

## AI Knowledge System

### How It Works

```
Session 1 (project-a):
  Claude → learn("Repository Pattern", ...) → kodex.h5 (knowledge_uuid=K-1)

Session 2 (project-b):
  Claude → knowledge_context() → "Repository Pattern (60%)"
  → same pattern → learn() → confidence 68%, observations 2

Session 10:
  → confidence 89% → established knowledge → available everywhere

Wrong?
  Claude → forget({"title": "Bad Pattern"}) → removed
```

### Setup

```bash
kodex install claude
```

Auto-adds to `.claude/settings.json`:
```json
{
  "mcpServers": {
    "kodex": { "command": "kodex", "args": ["serve"] }
  },
  "hooks": {
    "PostToolUse": [{
      "matcher": "Write",
      "command": "if echo \"$TOOL_INPUT\" | grep -q '.claude/memory'; then kodex import 2>/dev/null; fi"
    }]
  }
}
```

The hook auto-syncs Claude memory writes into kodex — every time Claude saves a memory file, kodex imports it.

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

## MCP Tools

### Knowledge lifecycle
| Tool | Description |
|------|-------------|
| `learn` | Store/reinforce knowledge (returns UUID). Pass `context_uuid` to auto-chain. |
| `recall` | Search by keyword/type |
| `recall_for_task` | Ranked retrieval by task context (question + files + nodes) |
| `get_task_context` | Full briefing: ranked knowledge + stale warnings |
| `knowledge_context` | Session bootstrap (all knowledge) |
| `update_knowledge` | Update status/scope/applies_when/superseded_by |
| `forget` | Delete knowledge |

### Knowledge graph
| Tool | Description |
|------|-------------|
| `link_knowledge` | Connect knowledge ↔ knowledge (bidirectional) |
| `link_knowledge_to_nodes` | Connect knowledge → code nodes |
| `remove_link` | Remove a specific link by source/target/relation |
| `clear_knowledge_links` | Remove all links for a knowledge entry |
| `knowledge_graph` | BFS multi-hop traversal (json or markdown) |
| `knowledge_neighbors` | 1-hop neighbors of a knowledge entry |
| `thought_chain` | Trace reasoning chain (leads_to/because/...) |
| `detect_stale` | Find knowledge linked to deleted nodes |

### Code graph
| Tool | Description |
|------|-------------|
| `query_graph` | BFS/DFS traversal |
| `get_node` | Node details |
| `god_nodes` | Most-connected entities |
| `graph_stats` | Counts |
| `save_insight` | Link nodes with pattern |
| `save_note` | Free-text memo |
| `add_edge` | Add relationship |

## Claude Memory Sync

Bidirectional sync between kodex knowledge and Claude Code's `~/.claude/memory/` system.

```bash
kodex import          # ~/.claude/**/memory/*.md → kodex.h5
kodex export          # kodex.h5 → ~/.claude/memory/kodex_*.md
```

**Auto-sync** (installed by `kodex install claude`):
- PostToolUse hook triggers `kodex import` whenever Claude writes to `~/.claude/memory/`
- Imported memories tagged `imported`/`claude-memory` to prevent circular sync
- Export skips already-imported entries

**Type mapping:**
| Claude memory type | kodex knowledge type |
|---|---|
| `feedback` | `preference` |
| `project` | `context` |
| `user` | `preference` |
| `reference` | `api` |

## Body-Aware Fingerprint

Functions and classes now have a `body_hash` — SHA256 of normalized (whitespace-stripped) body content. This allows UUID matching to distinguish:

```
Same file, similar position, different body → different entity (new UUID)
Same body, renamed function               → same entity (preserved UUID)
```

Matching policy with body_hash:
- Fingerprint match (body included): 40 pts
- Same file: 25 pts
- Line proximity: 15 pts
- Same type: 10 pts
- Label similarity: 15 pts
- Exact label: 10 pts
- Body hash match: 20 pts (only when both sides have it)

## Chain of Thought

Agent reasoning forms chains automatically via `context_uuid`:

```
Session 1:
  learn("auth is slow")                    → K1
  learn("N+1 query found", context=K1)     → K2  (K1 →leads_to→ K2)
  learn("eager loading applied", context=K2) → K3  (K2 →leads_to→ K3)

Session 2:
  thought_chain(uuid=K2)

  ## Thought Chain (3 steps)
  1. **auth is slow** (pattern, 60%)
     ↓ leads_to
  2. **N+1 query found** (bug_pattern, 60%)
     ↓ leads_to
  3. **eager loading applied** (decision, 60%)
```

Chain relations: `leads_to`, `because`, `resolved_by`, `therefore`, `implies`

Any node in the chain → auto-walks backward to root, forward to leaf.

## Knowledge Graph

Knowledge entries connect to each other and to code nodes, forming an Obsidian-like graph:

```
knowledge_graph()                    # entire graph
knowledge_graph(uuid="K1", depth=3)  # 3 hops from K1
knowledge_graph(format="markdown")   # agent-readable

  JWT Auth ──supports──→ Stateless API
  JWT Auth ──depends_on─→ Token Rotation
  JWT Auth ←─contradicts─ Session Auth
  JWT Auth ──related_to─→ authenticate()  (code node)
```

Link types:
| Relation | Reverse | Use |
|----------|---------|-----|
| `related_to` | `related_to` | General association |
| `depends_on` | `depended_by` | Prerequisite |
| `supports` | `supported_by` | Reinforcement |
| `contradicts` | `contradicts` | Conflict |
| `supersedes` | `superseded_by` | Replacement |
| `leads_to` | — | Chain of thought |

## Knowledge Lifecycle

```
Status transitions:
  active → needs_review (stale detection: linked nodes deleted)
  active → obsolete (superseded by newer knowledge)
  needs_review → active (validated by agent)
  tentative → active (confidence grows above threshold)
```

Agent can set `applies_when` conditions:
```json
{"uuid": "k-1", "applies_when": "auth module modification"}
```

Supersession chain:
```json
{"uuid": "k-new", "superseded_by": "", "supersedes": "k-old"}
```

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
