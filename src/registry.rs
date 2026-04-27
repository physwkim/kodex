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

    let mut registry = load();
    registry.projects.insert(
        key.clone(),
        ProjectEntry {
            path: canonical,
            last_run: now,
        },
    );
    save(&registry)?;
    Ok(key)
}

/// List registered projects.
pub fn list() -> Vec<(String, ProjectEntry)> {
    let registry = load();
    let mut entries: Vec<_> = registry.projects.into_iter().collect();
    entries.sort_by(|a, b| b.1.last_run.cmp(&a.1.last_run));
    entries
}
