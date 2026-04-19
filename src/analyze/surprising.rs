use std::collections::HashMap;

use crate::graph::KodexGraph;
use crate::types::Confidence;
use super::node_community_map;
use super::helpers::{is_file_node, is_concept_node};

/// A surprising connection between two nodes.
#[derive(Debug, Clone)]
pub struct SurprisingConnection {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: Confidence,
    pub score: usize,
    pub reasons: Vec<String>,
}

/// Find connections that are genuinely surprising.
///
/// Multi-file corpora: cross-file edges between real entities.
/// Scores by confidence (AMBIGUOUS > INFERRED > EXTRACTED), cross-file,
/// and cross-community factors.
pub fn surprising_connections(
    graph: &KodexGraph,
    communities: Option<&HashMap<usize, Vec<String>>>,
    top_n: usize,
) -> Vec<SurprisingConnection> {
    let node_comm = communities
        .map(|c| node_community_map(c))
        .unwrap_or_default();

    let mut surprises: Vec<SurprisingConnection> = Vec::new();

    for (src, tgt, edge) in graph.edges() {
        // Skip file nodes and concept nodes
        if is_file_node(graph, src) || is_file_node(graph, tgt) {
            continue;
        }
        if is_concept_node(graph, src) || is_concept_node(graph, tgt) {
            continue;
        }

        let mut score = 0;
        let mut reasons = Vec::new();

        // Confidence weight
        match edge.confidence {
            Confidence::AMBIGUOUS => {
                score += 3;
                reasons.push("AMBIGUOUS confidence".to_string());
            }
            Confidence::INFERRED => {
                score += 2;
                reasons.push("INFERRED confidence".to_string());
            }
            Confidence::EXTRACTED => {
                score += 1;
            }
        }

        // Cross-file bonus
        let src_node = graph.get_node(src);
        let tgt_node = graph.get_node(tgt);
        if let (Some(sn), Some(tn)) = (src_node, tgt_node) {
            if sn.source_file != tn.source_file {
                score += 2;
                reasons.push("cross-file".to_string());
            }
        }

        // Cross-community bonus
        if let (Some(&sc), Some(&tc)) = (node_comm.get(src), node_comm.get(tgt)) {
            if sc != tc {
                score += 1;
                reasons.push("cross-community".to_string());
            }
        }

        if score >= 2 {
            surprises.push(SurprisingConnection {
                source: src.to_string(),
                target: tgt.to_string(),
                relation: edge.relation.clone(),
                confidence: edge.confidence,
                score,
                reasons,
            });
        }
    }

    surprises.sort_by(|a, b| b.score.cmp(&a.score));
    surprises.truncate(top_n);
    surprises
}
