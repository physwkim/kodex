#[cfg(feature = "extract")]
use std::collections::{HashMap, HashSet};
#[cfg(feature = "extract")]
use std::path::Path;

#[cfg(feature = "extract")]
use tree_sitter::{Node, Parser};

#[cfg(feature = "extract")]
use crate::id::make_id;
#[cfg(feature = "extract")]
use crate::types::{Confidence, Edge, ExtractionResult, FileType, RawCall};

#[cfg(feature = "extract")]
use super::config::LanguageConfig;

/// Read text content of a tree-sitter node from the source bytes.
#[cfg(feature = "extract")]
fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    let start = node.start_byte();
    let end = node.end_byte();
    std::str::from_utf8(&source[start..end]).unwrap_or("")
}

/// Resolve the name of a node using the config's name_field.
#[cfg(feature = "extract")]
fn resolve_name(node: &Node, source: &[u8], config: &LanguageConfig) -> Option<String> {
    // Try custom resolver first
    if let Some(resolver) = config.resolve_function_name {
        if let Some(name) = resolver(node, source) {
            return Some(name);
        }
    }

    // Standard: child_by_field_name("name")
    if let Some(name_node) = node.child_by_field_name(config.name_field) {
        let text = read_text(&name_node, source).to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }

    None
}

/// Generic AST extractor driven by LanguageConfig.
///
/// This is the core extraction function that handles most languages.
/// It walks the AST and extracts classes, functions, imports, and call edges.
#[cfg(feature = "extract")]
pub fn extract_generic(path: &Path, config: &LanguageConfig) -> ExtractionResult {
    // Read file
    let source = match std::fs::read(path) {
        Ok(s) => s,
        Err(e) => {
            return ExtractionResult {
                error: Some(format!("Failed to read {}: {e}", path.display())),
                ..Default::default()
            };
        }
    };

    // Initialize parser
    let mut parser = Parser::new();
    let language = (config.ts_language)();
    if parser.set_language(&language).is_err() {
        return ExtractionResult {
            error: Some(format!("Failed to set language for {}", path.display())),
            ..Default::default()
        };
    }

    let tree = match parser.parse(&source, None) {
        Some(t) => t,
        None => {
            return ExtractionResult {
                error: Some(format!("Failed to parse {}", path.display())),
                ..Default::default()
            };
        }
    };

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let str_path = path.to_string_lossy().to_string();

    let mut nodes: Vec<crate::types::Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    // Store (caller_nid, body_start_byte, body_end_byte) to avoid lifetime issues
    let mut function_body_ranges: Vec<(String, usize, usize)> = Vec::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();

    // File-level node
    let file_nid = make_id(&[stem]);
    nodes.push(crate::types::Node {
        id: file_nid.clone(),
        label: stem.to_string(),
        file_type: FileType::Code,
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        confidence: Some(Confidence::EXTRACTED),
        confidence_score: Some(1.0),
        community: None,
        norm_label: None,
        degree: None,
        uuid: None,
        fingerprint: None,
        logical_key: None,
    });
    seen_ids.insert(file_nid.clone());

    // Walk AST
    let root_node = tree.root_node();
    walk(
        &root_node,
        &source,
        config,
        stem,
        &file_nid,
        &file_nid,
        &str_path,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &mut function_body_ranges,
    );

    // Call-graph pass: find body nodes by byte range in the tree
    let label_to_nid: HashMap<String, String> = nodes
        .iter()
        .map(|n| {
            let label = n.label.trim_end_matches("()").to_lowercase();
            (label, n.id.clone())
        })
        .collect();

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();

    for (caller_nid, start_byte, end_byte) in &function_body_ranges {
        // Find the node in the tree by byte range
        if let Some(body_node) = root_node.descendant_for_byte_range(*start_byte, *end_byte) {
            walk_calls(
                &body_node,
                &source,
                config,
                caller_nid,
                &label_to_nid,
                &mut edges,
                &mut raw_calls,
                &mut seen_call_pairs,
                &str_path,
            );
        }
    }

    // Deduplicate edges — keep the one with highest confidence
    let mut edge_map: HashMap<(String, String, String), Edge> = HashMap::new();
    for edge in edges {
        let key = (
            edge.source.clone(),
            edge.target.clone(),
            edge.relation.clone(),
        );
        let should_replace = edge_map
            .get(&key)
            .map(|existing| edge.confidence.default_score() > existing.confidence.default_score())
            .unwrap_or(true);
        if should_replace {
            edge_map.insert(key, edge);
        }
    }
    let clean_edges: Vec<Edge> = edge_map.into_values().collect();

    ExtractionResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        ..Default::default()
    }
}

/// Recursively walk AST to extract classes, functions, and imports.
#[cfg(feature = "extract")]
#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Node,
    source: &[u8],
    config: &LanguageConfig,
    stem: &str,
    file_nid: &str,
    container_nid: &str,
    str_path: &str,
    nodes: &mut Vec<crate::types::Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    function_body_ranges: &mut Vec<(String, usize, usize)>,
) {
    let node_type = node.kind();

    // Extra walk hook (JS arrow functions, C# namespaces, Swift enums)
    if let Some(extra_walk) = config.extra_walk {
        if extra_walk(
            node,
            source,
            stem,
            file_nid,
            str_path,
            nodes,
            edges,
            seen_ids,
            container_nid,
        ) {
            return; // Handled by extra_walk
        }
    }

    // --- Imports ---
    if config.import_types.contains(&node_type) {
        if let Some(handler) = config.import_handler {
            let import_edges = handler(node, source, file_nid, stem, str_path);
            for ie in import_edges {
                edges.push(Edge {
                    source: file_nid.to_string(),
                    target: ie.target_id,
                    relation: ie.relation,
                    confidence: Confidence::EXTRACTED,
                    source_file: str_path.to_string(),
                    source_location: Some(ie.source_location),
                    confidence_score: Some(1.0),
                    weight: 1.0,
                    original_src: None,
                    original_tgt: None,
                });
            }
        }
        return; // Don't recurse into import nodes
    }

    // --- Classes ---
    if config.class_types.contains(&node_type) {
        if let Some(class_name) = resolve_name(node, source, config) {
            let class_nid = make_id(&[stem, &class_name]);
            let line = node.start_position().row + 1;

            if seen_ids.insert(class_nid.clone()) {
                nodes.push(crate::types::Node {
                    id: class_nid.clone(),
                    label: class_name.clone(),
                    file_type: FileType::Code,
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    confidence: Some(Confidence::EXTRACTED),
                    confidence_score: Some(1.0),
                    community: None,
                    norm_label: None,
                    degree: None,
                    uuid: None,
                    fingerprint: None,
                    logical_key: None,
                });
            }

            edges.push(Edge {
                source: container_nid.to_string(),
                target: class_nid.clone(),
                relation: "contains".to_string(),
                confidence: Confidence::EXTRACTED,
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            });

            // Check for inheritance (superclass_types)
            extract_inheritance(node, source, stem, &class_nid, str_path, edges);

            // Recurse into class body with class as container
            let cursor = &mut node.walk();
            for child in node.children(cursor) {
                walk(
                    &child,
                    source,
                    config,
                    stem,
                    file_nid,
                    &class_nid,
                    str_path,
                    nodes,
                    edges,
                    seen_ids,
                    function_body_ranges,
                );
            }
            return;
        }
    }

    // --- Functions ---
    if config.function_types.contains(&node_type) {
        if let Some(func_name) = resolve_name(node, source, config) {
            let func_nid = make_id(&[stem, &func_name]);
            let line = node.start_position().row + 1;
            let label = if config.function_label_parens {
                format!("{func_name}()")
            } else {
                func_name.clone()
            };

            // Determine relation: method if inside a class, contains otherwise
            let relation = if container_nid != file_nid {
                "method"
            } else {
                "contains"
            };

            if seen_ids.insert(func_nid.clone()) {
                nodes.push(crate::types::Node {
                    id: func_nid.clone(),
                    label,
                    file_type: FileType::Code,
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    confidence: Some(Confidence::EXTRACTED),
                    confidence_score: Some(1.0),
                    community: None,
                    norm_label: None,
                    degree: None,
                    uuid: None,
                    fingerprint: None,
                    logical_key: None,
                });
            }

            edges.push(Edge {
                source: container_nid.to_string(),
                target: func_nid.clone(),
                relation: relation.to_string(),
                confidence: Confidence::EXTRACTED,
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            });

            // Save body byte range for call-graph pass
            if let Some(body) = node.child_by_field_name(config.body_field) {
                function_body_ranges.push((func_nid, body.start_byte(), body.end_byte()));
            }
            return; // Don't recurse into function body here
        }
    }

    // Recurse into children
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        walk(
            &child,
            source,
            config,
            stem,
            file_nid,
            container_nid,
            str_path,
            nodes,
            edges,
            seen_ids,
            function_body_ranges,
        );
    }
}

/// Extract inheritance edges from a class node (e.g., `class Foo(Bar):`)
#[cfg(feature = "extract")]
fn extract_inheritance(
    node: &Node,
    source: &[u8],
    stem: &str,
    class_nid: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    // Look for superclass list: argument_list, superclass, type_list, etc.
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        let kind = child.kind();
        if kind == "argument_list"
            || kind == "superclass"
            || kind == "type_list"
            || kind == "superclasses"
            || kind == "base_list"
        {
            let inner_cursor = &mut child.walk();
            for arg in child.children(inner_cursor) {
                let text = read_text(&arg, source).trim().to_string();
                if !text.is_empty() && text != "(" && text != ")" && text != "," && text != ":" {
                    let base_name = text.split('<').next().unwrap_or(&text).trim();
                    if !base_name.is_empty() {
                        let base_nid = make_id(&[stem, base_name]);
                        let line = child.start_position().row + 1;
                        edges.push(Edge {
                            source: class_nid.to_string(),
                            target: base_nid,
                            relation: "extends".to_string(),
                            confidence: Confidence::EXTRACTED,
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            confidence_score: Some(1.0),
                            weight: 1.0,
                            original_src: None,
                            original_tgt: None,
                        });
                    }
                }
            }
        }
    }
}

/// Walk function bodies to extract call edges.
#[cfg(feature = "extract")]
#[allow(clippy::too_many_arguments)]
fn walk_calls(
    node: &Node,
    source: &[u8],
    config: &LanguageConfig,
    caller_nid: &str,
    label_to_nid: &HashMap<String, String>,
    edges: &mut Vec<Edge>,
    raw_calls: &mut Vec<RawCall>,
    seen_call_pairs: &mut HashSet<(String, String)>,
    str_path: &str,
) {
    // Stop at function boundaries (don't descend into nested functions)
    if config.function_boundary_types.contains(&node.kind()) {
        return;
    }

    if config.call_types.contains(&node.kind()) {
        let callee_name = extract_callee_name(node, source, config);

        if let Some(name) = callee_name {
            let name_lower = name.to_lowercase();
            if let Some(tgt_nid) = label_to_nid.get(&name_lower) {
                if tgt_nid != caller_nid {
                    let pair = (caller_nid.to_string(), tgt_nid.clone());
                    if seen_call_pairs.insert(pair) {
                        let line = node.start_position().row + 1;
                        edges.push(Edge {
                            source: caller_nid.to_string(),
                            target: tgt_nid.clone(),
                            relation: "calls".to_string(),
                            confidence: Confidence::EXTRACTED,
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            confidence_score: Some(1.0),
                            weight: 1.0,
                            original_src: None,
                            original_tgt: None,
                        });
                    }
                }
            } else {
                // Save for cross-file resolution
                raw_calls.push(RawCall {
                    caller_nid: caller_nid.to_string(),
                    callee: name,
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{}", node.start_position().row + 1)),
                });
            }
        }
    }

    // Recurse
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        walk_calls(
            &child,
            source,
            config,
            caller_nid,
            label_to_nid,
            edges,
            raw_calls,
            seen_call_pairs,
            str_path,
        );
    }
}

/// Extract the callee name from a call expression node.
#[cfg(feature = "extract")]
fn extract_callee_name(node: &Node, source: &[u8], config: &LanguageConfig) -> Option<String> {
    let func_node = node.child_by_field_name(config.call_function_field)?;
    let func_type = func_node.kind();

    // Check if it's a member/attribute access (e.g., obj.method())
    if config.call_accessor_node_types.contains(&func_type) {
        // Get the method name from the accessor
        if let Some(attr_node) = func_node.child_by_field_name(config.call_accessor_field) {
            return Some(read_text(&attr_node, source).to_string());
        }
    }

    // Simple function call (e.g., foo())
    Some(read_text(&func_node, source).to_string())
}
