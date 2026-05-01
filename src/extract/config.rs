#[cfg(feature = "extract")]
use tree_sitter::{Language, Node};

/// Import edge produced by a language-specific import handler.
pub struct ImportEdge {
    pub target_id: String,
    pub relation: String,
    pub source_location: String,
}

/// Function pointer types for language-specific handlers.
#[cfg(feature = "extract")]
pub type ImportHandler =
    fn(node: &Node, source: &[u8], file_nid: &str, stem: &str, str_path: &str) -> Vec<ImportEdge>;

#[cfg(feature = "extract")]
pub type FunctionNameResolver = fn(node: &Node, source: &[u8]) -> Option<String>;

#[cfg(feature = "extract")]
pub type ExtraWalkFn = fn(
    node: &Node,
    source: &[u8],
    stem: &str,
    file_nid: &str,
    str_path: &str,
    nodes: &mut Vec<crate::types::Node>,
    edges: &mut Vec<crate::types::Edge>,
    seen_ids: &mut std::collections::HashSet<String>,
    container_nid: &str,
) -> bool;

/// Configuration for a language's AST extraction.
///
/// Each supported language provides one of these, specifying which tree-sitter
/// node types correspond to classes, functions, imports, calls, etc.
#[cfg(feature = "extract")]
pub struct LanguageConfig {
    /// Function that returns the tree-sitter Language object.
    pub ts_language: fn() -> Language,

    /// Node types for class/struct/interface declarations.
    pub class_types: &'static [&'static str],

    /// Node types for function/method declarations.
    pub function_types: &'static [&'static str],

    /// Node types for import/include statements.
    pub import_types: &'static [&'static str],

    /// Node types for function/method call expressions.
    pub call_types: &'static [&'static str],

    /// Field name to extract the name of a class/function.
    pub name_field: &'static str,

    /// Field name for the body of a function/class.
    pub body_field: &'static str,

    /// Field name on a call node for the callee.
    pub call_function_field: &'static str,

    /// Node types that represent member/attribute access (e.g., `member_expression`).
    pub call_accessor_node_types: &'static [&'static str],

    /// Field on accessor node for the method/attribute name.
    pub call_accessor_field: &'static str,

    /// Field on the *call node* (not the accessor) that holds the receiver
    /// expression. Set when a language's call type carries `object` /
    /// `receiver` directly (e.g. Java `method_invocation.object`,
    /// Python `call.function` is itself an attribute, etc.). For most
    /// languages this is `None` — receiver is the first named child of the
    /// accessor.
    pub call_object_field: Option<&'static str>,

    /// Stop recursion at these node types during call-graph walk.
    pub function_boundary_types: &'static [&'static str],

    /// If true, function labels get "()" appended (e.g., "foo()").
    pub function_label_parens: bool,

    /// Language-specific import handler.
    pub import_handler: Option<ImportHandler>,

    /// Custom function name resolver (e.g., C declarator unwrapping).
    pub resolve_function_name: Option<FunctionNameResolver>,

    /// Extra walk hook for language-specific nodes.
    pub extra_walk: Option<ExtraWalkFn>,
}
