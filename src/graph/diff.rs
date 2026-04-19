use std::collections::HashSet;

use super::KodexGraph;

/// Result of comparing two graph snapshots.
#[derive(Debug, Default)]
pub struct GraphDiff {
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub added_edges: Vec<(String, String)>,
    pub removed_edges: Vec<(String, String)>,
}

/// Compute changes between two graphs.
pub fn graph_diff(old: &KodexGraph, new: &KodexGraph) -> GraphDiff {
    let old_nodes: HashSet<&String> = old.node_ids().collect();
    let new_nodes: HashSet<&String> = new.node_ids().collect();

    let added_nodes: Vec<String> = new_nodes
        .difference(&old_nodes)
        .map(|s| (*s).clone())
        .collect();
    let removed_nodes: Vec<String> = old_nodes
        .difference(&new_nodes)
        .map(|s| (*s).clone())
        .collect();

    let old_edges: HashSet<(String, String)> = old
        .edges()
        .map(|(s, t, _)| (s.to_string(), t.to_string()))
        .collect();
    let new_edges: HashSet<(String, String)> = new
        .edges()
        .map(|(s, t, _)| (s.to_string(), t.to_string()))
        .collect();

    let added_edges: Vec<(String, String)> = new_edges.difference(&old_edges).cloned().collect();
    let removed_edges: Vec<(String, String)> = old_edges.difference(&new_edges).cloned().collect();

    GraphDiff {
        added_nodes,
        removed_nodes,
        added_edges,
        removed_edges,
    }
}
