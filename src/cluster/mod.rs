mod cohesion;
mod louvain;

use crate::graph::KodexGraph;
use std::collections::HashMap;

pub use cohesion::{cohesion_score, score_all};

/// Run community detection on a graph.
/// Returns community_id → list of node IDs, sorted by community size descending.
pub fn cluster(graph: &KodexGraph) -> HashMap<usize, Vec<String>> {
    let partition = louvain::louvain_communities(graph);

    // Convert node→community to community→nodes
    let mut communities: HashMap<usize, Vec<String>> = HashMap::new();
    for (node_id, community_id) in &partition {
        communities
            .entry(*community_id)
            .or_default()
            .push(node_id.clone());
    }

    // Handle isolates: each becomes its own community
    let max_cid = communities.keys().max().copied().unwrap_or(0);
    let mut next_cid = max_cid + 1;
    for node_id in graph.node_ids() {
        if !partition.contains_key(node_id) {
            communities.insert(next_cid, vec![node_id.clone()]);
            next_cid += 1;
        }
    }

    // Split oversized communities (>25% of total nodes, min 10 nodes)
    let total_nodes = graph.node_count();
    let threshold = (total_nodes / 4).max(10);
    let oversized: Vec<usize> = communities
        .iter()
        .filter(|(_, nodes)| nodes.len() > threshold)
        .map(|(cid, _)| *cid)
        .collect();

    for cid in oversized {
        if let Some(nodes) = communities.remove(&cid) {
            // Simple split: divide into two roughly equal halves
            let mid = nodes.len() / 2;
            communities.insert(cid, nodes[..mid].to_vec());
            communities.insert(next_cid, nodes[mid..].to_vec());
            next_cid += 1;
        }
    }

    // Re-index by size descending for deterministic ordering
    let mut sorted: Vec<(usize, Vec<String>)> = communities.into_iter().collect();
    sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut result = HashMap::new();
    for (new_cid, (_old_cid, nodes)) in sorted.into_iter().enumerate() {
        result.insert(new_cid, nodes);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn make_graph() -> KodexGraph {
        let mut g = KodexGraph::new();
        for id in &["a", "b", "c", "d", "e"] {
            g.add_node(Node {
                id: id.to_string(),
                label: id.to_string(),
                file_type: FileType::Code,
                source_file: "test.py".to_string(),
                source_location: None,
                confidence: None,
                confidence_score: None,
                community: None,
                norm_label: None,
                degree: None,
            });
        }
        // Create two clusters: {a,b,c} and {d,e}
        for (s, t) in &[("a", "b"), ("b", "c"), ("a", "c"), ("d", "e")] {
            g.add_edge(Edge {
                source: s.to_string(),
                target: t.to_string(),
                relation: "related".to_string(),
                confidence: Confidence::EXTRACTED,
                source_file: "test.py".to_string(),
                source_location: None,
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            });
        }
        g
    }

    #[test]
    fn test_cluster_produces_communities() {
        let g = make_graph();
        let communities = cluster(&g);

        // Should have at least 1 community
        assert!(!communities.is_empty());

        // All nodes should be assigned
        let total: usize = communities.values().map(|v| v.len()).sum();
        assert_eq!(total, 5);
    }
}
