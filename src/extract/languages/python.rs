use tree_sitter::Node;

use crate::extract::config::{ImportEdge, LanguageConfig};
use crate::id::make_id;

fn read_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn import_python(
    node: &Node,
    source: &[u8],
    _file_nid: &str,
    _stem: &str,
    _str_path: &str,
) -> Vec<ImportEdge> {
    let mut result = Vec::new();
    let line = node.start_position().row + 1;
    let loc = format!("L{line}");

    match node.kind() {
        "import_statement" => {
            let cursor = &mut node.walk();
            for child in node.children(cursor) {
                if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
                    let raw = read_text(&child, source);
                    let module_name = raw.split(" as ").next().unwrap_or(raw).trim().trim_start_matches('.');
                    if !module_name.is_empty() {
                        result.push(ImportEdge {
                            target_id: make_id(&[module_name]),
                            relation: "imports".to_string(),
                            source_location: loc.clone(),
                        });
                    }
                }
            }
        }
        "import_from_statement" => {
            if let Some(module_node) = node.child_by_field_name("module_name") {
                let raw = read_text(&module_node, source);
                let module_name = raw.trim_start_matches('.');
                if !module_name.is_empty() {
                    result.push(ImportEdge {
                        target_id: make_id(&[module_name]),
                        relation: "imports_from".to_string(),
                        source_location: loc,
                    });
                }
            }
        }
        _ => {}
    }

    result
}

pub static PYTHON_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_python::LANGUAGE.into(),
    class_types: &["class_definition"],
    function_types: &["function_definition"],
    import_types: &["import_statement", "import_from_statement"],
    call_types: &["call"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["attribute"],
    call_accessor_field: "attribute",
    function_boundary_types: &["function_definition"],
    function_label_parens: true,
    import_handler: Some(import_python),
    resolve_function_name: None,
    extra_walk: None,
};
