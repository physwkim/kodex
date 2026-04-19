use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_lua(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    // Lua uses require("module") - check if this is a require call
    let raw = read_text(node, source);
    if raw.contains("require") {
        // Extract the module name from require("foo") or require 'foo'
        if let Some(start) = raw.find(['\'', '"']) {
            let rest = &raw[start + 1..];
            if let Some(end) = rest.find(['\'', '"']) {
                let module = &rest[..end];
                if !module.is_empty() {
                    result.push(ImportEdge {
                        target_id: make_id(&[module.rsplit('.').next().unwrap_or(module)]),
                        relation: "imports".to_string(),
                        source_location: format!("L{line}"),
                    });
                }
            }
        }
    }
    result
}

pub static LUA_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_lua::LANGUAGE.into(),
    class_types: &[],
    function_types: &["function_declaration", "function_definition"],
    import_types: &["function_call"], // require() calls
    call_types: &["function_call"],
    name_field: "name",
    body_field: "body",
    call_function_field: "name",
    call_accessor_node_types: &["method_index_expression"],
    call_accessor_field: "method",
    function_boundary_types: &["function_declaration", "function_definition"],
    function_label_parens: true,
    import_handler: Some(import_lua),
    resolve_function_name: None,
    extra_walk: None,
};
