# Changelog

## Unreleased

### Receiver-aware call disambiguation

`calls` resolution now uses the call site's receiver expression to pick the right target when multiple nodes share a method name (e.g. `Database.query` and `HttpClient.query`). Both the same-file (`walk_calls`) and cross-file (`mod.rs`) paths apply the same disambiguation logic.

#### Method node IDs are now class-scoped

Methods defined inside a class now get IDs of the form `make_id([stem, class_label, name])` instead of `make_id([stem, name])`. Top-level functions are unchanged. This fixes the pre-existing collision where two classes in the same file with same-name methods collapsed into one node:

```
Before: Database.query and HttpClient.query in mod.rs → both "mod_query"
After:  → "mod_database_query" and "mod_httpclient_query"
```

- `RawCall` carries `receiver: Option<String>` and `receiver_is_self: bool` extracted from the AST: accessor child (Rust `field_expression.value`, Go `selector_expression.operand`, JS `member_expression.object`, …), the call node's own object field for languages where call IS accessor (Java `method_invocation.object`, Ruby `call.receiver`, PHP `member_call_expression.object`), and `Type::method` text-split for Rust/Ruby path-style calls.
- Resolution: `label → Vec<nid>` multi-map + `method → containing class label` (built from `method` edges) + `class → Set<base_class>` (built from `extends` edges, with cross-file fallback via `original_tgt`).
  - `self.method()` / `this.method()` → method whose containing class matches the caller's class. Falls back to walking the inheritance chain (BFS over `extends` edges) when no direct match.
  - `super.method()` → walks the inheritance chain from the caller's class, **skipping** the caller's own class. Routed to the cross-file resolver from `walk_calls` so a same-name override on the caller's own class can't incorrectly win.
  - `Type::method()` / `Type.method()` → method whose containing class matches the receiver text; also walks `Type`'s inheritance chain.
  - Variable-receiver bare calls with multiple candidates are **dropped** rather than mis-routed (a wrong `calls` edge silently misleads navigation; missing one is honest).
- **Cache schema bump** (`CACHE_SCHEMA_VERSION = 3`): old `kodex-out/cache/` entries become unreachable so `kodex update` / `kodex run` re-extracts cleanly with the new IDs and resolver.
- Languages: `call_object_field` added to all 14 language configs.

#### Behavior changes on upgrade

1. **Method node IDs change** for any method defined inside a class (top-level functions unchanged). Existing UUIDs / fingerprints survive, so attached knowledge stays linked. External tools that hardcoded the old ID format need to re-extract.
2. **Variable-receiver ambiguous calls now drop** instead of last-write-wins routing. Switch to `Type::method()` form or rely on local self-references for reliable resolution.
3. **`super.method()` now resolves** to the parent class's method via inheritance traversal (was previously dropped or routed wrong).
4. **Same-file same-name methods now produce two nodes** instead of one (Database.query and HttpClient.query in the same file become distinct).

### Auto-update on commit (registry-gated global git hook)

`kodex install claude` now installs a global git hook (`~/.kodex/git-hooks/`) via `git config --global core.hooksPath`. The hook checks the kodex registry on each commit:
- **Registered project** (`kodex run <path>` was called) → re-extract graph + ingest recent commits
- **Unregistered repo** → silent no-op

This solves stale graphs without requiring a per-project `kodex hook install`.

- New `kodex auto-update` subcommand: registry-gated update + ingest (used by hooks).
- New `kodex hook --global {install,uninstall,status}`: manage global hooks. Refuses to overwrite a non-kodex `core.hooksPath` (e.g. husky); points to per-project install in that case.
- `HOOK_SCRIPT` simplified to `kodex auto-update &` — same script works per-project or global.

## v0.11.0 (2026-05-01)

Chunk-level semantic retrieval — natural-language → top-K matching code chunks via cosine over BGE-small embeddings, with `attached_knowledge` joined in from the kodex knowledge graph. The kodex differentiator over plain vector retrieval is that each hit can carry the bug_patterns / decisions / conventions you've already accumulated for that node.

### `semantic_search` MCP tool

```jsonc
{
  "query": "where do we throttle outgoing requests",
  "top_k": 10,                    // clamped to [1, 500]
  "path_substring": "src/net/",   // plain substring, NOT a glob
  "language": "rust",
  "link_knowledge": true          // default; false skips the knowledge join
}
```

Returns:

```jsonc
{
  "query": "...",
  "count": N,
  "hits": [
    {
      "score": 0.842,
      "file_path": "myproj/src/net/throttle.rs",
      "start_line": 46,
      "end_line": 95,
      "content": "...50-line window of source...",
      "language": "rust",
      "node_id": "...",                           // when chunk maps to a graph node
      "attached_knowledge": [                     // active links only; obsolete filtered
        {"uuid": "...", "title": "...", "type": "bug_pattern", "confidence": 0.7, "relation": "warns_about"}
      ]
    }
  ]
}
```

Requires kodex built with `--features embeddings` AND `kodex embed` after `kodex run`. When `top_k` is requested above 500, the response includes `"truncated": true` and `"requested_top_k": N`.

### Schema v2: `chunks` + `chunk_embeddings`

Two new tables; new tables only, no ALTER. `CREATE TABLE IF NOT EXISTS` in `create_tables()` covers fresh DBs and v1 upgrades alike, so the v2 migration arm is a no-op user_version bump.

```sql
chunks (id PK, node_id NULL, file_path, start_line, end_line, language, content, content_hash, updated_at)
chunk_embeddings (chunk_id PK, model, dim, vec BLOB, updated_at)
```

`id` is `sha256(file_path | start-end | content_hash)` — stable so unchanged content reuses embeddings across re-ingests. `node_id` is best-effort: pick the first node whose `source_location` start line falls inside the chunk's line range. Deterministic per-run.

### Chunker (`src/extract/chunker.rs`)

50-line / 5-line-overlap windows over both code and prose (`.md`/`.txt`/`.rst`/`.html`). Pure line-based — no second tree-sitter pass. Windows shorter than 32 non-whitespace bytes are dropped (note: this can leave the very tail of a long file unsearchable when its last 5–6 lines are short, e.g. trailing close-braces only).

### `kodex run` + `kodex embed` integration

- `kodex run` now chunks code + document files post-extraction and persists them, with per-project GC. Reports `chunked N segments (M pruned)` alongside the existing graph stats.
- `kodex embed` now also embeds chunks under model id `BGE-small-en-v1.5/chunk-v1` (versioned independently from the node-embedding `/v2` because chunks embed raw 50-line content while nodes embed `label + signature + 2 lines above`).

### Performance & correctness shape

- **Two-stage load** in the search handler: `load_chunk_metadata` (no content) for the cosine pass, then `load_chunks_by_ids` for the top-K survivors only. Avoids materializing hundreds of MB of content per query on large repos.
- **Narrow id↔uuid bridge** via `storage::load_node_uuids_for_ids` — `attach_knowledge_for_nodes` no longer loads the full graph just to translate `node.id` to `node.uuid` for the link join.
- **Per-project GC scoped in Rust**: `prune_chunks_for_project` filters with `file_path.starts_with("{name}/")` instead of SQL `LIKE`, so project names containing `_` or `%` (single-char and any-char wildcards in `LIKE`) don't accidentally cross-delete other projects' chunks. Wrapped in a single transaction.
- **`top_k` cap raised** 100 → 500 with a `truncated` flag when the requested value exceeds it.

### Tests

- `cargo test`: 136 unit + 23 integration pass.
- `cargo test --features embeddings`: 147 unit + 23 integration pass.
- New: chunker (5), storage (chunks roundtrip, v1→v2 migration, project-name LIKE-collision regression, node-uuid lookup, metadata-strips-content, knowledge join with obsolete filter), actor (rank_chunks: order/truncate/path filter/language filter/missing-metadata race/dim-mismatch).
- The handler glue itself (Embedder construction + JSON shaping) is not covered by an end-to-end test because that requires a real BGE-small download. The pure cores (`rank_chunks`, `knowledge_for_node_ids`) are tested directly with synthetic vectors.

## v0.10.0 (2026-04-28)

Three correctness fixes from a code review — Obsidian/CLI contract drift, ad-hoc schema migration, and a shell-injection surface in the plugin.

### `graph.json` contract restored

- `kodex run` now emits `kodex-out/graph.json` alongside `graph.html` and `GRAPH_REPORT.md`. The Obsidian plugin and external visualizers expect this file (`graphJsonPath` defaults to `graph.json`); previously it was missing despite being the documented contract.
- `serve::load_graph_smart` now recognizes `.json` (networkx node-link format) and falls back to `<dir>/graph.json` when scanning a directory. Plugins pointing at a `kodex-out/` directory or a JSON file directly now resolve correctly without forcing SQLite.

### Versioned SQLite migrations

- Replaced the inline `migrate_columns` ad-hoc probe with a real migration framework: `PRAGMA user_version` tracks schema state, and a numbered match arm registers each migration step. `SCHEMA_VERSION` is the current head.
- v0→v1 (knowledge.fetch_count, knowledge.last_fetched — was the only existing migration) is now an explicit numbered step and runs idempotently. Future schema bumps just add a `migrate_vN_to_vM` arm.
- New `test_migration_v0_to_v1_idempotent` test synthesizes a legacy DB by stripping the columns + resetting `user_version`, then verifies a re-open re-applies the migration cleanly.

### Plugin shell-injection surface closed

- `obsidian-plugin/main.ts`: replaced `exec(string)` with `execFile(binary, args[])`. Node labels, file paths, and queries now pass to kodex verbatim — no shell metacharacter parsing, no injection. Affects `runKodex` and all 5 call sites (query, path, explain, god-nodes, rebuild).

### Tier 2 (separate work)

The same review flagged four larger structural opportunities — MCP tool registry unification, recall quality eval harness, actor graph cache, and extraction accuracy (cross-file call resolution). Each is its own milestone and is **not** in this release; they need design choices that warrant a separate cycle each.

## v0.9.1 (2026-04-28)

Documentation-only repositioning of the `embeddings` feature based on real-use evaluation: a 384-dim BGE-small cosine score is a worse equivalence judge than the LLM caller reading the inlined signature itself. The right framing is **candidate pre-filter at scale**, not "semantic match".

- `compare_graphs` description now states explicitly that `semantic_embedding=true` returns candidates for the LLM to judge — it doesn't decide equivalence. For small gap counts (tens), the lexical Jaccard pass + LLM reasoning over `with_signature` output already outperforms cosine, and the embedding pipeline (~33MB model, ORT native dep, ~5min `kodex embed`) is overhead. Reach for it only at scale (hundreds of gaps), in batch automation without an LLM in the loop, or when token-budget for direct LLM judgment is expensive.
- `kodex embed` CLI help and module docs updated with the same framing.
- `embedding` module top-level doc rewritten to lead with positioning before mechanics.
- No code changes — feature flag, defaults, and behavior are all unchanged.

## v0.9.0 (2026-04-28)

New `detect_renames` MCP tool — keeps knowledge memories from drifting silently when code is refactored. The classic failure mode: you `learn()` something pointing at `Server::handle_search`, the function is renamed to `handle_search_request`, the link's `node_uuid` no longer resolves, and the memory becomes invisible to future recalls. `detect_renames` catches these orphans and proposes replacements you can act on.

### Signals (combined)

- **`same_source_file`** — the orphan's `linked_logical_key` recorded the file path; current nodes in that file are the first-pass pool. Adds 0.4 to confidence on its own.
- **`token_jaccard_<x>`** — camelCase / snake_case identifier-token overlap between the lost label and each candidate. Adds up to 0.4 weighted by the score.
- **`body_hash_match`** / **`body_hash_match_cross_file`** — same body content under a different uuid is a near-certain rename. Floors confidence at 0.9-0.95, and is the only signal that crosses file boundaries (catches "moved to a new home" refactors).

### MCP shape

```jsonc
{
  "count": N,
  "orphans": [
    {
      "knowledge_uuid": "...",
      "knowledge_title": "...",            // when available
      "lost_node_uuid": "...",
      "lost_logical_key": "src/old.rs::doSomething",
      "candidates": [
        {
          "node_uuid": "...",
          "label": "do_something",
          "logical_key": "src/new.rs::do_something",
          "source_file": "src/new.rs",
          "confidence": 0.85,
          "signals": ["same_source_file", "token_jaccard_0.50"]
        }
      ]
    }
  ]
}
```

Orphans are sorted by their best candidate's confidence so the cleanest renames surface first. `apply_rename` (actually rewriting the link) is intentionally separate from this tool — `detect_renames` is read-only so an agent can review suggestions before mutation.

### Internals

- New `analyze::rename_detect` module with `detect_renames`, `DetectQuery`, `OrphanedLink`, `RenameCandidate`.
- 4 new tests (logical_key parsing, same-file token-Jaccard, cross-file body-hash, no-orphan health check). Lib total: 123.

## v0.8.2 (2026-04-28)

New `analyze_change` MCP tool — diff-aware change-impact briefing in a single call. Wraps `recall_for_diff` + per-file `co_changes` + the diff summary so the agent doesn't need N+1 round-trips when verifying a change or reviewing a PR.

- `auto=true` runs `git diff <base_ref>` (default HEAD) in the project working tree; otherwise pass `diff` directly.
- For each touched file (capped by `co_change_max_files`, default 5), the response includes its top architectural co-changers from git history.
- Combined response shape: `{ diff_summary: {changed_files, changed_node_uuids}, knowledge: [...], co_changes: [{file, target_commits, co_changes: [...]}], stale? }`.
- Smoke against the kodex repo: a 2-file edit returned 3 relevant knowledge entries, plus the cross-file co-change pattern (CHANGELOG/Cargo.lock at 50-72% with `src/commands/serve.rs`) — matches the "version-bump release" workflow exactly.

## v0.8.1 (2026-04-28)

Embedding quality boost: `kodex embed` now feeds the function signature + preceding doc comment to the embedding model instead of just the label. The same v0.6.2 `source_lookup` infrastructure used by `compare_graphs --with-signature` is reused, so no new disk-read code paths.

- The embedded text becomes `<label> (<basename>)\n<2 lines above source_location>\n<line at source_location>`. Param names, types, and preceding `///` / `/* */` doc comments all contribute to the vector.
- `MODEL_ID` bumped to `BGE-small-en-v1.5/v2` (suffix encodes the embedded-text schema). On the next `kodex embed` run, rows tagged with the older `BGE-small-en-v1.5` (v1, label-only) are auto-detected and re-embedded — no manual migration needed.
- Falls back to label-only when the source file isn't on disk (e.g. the project root has moved since ingestion).

## v0.8.0 (2026-04-28)

Embedding-based semantic similarity for cross-language API parity. Catches the cases the v0.7.0 token-Jaccard pass misses (e.g. C++ `Value::copyIn(const void*, StoreType)` ↔ Rust `Value::set<T>(...)` — different identifiers, same purpose).

### `embeddings` Cargo feature (default off)

- Optional dependency on `fastembed` 4.x (bundles ONNX runtime via `ort`). The default build stays unchanged in size and dependencies; users opt in with `cargo install kodex --features embeddings`.
- Default model: BAAI/BGE-small-en-v1.5 (384-dim, ~33MB on first-use download, cached under `~/.cache/fastembed/`).

### New CLI: `kodex embed`

- Walks the global graph, embeds every function/class/method node's label (augmented with the source file basename for tiny disambiguation context), and stores the resulting f32 vectors in a new `node_embeddings` SQLite table.
- Skips file-level / concept hubs (no semantic content) and existing rows by default. Narrow with `--source-pattern PATTERN`.
- Without `--features embeddings`: prints a clear "rebuild with --features embeddings" message instead of a confusing error.

### `compare_graphs --semantic-embedding`

- New `semantic_embedding=true` flag. When set, after the lexical Jaccard pass, each gap's label is embedded and cosine-compared against the precomputed embeddings of right-side labels (filtered by `right_pattern`). Top matches above `embedding_threshold` (default 0.65) are merged into `candidate_matches` with both `cosine` and `jaccard` populated, sorted by cosine.
- De-duplicates by label across the two passes, so a candidate that lights up on both scores well on both axes.
- Returns a clear error if `kodex embed` hasn't been run, or if the binary lacks the `embeddings` feature.

### Internals

- New `embedding` module: `Embedder` (model wrapper), `cosine`, `vec_to_bytes`/`bytes_to_vec` (BLOB codec).
- New `storage::store_embedding`/`store_embeddings_bulk`/`load_all_embeddings`/`count_embeddings` (always available — the BLOB is opaque without the feature).
- New `node_embeddings` SQLite table, transparently created by the existing `create_tables` migration.
- 5 new tests for embedding round-trip and cosine math (lib total: 124).

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
