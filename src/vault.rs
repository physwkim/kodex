use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use std::sync::LazyLock;

use crate::graph::GraphifyGraph;
use crate::id::make_id;
use crate::types::{Confidence, Edge, ExtractionResult, FileType, Node};

static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]").expect("wikilink regex"));

/// Load a complete graph from an Obsidian vault directory.
///
/// Parses every `.md` file's YAML frontmatter for node metadata and
/// `[[wikilinks]]` for edges. The vault is the source of truth;
/// graph.json is just a cache for performance.
pub fn load_graph_from_vault(vault_dir: &Path) -> crate::error::Result<GraphifyGraph> {
    let entries = collect_md_files(vault_dir)?;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut filename_to_id: HashMap<String, String> = HashMap::new();

    // Pass 1: parse all nodes from frontmatter
    for path in &entries {
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // Skip index and community overviews for node creation
        if filename == "index" {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let frontmatter = parse_frontmatter(&content);
        let is_community = filename.starts_with("_COMMUNITY_");
        let is_insight = filename.starts_with("_INSIGHT_");
        let is_note = filename.starts_with("_NOTE_");

        let node_id = frontmatter
            .get("id")
            .cloned()
            .unwrap_or_else(|| make_id(&[&filename]));

        let file_type = match frontmatter.get("file_type").or(frontmatter.get("type")).map(|s| s.as_str()) {
            Some("code") => FileType::Code,
            Some("document") => FileType::Document,
            Some("paper") => FileType::Paper,
            Some("image") => FileType::Image,
            Some("video") => FileType::Video,
            Some("rationale") | Some("insight") => FileType::Rationale,
            Some("community") => {
                // Community overviews aren't graph nodes
                filename_to_id.insert(filename, node_id);
                continue;
            }
            Some("note") if is_note => FileType::Document,
            _ if is_insight => FileType::Rationale,
            _ if is_community => continue,
            _ => FileType::Code,
        };

        let confidence = match frontmatter.get("confidence").map(|s| s.as_str()) {
            Some("INFERRED") => Confidence::INFERRED,
            Some("AMBIGUOUS") => Confidence::AMBIGUOUS,
            _ => Confidence::EXTRACTED,
        };

        let community = frontmatter
            .get("community")
            .and_then(|s| s.parse::<usize>().ok());

        nodes.push(Node {
            id: node_id.clone(),
            label: extract_title(&content).unwrap_or_else(|| filename.replace('_', " ")),
            file_type,
            source_file: frontmatter
                .get("source_file")
                .cloned()
                .unwrap_or_default(),
            source_location: frontmatter.get("location").cloned(),
            confidence: Some(confidence),
            confidence_score: frontmatter
                .get("confidence_score")
                .and_then(|s| s.parse().ok()),
            community,
            norm_label: None,
            degree: None,
        });

        filename_to_id.insert(filename, node_id);
    }

    // Pass 2: parse wikilinks as edges
    for path in &entries {
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        if filename == "index" || filename.starts_with("_COMMUNITY_") {
            continue;
        }

        let source_id = match filename_to_id.get(&filename) {
            Some(id) => id.clone(),
            None => continue,
        };

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Parse connections section for structured edges
        let parsed_edges = parse_connections(&content, &filename_to_id);
        if !parsed_edges.is_empty() {
            for (target_id, relation, confidence) in parsed_edges {
                edges.push(Edge {
                    source: source_id.clone(),
                    target: target_id,
                    relation,
                    confidence,
                    source_file: path.to_string_lossy().to_string(),
                    source_location: None,
                    confidence_score: Some(confidence.default_score()),
                    weight: 1.0,
                    original_src: None,
                    original_tgt: None,
                });
            }
        } else {
            // Fallback: raw wikilinks as edges
            let mut seen_targets = HashSet::new();
            for cap in WIKILINK_RE.captures_iter(&content) {
                if let Some(target_name) = cap.get(1) {
                    let target_str = target_name.as_str().to_string();
                    if target_str.starts_with("_COMMUNITY_") || target_str == "index" {
                        continue;
                    }
                    if let Some(target_id) = filename_to_id.get(&target_str) {
                        if *target_id != source_id && seen_targets.insert(target_id.clone()) {
                            edges.push(Edge {
                                source: source_id.clone(),
                                target: target_id.clone(),
                                relation: "linked".to_string(),
                                confidence: Confidence::EXTRACTED,
                                source_file: path.to_string_lossy().to_string(),
                                source_location: None,
                                confidence_score: Some(1.0),
                                weight: 1.0,
                                original_src: None,
                                original_tgt: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // Build graph
    let extraction = ExtractionResult {
        nodes,
        edges,
        ..Default::default()
    };
    Ok(crate::graph::build_from_extraction(&extraction))
}

/// Save the graph as a cached graph.json derived from vault.
pub fn cache_graph_from_vault(
    vault_dir: &Path,
    cache_path: &Path,
) -> crate::error::Result<()> {
    let graph = load_graph_from_vault(vault_dir)?;
    let communities = crate::cluster::cluster(&graph);
    crate::export::to_json(&graph, &communities, cache_path)?;
    Ok(())
}

/// Check if cached graph.json is stale compared to vault files.
pub fn is_cache_stale(vault_dir: &Path, cache_path: &Path) -> bool {
    let cache_mtime = match std::fs::metadata(cache_path)
        .and_then(|m| m.modified())
    {
        Ok(t) => t,
        Err(_) => return true, // No cache
    };

    // Check if any .md in vault is newer than cache
    if let Ok(entries) = std::fs::read_dir(vault_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(meta) = std::fs::metadata(&path) {
                    if let Ok(mtime) = meta.modified() {
                        if mtime > cache_mtime {
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}

// --- Parsers ---

fn collect_md_files(dir: &Path) -> crate::error::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Err(crate::error::GraphifyError::Other(format!(
            "Vault directory not found: {}", dir.display()
        )));
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            files.push(path);
        }
    }
    Ok(files)
}

fn parse_frontmatter(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if !content.starts_with("---") {
        return map;
    }
    let end = match content[3..].find("\n---") {
        Some(pos) => 3 + pos,
        None => return map,
    };
    let fm = &content[4..end];
    for line in fm.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let k = key.trim().to_string();
            let v = value.trim().trim_matches('"').to_string();
            if !k.is_empty() && !v.is_empty() {
                map.insert(k, v);
            }
        }
    }
    map
}

fn extract_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            return Some(title.trim().to_string());
        }
    }
    None
}

/// Parse structured connections: `- [[Target]] - relation [CONFIDENCE]`
fn parse_connections(
    content: &str,
    filename_to_id: &HashMap<String, String>,
) -> Vec<(String, String, Confidence)> {
    let mut edges = Vec::new();
    let mut in_connections = false;

    let conn_re: Regex = Regex::new(
        r"- \[\[([^\]|]+)(?:\|[^\]]+)?\]\]\s*-\s*(\S+)\s*\[(\w+)\]"
    ).unwrap();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "## Connections" {
            in_connections = true;
            continue;
        }
        if in_connections && trimmed.starts_with("## ") {
            break;
        }
        if !in_connections {
            continue;
        }

        if let Some(cap) = conn_re.captures(trimmed) {
            let target_name = cap.get(1).unwrap().as_str();
            let relation = cap.get(2).unwrap().as_str().to_string();
            let confidence_str = cap.get(3).unwrap().as_str();

            let confidence = match confidence_str {
                "INFERRED" => Confidence::INFERRED,
                "AMBIGUOUS" => Confidence::AMBIGUOUS,
                _ => Confidence::EXTRACTED,
            };

            if let Some(target_id) = filename_to_id.get(target_name) {
                edges.push((target_id.clone(), relation, confidence));
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\nid: foo\nfile_type: code\nsource_file: foo.py\n---\n# Foo\n";
        let fm = parse_frontmatter(content);
        assert_eq!(fm.get("id").unwrap(), "foo");
        assert_eq!(fm.get("file_type").unwrap(), "code");
    }

    #[test]
    fn test_extract_title() {
        assert_eq!(extract_title("---\n---\n# Hello\nBody"), Some("Hello".to_string()));
        assert_eq!(extract_title("No heading"), None);
    }

    #[test]
    fn test_load_vault_round_trip() {
        let dir = TempDir::new().unwrap();

        // Write two node files
        std::fs::write(
            dir.path().join("Alpha.md"),
            "---\nid: alpha\nfile_type: code\nsource_file: a.py\n---\n# Alpha\n\n## Connections\n- [[Beta]] - calls [EXTRACTED]\n",
        ).unwrap();
        std::fs::write(
            dir.path().join("Beta.md"),
            "---\nid: beta\nfile_type: code\nsource_file: b.py\n---\n# Beta\n",
        ).unwrap();

        let graph = load_graph_from_vault(dir.path()).unwrap();
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        assert!(graph.get_node("alpha").is_some());
        assert!(graph.get_node("beta").is_some());
    }

    #[test]
    fn test_cache_staleness() {
        let dir = TempDir::new().unwrap();
        let cache = dir.path().join("graph.json");

        // No cache → stale
        assert!(is_cache_stale(dir.path(), &cache));

        // Create cache
        std::fs::write(&cache, "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Add newer .md → stale
        std::fs::write(dir.path().join("test.md"), "# Test").unwrap();
        assert!(is_cache_stale(dir.path(), &cache));
    }
}
