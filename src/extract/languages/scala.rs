use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_scala(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    let raw = read_text(node, source);
    let name = raw
        .trim_start_matches("import ")
        .rsplit('.')
        .next()
        .unwrap_or(raw);
    if !name.is_empty() && name != "_" && name != "*" {
        result.push(ImportEdge {
            target_id: make_id(&[name.trim()]),
            relation: "imports".to_string(),
            source_location: format!("L{line}"),
        });
    }
    result
}

pub static SCALA_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_scala::LANGUAGE.into(),
    class_types: &["class_definition", "object_definition", "trait_definition"],
    function_types: &["function_definition"],
    import_types: &["import_declaration"],
    call_types: &["call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["field_expression"],
    call_accessor_field: "field",
    call_object_field: None,
    function_boundary_types: &["function_definition", "lambda_expression"],
    function_label_parens: true,
    import_handler: Some(import_scala),
    resolve_function_name: None,
    extra_walk: None,
};
