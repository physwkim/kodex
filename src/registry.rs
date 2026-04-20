//! Global project registry: tracks all projects that have run `kodex run`.
//!
//! ~/.kodex/
//! ├── registry.json   ← project paths + metadata
//! └── workspace.h5    ← unified knowledge across all projects

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const REGISTRY_DIR: &str = ".kodex";
const REGISTRY_FILE: &str = "registry.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub h5_path: PathBuf,
    pub last_run: u64,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Registry {
    pub projects: HashMap<String, ProjectEntry>,
}

/// Get the global kodex home directory (~/.kodex/).
pub fn kodex_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(REGISTRY_DIR)
}

/// Get the global workspace.h5 path.
pub fn workspace_h5() -> PathBuf {
    kodex_home().join("workspace.h5")
}

/// Load the registry from ~/.kodex/registry.json.
pub fn load() -> Registry {
    let path = kodex_home().join(REGISTRY_FILE);
    if !path.exists() {
        return Registry::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Save the registry to ~/.kodex/registry.json.
fn save(registry: &Registry) -> crate::error::Result<()> {
    let dir = kodex_home();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(REGISTRY_FILE);
    let json = serde_json::to_string_pretty(registry)
        .map_err(|e| crate::error::KodexError::Other(format!("registry serialize: {e}")))?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Register a project after `kodex run`. Returns the project key.
pub fn register(project_path: &Path) -> crate::error::Result<String> {
    let canonical = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());
    let key = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let h5_path = canonical.join("kodex-out/kodex.h5");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut registry = load();
    registry.projects.insert(
        key.clone(),
        ProjectEntry {
            path: canonical,
            h5_path,
            last_run: now,
        },
    );
    save(&registry)?;

    // Sync knowledge to global workspace
    let _ = sync_to_workspace(&registry);

    Ok(key)
}

/// List all registered projects.
pub fn list() -> Vec<(String, ProjectEntry)> {
    let registry = load();
    let mut entries: Vec<_> = registry.projects.into_iter().collect();
    entries.sort_by(|a, b| b.1.last_run.cmp(&a.1.last_run));
    entries
}

/// Sync knowledge from all project h5 files into ~/.kodex/workspace.h5.
fn sync_to_workspace(registry: &Registry) -> crate::error::Result<()> {
    let ws_path = workspace_h5();

    // Collect all knowledge from all projects
    let mut all_titles = Vec::new();
    let mut all_types = Vec::new();
    let mut all_descriptions = Vec::new();
    let mut all_confidences: Vec<f64> = Vec::new();
    let mut all_observations: Vec<u32> = Vec::new();
    let mut all_related: Vec<String> = Vec::new();
    let mut all_tags = Vec::new();

    // Track seen titles to merge duplicates
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (project_name, entry) in &registry.projects {
        if !entry.h5_path.exists() {
            continue;
        }
        let entries = match crate::storage::load_knowledge_entries(&entry.h5_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for (title, ktype, desc, conf, obs, related, tags) in entries {
            // Tag with project origin
            let tagged_related = if related.is_empty() {
                project_name.clone()
            } else {
                format!("{related},{project_name}")
            };

            if let Some(&idx) = seen.get(&title) {
                // Merge: keep higher confidence, sum observations
                all_observations[idx] += obs;
                if conf > all_confidences[idx] {
                    all_confidences[idx] = conf;
                }
                if !desc.is_empty() && all_descriptions[idx] != desc {
                    all_descriptions[idx] = format!(
                        "{}\n---\n[{}] {}",
                        all_descriptions[idx], project_name, desc
                    );
                }
                // Merge related
                for r in tagged_related.split(',').filter(|s| !s.is_empty()) {
                    if !all_related[idx].contains(r) {
                        all_related[idx] = format!("{},{r}", all_related[idx]);
                    }
                }
            } else {
                let idx = all_titles.len();
                seen.insert(title.clone(), idx);
                all_titles.push(title);
                all_types.push(ktype);
                all_descriptions.push(format!("[{project_name}] {desc}"));
                all_confidences.push(conf);
                all_observations.push(obs);
                all_related.push(tagged_related);
                all_tags.push(tags);
            }
        }
    }

    if all_titles.is_empty() {
        return Ok(());
    }

    // Build a minimal graph for workspace.h5 (just knowledge, no code nodes)
    let graph = crate::graph::KodexGraph::new();
    let communities = std::collections::HashMap::new();

    crate::storage::save_hdf5_with_knowledge_pub(
        &graph,
        &communities,
        &ws_path,
        &all_titles,
        &all_types,
        &all_descriptions,
        &all_confidences,
        &all_observations,
        &all_related,
        &all_tags,
    )
}
