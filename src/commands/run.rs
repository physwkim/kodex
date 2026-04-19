use std::path::{Path, PathBuf};

pub fn run_pipeline(path: &Path) {
    println!("kodex: analyzing {}", path.display());

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
            let result = kodex::extract::extract(&code_paths, Some(path));
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
        println!("  extract feature not enabled, skipping AST extraction");
        kodex::types::ExtractionResult::default()
    };

    let graph = kodex::graph::build_from_extraction(&extraction);
    println!(
        "  built graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    let communities = kodex::cluster::cluster(&graph);
    println!("  detected {} communities", communities.len());

    let cohesion = kodex::cluster::score_all(&graph, &communities);
    let gods = kodex::analyze::god_nodes(&graph, 10);
    let surprises = kodex::analyze::surprising_connections(&graph, Some(&communities), 5);
    let questions = kodex::analyze::suggest_questions(&graph, Some(&communities), 7);
    let labels = super::community_labels(&graph, &communities);

    let out_dir = path.join("kodex-out");
    let vault_dir = out_dir.join("vault");
    let _ = std::fs::create_dir_all(&out_dir);
    let _ = std::fs::create_dir_all(&vault_dir);

    super::export_all(&graph, &communities, &labels, &out_dir);
    println!("  exported kodex.h5, graph.html");

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
    let _ = std::fs::write(vault_dir.join("GRAPH_REPORT.md"), &report);

    match kodex::export::to_obsidian(
        &graph,
        &communities,
        &vault_dir,
        Some(&labels),
        Some(&cohesion),
    ) {
        Ok(count) => println!("  vault: {count} notes in {}", vault_dir.display()),
        Err(e) => eprintln!("  vault error: {e}"),
    }

    println!(
        "  done! Data: {} | Vault: {}",
        out_dir.display(),
        vault_dir.display()
    );
}
