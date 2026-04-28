use std::path::Path;
#[cfg(feature = "extract")]
use std::path::PathBuf;

pub fn run_pipeline(path: &Path) {
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
