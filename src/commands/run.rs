use std::path::Path;
use std::path::PathBuf;

pub fn run_pipeline(path: &Path, no_embed: bool) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let project_name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    println!("kodex: analyzing {} ({project_name})", path.display());

    let detection = kodex::detect::detect(path, false);
    println!(
        "  detected {} files ({} code, {} doc, {} paper, {} image, {} video)",
        detection.total_files,
        detection.files.code.len(),
        detection.files.document.len(),
        detection.files.paper.len(),
        detection.files.image.len(),
        detection.files.video.len(),
    );
    if !detection.skipped_sensitive.is_empty() {
        println!(
            "  skipped {} sensitive file(s)",
            detection.skipped_sensitive.len()
        );
    }

    #[cfg(feature = "extract")]
    let extraction = {
        let code_paths: Vec<PathBuf> = detection.files.code.iter().map(PathBuf::from).collect();
        if code_paths.is_empty() {
            println!("  no code files to extract");
            kodex::types::ExtractionResult::default()
        } else {
            println!("  extracting {} code files...", code_paths.len());
            let mut result = kodex::extract::extract(&code_paths, Some(path));
            // Tag nodes with project name for multi-project support
            for node in &mut result.nodes {
                if !node.source_file.starts_with(project_name) {
                    node.source_file = format!(
                        "{project_name}/{}",
                        node.source_file
                            .strip_prefix(path.to_str().unwrap_or(""))
                            .unwrap_or(&node.source_file)
                            .trim_start_matches('/')
                    );
                }
            }
            for edge in &mut result.edges {
                if !edge.source_file.starts_with(project_name) {
                    edge.source_file = format!(
                        "{project_name}/{}",
                        edge.source_file
                            .strip_prefix(path.to_str().unwrap_or(""))
                            .unwrap_or(&edge.source_file)
                            .trim_start_matches('/')
                    );
                }
            }
            // Add hierarchy nodes (project → crate → module → file)
            kodex::hierarchy::add_hierarchy(&mut result, path);
            println!(
                "  extracted {} nodes, {} edges",
                result.nodes.len(),
                result.edges.len()
            );
            result
        }
    };

    #[cfg(not(feature = "extract"))]
    let extraction = {
        println!("  extract feature not enabled");
        kodex::types::ExtractionResult::default()
    };

    // Build graph
    let graph = kodex::graph::build_from_extraction(&extraction);
    println!(
        "  built graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    let communities = kodex::cluster::cluster(&graph);
    println!("  detected {} communities", communities.len());

    // Merge into global db
    let db_path = kodex::registry::global_db();
    let _ = std::fs::create_dir_all(kodex::registry::kodex_home());

    match kodex::storage::merge_project(&db_path, project_name, &extraction) {
        Ok(()) => {
            println!("  merged into {}", db_path.display());
            // Post-merge: detect stale knowledge + refresh review queue
            let stale = kodex::learn::detect_stale_knowledge(&db_path).unwrap_or(0);
            if stale > 0 {
                println!("  {stale} knowledge entries marked for review (stale)");
            }
        }
        Err(e) => eprintln!("  SQLite error: {e}"),
    }

    // Chunk files for chunk-level semantic retrieval. Both code and prose
    // (markdown / txt / rst / html) are chunked — code chunks gain a
    // best-effort `node_id` when an extracted node starts inside the chunk's
    // line range; prose chunks always carry `node_id=NULL`.
    {
        let chunk_targets: Vec<PathBuf> = detection
            .files
            .code
            .iter()
            .chain(detection.files.document.iter())
            .map(PathBuf::from)
            .collect();
        if !chunk_targets.is_empty() {
            match build_and_persist_chunks(
                &db_path,
                path,
                project_name,
                &chunk_targets,
                &extraction,
            ) {
                Ok((written, pruned)) => {
                    println!("  chunked {written} segments ({pruned} pruned)");
                }
                Err(e) => eprintln!("  chunk: {e}"),
            }
        }
    }

    // Embed nodes + chunks so `semantic_search` works out of the box. Both
    // calls are delta-only (skip rows already encoded with the current
    // MODEL_ID), so re-runs are cheap. First invocation downloads BGE-small
    // (~30 MB cached under `~/.cache/`). User can opt out with `--no-embed`
    // for CI/automation flows that only need keyword/graph retrieval.
    #[cfg(feature = "embeddings")]
    if !no_embed {
        match super::embed::embed_nodes(&db_path, None, true) {
            Ok(0) => {}
            Ok(n) => println!("  embedded {n} nodes"),
            Err(e) => eprintln!("  embed (nodes): {e}"),
        }
        match super::embed::embed_chunks(&db_path, true) {
            Ok(0) => {}
            Ok(n) => println!("  embedded {n} chunks"),
            Err(e) => eprintln!("  embed (chunks): {e}"),
        }
    }
    #[cfg(not(feature = "embeddings"))]
    let _ = no_embed;

    // Ingest external knowledge (git commits, README)
    match kodex::ingest_knowledge::ingest_project(&db_path, path, 50) {
        Ok(0) => {}
        Ok(n) => println!("  ingested {n} knowledge entries from git/README"),
        Err(e) => eprintln!("  ingest: {e}"),
    }

    // Register project
    match kodex::registry::register(path) {
        Ok(key) => println!("  registered as '{key}'"),
        Err(e) => eprintln!("  registry: {e}"),
    }

    // Generate optional outputs in CWD
    let out_dir = path.join("kodex-out");
    let _ = std::fs::create_dir_all(&out_dir);

    // HTML visualization
    let labels = super::community_labels(&graph, &communities);
    match kodex::export::to_html(
        &graph,
        &communities,
        &out_dir.join("graph.html"),
        Some(&labels),
    ) {
        Ok(()) => println!("  exported graph.html"),
        Err(e) => eprintln!("  HTML: {e}"),
    }

    // JSON (networkx node-link) — consumed by the Obsidian plugin and any
    // external visualizer. Kept alongside graph.html so a single
    // `kodex run` covers both presentations of the graph.
    match kodex::export::to_json(&graph, &communities, &out_dir.join("graph.json")) {
        Ok(()) => println!("  exported graph.json"),
        Err(e) => eprintln!("  JSON: {e}"),
    }

    // Report
    let cohesion = kodex::cluster::score_all(&graph, &communities);
    let gods = kodex::analyze::god_nodes(&graph, 10);
    let surprises = kodex::analyze::surprising_connections(&graph, Some(&communities), 5);
    let questions = kodex::analyze::suggest_questions(&graph, Some(&communities), 7);
    let report = kodex::report::generate(
        &graph,
        &communities,
        &cohesion,
        &labels,
        &gods,
        &surprises,
        &detection,
        0,
        0,
        &path.display().to_string(),
        Some(&questions),
    );
    let _ = std::fs::write(out_dir.join("GRAPH_REPORT.md"), &report);
    println!("  exported GRAPH_REPORT.md");

    println!(
        "  done! Knowledge: {} | View: {}",
        db_path.display(),
        out_dir.display()
    );
}

/// Run the chunker over every code/document file and persist chunks to the
/// global db. Returns `(written, pruned)` — number of chunks upserted and
/// number of stale chunks GC'd. Paths are normalized to the same
/// `{project_name}/{relative}` convention used by `extract` so chunk
/// `node_id` mappings line up with the persisted nodes.
fn build_and_persist_chunks(
    db_path: &Path,
    project_root: &Path,
    project_name: &str,
    targets: &[PathBuf],
    extraction: &kodex::types::ExtractionResult,
) -> kodex::error::Result<(usize, usize)> {
    use std::collections::{HashMap, HashSet};

    // Group nodes by source_file once for O(1) per-file lookup.
    let mut nodes_by_file: HashMap<&str, Vec<&kodex::types::Node>> = HashMap::new();
    for n in &extraction.nodes {
        nodes_by_file
            .entry(n.source_file.as_str())
            .or_default()
            .push(n);
    }

    let mut all_chunks: Vec<kodex::storage::StoredChunk> = Vec::new();
    let mut keep_ids: HashSet<String> = HashSet::new();

    for disk_path in targets {
        // Build the registry-prefixed source_file the same way `run_pipeline`
        // does for extraction nodes: strip the project root, prepend the
        // project name. Falls back to the raw on-disk string if the strip
        // fails (matches how nodes whose path didn't match got left alone).
        let rel = disk_path
            .strip_prefix(project_root)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or_else(|| disk_path.to_str().unwrap_or(""));
        let source_file = if rel.is_empty() {
            continue;
        } else {
            format!("{project_name}/{}", rel.trim_start_matches('/'))
        };
        let language = kodex::extract::chunker::language_for_path(disk_path);
        let nodes_in_file: Vec<&kodex::types::Node> = nodes_by_file
            .get(source_file.as_str())
            .cloned()
            .unwrap_or_default();
        let chunks =
            kodex::extract::chunker::chunk_file(&source_file, disk_path, language, &nodes_in_file);
        for c in &chunks {
            keep_ids.insert(c.id.clone());
        }
        all_chunks.extend(chunks);
    }

    // Upsert in modest batches so a transaction-per-batch keeps memory low
    // on huge repos while still amortizing fsync cost.
    const BATCH: usize = 500;
    for batch in all_chunks.chunks(BATCH) {
        kodex::storage::store_chunks_bulk(db_path, batch)?;
    }
    // GC: only prune chunks belonging to *this project* — other projects'
    // chunks share the global db and must not be touched. Storage helper
    // does the project-prefix filtering in Rust (not SQL LIKE) to avoid
    // wildcard collisions on names containing `_` or `%`.
    let pruned = kodex::storage::prune_chunks_for_project(db_path, project_name, &keep_ids)?;
    Ok((all_chunks.len(), pruned))
}
