use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Resolve names that the default `child_by_field_name("name")` lookup misses.
/// Specifically, `impl_item` has no `name` field — its identity is the type
/// being implemented. Returning that type as the name causes the generic
/// extractor to treat each `impl Foo for Bar { ... }` block as a class node
/// with `Bar` as its label, so all methods across all impl blocks for `Bar`
/// become siblings under a single node (the impls' make_id collides on
/// "Bar", which is the desired merge).
///
/// Strips generic parameters (`Bar<T>` → `Bar`) since the user-facing query
/// is by base name.
fn resolve_name_rust(node: &Node, source: &[u8]) -> Option<String> {
    if node.kind() != "impl_item" {
        return None;
    }
    let type_node = node.child_by_field_name("type")?;
    let raw = read_text(&type_node, source).trim();
    if raw.is_empty() {
        return None;
    }
    // Drop everything from the first generic angle bracket onward.
    let base = raw.split('<').next().unwrap_or(raw).trim();
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

fn import_rust(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;

    // Extract the path from `use std::collections::HashMap;`
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        if child.kind() == "use_wildcard"
            || child.kind() == "use_list"
            || child.kind() == "scoped_identifier"
            || child.kind() == "identifier"
            || child.kind() == "scoped_use_list"
        {
            let raw = read_text(&child, source);
            // Take the last path segment
            let module_name = raw.rsplit("::").next().unwrap_or(raw).trim();
            if !module_name.is_empty() && module_name != "*" && module_name != "{" {
                result.push(ImportEdge {
                    target_id: make_id(&[module_name]),
                    relation: "imports".to_string(),
                    source_location: format!("L{line}"),
                });
            }
            break;
        }
    }

    result
}

pub static RUST_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_rust::LANGUAGE.into(),
    class_types: &["struct_item", "enum_item", "trait_item", "impl_item"],
    function_types: &["function_item"],
    import_types: &["use_declaration"],
    call_types: &["call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["field_expression"],
    call_accessor_field: "field",
    call_object_field: None,
    function_boundary_types: &["function_item", "closure_expression"],
    function_label_parens: true,
    import_handler: Some(import_rust),
    resolve_function_name: Some(resolve_name_rust),
    extra_walk: None,
};
