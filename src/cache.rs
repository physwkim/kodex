use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Bumped whenever the on-disk extraction format changes. Mixed into
/// `file_hash` so old cache entries become unreachable on upgrade and a
/// `kodex run` re-extracts cleanly. Bump this when adding/renaming fields on
/// `RawCall`, `Node`, `Edge`, or changing resolution semantics in a way that
/// alters the produced graph.
const CACHE_SCHEMA_VERSION: u32 = 3;

/// Strip YAML frontmatter from Markdown content, returning only the body.
fn body_content(content: &[u8]) -> &[u8] {
    let text = std::str::from_utf8(content).unwrap_or("");
    if let Some(rest) = text.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let offset = 3 + end + 4; // skip past "\n---"
            if offset <= content.len() {
                return &content[offset..];
            }
        }
    }
    content
}

/// Return the `kodex-out/cache/` directory, creating it if necessary.
pub fn cache_dir(root: &Path) -> PathBuf {
    let dir = root.join("kodex-out").join("cache");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// SHA256 of file contents + path relative to root.
///
/// Uses a relative path (not absolute) to make cache entries portable across
/// machines. For Markdown files, only the body below YAML frontmatter is hashed.
pub fn file_hash(path: &Path, root: &Path) -> std::io::Result<String> {
    let raw = std::fs::read(path)?;
    let is_md = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase() == "md")
        .unwrap_or(false);
    let content = if is_md { body_content(&raw) } else { &raw };

    let mut hasher = Sha256::new();
    hasher.update(b"kodex-cache-v");
    hasher.update(CACHE_SCHEMA_VERSION.to_le_bytes());
    hasher.update(b"\x00");
    hasher.update(content);
    hasher.update(b"\x00");

    let rel = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .strip_prefix(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()))
        .map(|r| r.to_string_lossy().to_string())
        .unwrap_or_else(|_| {
            path.canonicalize()
                .unwrap_or_else(|_| path.to_path_buf())
                .to_string_lossy()
                .to_string()
        });
    hasher.update(rel.as_bytes());

    Ok(format!("{:x}", hasher.finalize()))
}

/// Return cached extraction for this file if hash matches, else None.
pub fn load_cached(path: &Path, root: &Path) -> Option<serde_json::Value> {
    let h = file_hash(path, root).ok()?;
    let entry = cache_dir(root).join(format!("{h}.json"));
    if !entry.exists() {
        return None;
    }
    let text = std::fs::read_to_string(&entry).ok()?;
    serde_json::from_str(&text).ok()
}

/// Save extraction result for this file.
pub fn save_cached(path: &Path, result: &serde_json::Value, root: &Path) -> std::io::Result<()> {
    let h = file_hash(path, root)?;
    let entry = cache_dir(root).join(format!("{h}.json"));
    let tmp = entry.with_extension("tmp");

    std::fs::write(&tmp, serde_json::to_string(result).unwrap_or_default())?;
    match std::fs::rename(&tmp, &entry) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Fallback: copy then delete (Windows lock issue)
            std::fs::copy(&tmp, &entry)?;
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
    }
}

/// Return the set of file hashes that have cached entries.
pub fn cached_files(root: &Path) -> HashSet<String> {
    let dir = cache_dir(root);
    let mut hashes = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(hash) = name.strip_suffix(".json") {
                    hashes.insert(hash.to_string());
                }
            }
        }
    }
    hashes
}

/// Delete all cache files.
pub fn clear_cache(root: &Path) {
    let dir = cache_dir(root);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_body_content_strips_frontmatter() {
        let content = b"---\ntitle: Hello\n---\nBody here";
        let body = body_content(content);
        assert_eq!(std::str::from_utf8(body).unwrap(), "\nBody here");
    }

    #[test]
    fn test_body_content_no_frontmatter() {
        let content = b"Just regular text";
        let body = body_content(content);
        assert_eq!(body, content);
    }

    #[test]
    fn test_file_hash_deterministic() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.py");
        std::fs::write(&file, "def hello(): pass").unwrap();

        let h1 = file_hash(&file, dir.path()).unwrap();
        let h2 = file_hash(&file, dir.path()).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA256 hex length
    }

    #[test]
    fn test_cache_round_trip() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.py");
        std::fs::write(&file, "x = 1").unwrap();

        let data = serde_json::json!({"nodes": [], "edges": []});
        save_cached(&file, &data, dir.path()).unwrap();

        let loaded = load_cached(&file, dir.path());
        assert_eq!(loaded, Some(data));
    }

    #[test]
    fn test_md_frontmatter_ignored_for_hash() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("doc.md");

        // Same body, different frontmatter → same hash
        std::fs::write(&file, "---\ntags: [a]\n---\nBody").unwrap();
        let h1 = file_hash(&file, dir.path()).unwrap();

        std::fs::write(&file, "---\ntags: [b]\n---\nBody").unwrap();
        let h2 = file_hash(&file, dir.path()).unwrap();

        assert_eq!(h1, h2);
    }
}
