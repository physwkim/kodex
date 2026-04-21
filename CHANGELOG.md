# Changelog

## v0.2.8 (2026-04-21)

- Knowledge index: keyword → UUID reverse index for fast query_knowledge lookup
- Avoids full scan when query terms match indexed tokens

## v0.2.7 (2026-04-21)

- knowledge_context is now a compact summary (stats + high-confidence + recent + type counts), not a full dump
- High-confidence (>80%) items always shown regardless of age
- Use recall_for_task for detailed task-specific retrieval

## v0.2.6 (2026-04-21)

- Session continuity: knowledge_context shows recent (7 days) items first, then grouped by type
- Git hook: post-commit auto-runs `kodex ingest` for immediate knowledge capture
- Fuzzy query matching: score_nodes uses exact → token → edit distance (Levenshtein ≤ 2), also searches source_file and logical_key
- Project-scoped recall: knowledge tagged with current project gets +15pt boost, inferred from touched_files path

## v0.2.5 (2026-04-21)

- Auto-collect: CLAUDE.md instructs Claude to learn patterns/bugs/decisions automatically
- Git commit ingestion: classify commit messages → bug_pattern/decision/architecture/lesson/convention/performance
- README ingestion: extract project overview as architecture knowledge
- `kodex ingest <path>` CLI command for manual ingestion
- `kodex run` now auto-ingests git commits + README after merge
- Post-merge stale detection: marks affected knowledge as needs_review

## v0.2.4 (2026-04-21)

- Multi-language package detection: Cargo.toml, pyproject.toml, package.json, go.mod, pom.xml, build.gradle, Gemfile, composer.json, mix.exs, Package.swift, pubspec.yaml, .csproj, __init__.py
- Skip build output dirs: __pycache__, vendor, build, dist, bin, obj

## v0.2.3 (2026-04-21)

- Hierarchy nodes: project → crate → module → file → function structure
- Crate detection via Cargo.toml presence
- `get_node("epics-base-rs")` now works (crate-level navigation)
- Skill file: query guidance for code identifiers

## v0.2.2 (2026-04-21)

- Auto-link: new knowledge entries automatically linked to matching code nodes by keyword (title/description → node label matching, max 5 links, confidence 0.7, source="inferred")

## v0.2.1 (2026-04-21)

- **Critical fix**: save writes extraction data directly, no graph rebuild (was losing nodes/edges on every learn call)
- Default features now include all-languages (14 parsers)
- Actor: set accepted streams to blocking mode (was causing broken pipe)

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
