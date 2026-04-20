//! Diff-aware recall: parse git diff, map to node UUIDs, score knowledge.

use std::collections::HashSet;
use std::path::Path;

/// A parsed diff hunk with file and line range.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub file: String,
    pub old_file: String,
    pub start_line: u32,
    pub end_line: u32,
    pub old_start_line: u32,
    pub old_end_line: u32,
}

/// Parse unified diff output into hunks.
/// Tracks both old-side (deletions) and new-side (additions) for rename/delete handling.
pub fn parse_diff(diff_text: &str) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_file = String::new();
    let mut old_file = String::new();

    for line in diff_text.lines() {
        // --- a/path/to/file.py (old file, used for deletes/renames)
        if let Some(path) = line.strip_prefix("--- a/") {
            old_file = path.to_string();
            continue;
        }
        // +++ b/path/to/file.py
        if let Some(path) = line.strip_prefix("+++ b/") {
            current_file = path.to_string();
            continue;
        }
        // +++ /dev/null (file deleted)
        if line == "+++ /dev/null" {
            current_file = old_file.clone();
            continue;
        }
        // @@ -old_start,old_count +new_start,new_count @@
        if line.starts_with("@@ ") {
            if let Some((old, new)) = parse_hunk_header_both(line) {
                let file = if current_file.is_empty() {
                    old_file.clone()
                } else {
                    current_file.clone()
                };
                if !file.is_empty() {
                    hunks.push(DiffHunk {
                        file: file.clone(),
                        old_file: old_file.clone(),
                        start_line: new.0,
                        end_line: new.0 + new.1.saturating_sub(1),
                        old_start_line: old.0,
                        old_end_line: old.0 + old.1.saturating_sub(1),
                    });
                }
            }
        }
    }

    hunks
}

/// Parse "@@ -old_start,old_count +new_start,new_count @@"
fn parse_hunk_header_both(line: &str) -> Option<((u32, u32), (u32, u32))> {
    // Parse -N,M
    let minus = line.find('-')?;
    let after_minus = &line[minus + 1..];
    let space = after_minus.find(' ')?;
    let old_nums = &after_minus[..space];
    let old_parts: Vec<&str> = old_nums.split(',').collect();
    let old_start: u32 = old_parts.first()?.parse().ok()?;
    let old_count: u32 = old_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

    // Parse +N,M
    let plus = line.find('+')?;
    let after_plus = &line[plus + 1..];
    let end = after_plus.find(|c: char| !c.is_ascii_digit() && c != ',')?;
    let new_nums = &after_plus[..end];
    let new_parts: Vec<&str> = new_nums.split(',').collect();
    let new_start: u32 = new_parts.first()?.parse().ok()?;
    let new_count: u32 = new_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

    Some(((old_start, old_count), (new_start, new_count)))
}

/// Map diff hunks to node UUIDs by matching file + line range.
/// Checks both new-side and old-side ranges to handle deletions/renames.
pub fn diff_to_node_uuids(hunks: &[DiffHunk], data: &crate::types::KodexData) -> Vec<String> {
    let mut uuids = HashSet::new();

    for hunk in hunks {
        let new_filename = hunk.file.rsplit('/').next().unwrap_or(&hunk.file);
        let old_filename = hunk.old_file.rsplit('/').next().unwrap_or(&hunk.old_file);

        for node in &data.extraction.nodes {
            let node_filename = node.source_file.rsplit('/').next().unwrap_or(&node.source_file);

            if let Some(loc) = &node.source_location {
                let node_line: u32 = loc.trim_start_matches('L').parse().unwrap_or(0);

                // Match new-side (additions/modifications)
                let new_match = (node_filename == new_filename
                    || node.source_file.ends_with(&hunk.file))
                    && node_line >= hunk.start_line
                    && node_line <= hunk.end_line.saturating_add(5);

                // Match old-side (deletions/renames)
                let old_match = (node_filename == old_filename
                    || node.source_file.ends_with(&hunk.old_file))
                    && node_line >= hunk.old_start_line
                    && node_line <= hunk.old_end_line.saturating_add(5);

                if new_match || old_match {
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
        .flat_map(|h| [h.file.clone(), h.old_file.clone()])
        .filter(|f| !f.is_empty())
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
        assert_eq!(hunks[0].old_file, "src/auth.py");
        assert_eq!(hunks[0].start_line, 10);
        assert_eq!(hunks[0].old_start_line, 10);
        assert_eq!(hunks[1].start_line, 33);
        assert_eq!(hunks[1].old_start_line, 30);
    }

    #[test]
    fn test_parse_hunk_header_both() {
        let r = parse_hunk_header_both("@@ -10,5 +15,8 @@ def foo():");
        assert_eq!(r, Some(((10, 5), (15, 8))));
        let r = parse_hunk_header_both("@@ -1 +1,3 @@");
        assert_eq!(r, Some(((1, 1), (1, 3))));
    }

    #[test]
    fn test_parse_file_deletion() {
        let diff = r#"diff --git a/old.py b/old.py
--- a/old.py
+++ /dev/null
@@ -1,10 +0,0 @@
-def removed():
-    pass
"#;
        let hunks = parse_diff(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].file, "old.py");
        assert_eq!(hunks[0].old_file, "old.py");
        assert_eq!(hunks[0].old_start_line, 1);
    }
}
