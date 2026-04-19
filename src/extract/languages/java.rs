use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_java(
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
        if child.kind() == "scoped_identifier" || child.kind() == "identifier" {
            let raw = read_text(&child, source);
            let class_name = raw.rsplit('.').next().unwrap_or(raw);
            if !class_name.is_empty() && class_name != "*" {
                result.push(ImportEdge {
                    target_id: make_id(&[class_name]),
                    relation: "imports".to_string(),
                    source_location: format!("L{line}"),
                });
            }
            break;
        }
    }

    result
}

pub static JAVA_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_java::LANGUAGE.into(),
    class_types: &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
    ],
    function_types: &["method_declaration", "constructor_declaration"],
    import_types: &["import_declaration"],
    call_types: &["method_invocation"],
    name_field: "name",
    body_field: "body",
    call_function_field: "name",
    call_accessor_node_types: &["method_invocation"],
    call_accessor_field: "name",
    function_boundary_types: &[
        "method_declaration",
        "constructor_declaration",
        "lambda_expression",
    ],
    function_label_parens: true,
    import_handler: Some(import_java),
    resolve_function_name: None,
    extra_walk: None,
};
