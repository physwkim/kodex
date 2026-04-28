# Changelog

## v0.7.2 (2026-04-28)

`query_graph` outputs now expose enough metadata to chain into the next call without a follow-up `get_node`.

- Text mode: each NODE line gains `c=<community>` and `deg=<degree>` so you can pick a useful `community=N` filter or spot hubs immediately.
- New `format=json` returns a structured object: `{ nodes:[{id,label,source_file,source_location,community,degree,fan_in}], edges:[{source,target,relation,confidence}], stale? }`. Use when iterating programmatically (e.g. extracting all communities, feeding nodes to another tool) instead of parsing text.
- Mermaid mode unchanged.
- New `serve::subgraph_to_json` helper (re-exported).

## v0.7.1 (2026-04-28)

Composite priority for `compare_graphs` — replaces raw degree with a fan-in × public-boost score so the top of the gap list is "what's actually called by many places" instead of "what has many edges of any kind".

- New `compose_priority=true` flag on `compare_graphs`. When set, each gap's `priority_score = (fan_in + 1) × public_boost`. `fan_in` is incoming-edge count (callsite proxy); `public_boost = 2.0` when the source_file matches `public_pattern`, else `1.0`.
- New `KodexGraph::fan_in(id)` helper exposes incoming-edge count alongside total `degree`. Reusable for future scoring.
- Default off — existing callers sorting on degree keep working unchanged.
- 1 new test (`compose_priority_uses_fan_in_not_total_degree`).

## v0.7.0 (2026-04-28)

Four follow-ups from real-use feedback after v0.6.2 — closes most of the user-reported gaps in the parity workflow. Targets "discover" and "verify" stages, not just "compare".

### Staleness detection (real-use pain #4)

- `kodex run` now records the project's git HEAD SHA in the registry as `last_indexed_commit`.
- `compare_graphs`, `get_node`, and `query_graph` responses include a `stale: {indexed_commit, current_commit, hint}` field (or `[STALE: ...]` prefix on string returns) when the working tree has moved past the indexed snapshot. Stops the agent from trusting answers built on a stale graph.
- Pre-v0.7 registry entries continue to work — they just don't surface staleness until next `kodex run`.

### Co-change analysis (real-use pain #10)

- New `co_changes(file, commit_limit, top_n, min_weight)` MCP tool — scans the last N commits with `git log --name-only` and returns files that frequently co-change with the target, ranked by commit count and weight (=co_commits / target_commits). Reveals architectural seams that aren't visible in the static graph.
- Smoke test against the kodex repo itself: `Cargo.toml` co-changes with `CHANGELOG.md` (73%), `Cargo.lock` (54%), `src/storage.rs` (35%) — exactly the touch-set for a release.

### Rust trait-impl extraction (real-use pain #7)

- `impl Trait for Type { ... }` methods now attach to the `Type` node (previously orphaned at file scope because `impl_item` has no `name` field). All `impl` blocks for the same `Type` collide on a shared node id, so a single `get_node("LocalChannel", expand="contains")` enumerates the full API across direct + trait impls.
- New integration test (`test_extract_rust_trait_impl_attaches_methods_to_type`) on `tests/fixtures/sample.rs`.

### Semantic-token gap matching (real-use pain #8, lightweight v1)

- `compare_graphs` gains `semantic_threshold` (default 0 = off). When >0, each gap is scored against right-side labels by camelCase/snake_case token Jaccard; matches above the threshold are attached as `candidate_matches: [{label, source_file, jaccard}]`. Catches "this is implemented in right under a different name" cases (`tickSearch` → `process_search_request`).
- Smoke against pvxs↔pva-rs: gap `handle_CONNECTION_VALIDATION` now surfaces three candidates including `ConnectionValidationRequest` (jaccard 0.50) and `build_client_connection_validation` (0.40) — verifying which gaps are real vs. renamed without manual browse.
- True semantic embeddings (cross-language equivalence beyond shared tokens) deferred to a future release; this v1 catches the common cases without adding model dependencies.

### New helpers

- `analyze::co_change` module with `CoChangeQuery`, `CoChangeResult`.
- `analyze::compare::tokenize_label` — public for external token-Jaccard use.
- `registry::current_head_commit`, `registry::drift`, `registry::entry_for_dir` — used by the actor for staleness detection; reusable.

## v0.6.2 (2026-04-28)

Cuts FP verification time on parity workflows by inlining source context, and makes diff-aware retrieval one call instead of two. Driven by real-use feedback after v0.6.1.

- `compare_graphs` gains `with_signature=true` — inlines a few lines around each gap's source location (default 2 above + signature, configurable via `signature_lines_above`/`signature_lines_below`) so the caller can verify "is this gap real, or just renamed?" without re-grepping upstream sources. Bounded to top-K by `signature_max_top` (default 20) to keep the response size in check.
- `compare_graphs` gains `public_pattern` — path substring marking public/exported headers (e.g. `pvxs/src/pvxs/`). Gaps in matching files are promoted above all internals; `public_only=true` drops internals entirely; `internal_weight` (default 0) lets you keep internals at the bottom. On a real pvxs vs pva-rs run this collapsed top results from internal scheduler functions to the actual API surface (`sharedArray::reserve/resize/swap`, `client::info`, etc).
- New `source_lookup` module: registry-aware path resolution + line-bounded snippet reader. Used by `compare_graphs --with-signature`; available to other tools later.
- `recall_for_diff` gains `auto=true` (with optional `base_ref`, default `HEAD`) — actor runs `git diff` in the project working tree and feeds the result into recall. Eliminates the agent-side round-trip of pre-fetching diff output. Falls back to user-supplied `diff` if git fails. The response surfaces `diff_source` so the caller knows which path was taken.
- `query_graph` empty results now return a diagnostic string instead of `""` — names the failing stage (no fuzzy hit / BFS expanded 0 / hub-skipped / no seeds) so the agent can broaden the question, raise depth, or drop a filter without guessing.
- 5 new tests (public ranking, public_only, signature snippet I/O, line-number parsing, registry resolution).

## v0.6.1 (2026-04-28)

Workflow follow-ups based on real-use feedback from v0.6.0.

- `query_graph`: vague natural-language `question` paired with a precise `source_pattern` (e.g. `"how does monitor receive updates"` + `pvxs/src/clientmon`) used to return empty — fuzzy scoring found nothing in scope. Now falls back to seeding with the highest-degree filter-passing nodes, so the caller still gets an architectural overview.
- `get_node`: new `expand=<relation>` parameter (e.g. `expand="contains"`) enumerates the top candidate's outgoing neighbors via that relation, sorted by degree. Replaces grep + manual reading when listing a class's API surface. The candidate with the most matching outgoing edges is auto-selected (returned as `members_source`), so a file-level hub with 18 real methods beats an empty stub class on a tied fuzzy score.
- `get_node`: new `source_pattern` filter so cross-language overloaded names (`SharedPV` in pvxs vs pva-rs) can be disambiguated without iterating top-N.
- `compare_graphs` description rewritten to lead with the API-parity use case and recommend the `compare_graphs` → `get_node` → `query_graph` workflow.
- 2 new tests for `top_degree_in_filter` and `TraversalFilter::is_active`.

## v0.6.0 (2026-04-28)

MCP retrieval upgrade: filtering, set-difference, and nucleo-matcher fuzzy ranking.

The motivating use case is cross-codebase parity (e.g. "what's in pvxs that pva-rs is missing"). Previous workflow forced grep over upstream sources because `god_nodes` returned generic hubs (`ok()`, `len()`), `query_graph` BFS exploded through those same hubs, and there was no way to ask the graph for a set difference.

**New MCP tools**

- `compare_graphs` — set difference between two `source_file` patterns. Labels are normalized (camelCase ↔ snake_case ↔ scope qualifiers collapse to alphanumeric form) so naming-convention drift across languages doesn't generate false gaps. File-level / concept hubs are skipped by default; narrow further with `pattern` (label substring) or `min_degree`. Returns each gap's representative node (highest-degree occurrence) with degree/source.
- `list_communities` — enumerate Louvain communities with top high-degree symbols and dominant source files per cluster, so the new `community=N` filter on `query_graph` is actually usable.

**New filters on existing tools**

- `god_nodes`: `pattern`, `source_pattern`, `min_degree` — scope to a domain instead of always returning generic hubs.
- `query_graph`: `source_pattern`, `community`, `exclude_hubs` — `exclude_hubs=true` (or numeric threshold) stops BFS from expanding through high-degree nodes, eliminating the noise blast through `ok()`/`len()`.
- `get_node`: returns top-N scored candidates (default 3) with `score`/`degree`/`community`/`source_file` for disambiguation, plus `highlight` (e.g. `[close]()`) and `match_indices` so the agent can see *why* each candidate ranked.

**Fuzzy matcher upgrade (nucleo-matcher 0.3)**

- `score_nodes` rewritten on top of nucleo-matcher's fzf-style scoring. Camel/snake boundaries, path separators, and consecutive-match runs all earn bonuses. Path matching uses nucleo's `match_paths()` config.
- Composite score: `label×4 + source_file + logical_key`. The 4× label weight prevents a strong path hit from outvoting a perfect label match (without it, `closure()` in `yacc/closure.c` outranked the actual `close()`).
- Adds `nucleo-matcher` 0.3 dependency (one transitive: `unicode-segmentation`).

**Tests**

- 107 lib + 22 integration tests pass (12 new tests covering filters, set difference, community summaries, camel-boundary ranking, match-index helper).
- Removed the home-grown Levenshtein scorer.

## v0.4.3 (2026-04-28)

Cleanup of legacy HDF5 naming and new `kodex context` subcommand.

- Rename `h5_path`/`global_h5`/`resolve_h5`/`h5_in_dir`/`make_test_h5`/`load_hdf5`/`save_hdf5` to `db_path`/`global_db`/etc — completes the SQLite migration started in v0.4.0
- Update remaining `HDF5` doc comments and log strings to `SQLite`
- New `kodex context [--max-items N]` CLI subcommand that prints the knowledge_context summary to stdout — designed for SessionStart hooks
- Internal-only renames: no behavior change, all 96 tests pass

## v0.4.2 (2026-04-28)

Fix `recall` keyword search for multi-word queries.

- `query_knowledge`: multi-token queries now match if ANY token appears in title/description/tags (was: whole-query substring match — failed on phrases like `"pva wire format epics-pva-rs pvxs"`)
- `build_knowledge_index`: also indexes description tokens (was: title/tags/type only)
- Results ranked by token match count (title/tag weight 2, description weight 1)
- MCP tool descriptions clarify when to prefer `recall` (exact identifiers) vs `recall_for_task` (natural-language)
- Added `test_query_knowledge_multi_token` regression test

## v0.4.0 (2026-04-21)

**Breaking: storage backend changed from HDF5 to SQLite.**

Existing `~/.kodex/kodex.h5` files are not auto-migrated. Run `kodex run .` to rebuild.

- SQLite backend via rusqlite (bundled, no system dependency)
- WAL journal mode for concurrent read/write
- True incremental save: knowledge-only operations skip graph tables entirely
- Built-in SQL indexes on UUID, title, knowledge_uuid, node_uuid
- Tests 24x faster (5.05s → 0.16s)
- File size reduced: 3.3MB (HDF5+zstd) → 393KB (SQLite)
- File extension: `.db` (was `.h5`)
- Same public API — all 95 tests pass unchanged

## v0.3.1 (2026-04-21)

- `kodex install claude` now auto-adds kodex directives to `~/.claude/CLAUDE.md` (session start, auto-learn, query guidance)
- Duplicate-safe: skips if "kodex" already present in CLAUDE.md

## v0.3.0 (2026-04-21)

- rust-hdf5 0.2.7 from crates.io (no local path dependency)
- Cache memory limits: max 2 entries, max 64MB, evict on overflow

- **In-memory cache**: path-keyed cache avoids re-reading h5 on repeated operations. No test interference (each test uses unique temp path).
- **Incremental save**: save_knowledge_only uses open_rw + delete_group + recreate (knowledge/links only, nodes untouched)
- **load_knowledge_only**: 13 functions skip loading nodes/edges entirely
- Cache auto-updated on save, auto-merged on incremental save

## v0.2.9 (2026-04-21)

- **Incremental save**: learn/forget/update now use open_rw + delete_group + recreate for knowledge/links only. Nodes/edges untouched. No graph rebuild on knowledge operations.

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
