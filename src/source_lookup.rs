//! Resolve graph-stored `source_file` paths to absolute disk paths via the
//! project registry, then read code snippets for callers like `compare_graphs`
//! that want signatures + docstrings inlined.

use std::path::{Path, PathBuf};

/// Resolve a graph `source_file` (e.g. `pvxs/src/sharedpv.cpp`) to an
/// absolute path on disk. The convention used by the ingestor is that the
/// first path component is the registry key and the rest is relative to
/// the registered project root.
pub fn resolve_source_path(source_file: &str) -> Option<PathBuf> {
    if source_file.is_empty() {
        return None;
    }
    let direct = PathBuf::from(source_file);
    if direct.is_absolute() && direct.exists() {
        return Some(direct);
    }

    let registry = crate::registry::load();
    let (head, rest) = source_file.split_once('/').unwrap_or((source_file, ""));
    if let Some(entry) = registry.projects.get(head) {
        let candidate = if rest.is_empty() {
            entry.path.clone()
        } else {
            entry.path.join(rest)
        };
        if candidate.exists() {
            return Some(candidate);
        }
        // Also try without stripping the head — for ingestors that store paths
        // already relative to the registered root.
        let direct = entry.path.join(source_file);
        if direct.exists() {
            return Some(direct);
        }
    }
    None
}

/// Parse a kodex source_location string into a 1-based line number.
/// Accepts `L348`, `348`, or `L348-L355` (returns the start line).
pub fn parse_line_number(loc: &str) -> Option<usize> {
    let trimmed = loc.trim().trim_start_matches('L');
    let first = trimmed.split(['-', ',', ':']).next()?;
    first.trim_start_matches('L').parse().ok()
}

/// Read `lines_above` lines above and `lines_below` lines below `line` from
/// `path`. Returns the joined slice — typically the function signature plus a
/// preceding doc comment. Strips trailing whitespace per line.
pub fn read_snippet(
    path: &Path,
    line: usize,
    lines_above: usize,
    lines_below: usize,
) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    if line == 0 || line > lines.len() {
        return None;
    }
    let start = line.saturating_sub(lines_above + 1);
    let end = (line + lines_below).min(lines.len());
    let slice: Vec<String> = lines[start..end]
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect();
    Some(slice.join("\n"))
}

/// Convenience wrapper: resolve and read in one shot. Returns `None` when
/// the file isn't in the registry, or the location can't be parsed, or the
/// file doesn't exist on disk.
pub fn snippet_for(
    source_file: &str,
    source_location: Option<&str>,
    lines_above: usize,
    lines_below: usize,
) -> Option<String> {
    let path = resolve_source_path(source_file)?;
    let line = parse_line_number(source_location?)?;
    read_snippet(&path, line, lines_above, lines_below)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parses_line_number_variants() {
        assert_eq!(parse_line_number("L42"), Some(42));
        assert_eq!(parse_line_number("42"), Some(42));
        assert_eq!(parse_line_number("L100-L105"), Some(100));
        assert_eq!(parse_line_number("  L7  "), Some(7));
        assert_eq!(parse_line_number(""), None);
        assert_eq!(parse_line_number("not-a-number"), None);
    }

    #[test]
    fn reads_snippet_with_context() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("src.txt");
        std::fs::write(
            &path,
            "line1\nline2\n/// docstring\npub fn target() {\n    body\n}\n",
        )
        .unwrap();
        let snippet = read_snippet(&path, 4, 1, 0).unwrap();
        assert!(snippet.contains("/// docstring"), "got: {snippet}");
        assert!(snippet.contains("pub fn target()"));
    }

    #[test]
    fn out_of_range_line_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("src.txt");
        std::fs::write(&path, "line1\nline2\n").unwrap();
        assert!(read_snippet(&path, 0, 0, 0).is_none());
        assert!(read_snippet(&path, 99, 0, 0).is_none());
    }
}
