//! Diff-aware recall: parse git diff, map to node UUIDs, score knowledge.

use std::collections::HashSet;
use std::path::Path;

/// A parsed diff hunk with file and line range.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// Parse unified diff output into hunks.
pub fn parse_diff(diff_text: &str) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_file = String::new();

    for line in diff_text.lines() {
        // +++ b/path/to/file.py
        if let Some(path) = line.strip_prefix("+++ b/") {
            current_file = path.to_string();
            continue;
        }
        // @@ -old,count +new_start,count @@
        if line.starts_with("@@ ") {
            if let Some(hunk) = parse_hunk_header(line) {
                if !current_file.is_empty() {
                    hunks.push(DiffHunk {
                        file: current_file.clone(),
                        start_line: hunk.0,
                        end_line: hunk.0 + hunk.1.saturating_sub(1),
                    });
                }
            }
        }
    }

    hunks
}

/// Parse "@@ -old,count +new_start,count @@" → (new_start, count)
fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    // Find the +N,M part
    let plus = line.find('+')?;
    let rest = &line[plus + 1..];
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != ',')?;
    let nums = &rest[..end];
    let parts: Vec<&str> = nums.split(',').collect();
    let start: u32 = parts.first()?.parse().ok()?;
    let count: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    Some((start, count))
}

/// Map diff hunks to node UUIDs by matching file + line range.
pub fn diff_to_node_uuids(hunks: &[DiffHunk], data: &crate::types::KodexData) -> Vec<String> {
    let mut uuids = HashSet::new();

    for hunk in hunks {
        let hunk_filename = hunk.file.rsplit('/').next().unwrap_or(&hunk.file);

        for node in &data.extraction.nodes {
            // Match by filename
            let node_filename = node.source_file.rsplit('/').next().unwrap_or(&node.source_file);
            if node_filename != hunk_filename && !node.source_file.ends_with(&hunk.file) {
                continue;
            }

            // Match by line range
            if let Some(loc) = &node.source_location {
                let node_line: u32 = loc.trim_start_matches('L').parse().unwrap_or(0);
                if node_line >= hunk.start_line && node_line <= hunk.end_line + 5 {
                    if let Some(uuid) = &node.uuid {
                        uuids.insert(uuid.clone());
                    }
                }
            }
        }
    }

    uuids.into_iter().collect()
}

/// Detect which existing knowledge may be affected by a diff.
/// Returns node UUIDs that changed + files that changed.
pub fn analyze_diff(
    diff_text: &str,
    h5_path: &Path,
) -> crate::error::Result<DiffAnalysis> {
    let hunks = parse_diff(diff_text);
    let data = crate::storage::load(h5_path)?;

    let changed_files: Vec<String> = hunks
        .iter()
        .map(|h| h.file.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let changed_node_uuids = diff_to_node_uuids(&hunks, &data);

    // Find knowledge linked to changed nodes
    let stale_candidates: Vec<String> = data
        .links
        .iter()
        .filter(|l| {
            !l.is_knowledge_link() && changed_node_uuids.contains(&l.node_uuid)
        })
        .map(|l| l.knowledge_uuid.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    Ok(DiffAnalysis {
        hunks_count: hunks.len(),
        changed_files,
        changed_node_uuids,
        affected_knowledge_uuids: stale_candidates,
    })
}

/// Result of diff analysis.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiffAnalysis {
    pub hunks_count: usize,
    pub changed_files: Vec<String>,
    pub changed_node_uuids: Vec<String>,
    pub affected_knowledge_uuids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_diff() {
        let diff = r#"diff --git a/src/auth.py b/src/auth.py
--- a/src/auth.py
+++ b/src/auth.py
@@ -10,5 +10,8 @@ def authenticate():
+    validate_token()
+    check_permissions()
+
@@ -30,3 +33,4 @@ def logout():
+    clear_session()
"#;
        let hunks = parse_diff(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].file, "src/auth.py");
        assert_eq!(hunks[0].start_line, 10);
        assert_eq!(hunks[1].start_line, 33);
    }

    #[test]
    fn test_parse_hunk_header() {
        assert_eq!(parse_hunk_header("@@ -10,5 +15,8 @@ def foo():"), Some((15, 8)));
        assert_eq!(parse_hunk_header("@@ -1 +1,3 @@"), Some((1, 3)));
        assert_eq!(parse_hunk_header("@@ -0,0 +1 @@"), Some((1, 1)));
    }
}
