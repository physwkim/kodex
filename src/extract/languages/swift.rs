use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_swift(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    let raw = read_text(node, source);
    let name = raw.trim_start_matches("import ").trim();
    if !name.is_empty() {
        result.push(ImportEdge {
            target_id: make_id(&[name.rsplit('.').next().unwrap_or(name)]),
            relation: "imports".to_string(),
            source_location: format!("L{line}"),
        });
    }
    result
}

pub static SWIFT_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_swift::LANGUAGE.into(),
    class_types: &[
        "class_declaration",
        "struct_declaration",
        "protocol_declaration",
        "enum_declaration",
    ],
    function_types: &["function_declaration"],
    import_types: &["import_declaration"],
    call_types: &["call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["member_expression"],
    call_accessor_field: "member",
    call_object_field: None,
    function_boundary_types: &["function_declaration", "closure_expression"],
    function_label_parens: true,
    import_handler: Some(import_swift),
    resolve_function_name: None,
    extra_walk: None,
};
