use crate::id::normalize_id;
use crate::types::ExtractionResult;

use super::KodexGraph;

/// Build a graph from a single extraction result.
pub fn build_from_extraction(extraction: &ExtractionResult) -> KodexGraph {
    let mut graph = KodexGraph::new();

    // Add nodes
    for node in &extraction.nodes {
        graph.add_node(node.clone());
    }

    // Add edges with ID normalization
    for edge in &extraction.edges {
        let mut edge = edge.clone();

        // Normalize edge endpoints to match node IDs
        if !graph.node_index.contains_key(&edge.source) {
            let normalized = normalize_id(&edge.source);
            if graph.node_index.contains_key(&normalized) {
                edge.source = normalized;
            }
        }
        if !graph.node_index.contains_key(&edge.target) {
            let normalized = normalize_id(&edge.target);
            if graph.node_index.contains_key(&normalized) {
                edge.target = normalized;
            }
        }

        graph.add_edge(edge);
    }

    // Attach hyperedges
    graph.hyperedges = extraction.hyperedges.clone();

    graph
}

/// Merge multiple extraction results into one graph.
/// Duplicate node IDs are overwritten by later extractions.
pub fn build_merged(extractions: &[ExtractionResult]) -> KodexGraph {
    let mut graph = KodexGraph::new();

    for extraction in extractions {
        for node in &extraction.nodes {
            graph.add_node(node.clone());
        }
    }

    for extraction in extractions {
        for edge in &extraction.edges {
            let mut edge = edge.clone();

            // Handle legacy format: "from"/"to" → "source"/"target"
            // (Already handled by serde deserialization)

            // Normalize edge endpoints
            if !graph.node_index.contains_key(&edge.source) {
                let normalized = normalize_id(&edge.source);
                if graph.node_index.contains_key(&normalized) {
                    edge.source = normalized;
                }
            }
            if !graph.node_index.contains_key(&edge.target) {
                let normalized = normalize_id(&edge.target);
                if graph.node_index.contains_key(&normalized) {
                    edge.target = normalized;
                }
            }

            graph.add_edge(edge);
        }

        graph.hyperedges.extend(extraction.hyperedges.clone());
    }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn sample_extraction() -> ExtractionResult {
        ExtractionResult {
            nodes: vec![
                Node {
                    id: "main".to_string(),
                    label: "main".to_string(),
                    file_type: FileType::Code,
                    source_file: "main.py".to_string(),
                    source_location: Some("L1".to_string()),
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
                    id: "main_foo".to_string(),
                    label: "foo()".to_string(),
                    file_type: FileType::Code,
                    source_file: "main.py".to_string(),
                    source_location: Some("L5".to_string()),
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
                source: "main".to_string(),
                target: "main_foo".to_string(),
                relation: "contains".to_string(),
                confidence: Confidence::EXTRACTED,
                source_file: "main.py".to_string(),
                source_location: Some("L5".to_string()),
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_build_from_extraction() {
        let ext = sample_extraction();
        let g = build_from_extraction(&ext);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn test_build_merged() {
        let ext1 = sample_extraction();
        let ext2 = ExtractionResult {
            nodes: vec![Node {
                id: "utils".to_string(),
                label: "utils".to_string(),
                file_type: FileType::Code,
                source_file: "utils.py".to_string(),
                source_location: Some("L1".to_string()),
                confidence: Some(Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None,
                norm_label: None,
                degree: None,
                uuid: None,
                fingerprint: None,
                logical_key: None,
                body_hash: None,
            }],
            edges: vec![],
            ..Default::default()
        };

        let g = build_merged(&[ext1, ext2]);
        assert_eq!(g.node_count(), 3);
    }
}
