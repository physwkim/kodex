use std::collections::{HashMap, HashSet};

use crate::graph::GraphifyGraph;

/// Compute cohesion score for a community: ratio of actual to possible internal edges.
/// Range: 0.0 (no internal edges) to 1.0 (complete subgraph).
pub fn cohesion_score(graph: &GraphifyGraph, community_nodes: &[String]) -> f64 {
    let n = community_nodes.len();
    if n < 2 {
        return 1.0; // Single node or empty: perfect cohesion by definition
    }

    let node_set: HashSet<&str> = community_nodes.iter().map(|s| s.as_str()).collect();
    let max_possible = n * (n - 1) / 2; // undirected

    let mut actual_edges = 0;
    let mut seen: HashSet<(&str, &str)> = HashSet::new();

    for (src, tgt, _) in graph.edges() {
        if node_set.contains(src) && node_set.contains(tgt) {
            let pair = if src < tgt { (src, tgt) } else { (tgt, src) };
            if seen.insert(pair) {
                actual_edges += 1;
            }
        }
    }

    actual_edges as f64 / max_possible as f64
}

/// Compute cohesion scores for all communities.
pub fn score_all(
    graph: &GraphifyGraph,
    communities: &HashMap<usize, Vec<String>>,
) -> HashMap<usize, f64> {
    communities
        .iter()
        .map(|(&cid, nodes)| (cid, cohesion_score(graph, nodes)))
        .collect()
}
