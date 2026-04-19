use std::path::Path;

use crate::graph::KodexGraph;

/// True if node is a file-level hub or AST method stub.
///
/// Signals:
/// - Label matches source filename
/// - Label is ".method_name()" (method stub)
/// - Label is "function_name()" with ≤1 connection
pub fn is_file_node(graph: &KodexGraph, node_id: &str) -> bool {
    let node = match graph.get_node(node_id) {
        Some(n) => n,
        None => return false,
    };

    // Label matches source filename (stem)
    let stem = Path::new(&node.source_file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !stem.is_empty() && node.label == stem {
        return true;
    }

    // Method stubs: ".method()"
    if node.label.starts_with('.') && node.label.ends_with("()") {
        return true;
    }

    false
}

/// True if node is a manually-injected semantic annotation.
///
/// Signals:
/// - Empty source_file
/// - source_file has no file extension (not a real path)
pub fn is_concept_node(graph: &KodexGraph, node_id: &str) -> bool {
    let node = match graph.get_node(node_id) {
        Some(n) => n,
        None => return false,
    };

    if node.source_file.is_empty() {
        return true;
    }

    // No file extension → not a real file
    let ext = Path::new(&node.source_file).extension();
    if ext.is_none() && !node.source_file.contains('/') && !node.source_file.contains('\\') {
        return true;
    }

    false
}
