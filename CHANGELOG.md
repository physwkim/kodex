# Changelog

## v0.2.0 (2026-04-21)

- Zstd compression for HDF5 vlen strings (12MB → 3.3MB, 72% reduction)
- rust-hdf5 0.2.6 (O(n²) GCOL fix, slice bounds fix, open_rw corruption fix)
- MCP protocol: initialize handshake, tools/list, tools/call rewriting
- MCP registration in ~/.claude.json (Claude Code v2.x compatible)
- Hook format: matcher + hooks array
- Panic-safe actor: catch_unwind on process_request

## v0.1.0 (2026-04-20)

Initial release. Three generations of knowledge management in a single binary.

### Gen1: Knowledge Storage + Code Graph

- **Tree-sitter AST extraction** for 14 languages (Python, JS, TS, Go, Rust, Java, C, C++, Ruby, C#, Scala, PHP, Swift, Lua)
- **HDF5 storage** via rust-hdf5 (pure Rust, no C dependency)
- **Single global knowledge base** at `~/.kodex/kodex.h5` for all projects
- **Actor daemon** with Unix socket, thread-per-client, 5min idle timeout
- **MCP stdio proxy** for Claude Code / Cursor integration
- **Claude memory sync**: bidirectional import/export with `~/.claude/memory/`
- **Knowledge types**: architecture, pattern, decision, convention, coupling, domain, preference, bug_pattern, tech_debt, ops, performance, lesson + custom
- **Confidence accumulation**: 0.60 → 0.68 → 0.74 → 0.83 → 0.93 over observations
- **Multi-project**: merge into single h5, preserve per-project nodes
- **Export**: JSON, HTML (vis.js), GraphML, Cypher, Obsidian vault

### Gen2: Identity + Retrieval + Lifecycle

- **Body-aware fingerprint**: SHA256 of normalized body (comments/whitespace stripped) for functions AND classes
- **Stable UUID matching**: rename/move/reformat → UUID preserved. Split/delete+create → new UUID. Body mismatch penalty (-15pt)
- **10-signal relevance scoring**: node overlap (30), file mention (20), confidence (20), applies_when (15), scope (10), recency (10), keyword (10), type priority (5), observations (10), needs_review penalty (50%)
- **Diversity collapse**: >60% title overlap deduplicated in top-N
- **ScoreBreakdown**: per-signal scores + reason codes for debugging
- **Graduated staleness**: all nodes deleted (1.0), >50% deleted (0.3-0.7), body_hash drift (0.2-0.5), age >90d (0.3)
- **Link snapshots**: linked_body_hash + linked_logical_key captured at creation for drift detection
- **Knowledge schema**: scope, status, source, applies_when, evidence, created_at, updated_at, last_validated_at, author, trigger
- **Link metadata**: confidence, created_at, linked_body_hash, linked_logical_key, reason, source
- **Knowledge graph**: knowledge-to-knowledge links (supports, contradicts, depends_on, supersedes, leads_to)
- **Chain of thought**: auto-chain via context_uuid, backward root-finding, forward traversal
- **Conflict detection**: superseded-but-active, active contradictions, scope overlaps
- **Duplicate detection + merge**: title/desc/type/tag similarity, merge with evidence/applies_when/scope rules
- **Observability**: knowledge_health with 18 metrics including validation_overdue, recently_changed_7d/30d
- **Version migration**: v0.1→v0.5 auto-migration with semver-safe comparison

### Gen3: Decision System + Diff-Aware + Reasoning

- **Diff-aware recall**: parse unified diff → map hunks to node UUIDs (old+new side) → boost affected knowledge (+20pt)
- **Action-oriented context**: task_type (coding/bugfix/refactor/review) → type-specific recommendations (rule, hazard, test, coupling, constraint, opportunity, conflict)
- **Graph reasoning**: BFS confidence propagation through supports (+boost), contradicts (-penalty), supersedes (-penalty), depends_on (conditional). 0.7x decay per hop, clamped +-0.3, applied as +-10pt in recall.
- **Review queue**: persistent HDF5-backed queue, auto-enqueue stale/conflict/duplicate items, priority-ranked, complete/dismiss workflow
- **Provenance**: author + trigger fields on knowledge entries
- **Structured API**: TaskContext JSON with relevant (+ ScoreBreakdown), warnings, conflicts, recommendations
- **Unified context**: markdown and JSON paths both use task_type + recommendations

### Stats

- 91 tests (73 unit + 18 integration)
- `cargo clippy --all-targets --all-features -- -D warnings` clean
- 35+ MCP tools
- 14 language parsers
- HDF5 v0.5.0 schema with auto-migration
