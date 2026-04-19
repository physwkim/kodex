use crate::graph::KodexGraph;
use super::helpers::{is_file_node, is_concept_node};

/// Information about a high-degree "god node".
#[derive(Debug, Clone)]
pub struct GodNode {
    pub id: String,
    pub label: String,
    pub degree: usize,
    pub source_file: String,
}

/// Return the top N most-connected real entities.
///
/// Excludes file-level hub nodes and concept nodes that accumulate
/// mechanical edges rather than representing meaningful architecture.
pub fn god_nodes(graph: &KodexGraph, top_n: usize) -> Vec<GodNode> {
    let mut candidates: Vec<GodNode> = graph
        .node_ids()
        .filter(|id| !is_file_node(graph, id) && !is_concept_node(graph, id))
        .filter_map(|id| {
            let node = graph.get_node(id)?;
            let degree = graph.degree(id);
            if degree == 0 {
                return None;
            }
            Some(GodNode {
                id: id.clone(),
                label: node.label.clone(),
                degree,
                source_file: node.source_file.clone(),
            })
        })
        .collect();

    candidates.sort_by(|a, b| b.degree.cmp(&a.degree));
    candidates.truncate(top_n);
    candidates
}
