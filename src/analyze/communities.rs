use std::collections::HashMap;

use super::helpers::{is_concept_node, is_file_node};
use crate::graph::KodexGraph;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommunityNodeRef {
    pub label: String,
    pub source_file: String,
    pub degree: usize,
}

/// Summary of one community for [`community_summaries`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct CommunitySummary {
    pub id: usize,
    pub size: usize,
    pub top_nodes: Vec<CommunityNodeRef>,
    pub top_files: Vec<(String, usize)>,
}

/// Build a per-community summary so the caller can pick a useful
/// `community=N` filter for `query_graph`.
///
/// Each summary includes the top-K highest-degree real nodes (file/concept
/// hubs are skipped, since they always dominate degree but tell the caller
/// nothing about the community's *content*) and the top-K most common
/// source_files.
pub fn community_summaries(
    graph: &KodexGraph,
    top_per_community: usize,
    min_size: usize,
) -> Vec<CommunitySummary> {
    let mut by_cid: HashMap<usize, Vec<String>> = HashMap::new();
    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(id) {
            let cid = node.community.unwrap_or(0);
            by_cid.entry(cid).or_default().push(id.clone());
        }
    }

    let mut summaries: Vec<CommunitySummary> = by_cid
        .into_iter()
        .filter(|(_, ids)| ids.len() >= min_size)
        .map(|(cid, ids)| {
            // Top-K by degree, skipping file/concept hubs.
            let mut ranked: Vec<(usize, &str, &str)> = ids
                .iter()
                .filter(|id| !is_file_node(graph, id) && !is_concept_node(graph, id))
                .filter_map(|id| {
                    let node = graph.get_node(id)?;
                    Some((graph.degree(id), node.label.as_str(), node.source_file.as_str()))
                })
                .collect();
            ranked.sort_by(|a, b| b.0.cmp(&a.0));
            let top_nodes: Vec<CommunityNodeRef> = ranked
                .into_iter()
                .take(top_per_community)
                .map(|(d, l, s)| CommunityNodeRef {
                    label: l.to_string(),
                    source_file: s.to_string(),
                    degree: d,
                })
                .collect();

            // Top-K source_files (where the bulk of this community lives).
            let mut file_counts: HashMap<String, usize> = HashMap::new();
            for id in &ids {
                if let Some(node) = graph.get_node(id) {
                    if !node.source_file.is_empty() {
                        *file_counts.entry(node.source_file.clone()).or_insert(0) += 1;
                    }
                }
            }
            let mut top_files: Vec<(String, usize)> = file_counts.into_iter().collect();
            top_files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            top_files.truncate(top_per_community);

            CommunitySummary {
                id: cid,
                size: ids.len(),
                top_nodes,
                top_files,
            }
        })
        .collect();

    summaries.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.id.cmp(&b.id)));
    summaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_from_extraction;
    use crate::types::{Confidence, Edge, ExtractionResult, FileType, Node};

    fn mk_node_with_community(
        id: &str,
        label: &str,
        source_file: &str,
        community: Option<usize>,
    ) -> Node {
        Node {
            id: id.into(),
            label: label.into(),
            file_type: FileType::Code,
            source_file: source_file.into(),
            source_location: Some("L1".into()),
            confidence: Some(Confidence::EXTRACTED),
            confidence_score: Some(1.0),
            community,
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

    #[test]
    fn summarizes_communities_with_top_real_nodes() {
        let extraction = ExtractionResult {
            nodes: vec![
                // Community 1: search domain
                mk_node_with_community("a", "tickSearch()", "src/search.cpp", Some(1)),
                mk_node_with_community("b", "beaconRecv()", "src/search.cpp", Some(1)),
                mk_node_with_community("c", "search", "src/search.cpp", Some(1)), // file_node
                // Community 2: data
                mk_node_with_community("d", "encode()", "src/data.cpp", Some(2)),
                mk_node_with_community("e", "decode()", "src/data.cpp", Some(2)),
            ],
            edges: vec![mk_edge("a", "b"), mk_edge("a", "c"), mk_edge("d", "e")],
            ..Default::default()
        };
        let g = build_from_extraction(&extraction);
        let summaries = community_summaries(&g, 3, 2);

        assert_eq!(summaries.len(), 2);
        let c1 = summaries.iter().find(|s| s.id == 1).expect("c1");
        let labels: Vec<&str> = c1.top_nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.contains(&"tickSearch()") && labels.contains(&"beaconRecv()"),
            "community 1 should surface real symbols: {labels:?}"
        );
        assert!(
            !labels.contains(&"search"),
            "file_node `search` must be skipped: {labels:?}"
        );
    }

    #[test]
    fn min_size_filters_singletons() {
        let extraction = ExtractionResult {
            nodes: vec![
                mk_node_with_community("a", "alone()", "x.rs", Some(7)),
                mk_node_with_community("b", "first()", "x.rs", Some(8)),
                mk_node_with_community("c", "second()", "x.rs", Some(8)),
            ],
            ..Default::default()
        };
        let g = build_from_extraction(&extraction);
        let summaries = community_summaries(&g, 5, 2);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, 8);
    }
}
