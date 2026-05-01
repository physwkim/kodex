use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_ruby(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    // `require 'foo'` or `require_relative 'foo'`
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        if child.kind() == "string" || child.kind() == "string_content" {
            let raw = read_text(&child, source).trim_matches(|c| c == '\'' || c == '"');
            if !raw.is_empty() {
                result.push(ImportEdge {
                    target_id: make_id(&[raw.rsplit('/').next().unwrap_or(raw)]),
                    relation: "imports".to_string(),
                    source_location: format!("L{line}"),
                });
            }
            break;
        }
    }
    result
}

pub static RUBY_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_ruby::LANGUAGE.into(),
    class_types: &["class", "module"],
    function_types: &["method", "singleton_method"],
    import_types: &["call"], // require/require_relative are call nodes
    call_types: &["call"],
    name_field: "name",
    body_field: "body",
    call_function_field: "method",
    call_accessor_node_types: &["call"],
    call_accessor_field: "method",
    call_object_field: Some("receiver"),
    function_boundary_types: &["method", "singleton_method", "block", "lambda"],
    function_label_parens: true,
    import_handler: Some(import_ruby),
    resolve_function_name: None,
    extra_walk: None,
};
