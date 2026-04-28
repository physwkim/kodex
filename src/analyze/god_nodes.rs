use super::helpers::{is_concept_node, is_file_node};
use crate::graph::KodexGraph;

/// Information about a high-degree "god node".
#[derive(Debug, Clone)]
pub struct GodNode {
    pub id: String,
    pub label: String,
    pub degree: usize,
    pub source_file: String,
}

/// Optional filters for [`god_nodes_filtered`].
#[derive(Debug, Default, Clone)]
pub struct GodNodesFilter {
    /// Substring (case-insensitive) that must appear in the label.
    pub pattern: Option<String>,
    /// Substring that must appear in the source_file.
    pub source_pattern: Option<String>,
    /// Lower bound on degree.
    pub min_degree: Option<usize>,
}

/// Return the top N most-connected real entities.
///
/// Excludes file-level hub nodes and concept nodes that accumulate
/// mechanical edges rather than representing meaningful architecture.
pub fn god_nodes(graph: &KodexGraph, top_n: usize) -> Vec<GodNode> {
    god_nodes_filtered(graph, top_n, &GodNodesFilter::default())
}

/// Filtered variant of [`god_nodes`]. Useful when generic hubs (`ok()`, `len()`)
/// dominate the unfiltered top-N and the caller wants to narrow to a domain.
pub fn god_nodes_filtered(
    graph: &KodexGraph,
    top_n: usize,
    filter: &GodNodesFilter,
) -> Vec<GodNode> {
    let pat = filter.pattern.as_deref().map(str::to_lowercase);
    let src_pat = filter.source_pattern.as_deref().map(str::to_lowercase);
    let min_degree = filter.min_degree.unwrap_or(0);

    let mut candidates: Vec<GodNode> = graph
        .node_ids()
        .filter(|id| !is_file_node(graph, id) && !is_concept_node(graph, id))
        .filter_map(|id| {
            let node = graph.get_node(id)?;
            let degree = graph.degree(id);
            if degree == 0 || degree < min_degree {
                return None;
            }
            if let Some(p) = pat.as_deref() {
                if !node.label.to_lowercase().contains(p) {
                    return None;
                }
            }
            if let Some(sp) = src_pat.as_deref() {
                if !node.source_file.to_lowercase().contains(sp) {
                    return None;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_from_extraction;
    use crate::types::{Confidence, Edge, ExtractionResult, FileType, Node};

    fn mk_node(id: &str, label: &str, source_file: &str) -> Node {
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

    fn mk_edge(src: &str, tgt: &str) -> Edge {
        Edge {
            source: src.into(),
            target: tgt.into(),
            relation: "calls".into(),
            confidence: Confidence::EXTRACTED,
            source_file: "x.rs".into(),
            source_location: None,
            confidence_score: Some(1.0),
            weight: 1.0,
            original_src: None,
            original_tgt: None,
        }
    }

    /// Build a graph where a generic hub `ok()` outranks a domain hub
    /// `Server::process()` so we can verify pattern filtering.
    fn graph_with_generic_and_domain_hubs() -> KodexGraph {
        let mut nodes = vec![
            mk_node("ok", "ok()", "lib/util.rs"),
            mk_node("srv", "Server::process", "ca-rs/server.rs"),
            mk_node("c1", "client_a", "ca-rs/client.rs"),
            mk_node("c2", "client_b", "ca-rs/client.rs"),
            mk_node("c3", "client_c", "ca-rs/client.rs"),
        ];
        // ok() touches everyone (degree 4), Server::process touches 2.
        for i in 0..6 {
            nodes.push(mk_node(&format!("u{i}"), &format!("user_{i}"), "lib/util.rs"));
        }
        let edges = vec![
            mk_edge("ok", "u0"),
            mk_edge("ok", "u1"),
            mk_edge("ok", "u2"),
            mk_edge("ok", "u3"),
            mk_edge("ok", "u4"),
            mk_edge("ok", "u5"),
            mk_edge("srv", "c1"),
            mk_edge("srv", "c2"),
            mk_edge("srv", "c3"),
        ];
        build_from_extraction(&ExtractionResult {
            nodes,
            edges,
            ..Default::default()
        })
    }

    #[test]
    fn unfiltered_top_returns_generic_hub_first() {
        let g = graph_with_generic_and_domain_hubs();
        let result = god_nodes(&g, 1);
        assert_eq!(result[0].label, "ok()");
    }

    #[test]
    fn source_pattern_filter_isolates_domain() {
        let g = graph_with_generic_and_domain_hubs();
        let filter = GodNodesFilter {
            source_pattern: Some("ca-rs".into()),
            ..Default::default()
        };
        let result = god_nodes_filtered(&g, 5, &filter);
        assert!(
            result.iter().all(|g| g.source_file.contains("ca-rs")),
            "all results should be in ca-rs/: {result:?}"
        );
        assert_eq!(result[0].label, "Server::process");
    }

    #[test]
    fn pattern_filter_matches_label_substring() {
        let g = graph_with_generic_and_domain_hubs();
        let filter = GodNodesFilter {
            pattern: Some("server".into()),
            ..Default::default()
        };
        let result = god_nodes_filtered(&g, 5, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "Server::process");
    }

    #[test]
    fn min_degree_excludes_low_connection_nodes() {
        let g = graph_with_generic_and_domain_hubs();
        let filter = GodNodesFilter {
            min_degree: Some(5),
            ..Default::default()
        };
        let result = god_nodes_filtered(&g, 10, &filter);
        // Only ok() has degree 6; Server::process has 3.
        assert!(result.iter().all(|n| n.degree >= 5));
        assert_eq!(result.len(), 1);
    }
}
