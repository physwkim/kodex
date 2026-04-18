use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_js(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    let loc = format!("L{line}");

    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        if child.kind() == "string" || child.kind() == "string_fragment" {
            let raw = read_text(&child, source).trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ' ');
            if raw.is_empty() {
                break;
            }
            let target_id = if raw.starts_with('.') {
                // Relative import
                make_id(&[raw.trim_start_matches("./").trim_end_matches(".js").trim_end_matches(".ts")])
            } else {
                // Bare/scoped import - use last segment
                let module_name = raw.rsplit('/').next().unwrap_or(raw);
                if module_name.is_empty() {
                    break;
                }
                make_id(&[module_name])
            };
            result.push(ImportEdge {
                target_id,
                relation: "imports_from".to_string(),
                source_location: loc,
            });
            break; // Only first string
        }
    }

    result
}

pub static JS_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_javascript::LANGUAGE.into(),
    class_types: &["class_declaration"],
    function_types: &["function_declaration", "method_definition"],
    import_types: &["import_statement"],
    call_types: &["call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["member_expression"],
    call_accessor_field: "property",
    function_boundary_types: &["function_declaration", "arrow_function", "method_definition"],
    function_label_parens: true,
    import_handler: Some(import_js),
    resolve_function_name: None,
    extra_walk: None,
};
