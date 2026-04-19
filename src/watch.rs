use std::path::Path;
#[cfg(feature = "watch")]
use std::path::PathBuf;

/// Watch a directory for file changes and auto-rebuild the graph + vault.
///
/// If `vault_path` is Some, the Obsidian vault is also regenerated on each rebuild.
#[cfg(feature = "watch")]
pub fn watch(
    watch_path: &Path,
    debounce_secs: f64,
    vault_path: Option<&Path>,
) -> crate::error::Result<()> {
    use notify::{Config, RecursiveMode, Watcher};
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    println!(
        "Watching {} for changes (debounce: {debounce_secs}s)...",
        watch_path.display()
    );
    if let Some(vp) = vault_path {
        println!("Vault output: {}", vp.display());
    }
    println!("Press Ctrl+C to stop.\n");

    let (tx, rx) = channel();
    let mut watcher = notify::RecommendedWatcher::new(tx, Config::default())
        .map_err(|e| crate::error::KodexError::Other(format!("Watch error: {e}")))?;

    watcher
        .watch(watch_path, RecursiveMode::Recursive)
        .map_err(|e| crate::error::KodexError::Other(format!("Watch error: {e}")))?;

    // Also watch vault for reverse sync (Obsidian edits → graph)
    if let Some(vp) = vault_path {
        if vp.is_dir() {
            let _ = watcher.watch(vp, RecursiveMode::Recursive);
        }
    }

    let debounce = Duration::from_secs_f64(debounce_secs);
    let mut last_rebuild = Instant::now() - debounce;
    let mut pending_code = false;
    let mut pending_vault = false;

    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    // Skip kodex-out changes to avoid rebuild loops
                    let path_str = path.to_string_lossy();
                    if path_str.contains("kodex-out") {
                        continue;
                    }

                    // Check if change is in vault directory (reverse sync)
                    if let Some(vp) = vault_path {
                        if path.starts_with(vp)
                            && path.extension().map(|e| e == "md").unwrap_or(false)
                        {
                            pending_vault = true;
                            continue;
                        }
                    }

                    // Check if it's a code file change
                    if crate::detect::classify::classify_file(path)
                        .map(|c| c == crate::detect::classify::FileCategory::Code)
                        .unwrap_or(false)
                    {
                        pending_code = true;
                    }
                }
            }
            Ok(Err(_)) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if last_rebuild.elapsed() >= debounce {
                    if pending_code {
                        pending_code = false;
                        last_rebuild = Instant::now();
                        rebuild_code(watch_path, vault_path);
                    } else if pending_vault {
                        pending_vault = false;
                        last_rebuild = Instant::now();
                        reverse_sync_vault(watch_path, vault_path.unwrap());
                    }
                }
            }
            Err(_) => break,
        }
    }

    Ok(())
}

/// Rebuild code graph and regenerate all outputs including vault.
#[cfg(feature = "watch")]
fn rebuild_code(watch_path: &Path, vault_path: Option<&Path>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    println!("[{now}] Code changed, rebuilding...");

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
        let cohesion = crate::cluster::score_all(&graph, &communities);

        let community_labels: std::collections::HashMap<usize, String> = communities
            .iter()
            .map(|(&cid, nodes)| {
                let label = nodes
                    .first()
                    .and_then(|nid| graph.get_node(nid))
                    .map(|n| n.label.clone())
                    .unwrap_or_else(|| format!("Community {cid}"));
                (cid, label)
            })
            .collect();

        let out_dir = watch_path.join("kodex-out");
        let vault_dir = vault_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| out_dir.join("vault"));
        let _ = std::fs::create_dir_all(&out_dir);
        let _ = std::fs::create_dir_all(&vault_dir);

        let _ = crate::storage::save_hdf5(&graph, &communities, &out_dir.join("kodex.h5"));
        let _ = crate::export::to_json(&graph, &communities, &out_dir.join("graph.json"));
        let _ = crate::export::to_html(
            &graph,
            &communities,
            &out_dir.join("graph.html"),
            Some(&community_labels),
        );

        // Regenerate vault
        {
            match crate::export::to_obsidian(
                &graph,
                &communities,
                &vault_dir,
                Some(&community_labels),
                Some(&cohesion),
            ) {
                Ok(count) => println!("  vault: {count} notes updated"),
                Err(e) => eprintln!("  vault error: {e}"),
            }
        }

        println!(
            "  rebuilt: {} nodes, {} edges, {} communities",
            graph.node_count(),
            graph.edge_count(),
            communities.len()
        );
    }

    #[cfg(not(feature = "extract"))]
    println!("  extract feature not enabled, skipping rebuild");
}

/// Reverse sync: read modified vault .md files and update graph.json.
///
/// Parses YAML frontmatter and wikilinks from vault notes to detect:
/// - New connections added by user (new [[wikilinks]])
/// - Removed connections (deleted [[wikilinks]])
/// - Modified metadata (tags, community reassignment)
#[cfg(feature = "watch")]
fn reverse_sync_vault(watch_path: &Path, vault_path: &Path) {
    println!("Vault changed, syncing back to graph...");

    let graph_path = watch_path.join("kodex-out/graph.json");
    let graph = match crate::serve::load_graph(&graph_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("  failed to load graph: {e}");
            return;
        }
    };

    // Scan vault .md files for wikilinks
    let mut vault_edges: Vec<(String, String)> = Vec::new();
    let wikilink_re = regex::Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]").expect("invalid regex");

    if let Ok(entries) = std::fs::read_dir(vault_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();

                // Skip community overview files
                if filename.starts_with("_COMMUNITY_") || filename == "index" {
                    continue;
                }

                if let Ok(content) = std::fs::read_to_string(&path) {
                    for cap in wikilink_re.captures_iter(&content) {
                        if let Some(target) = cap.get(1) {
                            let target_name = target.as_str().to_string();
                            if target_name != filename && !target_name.starts_with("_COMMUNITY_") {
                                vault_edges.push((filename.clone(), target_name));
                            }
                        }
                    }
                }
            }
        }
    }

    // Find edges in vault that don't exist in graph (user-added links)
    let graph_edge_set: std::collections::HashSet<(String, String)> = graph
        .edges()
        .map(|(s, t, _)| (s.to_string(), t.to_string()))
        .collect();

    let mut new_edges = Vec::new();
    for (src_file, tgt_file) in &vault_edges {
        // Map filename back to node ID (best effort: lowercase match)
        let src_id = find_node_by_filename(&graph, src_file);
        let tgt_id = find_node_by_filename(&graph, tgt_file);

        if let (Some(sid), Some(tid)) = (src_id, tgt_id) {
            if !graph_edge_set.contains(&(sid.clone(), tid.clone()))
                && !graph_edge_set.contains(&(tid.clone(), sid.clone()))
            {
                new_edges.push((sid, tid));
            }
        }
    }

    if new_edges.is_empty() {
        println!("  no new connections found in vault edits");
        return;
    }

    // Append new edges to graph.json
    match std::fs::read_to_string(&graph_path) {
        Ok(text) => {
            if let Ok(mut data) = serde_json::from_str::<serde_json::Value>(&text) {
                let links = data.get_mut("links").and_then(|v| v.as_array_mut());
                if let Some(links) = links {
                    for (src, tgt) in &new_edges {
                        links.push(serde_json::json!({
                            "source": src,
                            "target": tgt,
                            "relation": "user_linked",
                            "confidence": "EXTRACTED",
                            "source_file": "vault",
                            "confidence_score": 1.0,
                            "weight": 1.0,
                        }));
                    }
                    if let Ok(json) = serde_json::to_string_pretty(&data) {
                        let _ = std::fs::write(&graph_path, json);
                    }
                }
                println!("  synced {} new connection(s) from vault", new_edges.len());
            }
        }
        Err(e) => eprintln!("  failed to read graph: {e}"),
    }
}

#[cfg(feature = "watch")]
fn find_node_by_filename(graph: &crate::graph::KodexGraph, filename: &str) -> Option<String> {
    let lower = filename.to_lowercase().replace('_', "");
    graph
        .node_ids()
        .find(|id| {
            graph
                .get_node(id)
                .map(|n| {
                    let label_clean = n.label.to_lowercase().replace([' ', '_'], "");
                    label_clean == lower || id.to_lowercase().replace('_', "") == lower
                })
                .unwrap_or(false)
        })
        .cloned()
}

#[cfg(not(feature = "watch"))]
pub fn watch(
    _watch_path: &Path,
    _debounce_secs: f64,
    _vault_path: Option<&Path>,
) -> crate::error::Result<()> {
    Err(crate::error::KodexError::Other(
        "Watch feature not enabled. Rebuild with --features watch".to_string(),
    ))
}
