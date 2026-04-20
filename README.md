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
kodex.h5 (version 0.5.0)
├── /nodes/                  ← code entities
│   ├── uuid                 ← stable identity (survives renames/moves)
│   ├── id, label            ← current name
│   ├── fingerprint          ← matching key (comment/whitespace normalized)
│   ├── body_hash            ← SHA256 of normalized function/class body
│   ├── logical_key          ← human-readable (project/file.py::Class.method)
│   ├── file_type, source_file, source_location, confidence
│   └── community (u32)
├── /edges/                  ← code relationships
│   ├── source, target, relation, confidence
│   ├── source_file, source_location
│   └── weight (f64)
├── /hyperedges/             ← multi-node groups
│   ├── id, label, nodes, confidence, source_file
├── /knowledge/              ← AI-accumulated knowledge
│   ├── uuid                 ← knowledge identity
│   ├── titles, types, descriptions, tags
│   ├── scope, status, source, applies_when
│   ├── evidence             ← where this knowledge came from
│   ├── supersedes / superseded_by
│   ├── confidence (f64), observations (u32)
│   └── created_at, updated_at, last_validated_at (u64)
└── /links/                  ← knowledge ↔ node or knowledge ↔ knowledge
    ├── knowledge_uuid, node_uuid, relation, target_type
    ├── confidence (f64)     ← link confidence
    ├── created_at (u64)     ← when link was created
    ├── linked_body_hash     ← body snapshot for drift detection
    ├── linked_logical_key   ← logical_key snapshot
    ├── link_reason          ← why this link was created
    └── link_source          ← human / agent / inferred
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
3. Body mismatch penalty (-15) → prevents false matches at same position
4. Below threshold → new UUID

Identity rules:
| Scenario | UUID | Reason |
|----------|------|--------|
| Rename, same body | Preserved | body_hash + file match |
| Move to new dir, same body | Preserved | filename + body_hash match |
| Split function | New UUIDs | body mismatch penalty |
| Delete + create at same line | New UUID | body mismatch penalty |
| Reformat only | Preserved | comment/whitespace stripped from hash |

## Version Migration

kodex.h5 auto-migrates when opened by a newer version:

```
v0.1.0 (no uuid/fingerprint)   → auto-generates on load
v0.2.0 (no knowledge uuid)     → auto-generates on load
v0.3.0 (no knowledge metadata) → defaults added on load
v0.4.0 (no evidence/timestamps)→ defaults added on load
v0.5.0 (current)                → no migration needed
```

Semver-safe version comparison (handles 0.10.0, 1.0.0 correctly).

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
| `recall_for_task` | Ranked retrieval (question + files + nodes), diversity-collapsed |
| `recall_for_task_structured` | Same as above + full `ScoreBreakdown` per item |
| `get_task_context` | Full briefing (markdown or `format=json` for structured `TaskContext`) |
| `knowledge_context` | Session bootstrap (all knowledge) |
| `update_knowledge` | Update status/scope/applies_when/superseded_by |
| `validate_knowledge` | Mark as valid, refresh link snapshots, log evidence |
| `mark_obsolete` | Mark as obsolete with reason |
| `forget` | Delete knowledge |

### Knowledge graph
| Tool | Description |
|------|-------------|
| `link_knowledge` | Connect knowledge ↔ knowledge (bidirectional) |
| `link_knowledge_to_nodes` | Connect knowledge → code nodes (with snapshot) |
| `remove_link` | Remove a specific link by source/target/relation |
| `clear_knowledge_links` | Remove all links for a knowledge entry |
| `knowledge_graph` | BFS multi-hop traversal (json or markdown) |
| `knowledge_neighbors` | 1-hop neighbors of a knowledge entry |
| `thought_chain` | Trace reasoning chain (leads_to/because/...) |

### Quality management
| Tool | Description |
|------|-------------|
| `detect_stale` | Find stale knowledge (graduated: deleted nodes, drift, age). `detailed=true` for full report. |
| `find_duplicates` | Detect similar knowledge entries (title + description + type + tags) |
| `merge_knowledge` | Merge duplicate: absorb observations/tags/evidence/links/scope |
| `detect_conflicts` | Find contradictions, superseded-but-active, scope overlaps |
| `knowledge_health` | Health metrics: status counts, orphans, duplicates, overdue, recency |

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

Functions and classes have a `body_hash` — SHA256 of normalized body content (comments, whitespace, formatting stripped). This allows UUID matching to distinguish:

```
Same file, similar position, different body → different entity (new UUID)
Same body, renamed function               → same entity (preserved UUID)
Same body, reformatted code               → same entity (preserved UUID)
```

Normalization strips: `// /* */ #` comments, all whitespace. Only structural code affects the hash.

Matching signals:
| Signal | Points | Notes |
|--------|--------|-------|
| Fingerprint match | 40 | Includes body content |
| Same file | 25 | Full path match |
| Same filename | 15 | Filename only (survives directory moves) |
| Line proximity | 6-15 | Within 20 lines |
| Same type | 10 | Code/Document/etc |
| Label similarity | 0-15 | Token overlap |
| Exact label | 10 | Full label match |
| Body hash match | 25 | Only when both have it |
| Body hash **mismatch** | -15 | Active penalty prevents false matches |

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
  active → needs_review (linked nodes deleted or >50% lost)
  active → needs_review (no validation for 90+ days)
  active → obsolete (superseded by newer knowledge)
  needs_review → active (validated by agent)
  tentative → active (confidence grows above threshold)
```

Staleness detection (graduated):
| Condition | Staleness | Action |
|-----------|-----------|--------|
| All linked nodes deleted | 1.0 | needs_review + confidence decay |
| >50% linked nodes deleted | 0.3-0.7 | needs_review |
| Linked body_hash changed (drift) | 0.2-0.5 | Advisory (no status change) |
| Not validated for 90+ days | 0.3 | needs_review |

Link snapshots: `linked_body_hash` and `linked_logical_key` are captured at link creation. On re-extraction, if the current values differ from the snapshot, drift is detected.

### Lifecycle APIs

```
validate_knowledge(uuid, note="checked against prod")
  → status=active, refresh link snapshots, log evidence

mark_obsolete(uuid, reason="replaced by v2 auth")
  → status=obsolete, log reason to evidence

learn(uuid=K, context_uuid=K_prev)
  → update + auto-chain K_prev →leads_to→ K
```

### Merge Rules

When merging duplicate knowledge (`merge_knowledge`):
| Field | Rule |
|-------|------|
| observations | Sum both |
| confidence | Exponential boost (0.8^n) |
| tags | Union |
| description | Append if different |
| evidence | Concatenate both |
| applies_when | Join if different |
| scope | Keep narrower (node > file > module > project > repo) |
| links | Transfer all (node + knowledge), remove self-referential |

### Retrieval Quality

`recall_for_task` applies:
1. **10-signal scoring**: node overlap (30), file mention (20), confidence (20), applies_when (15), scope (10), recency (10), keyword (10), type priority (5), observations (10), needs_review penalty (50%)
2. **Diversity collapse**: entries with >60% title overlap are deduplicated in top-N
3. **Score breakdown**: `recall_for_task_structured` returns `ScoreBreakdown` with reasons per item

`get_task_context(format="json")` returns structured `TaskContext`:
```json
{
  "relevant": [{"knowledge": {...}, "score": {"total": 75, "reasons": ["linked to code in scope"]}}],
  "warnings": [{"uuid": "...", "reason": "linked nodes may have changed"}],
  "conflicts": [{"uuid_a": "...", "uuid_b": "...", "description": "..."}]
}
```

### Observability

`knowledge_health` returns:
```
active: 42, tentative: 3, needs_review: 5, obsolete: 12
node_links: 87, knowledge_links: 23
orphan_node_links: 0, orphan_knowledge_links: 1
duplicate_candidates: 2, conflicts: 1
validation_overdue: 3, recently_changed_7d: 8, recently_changed_30d: 15
avg_confidence: 0.74, avg_observations: 3.2
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
