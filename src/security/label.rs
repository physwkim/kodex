use regex::Regex;
use std::sync::LazyLock;

const MAX_LABEL_LEN: usize = 256;

static CONTROL_CHAR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\x00-\x1f\x7f]").expect("invalid regex for control chars"));

/// Strip control characters and cap length (by characters, not bytes).
///
/// Safe for embedding in JSON data and plain text.
/// For direct HTML injection, wrap the result with HTML escaping.
pub fn sanitize_label(text: &str) -> String {
    let cleaned = CONTROL_CHAR_RE.replace_all(text, "");
    if cleaned.chars().count() > MAX_LABEL_LEN {
        cleaned.chars().take(MAX_LABEL_LEN).collect()
    } else {
        cleaned.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strips_control_chars() {
        assert_eq!(sanitize_label("hello\x00world"), "helloworld");
        assert_eq!(sanitize_label("line\n\ttab"), "linetab");
    }

    #[test]
    fn test_caps_length() {
        let long = "a".repeat(300);
        let result = sanitize_label(&long);
        assert_eq!(result.chars().count(), MAX_LABEL_LEN);
    }

    #[test]
    fn test_normal_text() {
        assert_eq!(sanitize_label("Hello World"), "Hello World");
    }

    #[test]
    fn test_multibyte_truncation_no_panic() {
        // 255 ASCII chars + a 4-byte emoji: should truncate without panic
        let mut input = "a".repeat(255);
        input.push('\u{1F600}'); // 😀
        let result = sanitize_label(&input);
        assert_eq!(result.chars().count(), MAX_LABEL_LEN);
    }
}
