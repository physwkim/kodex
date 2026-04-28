#[cfg(feature = "embeddings")]
pub mod embed;
pub mod query;
pub mod run;
pub mod serve;
pub mod workspace;

use std::path::{Path, PathBuf};

/// Helper: load graph with error message.
pub fn load_graph(graph_path: &Path) -> Option<kodex::graph::KodexGraph> {
    match kodex::serve::load_graph_smart(graph_path) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("Failed to load graph: {e}");
            None
        }
    }
}

/// Helper: generate community labels from graph.
pub fn community_labels(
    graph: &kodex::graph::KodexGraph,
    communities: &std::collections::HashMap<usize, Vec<String>>,
) -> std::collections::HashMap<usize, String> {
    communities
        .iter()
        .map(|(&cid, nodes)| {
            let label = nodes
                .first()
                .and_then(|nid| graph.get_node(nid))
                .map(|n| n.label.clone())
                .unwrap_or_else(|| format!("Community {cid}"));
            (cid, label)
        })
        .collect()
}

pub fn path(source: &str, target: &str, graph_path: &Path) {
    let graph = match load_graph(graph_path) {
        Some(g) => g,
        None => return,
    };
    let src_nodes = kodex::serve::score_nodes(&graph, &[source.to_lowercase()]);
    let tgt_nodes = kodex::serve::score_nodes(&graph, &[target.to_lowercase()]);

    match (src_nodes.first(), tgt_nodes.first()) {
        (Some((_, src_id)), Some((_, tgt_id))) => {
            let src_idx = graph.node_index.get(src_id);
            let tgt_idx = graph.node_index.get(tgt_id);
            if let (Some(&si), Some(&ti)) = (src_idx, tgt_idx) {
                if let Some(result) =
                    petgraph::algo::astar(&graph.inner, si, |n| n == ti, |_| 1, |_| 0)
                {
                    println!("Path ({} hops):", result.0);
                    for idx in &result.1 {
                        let node = &graph.inner[*idx];
                        println!("  -> {} ({})", node.label, node.source_file);
                    }
                } else {
                    println!("No path found between '{source}' and '{target}'");
                }
            }
        }
        _ => println!("Could not find nodes matching '{source}' and/or '{target}'"),
    }
}

pub fn explain(node_label: &str, graph_path: &Path) {
    let graph = match load_graph(graph_path) {
        Some(g) => g,
        None => return,
    };
    let matches = kodex::serve::score_nodes(&graph, &[node_label.to_lowercase()]);
    if let Some((_, node_id)) = matches.first() {
        if let Some(node) = graph.get_node(node_id) {
            println!("Node: {}", node.label);
            println!("File: {}", node.source_file);
            println!("Type: {}", node.file_type);
            if let Some(loc) = &node.source_location {
                println!("Location: {loc}");
            }
            println!("Degree: {}", graph.degree(node_id));

            let neighbors = graph.neighbors(node_id);
            if !neighbors.is_empty() {
                println!("\nNeighbors ({}):", neighbors.len());
                for nid in &neighbors {
                    if let Some(n) = graph.get_node(nid) {
                        let edge_info = graph
                            .edges()
                            .find(|(s, t, _)| {
                                (*s == *node_id && *t == *nid) || (*t == *node_id && *s == *nid)
                            })
                            .map(|(_, _, e)| format!(" [{}] {}", e.confidence, e.relation))
                            .unwrap_or_default();
                        println!("  {} ({}){edge_info}", n.label, n.source_file);
                    }
                }
            }
        }
    } else {
        println!("No node found matching '{node_label}'");
    }
}

pub fn benchmark(graph_path: &Path) {
    let graph = match load_graph(graph_path) {
        Some(g) => g,
        None => return,
    };
    let result = kodex::benchmark::run_benchmark(&graph, None, None);
    kodex::benchmark::print_benchmark(&result);
}

pub fn update(path: &Path) {
    println!(
        "kodex update: re-extracting code files in {}",
        path.display()
    );

    let detection = kodex::detect::detect(path, false);
    let code_paths: Vec<PathBuf> = detection.files.code.iter().map(PathBuf::from).collect();

    if code_paths.is_empty() {
        println!("  no code files found");
        return;
    }

    #[cfg(feature = "extract")]
    {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let project_name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        println!("  extracting {} code files...", code_paths.len());
        let mut extraction = kodex::extract::extract(&code_paths, Some(path));
        // Tag with project name (same normalization as run.rs)
        let path_str = path.to_str().unwrap_or("");
        for node in &mut extraction.nodes {
            if !node.source_file.starts_with(project_name) {
                let relative = node
                    .source_file
                    .strip_prefix(path_str)
                    .unwrap_or(&node.source_file)
                    .trim_start_matches('/');
                node.source_file = format!("{project_name}/{relative}");
            }
        }
        for edge in &mut extraction.edges {
            if !edge.source_file.starts_with(project_name) {
                let relative = edge
                    .source_file
                    .strip_prefix(path_str)
                    .unwrap_or(&edge.source_file)
                    .trim_start_matches('/');
                edge.source_file = format!("{project_name}/{relative}");
            }
        }

        let db = kodex::registry::global_db();
        match kodex::storage::merge_project(&db, project_name, &extraction) {
            Ok(()) => println!(
                "  merged: {} nodes, {} edges",
                extraction.nodes.len(),
                extraction.edges.len()
            ),
            Err(e) => eprintln!("  merge error: {e}"),
        }
    }

    #[cfg(not(feature = "extract"))]
    println!("  extract feature not enabled");
}

pub fn cluster_only(_path: &Path) {
    let db = kodex::registry::global_db();
    let graph = match load_graph(&db) {
        Some(g) => g,
        None => return,
    };

    let communities = kodex::cluster::cluster(&graph);
    let cohesion = kodex::cluster::score_all(&graph, &communities);
    println!("Re-clustered: {} communities", communities.len());
    for (cid, nodes) in &communities {
        let coh = cohesion.get(cid).copied().unwrap_or(0.0);
        println!(
            "  Community {cid}: {} nodes (cohesion {coh:.2})",
            nodes.len()
        );
    }

    // Load full data to preserve knowledge + links, only re-cluster
    match kodex::storage::load(&db) {
        Ok(data) => {
            // Re-cluster only changes community assignments, not knowledge/links
            match kodex::storage::save(&db, &data) {
                Ok(()) => println!("  saved to {}", db.display()),
                Err(e) => eprintln!("  save error: {e}"),
            }
        }
        Err(e) => eprintln!("  load error: {e}"),
    }
}

#[allow(unused_variables)]
pub fn add(url: &str, author: Option<&str>, contributor: Option<&str>, dir: &Path) {
    let url_type = kodex::ingest::detect_url_type(url);
    println!("kodex add: fetching {url} (type: {url_type})");

    #[cfg(feature = "fetch")]
    {
        match kodex::ingest::ingest(url, dir, author, contributor) {
            Ok(path) => println!("  saved to {}", path.display()),
            Err(e) => eprintln!("  fetch failed: {e}"),
        }
    }

    #[cfg(not(feature = "fetch"))]
    {
        if let Err(e) = kodex::security::validate_url(url) {
            eprintln!("URL validation failed: {e}");
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let safe_name: String = url
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(50)
            .collect();
        let filename = format!("{url_type}_{safe_name}_{now}.md");
        let out_path = dir.join(&filename);
        let content = format!("---\nsource_url: \"{url}\"\ntype: {url_type}\ncaptured_at: {now}\n---\n\n# {url}\n\nFetch feature not enabled.\n");
        match std::fs::write(&out_path, &content) {
            Ok(()) => println!("  saved stub to {}", out_path.display()),
            Err(e) => eprintln!("  failed to save: {e}"),
        }
    }
}
