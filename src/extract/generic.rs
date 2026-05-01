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

/// Normalize body text for hashing: strip comments, whitespace, formatting.
/// Goal: identical structure produces same hash even after reformatting.
#[cfg(feature = "extract")]
fn normalize_body_for_hash(body: &str) -> String {
    let mut result = String::with_capacity(body.len());
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut prev_char = '\0';

    for ch in body.chars() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_block_comment {
            if prev_char == '*' && ch == '/' {
                in_block_comment = false;
                prev_char = '\0';
                continue;
            }
            prev_char = ch;
            continue;
        }

        if ch == '/' && prev_char == '/' {
            in_line_comment = true;
            result.pop(); // remove the first '/'
            prev_char = ch;
            continue;
        }
        if ch == '*' && prev_char == '/' {
            in_block_comment = true;
            result.pop(); // remove the first '/'
            prev_char = ch;
            continue;
        }
        // Also handle Python # comments
        if ch == '#' {
            in_line_comment = true;
            continue;
        }

        prev_char = ch;

        // Strip whitespace
        if ch.is_whitespace() {
            continue;
        }
        result.push(ch);
    }
    result
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
        body_hash: None,
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
        None,
        &str_path,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &mut function_body_ranges,
    );

    // Call-graph pass: build per-file lookup tables.
    // `label_to_nids` is a multi-map so two methods sharing a label (e.g.
    // `Database.query` and `HttpClient.query` in the same file) keep
    // distinct entries — receiver disambiguation picks the right one.
    let mut label_to_nids: HashMap<String, Vec<String>> = HashMap::new();
    for n in &nodes {
        let label = n.label.trim_end_matches("()").to_lowercase();
        if !label.is_empty() {
            label_to_nids.entry(label).or_default().push(n.id.clone());
        }
    }
    let nid_to_label_local: HashMap<String, String> = nodes
        .iter()
        .map(|n| (n.id.clone(), n.label.trim_end_matches("()").to_lowercase()))
        .collect();
    let mut method_to_class_local: HashMap<String, String> = HashMap::new();
    for edge in &edges {
        if edge.relation == "method" {
            if let Some(class_label) = nid_to_label_local.get(&edge.source) {
                method_to_class_local.insert(edge.target.clone(), class_label.clone());
            }
        }
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();

    for (caller_nid, start_byte, end_byte) in &function_body_ranges {
        // Find the node in the tree by byte range
        if let Some(body_node) = root_node.descendant_for_byte_range(*start_byte, *end_byte) {
            walk_calls(
                &body_node,
                &source,
                config,
                caller_nid,
                &label_to_nids,
                &method_to_class_local,
                &mut edges,
                &mut raw_calls,
                &mut seen_call_pairs,
                &str_path,
            );
        }
    }

    // Compute body hashes for functions/classes
    {
        use sha2::{Digest, Sha256};
        // Collect (index, hash) pairs to avoid borrow conflicts
        let nid_to_idx: HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        let hashes: Vec<(usize, String)> = function_body_ranges
            .iter()
            .filter_map(|(caller_nid, start_byte, end_byte)| {
                let idx = *nid_to_idx.get(caller_nid)?;
                if *start_byte < source.len() && *end_byte <= source.len() {
                    let body_text =
                        std::str::from_utf8(&source[*start_byte..*end_byte]).unwrap_or("");
                    let normalized = normalize_body_for_hash(body_text);
                    if normalized.is_empty() {
                        return None; // skip empty/whitespace-only bodies
                    }
                    let mut hasher = Sha256::new();
                    hasher.update(normalized.as_bytes());
                    let hash = format!("{:x}", hasher.finalize());
                    Some((idx, hash[..16].to_string()))
                } else {
                    None
                }
            })
            .collect();
        for (idx, hash) in hashes {
            nodes[idx].body_hash = Some(hash);
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
// `container_label` is the label of the enclosing class (for scoping method
// node IDs): `None` at file scope; `Some(class_name)` inside an `impl` /
// `class` body. Two methods sharing a name in the same file but in different
// classes get distinct IDs because of the class-name segment.
#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Node,
    source: &[u8],
    config: &LanguageConfig,
    stem: &str,
    file_nid: &str,
    container_nid: &str,
    container_label: Option<&str>,
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
                    body_hash: None,
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

            // Record class body range for body_hash (same as functions)
            if let Some(body) = node.child_by_field_name(config.body_field) {
                function_body_ranges.push((class_nid.clone(), body.start_byte(), body.end_byte()));
            } else {
                // Fallback: use entire class node range
                function_body_ranges.push((class_nid.clone(), node.start_byte(), node.end_byte()));
            }

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
                    Some(&class_name),
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
            // Methods get class-scoped IDs so two same-name methods in the
            // same file don't collide on `make_id([stem, name])`. Top-level
            // functions keep the bare `[stem, name]` form (unchanged ID).
            let func_nid = match container_label {
                Some(cls) => make_id(&[stem, cls, &func_name]),
                None => make_id(&[stem, &func_name]),
            };
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
                    body_hash: None,
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
            container_label,
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
                            // Store the literal base name so cross-file
                            // inheritance can resolve when the target nid
                            // (built from caller's stem) doesn't point at a
                            // real node — the base class is in another file.
                            original_tgt: Some(base_name.to_string()),
                        });
                    }
                }
            }
        }
    }
}

/// Walk function bodies to extract call edges. Handles in-file resolution
/// with the same receiver-aware disambiguation as the cross-file resolver:
/// when a callee name has multiple in-file candidates, `self.method()` picks
/// the caller's class and `Type::method()` picks the matching class.
/// Anything else (variable receiver, `super`, no candidates) falls through
/// to `raw_calls` for the cross-file pass.
#[cfg(feature = "extract")]
#[allow(clippy::too_many_arguments)]
fn walk_calls(
    node: &Node,
    source: &[u8],
    config: &LanguageConfig,
    caller_nid: &str,
    label_to_nids: &HashMap<String, Vec<String>>,
    method_to_class_local: &HashMap<String, String>,
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
        if let Some(target) = extract_call_target(node, source, config) {
            let name_lower = target.callee.to_lowercase();
            let is_super = target
                .receiver
                .as_deref()
                .map(crate::extract::is_super_ref)
                .unwrap_or(false);

            // Pick an in-file target if we can do so unambiguously.
            // `super.method()` always falls through to raw_calls (cross-file
            // resolver walks the inheritance chain).
            let in_file_hit: Option<&String> = if is_super {
                None
            } else if let Some(candidates) = label_to_nids.get(&name_lower) {
                if candidates.len() == 1 {
                    Some(&candidates[0])
                } else if target.receiver_is_self {
                    method_to_class_local
                        .get(caller_nid)
                        .and_then(|caller_class| {
                            candidates.iter().find(|nid| {
                                method_to_class_local.get(*nid) == Some(caller_class)
                            })
                        })
                } else if let Some(recv) = target.receiver.as_deref() {
                    let recv_lower = recv.to_lowercase();
                    candidates.iter().find(|nid| {
                        method_to_class_local
                            .get(*nid)
                            .map(|c| c == &recv_lower)
                            .unwrap_or(false)
                    })
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(tgt_nid) = in_file_hit {
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
                // Save for cross-file resolution (or super fallthrough, or
                // an in-file ambiguity that the cross-file pass might solve
                // via inheritance traversal across the project).
                raw_calls.push(RawCall {
                    caller_nid: caller_nid.to_string(),
                    callee: target.callee,
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{}", node.start_position().row + 1)),
                    receiver: target.receiver,
                    receiver_is_self: target.receiver_is_self,
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
            label_to_nids,
            method_to_class_local,
            edges,
            raw_calls,
            seen_call_pairs,
            str_path,
        );
    }
}

/// Result of resolving a call site: callee name + optional receiver.
#[cfg(feature = "extract")]
pub(crate) struct CallTarget {
    pub callee: String,
    pub receiver: Option<String>,
    pub receiver_is_self: bool,
}

/// Extract the callee name + receiver from a call expression node.
///
/// Three cases handled:
/// 1. **Call node IS the accessor** (Java `method_invocation`, Ruby `call`,
///    PHP `member_call_expression`). Receiver lives at `config.call_object_field`
///    on the call node itself.
/// 2. **Standard accessor child** (Rust `field_expression`, Go
///    `selector_expression`, JS `member_expression`, Python `attribute`, …).
///    The call's `function` field is the accessor; receiver is the first named
///    child of the accessor that isn't the method name.
/// 3. **Bare or path-style** (`foo()`, `Type::method()`, `Module::function`).
///    Plain text is split on the last `::` to recover Rust/Ruby path receivers.
#[cfg(feature = "extract")]
fn extract_call_target(node: &Node, source: &[u8], config: &LanguageConfig) -> Option<CallTarget> {
    // Case 1: call node itself is in accessor types (Java/Ruby/PHP-member).
    if config.call_accessor_node_types.contains(&node.kind()) {
        let method_node = node.child_by_field_name(config.call_accessor_field)?;
        let callee = read_text(&method_node, source).to_string();
        let receiver = config
            .call_object_field
            .and_then(|f| node.child_by_field_name(f))
            .map(|n| read_text(&n, source).to_string());
        let receiver_is_self = receiver.as_deref().map(is_self_ref).unwrap_or(false);
        return Some(CallTarget {
            callee,
            receiver,
            receiver_is_self,
        });
    }

    let func_node = node.child_by_field_name(config.call_function_field)?;
    let func_type = func_node.kind();

    // Case 2: function is an accessor (member/attribute access).
    if config.call_accessor_node_types.contains(&func_type) {
        if let Some(method_node) = func_node.child_by_field_name(config.call_accessor_field) {
            let callee = read_text(&method_node, source).to_string();
            let receiver = first_other_named_child(&func_node, &method_node, source);
            let receiver_is_self = receiver.as_deref().map(is_self_ref).unwrap_or(false);
            return Some(CallTarget {
                callee,
                receiver,
                receiver_is_self,
            });
        }
    }

    // Case 3: bare call or path-style (Type::method, Module::function).
    let text = read_text(&func_node, source);
    if let Some((recv, method)) = text.rsplit_once("::") {
        let recv = recv.trim();
        let method = method.trim();
        if !recv.is_empty() && !method.is_empty() {
            return Some(CallTarget {
                callee: method.to_string(),
                receiver: Some(recv.to_string()),
                receiver_is_self: is_self_ref(recv),
            });
        }
    }

    Some(CallTarget {
        callee: text.to_string(),
        receiver: None,
        receiver_is_self: false,
    })
}

/// Return the first named child of `parent` that isn't `skip` — used to read
/// the receiver text from a member/selector expression.
///
/// Uses `Node::id()` for the skip comparison; tree-sitter guarantees this is
/// unique within a tree, so it's robust even when the method name shares its
/// span with a wrapping node (rare, but possible with grammars that emit
/// extras).
#[cfg(feature = "extract")]
fn first_other_named_child(parent: &Node, skip: &Node, source: &[u8]) -> Option<String> {
    let skip_id = skip.id();
    let cursor = &mut parent.walk();
    for child in parent.children(cursor) {
        if !child.is_named() {
            continue;
        }
        if child.id() == skip_id {
            continue;
        }
        return Some(read_text(&child, source).to_string());
    }
    None
}

#[cfg(feature = "extract")]
fn is_self_ref(s: &str) -> bool {
    matches!(s.trim(), "self" | "this" | "Self")
}
