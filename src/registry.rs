//! Global kodex home and project registry.
//!
//! ~/.kodex/
//! ├── kodex.db         ← single knowledge base (all projects + knowledge)
//! ├── kodex.sock       ← actor socket
//! └── registry.json    ← tracked project paths

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const KODEX_DIR: &str = ".kodex";
const REGISTRY_FILE: &str = "registry.json";

/// Global kodex home directory.
pub fn kodex_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(KODEX_DIR)
}

/// The single global knowledge base.
pub fn global_db() -> PathBuf {
    kodex_home().join("kodex.db")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub last_run: u64,
    /// Git HEAD SHA at the moment of the last `kodex run`. Used by query
    /// handlers to flag responses with `stale: true` when the working tree
    /// has moved past the indexed snapshot. `None` for pre-v0.7 entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_indexed_commit: Option<String>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Registry {
    pub projects: HashMap<String, ProjectEntry>,
}

pub fn load() -> Registry {
    let path = kodex_home().join(REGISTRY_FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(registry: &Registry) -> crate::error::Result<()> {
    let dir = kodex_home();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(REGISTRY_FILE);
    let json = serde_json::to_string_pretty(registry)
        .map_err(|e| crate::error::KodexError::Other(format!("registry: {e}")))?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Register a project. Called after `kodex run`.
pub fn register(project_path: &Path) -> crate::error::Result<String> {
    let canonical = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());
    let key = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let last_indexed_commit = current_head_commit(&canonical);

    let mut registry = load();
    registry.projects.insert(
        key.clone(),
        ProjectEntry {
            path: canonical,
            last_run: now,
            last_indexed_commit,
        },
    );
    save(&registry)?;
    Ok(key)
}

/// Return the current HEAD commit SHA in `dir`, if it's a git repo.
pub fn current_head_commit(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Check whether `dir` (a project working tree) has drifted from the indexed
/// snapshot. Returns the current HEAD when it differs from `last_indexed_commit`,
/// `None` when in sync or unknown.
pub fn drift(entry: &ProjectEntry, dir: &Path) -> Option<String> {
    let stored = entry.last_indexed_commit.as_deref()?;
    let current = current_head_commit(dir)?;
    if current == stored {
        None
    } else {
        Some(current)
    }
}

/// Resolve the registry entry whose path matches `dir` (or is its ancestor).
/// Used by query handlers to attach a stale_warning per request based on
/// `project_dir` (the CWD MCP injects).
pub fn entry_for_dir(dir: &Path) -> Option<ProjectEntry> {
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let registry = load();
    // Exact path match first; otherwise fall back to longest-prefix match so
    // queries from a sub-directory still resolve to the registered root.
    if let Some(direct) = registry
        .projects
        .values()
        .find(|e| e.path == canonical)
        .cloned()
    {
        return Some(direct);
    }
    registry
        .projects
        .into_values()
        .filter(|e| canonical.starts_with(&e.path))
        .max_by_key(|e| e.path.components().count())
}

/// List registered projects.
pub fn list() -> Vec<(String, ProjectEntry)> {
    let registry = load();
    let mut entries: Vec<_> = registry.projects.into_iter().collect();
    entries.sort_by(|a, b| b.1.last_run.cmp(&a.1.last_run));
    entries
}
