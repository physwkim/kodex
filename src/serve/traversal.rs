use std::collections::HashSet;

use crate::export::strip_diacritics;
use crate::graph::KodexGraph;

/// Score nodes by keyword matching with fuzzy support.
/// Matches: exact substring > token overlap > edit distance.
pub fn score_nodes(graph: &KodexGraph, terms: &[String]) -> Vec<(usize, String)> {
    let mut scored: Vec<(usize, String)> = Vec::new();
    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(id) {
            let label = strip_diacritics(&node.label).to_lowercase();
            let source = node.source_file.to_lowercase();
            let logical = node.logical_key.as_deref().unwrap_or("").to_lowercase();

            let mut score = 0usize;
            for term in terms {
                // Exact substring in label (strongest)
                if label.contains(term.as_str()) {
                    score += 10;
                    continue;
                }
                // Exact substring in source_file or logical_key
                if source.contains(term.as_str()) || logical.contains(term.as_str()) {
                    score += 5;
                    continue;
                }
                // Token overlap: split label into parts and match
                let label_tokens: Vec<&str> = label
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|s| s.len() > 1)
                    .collect();
                if label_tokens.iter().any(|lt| lt.contains(term.as_str())) {
                    score += 7;
                    continue;
                }
                // Fuzzy: edit distance ≤ 2 for tokens > 4 chars
                if term.len() > 4 {
                    for lt in &label_tokens {
                        if lt.len() > 4 && edit_distance(term, lt) <= 2 {
                            score += 3;
                            break;
                        }
                    }
                }
            }
            if score > 0 {
                scored.push((score, id.clone()));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
}

/// Simple edit distance (Levenshtein).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev = (0..=b.len()).collect::<Vec<_>>();
    for (i, ca) in a.iter().enumerate() {
        let mut curr = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        prev = curr;
    }
    prev[b.len()]
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

/// Render a subgraph as a Mermaid flowchart suitable for pasting into docs.
/// Mermaid sanitizes node ids (alnum + underscore) and quotes labels.
pub fn subgraph_to_mermaid(
    graph: &KodexGraph,
    nodes: &HashSet<String>,
    _edges: &[(String, String)],
) -> String {
    fn sanitize_id(raw: &str) -> String {
        let mut s = String::with_capacity(raw.len());
        for c in raw.chars() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
            } else {
                s.push('_');
            }
        }
        if s.chars().next().map(|c| c.is_numeric()).unwrap_or(true) {
            s = format!("n_{s}");
        }
        s
    }
    fn escape_label(s: &str) -> String {
        s.replace('"', "'")
    }

    let mut lines = vec!["flowchart LR".to_string()];
    for nid in nodes {
        let label = graph
            .get_node(nid)
            .map(|n| n.label.clone())
            .unwrap_or_else(|| nid.clone());
        lines.push(format!(
            "    {}[\"{}\"]",
            sanitize_id(nid),
            escape_label(&label)
        ));
    }
    for (src, tgt, edge) in graph.edges() {
        if nodes.contains(src) && nodes.contains(tgt) {
            lines.push(format!(
                "    {} -->|{}| {}",
                sanitize_id(src),
                escape_label(&edge.relation),
                sanitize_id(tgt)
            ));
        }
    }
    lines.join("\n")
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
            let src_label = graph.get_node(src).map(|n| n.label.as_str()).unwrap_or(src);
            let tgt_label = graph.get_node(tgt).map(|n| n.label.as_str()).unwrap_or(tgt);
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

#[cfg(test)]
mod traversal_tests {
    use super::*;
    use crate::graph::build_from_extraction;
    use crate::types::{Confidence, Edge, ExtractionResult, FileType, Node};

    fn make_graph() -> KodexGraph {
        let extraction = ExtractionResult {
            nodes: vec![
                Node {
                    id: "a-mod.foo".into(),
                    label: "foo()".into(),
                    file_type: FileType::Code,
                    source_file: "a.py".into(),
                    source_location: Some("L1".into()),
                    confidence: Some(Confidence::EXTRACTED),
                    confidence_score: Some(1.0),
                    community: None,
                    norm_label: None,
                    degree: None,
                    uuid: None,
                    fingerprint: None,
                    logical_key: None,
                    body_hash: None,
                },
                Node {
                    id: "b-mod.bar".into(),
                    label: "bar()".into(),
                    file_type: FileType::Code,
                    source_file: "b.py".into(),
                    source_location: Some("L1".into()),
                    confidence: Some(Confidence::EXTRACTED),
                    confidence_score: Some(1.0),
                    community: None,
                    norm_label: None,
                    degree: None,
                    uuid: None,
                    fingerprint: None,
                    logical_key: None,
                    body_hash: None,
                },
            ],
            edges: vec![Edge {
                source: "a-mod.foo".into(),
                target: "b-mod.bar".into(),
                relation: "calls".into(),
                confidence: Confidence::EXTRACTED,
                source_file: "a.py".into(),
                source_location: Some("L5".into()),
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            }],
            ..Default::default()
        };
        build_from_extraction(&extraction)
    }

    #[test]
    fn test_subgraph_to_mermaid_renders_flowchart() {
        let graph = make_graph();
        let mut nodes = HashSet::new();
        nodes.insert("a-mod.foo".to_string());
        nodes.insert("b-mod.bar".to_string());
        let edges = vec![("a-mod.foo".into(), "b-mod.bar".into())];
        let out = subgraph_to_mermaid(&graph, &nodes, &edges);
        assert!(
            out.starts_with("flowchart LR"),
            "should be a Mermaid flowchart: {out}"
        );
        assert!(out.contains("foo()"));
        assert!(out.contains("bar()"));
        assert!(out.contains("|calls|"));
        // Ids must be alphanum/underscore (no '.' or '-')
        for line in out.lines().skip(1) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Each subsequent line either declares a node ("id[\"label\"]") or an edge.
            let id_part = trimmed.split('[').next().unwrap_or("").trim();
            let id_part = id_part.split_whitespace().next().unwrap_or("");
            for c in id_part.chars() {
                assert!(
                    c.is_alphanumeric() || c == '_',
                    "invalid Mermaid id char {c:?} in line {trimmed}"
                );
            }
        }
    }
}
