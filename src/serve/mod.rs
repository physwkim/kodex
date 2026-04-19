mod traversal;

pub use traversal::{bfs, dfs, subgraph_to_text, score_nodes};

use std::collections::HashMap;
use std::path::Path;

use crate::graph::KodexGraph;
use crate::graph::build_from_extraction;
use crate::types::ExtractionResult;

/// Smart graph loading: try vault first, fall back to graph.json.
///
/// If `path` is a directory containing .md files, loads from vault (source of truth).
/// If `path` is a .json file, loads from JSON (cache).
/// If graph.json exists but is stale relative to vault, reloads from vault.
pub fn load_graph_smart(path: &Path) -> crate::error::Result<KodexGraph> {
    // If path is a directory, treat as vault
    if path.is_dir() {
        return crate::vault::load_graph_from_vault(path);
    }

    // Explicit HDF5 file
    if path.extension().map(|e| e == "h5" || e == "hdf5").unwrap_or(false) {
        return crate::storage::load_hdf5(path);
    }

    // If asking for JSON but HDF5 exists alongside, prefer HDF5
    if path.extension().map(|e| e == "json").unwrap_or(false) {
        let h5_path = path.with_extension("h5");
        if h5_path.exists() {
            return crate::storage::load_hdf5(&h5_path);
        }
    }

    // If it's a JSON file, check if a vault directory exists alongside it
    if path.extension().map(|e| e == "json").unwrap_or(false) {
        if let Some(parent) = path.parent() {
            // Check if parent is a vault (has .md files)
            let has_md = std::fs::read_dir(parent)
                .map(|entries| {
                    entries
                        .flatten()
                        .any(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
                })
                .unwrap_or(false);

            if has_md && crate::vault::is_cache_stale(parent, path) {
                // Vault is newer than cache — reload from vault and update cache
                let graph = crate::vault::load_graph_from_vault(parent)?;
                let communities = crate::cluster::cluster(&graph);
                let _ = crate::export::to_json(&graph, &communities, path);
                return Ok(graph);
            }
        }
    }

    // Default: load from JSON
    load_graph(path)
}

/// Load a graph from a JSON file (networkx node-link format).
pub fn load_graph(graph_path: &Path) -> crate::error::Result<KodexGraph> {
    let text = std::fs::read_to_string(graph_path)?;
    let data: serde_json::Value = serde_json::from_str(&text)?;

    // Parse nodes and edges from node-link format
    let nodes_val = data.get("nodes").and_then(|v| v.as_array());
    let links_val = data
        .get("links")
        .or_else(|| data.get("edges"))
        .and_then(|v| v.as_array());

    let mut extraction = ExtractionResult::default();

    if let Some(nodes) = nodes_val {
        for node_val in nodes {
            if let Ok(node) = serde_json::from_value(node_val.clone()) {
                extraction.nodes.push(node);
            }
        }
    }

    if let Some(links) = links_val {
        for link_val in links {
            if let Ok(edge) = serde_json::from_value(link_val.clone()) {
                extraction.edges.push(edge);
            }
        }
    }

    if let Some(hyper) = data.get("hyperedges").and_then(|v| v.as_array()) {
        for h in hyper {
            if let Ok(he) = serde_json::from_value(h.clone()) {
                extraction.hyperedges.push(he);
            }
        }
    }

    Ok(build_from_extraction(&extraction))
}

/// Reconstruct communities from node community attributes.
pub fn communities_from_graph(graph: &KodexGraph) -> HashMap<usize, Vec<String>> {
    let mut communities: HashMap<usize, Vec<String>> = HashMap::new();
    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(id) {
            let cid = node.community.unwrap_or(0);
            communities.entry(cid).or_default().push(id.clone());
        }
    }
    communities
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Test round-trip: export → load → verify
    #[test]
    fn test_graph_json_round_trip() {
        let dir = TempDir::new().unwrap();
        let json_path = dir.path().join("graph.json");

        // Build a small graph
        let extraction = ExtractionResult {
            nodes: vec![
                crate::types::Node {
                    id: "main".to_string(),
                    label: "main".to_string(),
                    file_type: crate::types::FileType::Code,
                    source_file: "main.py".to_string(),
                    source_location: Some("L1".to_string()),
                    confidence: Some(crate::types::Confidence::EXTRACTED),
                    confidence_score: Some(1.0),
                    community: None,
                    norm_label: None,
                    degree: None,
                },
                crate::types::Node {
                    id: "main_foo".to_string(),
                    label: "foo()".to_string(),
                    file_type: crate::types::FileType::Code,
                    source_file: "main.py".to_string(),
                    source_location: Some("L5".to_string()),
                    confidence: Some(crate::types::Confidence::EXTRACTED),
                    confidence_score: Some(1.0),
                    community: None,
                    norm_label: None,
                    degree: None,
                },
            ],
            edges: vec![crate::types::Edge {
                source: "main".to_string(),
                target: "main_foo".to_string(),
                relation: "contains".to_string(),
                confidence: crate::types::Confidence::EXTRACTED,
                source_file: "main.py".to_string(),
                source_location: Some("L5".to_string()),
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            }],
            ..Default::default()
        };

        let graph = build_from_extraction(&extraction);
        let communities = crate::cluster::cluster(&graph);

        // Export to JSON
        crate::export::to_json(&graph, &communities, &json_path).unwrap();

        // Load back
        let loaded = load_graph(&json_path).unwrap();
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);
        assert!(loaded.get_node("main").is_some());
        assert!(loaded.get_node("main_foo").is_some());
    }

    /// Test loading a networkx-style JSON with "links" key
    #[test]
    fn test_load_networkx_format() {
        let dir = TempDir::new().unwrap();
        let json_path = dir.path().join("graph.json");

        let data = serde_json::json!({
            "directed": false,
            "multigraph": false,
            "nodes": [
                {"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"},
                {"id": "b", "label": "B", "file_type": "code", "source_file": "b.py"}
            ],
            "links": [
                {"source": "a", "target": "b", "relation": "imports", "confidence": "EXTRACTED", "source_file": "a.py"}
            ]
        });
        std::fs::write(&json_path, serde_json::to_string(&data).unwrap()).unwrap();

        let loaded = load_graph(&json_path).unwrap();
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);
    }
}
