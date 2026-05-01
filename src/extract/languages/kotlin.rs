use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_kotlin(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        if child.kind() == "identifier" || child.kind() == "user_type" {
            let raw = read_text(&child, source);
            let name = raw.rsplit('.').next().unwrap_or(raw);
            if !name.is_empty() && name != "*" {
                result.push(ImportEdge {
                    target_id: make_id(&[name]),
                    relation: "imports".to_string(),
                    source_location: format!("L{line}"),
                });
            }
            break;
        }
    }
    result
}

pub static KOTLIN_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_kotlin::language().into(),
    class_types: &[
        "class_declaration",
        "object_declaration",
        "interface_declaration",
    ],
    function_types: &["function_declaration"],
    import_types: &["import_header"],
    call_types: &["call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["navigation_expression"],
    call_accessor_field: "navigation_suffix",
    call_object_field: None,
    function_boundary_types: &["function_declaration", "lambda_literal"],
    function_label_parens: true,
    import_handler: Some(import_kotlin),
    resolve_function_name: None,
    extra_walk: None,
};
