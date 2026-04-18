use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_c(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    if let Some(path_node) = node.child_by_field_name("path") {
        let raw = read_text(&path_node, source).trim_matches(|c| c == '<' || c == '>' || c == '"');
        if !raw.is_empty() {
            result.push(ImportEdge {
                target_id: make_id(&[raw]),
                relation: "imports".to_string(),
                source_location: format!("L{line}"),
            });
        }
    }
    result
}

/// Unwrap C declarator chains to find the innermost identifier.
fn get_c_func_name(node: &Node, source: &[u8]) -> Option<String> {
    if let Some(declarator) = node.child_by_field_name("declarator") {
        return find_identifier(&declarator, source);
    }
    None
}

fn find_identifier(node: &Node, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(read_text(node, source).to_string());
    }
    // Recurse into declarators
    if let Some(decl) = node.child_by_field_name("declarator") {
        return find_identifier(&decl, source);
    }
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        if child.kind() == "identifier" {
            return Some(read_text(&child, source).to_string());
        }
    }
    None
}

pub static C_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_c::LANGUAGE.into(),
    class_types: &["struct_specifier", "enum_specifier", "union_specifier"],
    function_types: &["function_definition"],
    import_types: &["preproc_include"],
    call_types: &["call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["field_expression"],
    call_accessor_field: "field",
    function_boundary_types: &["function_definition"],
    function_label_parens: true,
    import_handler: Some(import_c),
    resolve_function_name: Some(get_c_func_name),
    extra_walk: None,
};
