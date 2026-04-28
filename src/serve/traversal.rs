use std::collections::HashSet;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::graph::KodexGraph;

/// Optional filters for [`score_nodes_filtered`] and [`bfs_filtered`].
///
/// `source_pattern` and `community` constrain *both* seeding and expansion.
/// `hub_threshold` only constrains expansion: BFS will visit a hub but not
/// traverse outward through it, preventing the explosion through generic
/// nodes like `ok()`, `len()`, file-level containers.
#[derive(Debug, Default, Clone)]
pub struct TraversalFilter {
    pub source_pattern: Option<String>,
    pub community: Option<usize>,
    pub hub_threshold: Option<usize>,
}

impl TraversalFilter {
    fn matches_node(&self, graph: &KodexGraph, id: &str) -> bool {
        let node = match graph.get_node(id) {
            Some(n) => n,
            None => return false,
        };
        if let Some(sp) = self.source_pattern.as_deref() {
            if !node.source_file.to_lowercase().contains(&sp.to_lowercase()) {
                return false;
            }
        }
        if let Some(cid) = self.community {
            if node.community != Some(cid) {
                return false;
            }
        }
        true
    }
}

/// Score nodes by keyword matching using nucleo-matcher (fzf-style fuzzy).
///
/// The query is scored against `label` (weight 2), `source_file` (path-aware
/// matcher, weight 1), and `logical_key` (path-aware, weight 1). Camel/snake
/// boundaries, path separators, and consecutive matches all earn bonuses, so
/// `tickSearch` ranks `tickSearch()` above `tickSetSomethingElse()`.
pub fn score_nodes(graph: &KodexGraph, terms: &[String]) -> Vec<(usize, String)> {
    score_nodes_filtered(graph, terms, &TraversalFilter::default())
}

/// Filtered variant of [`score_nodes`].
pub fn score_nodes_filtered(
    graph: &KodexGraph,
    terms: &[String],
    filter: &TraversalFilter,
) -> Vec<(usize, String)> {
    if terms.is_empty() {
        return Vec::new();
    }
    let query: String = terms.join(" ");
    let pattern = Pattern::parse(&query, CaseMatching::Smart, Normalization::Smart);

    let mut name_matcher = Matcher::new(Config::DEFAULT);
    let mut path_matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut buf: Vec<char> = Vec::new();

    let mut scored: Vec<(usize, String)> = Vec::new();
    for id in graph.node_ids() {
        if !filter.matches_node(graph, id) {
            continue;
        }
        let node = match graph.get_node(id) {
            Some(n) => n,
            None => continue,
        };

        let label_score = pattern
            .score(Utf32Str::new(&node.label, &mut buf), &mut name_matcher)
            .unwrap_or(0) as usize;
        let path_score = pattern
            .score(
                Utf32Str::new(&node.source_file, &mut buf),
                &mut path_matcher,
            )
            .unwrap_or(0) as usize;
        let logical_score = node
            .logical_key
            .as_deref()
            .map(|s| {
                pattern
                    .score(Utf32Str::new(s, &mut buf), &mut path_matcher)
                    .unwrap_or(0) as usize
            })
            .unwrap_or(0);

        // Label dominates (weight 4) — without this, a weak label match plus a
        // strong path match can outvote a perfect label hit. Path/logical act
        // as tiebreakers between similarly-labeled candidates.
        let total = label_score.saturating_mul(4) + path_score + logical_score;
        if total > 0 {
            scored.push((total, id.clone()));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
}

/// True if any field of `filter` constrains node selection. Used by the
/// query_graph fallback: a vague natural-language question against a precise
/// `source_pattern` produces zero scored hits because nucleo terms don't
/// fuzzy-match anything in the scoped subset — in that case the caller wants
/// "show me what's in this area" instead of an empty result.
impl TraversalFilter {
    pub fn is_active(&self) -> bool {
        self.source_pattern.is_some() || self.community.is_some()
    }
}

/// Top-N filter-passing nodes by degree. Used as fallback seeds when fuzzy
/// scoring produces nothing within the filter's scope.
pub fn top_degree_in_filter(
    graph: &KodexGraph,
    filter: &TraversalFilter,
    n: usize,
) -> Vec<String> {
    let mut ranked: Vec<(usize, String)> = graph
        .node_ids()
        .filter(|id| filter.matches_node(graph, id))
        .map(|id| (graph.degree(id), id.clone()))
        .filter(|(d, _)| *d > 0)
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    ranked.into_iter().take(n).map(|(_, id)| id).collect()
}

/// Return the match positions (char indices) of `query` against `label`,
/// using the same matcher kodex uses for ranking. Empty vec when no match.
/// Useful for `get_node` to highlight which characters made a candidate rank.
pub fn label_match_indices(label: &str, query: &str) -> Vec<u32> {
    if query.is_empty() {
        return Vec::new();
    }
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf: Vec<char> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    pattern.indices(
        Utf32Str::new(label, &mut buf),
        &mut matcher,
        &mut indices,
    );
    indices.sort_unstable();
    indices.dedup();
    indices
}

/// Breadth-first traversal from start nodes.
pub fn bfs(
    graph: &KodexGraph,
    start_nodes: &[String],
    depth: usize,
) -> (HashSet<String>, Vec<(String, String)>) {
    bfs_filtered(graph, start_nodes, depth, &TraversalFilter::default())
}

/// Filtered BFS. Neighbors that fail [`TraversalFilter::matches_node`] are
/// dropped entirely. Nodes whose degree exceeds `hub_threshold` are added to
/// the visited set (so the caller can see them as boundary) but not expanded.
pub fn bfs_filtered(
    graph: &KodexGraph,
    start_nodes: &[String],
    depth: usize,
    filter: &TraversalFilter,
) -> (HashSet<String>, Vec<(String, String)>) {
    let mut visited: HashSet<String> = start_nodes.iter().cloned().collect();
    let mut frontier: HashSet<String> = visited.clone();
    let mut result_edges = Vec::new();

    for _ in 0..depth {
        let mut next_frontier = HashSet::new();
        for nid in &frontier {
            // Stop expanding through hubs to prevent explosion through ok()/len() etc.
            if let Some(t) = filter.hub_threshold {
                if graph.degree(nid) > t {
                    continue;
                }
            }
            for neighbor in graph.neighbors(nid) {
                if visited.contains(&neighbor) {
                    continue;
                }
                if !filter.matches_node(graph, &neighbor) {
                    continue;
                }
                result_edges.push((nid.clone(), neighbor.clone()));
                next_frontier.insert(neighbor);
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

    fn mk_simple_node(id: &str, label: &str, source_file: &str) -> Node {
        Node {
            id: id.into(),
            label: label.into(),
            file_type: FileType::Code,
            source_file: source_file.into(),
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
        }
    }

    #[test]
    fn nucleo_ranks_camelcase_boundary_match_above_buried_substring() {
        // Both labels contain "tickSearch" as a substring, but only one has it
        // at a camelCase boundary. nucleo's word-boundary bonus should put the
        // boundary match first.
        let extraction = ExtractionResult {
            nodes: vec![
                mk_simple_node("a", "tickSearch()", "client.cpp"),
                mk_simple_node("b", "internal_tickSearchCallback()", "client.cpp"),
            ],
            ..Default::default()
        };
        let g = build_from_extraction(&extraction);
        let scored = score_nodes(&g, &["ticksearch".into()]);
        assert!(!scored.is_empty(), "expected at least one match");
        assert_eq!(
            scored[0].1, "a",
            "tickSearch() should outrank internal_tickSearchCallback() — boundary win"
        );
    }

    #[test]
    fn label_match_indices_marks_matched_chars() {
        // "close" against "close_file": match positions are 0,1,2,3,4.
        let indices = label_match_indices("close_file", "close");
        assert_eq!(indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn label_match_indices_empty_query_returns_empty() {
        assert!(label_match_indices("anything", "").is_empty());
    }

    #[test]
    fn top_degree_in_filter_returns_high_degree_in_scope() {
        // Build: 1 high-degree node in `domain.rs`, 1 high-degree node out of scope.
        // top_degree_in_filter should only return the in-scope one.
        let mut nodes = vec![
            mk_simple_node("a", "in_scope_hub", "domain.rs"),
            mk_simple_node("b", "out_of_scope", "other.rs"),
        ];
        let mut edges = Vec::new();
        for i in 0..6 {
            nodes.push(mk_simple_node(&format!("u{i}"), &format!("user{i}"), "domain.rs"));
            edges.push(Edge {
                source: "a".into(),
                target: format!("u{i}"),
                relation: "calls".into(),
                confidence: Confidence::EXTRACTED,
                source_file: "domain.rs".into(),
                source_location: None,
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            });
            edges.push(Edge {
                source: "b".into(),
                target: format!("u{i}"),
                relation: "calls".into(),
                confidence: Confidence::EXTRACTED,
                source_file: "other.rs".into(),
                source_location: None,
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            });
        }
        let g = build_from_extraction(&ExtractionResult {
            nodes,
            edges,
            ..Default::default()
        });
        let filter = TraversalFilter {
            source_pattern: Some("domain".into()),
            ..Default::default()
        };
        let top = top_degree_in_filter(&g, &filter, 3);
        assert!(top.contains(&"a".to_string()), "in-scope hub must seed: {top:?}");
        assert!(
            !top.contains(&"b".to_string()),
            "out-of-scope must not seed: {top:?}"
        );
    }

    #[test]
    fn filter_is_active_detects_constraints() {
        assert!(!TraversalFilter::default().is_active());
        assert!(TraversalFilter {
            source_pattern: Some("x".into()),
            ..Default::default()
        }
        .is_active());
        assert!(TraversalFilter {
            community: Some(7),
            ..Default::default()
        }
        .is_active());
        // hub_threshold alone is not a "constraint" — it tunes BFS but doesn't
        // restrict the scope.
        assert!(!TraversalFilter {
            hub_threshold: Some(50),
            ..Default::default()
        }
        .is_active());
    }

    #[test]
    fn empty_terms_returns_empty() {
        let extraction = ExtractionResult {
            nodes: vec![mk_simple_node("a", "foo", "f.rs")],
            ..Default::default()
        };
        let g = build_from_extraction(&extraction);
        assert!(score_nodes(&g, &[]).is_empty());
    }

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

    /// Build a graph where a hub node connects unrelated clusters: a seed
    /// node `seed` is one hop from `hub`, and `hub` connects to many
    /// `noise_*` nodes. Without a hub threshold, BFS depth=2 from `seed`
    /// drags every noise node in.
    fn graph_with_hub() -> KodexGraph {
        let mut nodes = vec![
            Node {
                id: "seed".into(),
                label: "seed".into(),
                file_type: FileType::Code,
                source_file: "domain.rs".into(),
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
                id: "hub".into(),
                label: "ok".into(),
                file_type: FileType::Code,
                source_file: "util.rs".into(),
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
        ];
        let mut edges = vec![Edge {
            source: "seed".into(),
            target: "hub".into(),
            relation: "calls".into(),
            confidence: Confidence::EXTRACTED,
            source_file: "domain.rs".into(),
            source_location: None,
            confidence_score: Some(1.0),
            weight: 1.0,
            original_src: None,
            original_tgt: None,
        }];
        for i in 0..15 {
            nodes.push(Node {
                id: format!("noise_{i}"),
                label: format!("noise_{i}"),
                file_type: FileType::Code,
                source_file: "util.rs".into(),
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
            });
            edges.push(Edge {
                source: "hub".into(),
                target: format!("noise_{i}"),
                relation: "calls".into(),
                confidence: Confidence::EXTRACTED,
                source_file: "util.rs".into(),
                source_location: None,
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            });
        }
        crate::graph::build_from_extraction(&ExtractionResult {
            nodes,
            edges,
            ..Default::default()
        })
    }

    #[test]
    fn bfs_unfiltered_explodes_through_hub() {
        let g = graph_with_hub();
        let (visited, _) = bfs(&g, &["seed".to_string()], 2);
        // seed → hub → 15 noise nodes
        assert!(visited.len() >= 16, "should pull in all noise: {visited:?}");
    }

    #[test]
    fn bfs_with_hub_threshold_stops_at_hub() {
        let g = graph_with_hub();
        let filter = TraversalFilter {
            hub_threshold: Some(5),
            ..Default::default()
        };
        let (visited, _) = bfs_filtered(&g, &["seed".to_string()], 2, &filter);
        // seed + hub only — hub has degree 16, not expanded.
        assert!(visited.contains("seed"));
        assert!(visited.contains("hub"));
        assert!(
            !visited.iter().any(|v| v.starts_with("noise_")),
            "noise should not leak through hub: {visited:?}"
        );
    }

    #[test]
    fn bfs_with_source_pattern_skips_unmatched_files() {
        let g = graph_with_hub();
        let filter = TraversalFilter {
            source_pattern: Some("domain".into()),
            ..Default::default()
        };
        let (visited, _) = bfs_filtered(&g, &["seed".to_string()], 3, &filter);
        // hub is in util.rs, gets filtered out; only seed remains.
        assert_eq!(visited.len(), 1);
        assert!(visited.contains("seed"));
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
