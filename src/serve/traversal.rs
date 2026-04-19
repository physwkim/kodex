use std::collections::HashSet;

use crate::graph::KodexGraph;
use crate::export::strip_diacritics;

/// Score nodes by keyword matching (diacritics-insensitive).
pub fn score_nodes(graph: &KodexGraph, terms: &[String]) -> Vec<(usize, String)> {
    let mut scored: Vec<(usize, String)> = Vec::new();
    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(id) {
            let label = strip_diacritics(&node.label).to_lowercase();
            let score = terms.iter().filter(|t| label.contains(t.as_str())).count();
            if score > 0 {
                scored.push((score, id.clone()));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
}

/// Breadth-first traversal from start nodes.
pub fn bfs(
    graph: &KodexGraph,
    start_nodes: &[String],
    depth: usize,
) -> (HashSet<String>, Vec<(String, String)>) {
    let mut visited: HashSet<String> = start_nodes.iter().cloned().collect();
    let mut frontier: HashSet<String> = visited.clone();
    let mut result_edges = Vec::new();

    for _ in 0..depth {
        let mut next_frontier = HashSet::new();
        for nid in &frontier {
            for neighbor in graph.neighbors(nid) {
                if !visited.contains(&neighbor) {
                    result_edges.push((nid.clone(), neighbor.clone()));
                    next_frontier.insert(neighbor);
                }
            }
        }
        visited.extend(next_frontier.iter().cloned());
        frontier = next_frontier;
    }

    (visited, result_edges)
}

/// Depth-first traversal from start nodes.
pub fn dfs(
    graph: &KodexGraph,
    start_nodes: &[String],
    depth: usize,
) -> (HashSet<String>, Vec<(String, String)>) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut result_edges = Vec::new();

    for start in start_nodes {
        dfs_recurse(graph, start, depth, &mut visited, &mut result_edges);
    }

    (visited, result_edges)
}

fn dfs_recurse(
    graph: &KodexGraph,
    node: &str,
    depth: usize,
    visited: &mut HashSet<String>,
    edges: &mut Vec<(String, String)>,
) {
    if depth == 0 || !visited.insert(node.to_string()) {
        return;
    }
    for neighbor in graph.neighbors(node) {
        if !visited.contains(&neighbor) {
            edges.push((node.to_string(), neighbor.clone()));
            dfs_recurse(graph, &neighbor, depth - 1, visited, edges);
        }
    }
}

/// Render a subgraph as text, limited by token budget (~4 chars per token).
pub fn subgraph_to_text(
    graph: &KodexGraph,
    nodes: &HashSet<String>,
    _edges: &[(String, String)],
    token_budget: usize,
) -> String {
    let max_chars = token_budget * 4;
    let mut lines = Vec::new();

    for nid in nodes {
        if let Some(node) = graph.get_node(nid) {
            lines.push(format!(
                "NODE {} src={} loc={}",
                node.label,
                node.source_file,
                node.source_location.as_deref().unwrap_or("")
            ));
        }
    }

    for (src, tgt, edge) in graph.edges() {
        if nodes.contains(src) && nodes.contains(tgt) {
            let src_label = graph
                .get_node(src)
                .map(|n| n.label.as_str())
                .unwrap_or(src);
            let tgt_label = graph
                .get_node(tgt)
                .map(|n| n.label.as_str())
                .unwrap_or(tgt);
            lines.push(format!(
                "EDGE {src_label} --{}--> {tgt_label} [{}]",
                edge.relation, edge.confidence
            ));
        }
    }

    let result = lines.join("\n");
    if result.len() > max_chars {
        // Truncate at a valid UTF-8 char boundary
        let mut end = max_chars;
        while end > 0 && !result.is_char_boundary(end) {
            end -= 1;
        }
        result[..end].to_string()
    } else {
        result
    }
}
