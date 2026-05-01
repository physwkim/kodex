//! Chunk-level slicing of source/document files for semantic retrieval.
//!
//! Produces fixed line-window chunks (50 lines, 5-line overlap) over both
//! code and document files, then maps each chunk to a graph node when one of
//! the already-extracted nodes starts inside the chunk's line range. The
//! chunker is intentionally model-agnostic — embedding is a separate step
//! (`kodex embed`) that reads `chunks` and writes `chunk_embeddings`.
//!
//! v1 strategy: pure line-window splitting. AST-aware boundary alignment
//! (e.g. "one chunk = one short function") is a future enhancement; the
//! best-effort `node_id` mapping here gives most of the practical value
//! (callers see `attached_knowledge` for chunks inside known nodes) without
//! a second tree-sitter pass.

use sha2::{Digest, Sha256};
use std::path::Path;

use crate::storage::StoredChunk;
use crate::types::Node;

/// Lines per chunk window. Chosen to fit comfortably under BGE-small's
/// 512-token input limit (rough rule of thumb: ~10 tokens / line of code).
pub const CHUNK_LINES: usize = 50;

/// Lines of overlap between adjacent windows. Mirrors semble's default.
/// Catches identifiers that span a chunk boundary.
pub const CHUNK_OVERLAP: usize = 5;

/// Skip windows whose non-whitespace content is shorter than this. A nearly
/// empty window adds no retrieval value and wastes an embedding slot. Note
/// the trade-off: a long file whose final 5–6 lines are short (e.g.
/// trailing close-braces only, ~12 bytes) will have its tail window
/// dropped, leaving those lines unsearchable via chunks. Acceptable for
/// retrieval purposes — the preceding chunk's overlap region still covers
/// most of the lost context.
const MIN_CHUNK_BYTES: usize = 32;

/// Build chunks for one file. `source_file` is the registry-prefixed path
/// (e.g. `myproj/src/foo.rs`) that must match what's stored on `Node`s for
/// node_id mapping to work. `nodes` should be the slice of extracted nodes
/// belonging to this file.
pub fn chunk_file(
    source_file: &str,
    disk_path: &Path,
    language: Option<&str>,
    nodes_in_file: &[&Node],
) -> Vec<StoredChunk> {
    let content = match std::fs::read_to_string(disk_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    chunk_text(source_file, &content, language, nodes_in_file)
}

/// Lower-level: chunk an in-memory string. Exposed for unit testing.
pub fn chunk_text(
    source_file: &str,
    content: &str,
    language: Option<&str>,
    nodes_in_file: &[&Node],
) -> Vec<StoredChunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut start = 0usize;
    let stride = CHUNK_LINES.saturating_sub(CHUNK_OVERLAP).max(1);
    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        let body = lines[start..end].join("\n");
        let trimmed_len = body.trim().len();
        if trimmed_len >= MIN_CHUNK_BYTES {
            let start_line = (start + 1) as i64; // 1-based, inclusive
            let end_line = end as i64; // 1-based, inclusive
            let content_hash = sha256_hex(&body);
            // Stable id: (file, range, hash). Re-running on unchanged content
            // produces the same id so embeddings can be reused.
            let id = sha256_hex(&format!(
                "{source_file}|{start_line}-{end_line}|{content_hash}"
            ));
            let node_id = match_node_in_range(nodes_in_file, start_line, end_line);
            out.push(StoredChunk {
                id,
                node_id,
                file_path: source_file.to_string(),
                start_line,
                end_line,
                language: language.map(str::to_string),
                content: body,
                content_hash,
            });
        }
        if end >= lines.len() {
            break;
        }
        start += stride;
    }
    out
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

/// Best-effort node mapping: pick the first node whose `source_location`
/// start line falls inside `[start_line, end_line]` (1-based, inclusive).
/// Returns `None` for chunks outside any extracted node — typical for
/// markdown / config files, or for top-level code regions between
/// definitions.
///
/// "First" is the iteration order of `nodes_in_file`, which is the order
/// nodes were emitted by the extract pipeline for this file. That order is
/// deterministic per-run (per-file extraction is sequential within a file
/// even when files are extracted in parallel via rayon), so two runs over
/// the same source produce the same chunk-to-node mapping. With overloaded
/// names sharing a start line, the earliest-extracted node wins.
fn match_node_in_range(
    nodes_in_file: &[&Node],
    start_line: i64,
    end_line: i64,
) -> Option<String> {
    for n in nodes_in_file {
        let loc = n.source_location.as_deref()?;
        let line = crate::source_lookup::parse_line_number(loc)? as i64;
        if line >= start_line && line <= end_line {
            return Some(n.id.clone());
        }
    }
    None
}

/// Map a file-extension to the language string we store on chunks. Only
/// covers the common code extensions plus prose formats; returns `None` for
/// anything else (the chunker still emits chunks, just without a language
/// tag).
pub fn language_for_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    Some(match ext.as_str() {
        "py" => "python",
        "js" | "jsx" | "mjs" | "ejs" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "rs" => "rust",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "rb" => "ruby",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "scala" => "scala",
        "php" => "php",
        "lua" | "toc" => "lua",
        "vue" | "svelte" => "javascript",
        "md" | "mdx" => "markdown",
        "txt" | "rst" => "text",
        "html" => "html",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Confidence, FileType, Node};

    fn mk_node(id: &str, source_file: &str, line: usize) -> Node {
        Node {
            id: id.to_string(),
            label: id.to_string(),
            file_type: FileType::Code,
            source_file: source_file.to_string(),
            source_location: Some(format!("L{line}")),
            confidence: Some(Confidence::EXTRACTED),
            confidence_score: None,
            community: None,
            norm_label: None,
            degree: None,
            uuid: None,
            fingerprint: None,
            logical_key: None,
            body_hash: None,
        }
    }

    #[test]
    fn chunks_short_file_into_one_window() {
        let body = (1..=20).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let chunks = chunk_text("p/foo.rs", &body, Some("rust"), &[]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 20);
        assert_eq!(chunks[0].language.as_deref(), Some("rust"));
        assert!(chunks[0].node_id.is_none());
    }

    #[test]
    fn chunks_long_file_with_overlap() {
        let body = (1..=120).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let chunks = chunk_text("p/long.rs", &body, Some("rust"), &[]);
        // Stride = 45, so chunks at [1-50], [46-95], [91-120]
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 50);
        assert_eq!(chunks[1].start_line, 46);
        assert_eq!(chunks[1].end_line, 95);
        // Verify overlap exists (lines 46-50 appear in both chunks 0 and 1).
        let c0_last_line = chunks[0].content.lines().last().unwrap();
        let c1_first_line = chunks[1].content.lines().next().unwrap();
        assert_eq!(c0_last_line, "line 50");
        assert_eq!(c1_first_line, "line 46");
    }

    #[test]
    fn maps_chunk_to_first_overlapping_node() {
        let body = (1..=80).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let n1 = mk_node("nid_1", "p/foo.rs", 10);
        let n2 = mk_node("nid_2", "p/foo.rs", 60);
        let nodes: Vec<&Node> = vec![&n1, &n2];
        let chunks = chunk_text("p/foo.rs", &body, Some("rust"), &nodes);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].node_id.as_deref(), Some("nid_1"));
        // Second chunk starts at line 46 — node at line 60 falls inside [46, 80].
        assert_eq!(chunks[1].node_id.as_deref(), Some("nid_2"));
    }

    #[test]
    fn skips_whitespace_only_window() {
        let body = "\n\n\n   \n\n";
        let chunks = chunk_text("p/blank.rs", body, Some("rust"), &[]);
        assert!(chunks.is_empty());
    }

    #[test]
    fn id_is_stable_for_unchanged_content() {
        let body = (1..=20).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let a = chunk_text("p/foo.rs", &body, Some("rust"), &[]);
        let b = chunk_text("p/foo.rs", &body, Some("rust"), &[]);
        assert_eq!(a[0].id, b[0].id);
    }
}
