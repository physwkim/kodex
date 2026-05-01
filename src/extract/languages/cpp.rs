use crate::extract::config::LanguageConfig;
use crate::extract::languages::c;

pub static CPP_CONFIG: LanguageConfig = LanguageConfig {
    ts_language: || tree_sitter_cpp::LANGUAGE.into(),
    class_types: &["class_specifier", "struct_specifier", "enum_specifier"],
    function_types: &["function_definition"],
    import_types: &["preproc_include"],
    call_types: &["call_expression"],
    name_field: "name",
    body_field: "body",
    call_function_field: "function",
    call_accessor_node_types: &["field_expression", "qualified_identifier"],
    call_accessor_field: "field",
    call_object_field: None,
    function_boundary_types: &["function_definition", "lambda_expression"],
    function_label_parens: true,
    import_handler: c::C_CONFIG.import_handler,
    resolve_function_name: c::C_CONFIG.resolve_function_name,
    extra_walk: None,
};
