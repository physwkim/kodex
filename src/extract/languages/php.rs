use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_php(
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
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let raw = read_text(&child, source);
            let name = raw.rsplit('\\').next().unwrap_or(raw);
            if !name.is_empty() {
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

pub static PHP_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_php::LANGUAGE_PHP.into(),
    class_types: &[
        "class_declaration",
        "interface_declaration",
        "trait_declaration",
    ],
    function_types: &["function_definition", "method_declaration"],
    import_types: &["namespace_use_declaration"],
    call_types: &["function_call_expression", "member_call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["member_call_expression"],
    call_accessor_field: "name",
    call_object_field: Some("object"),
    function_boundary_types: &[
        "function_definition",
        "method_declaration",
        "anonymous_function_creation_expression",
    ],
    function_label_parens: true,
    import_handler: Some(import_php),
    resolve_function_name: None,
    extra_walk: None,
};
