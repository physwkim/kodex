use std::collections::HashSet;

use serde_json::Value;

const VALID_FILE_TYPES: &[&str] = &["code", "document", "image", "paper", "rationale", "video"];
const VALID_CONFIDENCES: &[&str] = &["AMBIGUOUS", "EXTRACTED", "INFERRED"];
const REQUIRED_NODE_FIELDS: &[&str] = &["id", "label", "file_type", "source_file"];
const REQUIRED_EDGE_FIELDS: &[&str] = &["source", "target", "relation", "confidence", "source_file"];

/// Validate an extraction JSON value against the graphify schema.
/// Returns a list of error strings -- empty list means valid.
pub fn validate_extraction(data: &Value) -> Vec<String> {
    let mut errors = Vec::new();

    let obj = match data.as_object() {
        Some(o) => o,
        None => return vec!["Extraction must be a JSON object".to_string()],
    };

    // --- Nodes ---
    match obj.get("nodes") {
        None => errors.push("Missing required key 'nodes'".to_string()),
        Some(nodes_val) => match nodes_val.as_array() {
            None => errors.push("'nodes' must be a list".to_string()),
            Some(nodes) => {
                for (i, node) in nodes.iter().enumerate() {
                    let node_obj = match node.as_object() {
                        Some(o) => o,
                        None => {
                            errors.push(format!("Node {i} must be an object"));
                            continue;
                        }
                    };
                    let node_id = node_obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    for &field in REQUIRED_NODE_FIELDS {
                        if !node_obj.contains_key(field) {
                            errors.push(format!(
                                "Node {i} (id={node_id:?}) missing required field '{field}'"
                            ));
                        }
                    }
                    if let Some(ft) = node_obj.get("file_type").and_then(|v| v.as_str()) {
                        if !VALID_FILE_TYPES.contains(&ft) {
                            errors.push(format!(
                                "Node {i} (id={node_id:?}) has invalid file_type \
                                 '{ft}' - must be one of {VALID_FILE_TYPES:?}"
                            ));
                        }
                    }
                }
            }
        },
    }

    // --- Edges (accept "links" as fallback for NetworkX <= 3.1) ---
    let edge_list = obj
        .get("edges")
        .or_else(|| obj.get("links"));
    match edge_list {
        None => errors.push("Missing required key 'edges'".to_string()),
        Some(edges_val) => match edges_val.as_array() {
            None => errors.push("'edges' must be a list".to_string()),
            Some(edges) => {
                let node_ids: HashSet<&str> = obj
                    .get("nodes")
                    .and_then(|v| v.as_array())
                    .map(|nodes| {
                        nodes
                            .iter()
                            .filter_map(|n| n.as_object())
                            .filter_map(|n| n.get("id"))
                            .filter_map(|v| v.as_str())
                            .collect()
                    })
                    .unwrap_or_default();

                for (i, edge) in edges.iter().enumerate() {
                    let edge_obj = match edge.as_object() {
                        Some(o) => o,
                        None => {
                            errors.push(format!("Edge {i} must be an object"));
                            continue;
                        }
                    };
                    for &field in REQUIRED_EDGE_FIELDS {
                        if !edge_obj.contains_key(field) {
                            errors.push(format!("Edge {i} missing required field '{field}'"));
                        }
                    }
                    if let Some(conf) = edge_obj.get("confidence").and_then(|v| v.as_str()) {
                        if !VALID_CONFIDENCES.contains(&conf) {
                            errors.push(format!(
                                "Edge {i} has invalid confidence '{conf}' \
                                 - must be one of {VALID_CONFIDENCES:?}"
                            ));
                        }
                    }
                    if !node_ids.is_empty() {
                        if let Some(src) = edge_obj.get("source").and_then(|v| v.as_str()) {
                            if !node_ids.contains(src) {
                                errors.push(format!(
                                    "Edge {i} source '{src}' does not match any node id"
                                ));
                            }
                        }
                        if let Some(tgt) = edge_obj.get("target").and_then(|v| v.as_str()) {
                            if !node_ids.contains(tgt) {
                                errors.push(format!(
                                    "Edge {i} target '{tgt}' does not match any node id"
                                ));
                            }
                        }
                    }
                }
            }
        },
    }

    errors
}

/// Panics with all errors if extraction is invalid.
pub fn assert_valid(data: &Value) -> crate::error::Result<()> {
    let errors = validate_extraction(data);
    if errors.is_empty() {
        return Ok(());
    }
    let msg = format!(
        "Extraction JSON has {} error(s):\n{}",
        errors.len(),
        errors
            .iter()
            .map(|e| format!("  \u{2022} {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Err(crate::error::GraphifyError::Validation(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_extraction() {
        let data = json!({
            "nodes": [{
                "id": "foo",
                "label": "Foo",
                "file_type": "code",
                "source_file": "foo.py"
            }],
            "edges": [{
                "source": "foo",
                "target": "foo",
                "relation": "contains",
                "confidence": "EXTRACTED",
                "source_file": "foo.py"
            }]
        });
        assert!(validate_extraction(&data).is_empty());
    }

    #[test]
    fn test_missing_nodes() {
        let data = json!({"edges": []});
        let errs = validate_extraction(&data);
        assert!(errs.iter().any(|e| e.contains("Missing required key 'nodes'")));
    }

    #[test]
    fn test_invalid_file_type() {
        let data = json!({
            "nodes": [{"id": "a", "label": "A", "file_type": "unknown", "source_file": "a.py"}],
            "edges": []
        });
        let errs = validate_extraction(&data);
        assert!(errs.iter().any(|e| e.contains("invalid file_type")));
    }

    #[test]
    fn test_invalid_confidence() {
        let data = json!({
            "nodes": [{"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"}],
            "edges": [{"source": "a", "target": "a", "relation": "r", "confidence": "BAD", "source_file": "a.py"}]
        });
        let errs = validate_extraction(&data);
        assert!(errs.iter().any(|e| e.contains("invalid confidence")));
    }

    #[test]
    fn test_dangling_edge() {
        let data = json!({
            "nodes": [{"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"}],
            "edges": [{"source": "a", "target": "missing", "relation": "r", "confidence": "EXTRACTED", "source_file": "a.py"}]
        });
        let errs = validate_extraction(&data);
        assert!(errs.iter().any(|e| e.contains("does not match any node id")));
    }

    #[test]
    fn test_links_fallback() {
        let data = json!({
            "nodes": [{"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"}],
            "links": [{"source": "a", "target": "a", "relation": "r", "confidence": "EXTRACTED", "source_file": "a.py"}]
        });
        assert!(validate_extraction(&data).is_empty());
    }
}
