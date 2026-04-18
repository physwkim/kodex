use std::path::Path;
#[cfg(feature = "watch")]
use std::path::PathBuf;

/// Watch a directory for file changes and auto-rebuild the graph.
#[cfg(feature = "watch")]
pub fn watch(watch_path: &Path, debounce_secs: f64) -> crate::error::Result<()> {
    use notify::{Watcher, RecursiveMode, Config};
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    println!("Watching {} for changes (debounce: {debounce_secs}s)...", watch_path.display());
    println!("Press Ctrl+C to stop.\n");

    let (tx, rx) = channel();
    let mut watcher = notify::RecommendedWatcher::new(tx, Config::default())
        .map_err(|e| crate::error::GraphifyError::Other(format!("Watch error: {e}")))?;

    watcher
        .watch(watch_path, RecursiveMode::Recursive)
        .map_err(|e| crate::error::GraphifyError::Other(format!("Watch error: {e}")))?;

    let debounce = Duration::from_secs_f64(debounce_secs);
    let mut last_rebuild = Instant::now() - debounce; // allow immediate first rebuild
    let mut pending = false;

    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(event)) => {
                // Filter: only react to code file changes
                let dominated_by_code = event.paths.iter().any(|p| {
                    crate::detect::classify::classify_file(p)
                        .map(|c| c == crate::detect::classify::FileCategory::Code)
                        .unwrap_or(false)
                });
                if dominated_by_code {
                    pending = true;
                }
            }
            Ok(Err(_)) => {} // watch error, ignore
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if pending && last_rebuild.elapsed() >= debounce {
                    pending = false;
                    last_rebuild = Instant::now();
                    rebuild_code(watch_path);
                }
            }
            Err(_) => break,
        }
    }

    Ok(())
}

#[cfg(feature = "watch")]
fn rebuild_code(watch_path: &Path) {
    println!("[{:.1}s] Change detected, rebuilding...",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64() % 100000.0
    );

    let detection = crate::detect::detect(watch_path, false);
    let code_paths: Vec<PathBuf> = detection.files.code.iter().map(PathBuf::from).collect();

    if code_paths.is_empty() {
        println!("  no code files found");
        return;
    }

    #[cfg(feature = "extract")]
    {
        let extraction = crate::extract::extract(&code_paths, Some(watch_path));
        let graph = crate::graph::build_from_extraction(&extraction);
        let communities = crate::cluster::cluster(&graph);

        let community_labels: std::collections::HashMap<usize, String> = communities
            .iter()
            .map(|(&cid, nodes)| {
                let label = nodes.first()
                    .and_then(|nid| graph.get_node(nid))
                    .map(|n| n.label.clone())
                    .unwrap_or_else(|| format!("Community {cid}"));
                (cid, label)
            })
            .collect();

        let out_dir = watch_path.join("graphify-out");
        let _ = std::fs::create_dir_all(&out_dir);
        let _ = crate::export::to_json(&graph, &communities, &out_dir.join("graph.json"));
        let _ = crate::export::to_html(&graph, &communities, &out_dir.join("graph.html"), Some(&community_labels));

        println!("  rebuilt: {} nodes, {} edges, {} communities",
            graph.node_count(), graph.edge_count(), communities.len());
    }

    #[cfg(not(feature = "extract"))]
    println!("  extract feature not enabled, skipping rebuild");
}

#[cfg(not(feature = "watch"))]
pub fn watch(_watch_path: &Path, _debounce_secs: f64) -> crate::error::Result<()> {
    Err(crate::error::GraphifyError::Other(
        "Watch feature not enabled. Rebuild with --features watch".to_string(),
    ))
}
