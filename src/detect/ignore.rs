use std::fs;
use std::path::Path;

/// Load `.graphifyignore` patterns by walking up from `root` to find the file.
/// Returns a list of glob patterns (like .gitignore syntax).
pub fn load_graphifyignore(root: &Path) -> Vec<String> {
    let mut patterns = Vec::new();

    // Check root and parent directories
    let mut dir = root.to_path_buf();
    loop {
        let ignore_file = dir.join(".graphifyignore");
        if ignore_file.is_file() {
            if let Ok(content) = fs::read_to_string(&ignore_file) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        patterns.push(trimmed.to_string());
                    }
                }
            }
            break; // Use only the nearest .graphifyignore
        }
        if !dir.pop() {
            break;
        }
    }

    patterns
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_no_ignorefile() {
        let dir = TempDir::new().unwrap();
        let patterns = load_graphifyignore(dir.path());
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_load_patterns() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".graphifyignore"),
            "# comment\nvendor/\n*.generated.py\n",
        )
        .unwrap();
        let patterns = load_graphifyignore(dir.path());
        assert_eq!(patterns, vec!["vendor/", "*.generated.py"]);
    }
}
