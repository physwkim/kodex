# kodex

AI knowledge graph that learns across sessions. Accumulates patterns, decisions, conventions, and domain knowledge as you work — so the next session starts where the last one left off.

Inspired by [graphify](https://github.com/safishamsi/graphify). Built from scratch in Rust with SQLite as the storage engine.

## Install

```bash
cargo install --path .
kodex install claude        # register MCP server in Claude Code
```

## Quick Start

```bash
kodex run .                             # analyze codebase → ~/.kodex/kodex.db (registers project)
kodex query "how does auth work"        # search
kodex explain "AuthService"             # node details
kodex list                              # registered projects
kodex forget --below 0.3                # clean low-confidence knowledge
```

After `kodex run`, the project is registered and a global git hook (installed by `kodex install claude`) auto-updates the graph + ingests new knowledge on every commit. Unregistered repos see no effect. See [Auto-Ingestion](#auto-ingestion).

## Architecture

```
~/.kodex/                              ← single source of truth
├── kodex.db                           ← all projects + all knowledge (SQLite)
├── kodex.sock                         ← actor Unix socket
└── registry.json                      ← project paths

~/codes/my-project/kodex-out/          ← view files (optional)
├── graph.html                         ← interactive visualization
└── GRAPH_REPORT.md                    ← analysis report
```

### SQLite Schema

```
kodex.db (version 0.5.0)

nodes                        ← code entities
  id, label, file_type, source_file, source_location, confidence,
  uuid, fingerprint, logical_key, body_hash, community

edges                        ← code relationships
  source, target, relation, confidence,
  source_file, source_location, weight

hyperedges                   ← multi-node groups
  id, label, nodes, confidence, source_file

knowledge                    ← AI-accumulated knowledge
  uuid, title, knowledge_type, description, confidence, observations,
  tags, scope, status, source, applies_when, evidence,
  supersedes, superseded_by, created_at, updated_at, last_validated_at

links                        ← knowledge ↔ node or knowledge ↔ knowledge
  knowledge_uuid, node_uuid, relation, target_type, confidence,
  created_at, linked_body_hash, linked_logical_key, reason, source

review_queue                 ← auto-triage queue
  knowledge_uuid, reason, created_at, priority, completed
```

Single-file SQLite database. Powered by [rusqlite](https://crates.io/crates/rusqlite) (bundled, no system dependency).

### Process Model

```
kodex actor (single daemon, auto-managed)
  ├─ owns kodex.db exclusively
  ├─ handles concurrent sessions via thread-per-client
  ├─ auto-started by first kodex serve
  └─ auto-exits after 5 min idle

kodex serve (per Claude session, MCP stdio proxy)
  ├─ Claude ←stdin/stdout→ serve ←socket→ actor
  └─ exits when Claude session ends (stdin EOF)
```

```
Claude A → kodex serve → ┐
Claude B → kodex serve → ├─ kodex.sock → kodex actor → kodex.db
Claude C → kodex serve → ┘
```

### Data Flow

```
kodex run ./my-project
  ├─ detect → extract (tree-sitter) → hierarchy → cluster → analyze
  ├─ hierarchy: project → crate/package → module → file → function
  ├─ merge into ~/.kodex/kodex.db (preserves other projects)
  ├─ assign stable UUIDs via fingerprint matching
  ├─ ingest git commits + README → auto-learn knowledge
  ├─ register in ~/.kodex/registry.json
  └─ generate graph.html + GRAPH_REPORT.md

kodex serve (MCP — 35+ tools)
  ├─ learn → knowledge with UUID, chain of thought
  ├─ recall_for_task → 10-signal scoring + graph reasoning + diversity
  ├─ recall_for_diff → git diff → affected knowledge ranking
  ├─ get_task_context → briefing with recommendations + warnings + conflicts
  ├─ knowledge_graph → BFS multi-hop + confidence propagation
  ├─ review queue → stale/conflict/duplicate auto-triage
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

kodex.db auto-migrates when opened by a newer version. Schema version stored in `meta` table.

Old databases just work. No manual steps.

## Commands

| Command | Description |
|---------|-------------|
| `kodex run <path>` | Analyze + merge into global db |
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
| `kodex ingest <path>` | Ingest git commits + README as knowledge |
| `kodex benchmark` | Token reduction ratio |
| `kodex watch <path>` | Auto-rebuild on changes (foreground watcher) |
| `kodex auto-update` | Re-extract + ingest if cwd is registered (used by git hooks) |
| `kodex hook [--global] {install,uninstall,status}` | Manage git hooks (global = `core.hooksPath`) |

## AI Knowledge System

### How It Works

```
Session 1 (project-a):
  Claude → learn("Repository Pattern", ...) → kodex.db (knowledge_uuid=K-1)

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

This is a one-time setup that does three things:

**1. MCP server** registered in `~/.claude.json`:
```json
{ "mcpServers": { "kodex": { "type": "stdio", "command": "kodex", "args": ["serve"] } } }
```

**2. Claude Code hooks** in `~/.claude/settings.json` (memory sync + session-start context):
```json
{ "hooks": { "PostToolUse": [{ "matcher": "Write", "hooks": [{ "type": "command",
  "command": "if echo \"$TOOL_INPUT\" | grep -q '.claude/memory'; then kodex import 2>/dev/null; fi" }] }] } }
```
Every time Claude saves a memory file, kodex imports it.

**3. Global git hook** in `~/.kodex/git-hooks/` via `git config --global core.hooksPath`. On every commit, the hook runs `kodex auto-update`, which:
- Re-extracts the graph + ingests new knowledge **if the cwd is a registered kodex project** (added via `kodex run <path>`).
- Silent no-op otherwise — unregistered repos are unaffected.

If `core.hooksPath` is already set to a non-kodex location (e.g. husky), kodex refuses to overwrite it and prints a per-project install path instead. Manage manually:

```bash
kodex hook --global status        # check installation
kodex hook --global uninstall     # remove + unset core.hooksPath
kodex hook install                # per-project install (no global config change)
```

Also: `kodex install cursor`, `kodex install vscode`, `kodex install codex`, `kodex install kiro` (these don't install the git hook — run `kodex hook --global install` separately if you want it).

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
| `learn` | Store/reinforce knowledge (returns UUID). `context_uuid` for chain of thought. |
| `recall` | Search by keyword/type |
| `recall_for_task` | 10-signal ranked retrieval + graph reasoning + diversity collapse |
| `recall_for_task_structured` | Same + full `ScoreBreakdown` per item |
| `recall_for_diff` | Git diff text → affected knowledge ranked with +20pt boost |
| `get_task_context` | Briefing with recommendations. `format=json` for `TaskContext`. `task_type` for action-oriented recs. |
| `knowledge_context` | Compact session summary: established + recent + type counts |
| `update_knowledge` | Update status/scope/applies_when/superseded_by |
| `validate_knowledge` | Mark valid, refresh link snapshots, log evidence |
| `mark_obsolete` | Mark obsolete with reason |
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
| `detect_stale` | Graduated staleness: deleted nodes, body drift, age. `detailed=true` for full report. |
| `find_duplicates` | Similar entries by title + description + type + tags |
| `merge_knowledge` | Absorb observations/tags/evidence/links/scope. Evidence + applies_when merge rules. |
| `detect_conflicts` | Contradictions, superseded-but-active, scope overlaps |
| `knowledge_health` | Status counts, orphans, duplicates, overdue, recency trends |
| `reason` | Graph reasoning: confidence propagation through supports/contradicts/supersedes |

### Review queue
| Tool | Description |
|------|-------------|
| `refresh_review_queue` | Auto-enqueue stale + conflict + duplicate items |
| `get_review_queue` | Pending items sorted by priority |
| `complete_review` | Mark item as reviewed |

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
kodex import          # ~/.claude/**/memory/*.md → kodex.db
kodex export          # kodex.db → ~/.claude/memory/kodex_*.md
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

## Diff-Aware Recall

```
recall_for_diff(diff="<git diff output>", max_items=10)

→ {
    "analysis": {
      "hunks_count": 2,
      "changed_files": ["src/auth.py"],
      "changed_node_uuids": ["node-auth"],
      "affected_knowledge_uuids": ["k-jwt"]
    },
    "relevant_knowledge": [
      {"knowledge": {"title": "JWT Auth"}, "score": {"total": 95, "reasons": ["directly affected by diff"]}}
    ]
  }
```

- Parses unified diff → hunks → maps to node UUIDs via source_location
- Tracks both new-side (additions) and old-side (deletions/renames)
- Affected knowledge gets +20pt boost, re-ranked after scoring

## Action-Oriented Context

```
get_task_context(task_type="bugfix", format="json")
```

Produces task-specific recommendations:
| task_type | Recommendations |
|-----------|----------------|
| `coding` | conventions, architecture constraints |
| `bugfix` | bug_pattern warnings + "add test" recommendations |
| `refactor` | respect decisions + tech_debt opportunities |
| `review` | bug_pattern + coupling checks |

Categories: `rule`, `hazard`, `test`, `coupling`, `constraint`, `opportunity`, `conflict`

## Graph Reasoning

Confidence propagation through knowledge links:

```
reason(uuids=["k-jwt"], depth=3)

→ {
    "adjustments": {"k-session": -0.25},
    "paths": [{"from": "k-jwt", "to": "k-session", "relation": "contradicts", "effect": -0.25}]
  }
```

| Relation | Effect | Decay |
|----------|--------|-------|
| `supports` | +boost to target | 0.7x per hop |
| `contradicts` | -penalty to target | 0.7x per hop |
| `supersedes` | -penalty to superseded | 0.7x per hop |
| `depends_on` | penalty if dependency is weak | conditional |

Adjustments clamped to +-0.3. Applied as +-10pt in recall scoring.

### Retrieval Quality

`recall_for_task` applies:
1. **10-signal scoring**: node overlap (30), file mention (20), confidence (20), applies_when (15), scope (10), recency (10), keyword (10), type priority (5), observations (10), needs_review penalty (50%)
2. **Diversity collapse**: entries with >60% title overlap are deduplicated in top-N
3. **Score breakdown**: `recall_for_task_structured` returns `ScoreBreakdown` with reasons per item

`get_task_context(format="json", task_type="bugfix")` returns structured `TaskContext`:
```json
{
  "relevant": [{"knowledge": {...}, "score": {"total": 75, "reasons": ["linked to code in scope", "graph reasoning: +3.2"]}}],
  "warnings": [{"uuid": "...", "reason": "linked nodes may have changed"}],
  "conflicts": [{"uuid_a": "...", "uuid_b": "...", "description": "..."}],
  "recommendations": [{"action": "Add test for: Off-by-one", "category": "test", "priority": 8}]
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

## Hierarchy Nodes

`kodex run` generates project structure nodes automatically:

```
epics-rs → crates → epics-base-rs → record → mod.rs → clamp_position()
                  → motor-rs → motor_record.rs → process()
                  → ad-core-rs → ...
```

- Detects package boundaries via markers (Cargo.toml, package.json, pyproject.toml, go.mod, pom.xml, etc.)
- Creates `contains` edges: project → package → module → file → function
- `get_node("epics-base-rs")` works for crate-level navigation

Supported markers: Cargo.toml, pyproject.toml, setup.py, package.json, go.mod, pom.xml, build.gradle, Gemfile, composer.json, mix.exs, Package.swift, pubspec.yaml, .csproj, \_\_init\_\_.py

## Auto-Ingestion

`kodex run` automatically extracts knowledge from:
- **Git commits**: classified into bug_pattern, decision, architecture, lesson, convention, performance
- **README.md**: project overview as architecture knowledge
- **Claude agent**: auto-learns patterns/bugs/decisions during work (via CLAUDE.md directive)

```bash
kodex ingest <path> --max-commits 100    # manual ingestion
kodex run .                               # auto-ingests after merge (also registers project)
```

**Auto-update on commit.** `kodex install claude` installs a global git hook (`~/.kodex/git-hooks/` via `core.hooksPath`) that runs `kodex auto-update` on every commit. This re-extracts the graph and ingests the 5 most recent commits as knowledge — but only for repos registered via `kodex run`. Unregistered repos see a silent no-op, so it's safe to leave the global hook on.

```bash
# Per-project alternative if you don't want the global hook:
kodex hook install                        # writes to .git/hooks/post-commit + post-checkout
```

## Performance

| Operation | Optimization |
|-----------|-------------|
| `learn` / `forget` / `update` | Incremental save (knowledge only, no graph rebuild) |
| `learn` / `forget` / `update` | load_knowledge_only (skips nodes/edges) |
| Repeated operations | Path-keyed in-memory cache (write-through, max 2 entries / 64MB) |
| `query_knowledge` | Keyword index (title/tag/type → UUID reverse lookup) |
| `query_graph` | Fuzzy matching: exact → token → source_file → edit distance ≤ 2 |
| `recall_for_task` | Project affinity: +15pt boost for same-project knowledge |
| SQLite WAL mode | Fast concurrent reads |

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

SQLite via [rusqlite](https://crates.io/crates/rusqlite) always included (bundled).

## License

MIT
