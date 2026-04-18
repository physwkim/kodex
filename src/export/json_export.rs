use std::collections::HashMap;
use std::path::Path;

use crate::graph::GraphifyGraph;
use super::{node_community_map, strip_diacritics};

/// Export graph to JSON in networkx node-link format.
pub fn to_json(
    graph: &GraphifyGraph,
    communities: &HashMap<usize, Vec<String>>,
    output_path: &Path,
) -> std::io::Result<()> {
    let node_comm = node_community_map(communities);

    // Build nodes array
    let nodes: Vec<serde_json::Value> = graph
        .node_ids()
        .filter_map(|id| {
            let node = graph.get_node(id)?;
            let mut obj = serde_json::to_value(node).ok()?;
            let map = obj.as_object_mut()?;
            map.insert(
                "community".to_string(),
                serde_json::json!(node_comm.get(id).copied().unwrap_or(0)),
            );
            map.insert(
                "norm_label".to_string(),
                serde_json::json!(strip_diacritics(&node.label).to_lowercase()),
            );
            Some(obj)
        })
        .collect();

    // Build links array
    let links: Vec<serde_json::Value> = graph
        .edges()
        .map(|(src, tgt, edge)| {
            let score = edge.confidence_score.unwrap_or_else(|| edge.confidence.default_score());
            serde_json::json!({
                "source": src,
                "target": tgt,
                "relation": edge.relation,
                "confidence": edge.confidence.to_string(),
                "confidence_score": score,
                "source_file": edge.source_file,
                "weight": edge.weight,
            })
        })
        .collect();

    // Build hyperedges array
    let hyperedges: Vec<serde_json::Value> = graph
        .hyperedges
        .iter()
        .filter_map(|h| serde_json::to_value(h).ok())
        .collect();

    let data = serde_json::json!({
        "directed": true,
        "multigraph": false,
        "nodes": nodes,
        "links": links,
        "hyperedges": hyperedges,
    });

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(output_path, json)
}
