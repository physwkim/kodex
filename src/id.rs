use regex::Regex;
use std::sync::LazyLock;

static NON_ALNUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9]+").expect("invalid regex"));

/// Build a stable node ID from one or more name parts.
///
/// Mirrors Python `extract._make_id`: joins parts with `_`, replaces
/// non-alphanumeric runs with `_`, strips leading/trailing `_`, lowercases.
pub fn make_id(parts: &[&str]) -> String {
    let combined: String = parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.trim_matches(|c| c == '_' || c == '.'))
        .collect::<Vec<_>>()
        .join("_");

    let cleaned = NON_ALNUM.replace_all(&combined, "_");
    cleaned.trim_matches('_').to_lowercase()
}

/// Normalize an ID string the same way `make_id` does.
///
/// Used to reconcile edge endpoints when the LLM generates IDs with slightly
/// different punctuation or casing than the AST extractor.
pub fn normalize_id(s: &str) -> String {
    let cleaned = NON_ALNUM.replace_all(s, "_");
    cleaned.trim_matches('_').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_id_basic() {
        assert_eq!(make_id(&["module", "ClassName"]), "module_classname");
    }

    #[test]
    fn test_make_id_strips_dots_underscores() {
        assert_eq!(make_id(&["__init__", ".module"]), "init_module");
    }

    #[test]
    fn test_make_id_special_chars() {
        assert_eq!(make_id(&["my-module", "Foo::Bar"]), "my_module_foo_bar");
    }

    #[test]
    fn test_make_id_empty_parts() {
        assert_eq!(make_id(&["", "name", ""]), "name");
    }

    #[test]
    fn test_normalize_id() {
        assert_eq!(
            normalize_id("Session_ValidateToken"),
            "session_validatetoken"
        );
        assert_eq!(normalize_id("foo--bar"), "foo_bar");
    }
}
