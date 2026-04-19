use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_go(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    collect_import_specs(node, source, &mut result, line);
    result
}

#[allow(clippy::only_used_in_recursion)]
fn collect_import_specs(
    node: &Node,
    source: &[u8],
    result: &mut Vec<ImportEdge>,
    default_line: usize,
) {
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        if child.kind() == "import_spec" {
            if let Some(path_node) = child.child_by_field_name("path") {
                let raw = read_text(&path_node, source).trim_matches('"');
                let module_name = raw.rsplit('/').next().unwrap_or(raw);
                if !module_name.is_empty() {
                    let line = child.start_position().row + 1;
                    result.push(ImportEdge {
                        target_id: make_id(&[module_name]),
                        relation: "imports".to_string(),
                        source_location: format!("L{line}"),
                    });
                }
            }
        } else if child.kind() == "import_spec_list" {
            collect_import_specs(&child, source, result, default_line);
        }
    }
}

pub static GO_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_go::LANGUAGE.into(),
    class_types: &["type_declaration"],
    function_types: &["function_declaration", "method_declaration"],
    import_types: &["import_declaration"],
    call_types: &["call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["selector_expression"],
    call_accessor_field: "field",
    function_boundary_types: &["function_declaration", "method_declaration", "func_literal"],
    function_label_parens: true,
    import_handler: Some(import_go),
    resolve_function_name: None,
    extra_walk: None,
};
