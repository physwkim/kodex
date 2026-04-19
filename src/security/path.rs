use std::path::{Path, PathBuf};

/// Resolve a path and verify it stays inside the base directory.
///
/// Base defaults to the `engram-out` directory relative to CWD.
/// Raises error if path escapes base, or base does not exist.
pub fn validate_graph_path(
    path: &str,
    base: Option<&Path>,
) -> crate::error::Result<PathBuf> {
    let base = match base {
        Some(b) => b.to_path_buf(),
        None => {
            // Try to find engram-out in the resolved path's ancestors
            let resolved = PathBuf::from(path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(path));
            let mut found = None;
            for ancestor in resolved.ancestors() {
                if ancestor.file_name().map(|n| n == "engram-out").unwrap_or(false) {
                    found = Some(ancestor.to_path_buf());
                    break;
                }
            }
            found.unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join("engram-out")
            })
        }
    };

    let base = base
        .canonicalize()
        .map_err(|_| crate::error::EngramError::PathEscape(format!(
            "Graph base directory does not exist: {}. Run engram first to build the graph.",
            base.display()
        )))?;

    let resolved = PathBuf::from(path)
        .canonicalize()
        .map_err(|_| crate::error::EngramError::FileNotFound(format!(
            "Graph file not found: {path}"
        )))?;

    resolved
        .strip_prefix(&base)
        .map_err(|_| crate::error::EngramError::PathEscape(format!(
            "Path {path:?} escapes the allowed directory {}. \
             Only paths inside engram-out/ are permitted.",
            base.display()
        )))?;

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_valid_path() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().join("engram-out");
        std::fs::create_dir_all(&base).unwrap();
        let file = base.join("graph.json");
        std::fs::write(&file, "{}").unwrap();

        let result = validate_graph_path(
            file.to_str().unwrap(),
            Some(&base),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_path_escape() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().join("engram-out");
        std::fs::create_dir_all(&base).unwrap();

        // Create a file outside base
        let outside = dir.path().join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();

        let result = validate_graph_path(
            outside.to_str().unwrap(),
            Some(&base),
        );
        assert!(result.is_err());
    }
}
