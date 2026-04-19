mod classify;
mod ignore;
mod sensitive;
mod paper;
mod manifest;

pub use classify::{classify_file, FileCategory, CODE_EXTENSIONS, DOC_EXTENSIONS, PAPER_EXTENSIONS, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS, OFFICE_EXTENSIONS};
pub use ignore::load_engramignore;
pub use sensitive::is_sensitive;
pub use paper::looks_like_paper;
pub use manifest::{load_manifest, save_manifest};

use crate::types::{DetectedFiles, DetectionResult};
use std::path::Path;
use walkdir::WalkDir;

/// Corpus size thresholds.
pub const CORPUS_WARN_THRESHOLD: usize = 50_000;
pub const CORPUS_UPPER_THRESHOLD: usize = 500_000;
pub const FILE_COUNT_UPPER: usize = 200;

/// Noise directories to always skip.
const NOISE_DIRS: &[&str] = &[
    "node_modules", "__pycache__", ".git", ".hg", ".svn",
    "target", "dist", "build", ".tox", ".mypy_cache",
    ".pytest_cache", ".ruff_cache", "venv", ".venv",
    "engram-out", ".engram-out",
];

/// Find all extractable files in `root` directory.
pub fn detect(root: &Path, follow_symlinks: bool) -> DetectionResult {
    let ignore_patterns = load_engramignore(root);
    let pattern_count = ignore_patterns.len();

    let mut files = DetectedFiles::default();
    let mut total_words: usize = 0;
    let mut skipped_sensitive = Vec::new();

    let walker = WalkDir::new(root)
        .follow_links(follow_symlinks)
        .into_iter()
        .filter_entry(|entry| {
            if entry.file_type().is_dir() {
                let name = entry.file_name().to_string_lossy();
                // Skip hidden dirs (except root) and noise dirs
                if name.starts_with('.') && entry.depth() > 0 {
                    return false;
                }
                if NOISE_DIRS.contains(&name.as_ref()) {
                    return false;
                }
            }
            true
        });

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();

        // Check .engramignore patterns
        if let Ok(rel) = path.strip_prefix(root) {
            let rel_str = rel.to_string_lossy();
            if ignore_patterns.iter().any(|pat| {
                globset::Glob::new(pat)
                    .ok()
                    .and_then(|g| g.compile_matcher().is_match(rel_str.as_ref()).then_some(()))
                    .is_some()
            }) {
                continue;
            }
        }

        // Check sensitive files
        if is_sensitive(path) {
            skipped_sensitive.push(path_str);
            continue;
        }

        match classify_file(path) {
            Some(FileCategory::Code) => {
                total_words += count_words(path);
                files.code.push(path_str);
            }
            Some(FileCategory::Document) => {
                total_words += count_words(path);
                files.document.push(path_str);
            }
            Some(FileCategory::Paper) => {
                files.paper.push(path_str);
            }
            Some(FileCategory::Image) => {
                files.image.push(path_str);
            }
            Some(FileCategory::Video) => {
                files.video.push(path_str);
            }
            Some(FileCategory::Office) => {
                // Treated as document
                files.document.push(path_str);
            }
            None => {}
        }
    }

    let total_files = files.code.len()
        + files.document.len()
        + files.paper.len()
        + files.image.len()
        + files.video.len();

    let needs_graph = total_words >= CORPUS_WARN_THRESHOLD;

    let warning = if total_files > 0 && !needs_graph {
        Some(format!(
            "Corpus has only {total_words} words across {total_files} files. \
             You may not need a graph for this."
        ))
    } else if total_words > CORPUS_UPPER_THRESHOLD {
        Some(format!(
            "Large corpus: {total_words} words. This may use significant tokens."
        ))
    } else if total_files > FILE_COUNT_UPPER {
        Some(format!(
            "Large file count: {total_files} files. Consider using .engramignore."
        ))
    } else {
        None
    };

    DetectionResult {
        files,
        total_files,
        total_words,
        needs_graph,
        warning,
        skipped_sensitive,
        engramignore_patterns: pattern_count,
    }
}

/// Rough word count for a text file.
pub fn count_words(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|content| content.split_whitespace().count())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detect_simple() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.py"), "def hello():\n    pass\n").unwrap();
        fs::write(dir.path().join("README.md"), "# Hello\nWorld\n").unwrap();

        let result = detect(dir.path(), false);
        assert_eq!(result.files.code.len(), 1);
        assert_eq!(result.files.document.len(), 1);
        assert_eq!(result.total_files, 2);
    }

    #[test]
    fn test_skips_sensitive_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".env"), "SECRET=123").unwrap();
        fs::write(dir.path().join("main.py"), "x = 1").unwrap();

        let result = detect(dir.path(), false);
        assert_eq!(result.files.code.len(), 1);
        assert_eq!(result.skipped_sensitive.len(), 1);
    }
}
