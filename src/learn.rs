use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::id::make_id;

// ---------------------------------------------------------------------------
// Knowledge types that Claude can accumulate
// ---------------------------------------------------------------------------

/// Categories of learnable knowledge.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeType {
    /// Architectural pattern: "Repository pattern for data access"
    Pattern,
    /// Design decision: "Chose JWT over sessions because of microservices"
    Decision,
    /// Code convention: "All errors wrapped in AppError"
    Convention,
    /// Coupling: "Module A always changes with Module B"
    Coupling,
    /// User preference: "Prefers functional style, avoids deep inheritance"
    Preference,
    /// Bug pattern: "Off-by-one errors common in pagination code"
    BugPattern,
    /// Domain concept: "A 'trade' can be in states: pending, filled, cancelled"
    Domain,
}

impl std::fmt::Display for KnowledgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pattern => write!(f, "pattern"),
            Self::Decision => write!(f, "decision"),
            Self::Convention => write!(f, "convention"),
            Self::Coupling => write!(f, "coupling"),
            Self::Preference => write!(f, "preference"),
            Self::BugPattern => write!(f, "bug_pattern"),
            Self::Domain => write!(f, "domain"),
        }
    }
}

/// A piece of learned knowledge.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Knowledge {
    pub knowledge_type: KnowledgeType,
    pub title: String,
    pub description: String,
    /// Node IDs this knowledge relates to
    pub related_nodes: Vec<String>,
    /// How confident: 0.0–1.0 (accumulates with repeated observations)
    pub confidence: f64,
    /// How many times this was observed
    pub observations: u32,
    /// Tags for querying
    pub tags: Vec<String>,
    /// When first observed (unix timestamp)
    pub first_seen: u64,
    /// When last reinforced
    pub last_seen: u64,
}

// ---------------------------------------------------------------------------
// Knowledge store — reads/writes vault .md files
// ---------------------------------------------------------------------------

const KNOWLEDGE_PREFIX: &str = "_KNOWLEDGE_";

/// Load all accumulated knowledge from the vault.
pub fn load_knowledge(vault_dir: &Path) -> Vec<Knowledge> {
    let mut items = Vec::new();
    let entries = match std::fs::read_dir(vault_dir) {
        Ok(e) => e,
        Err(_) => return items,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if !filename.starts_with(KNOWLEDGE_PREFIX) {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(k) = parse_knowledge_note(&content) {
                items.push(k);
            }
        }
    }

    items.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    items
}

/// Store or reinforce a piece of knowledge.
///
/// If knowledge with the same title exists, increments observations and
/// raises confidence. Otherwise creates new.
pub fn learn(
    vault_dir: &Path,
    graph_path: Option<&Path>,
    knowledge_type: KnowledgeType,
    title: &str,
    description: &str,
    related_nodes: &[String],
    tags: &[String],
) -> crate::error::Result<PathBuf> {
    std::fs::create_dir_all(vault_dir)?;

    let safe_name = title
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ' '], "_");
    let filename = format!("{KNOWLEDGE_PREFIX}{safe_name}.md");
    let path = vault_dir.join(&filename);
    let now = timestamp();

    // Check if exists — reinforce if so
    let mut knowledge = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        parse_knowledge_note(&content).unwrap_or_else(|| Knowledge {
            knowledge_type: knowledge_type.clone(),
            title: title.to_string(),
            description: description.to_string(),
            related_nodes: related_nodes.to_vec(),
            confidence: 0.5,
            observations: 0,
            tags: tags.to_vec(),
            first_seen: now,
            last_seen: now,
        })
    } else {
        Knowledge {
            knowledge_type: knowledge_type.clone(),
            title: title.to_string(),
            description: description.to_string(),
            related_nodes: related_nodes.to_vec(),
            confidence: 0.5,
            observations: 0,
            tags: tags.to_vec(),
            first_seen: now,
            last_seen: now,
        }
    };

    // Reinforce
    knowledge.observations += 1;
    knowledge.last_seen = now;
    // Confidence grows asymptotically toward 1.0
    knowledge.confidence = 1.0 - (1.0 - knowledge.confidence) * 0.8;
    // Merge new description if different
    if knowledge.description != description && !description.is_empty() {
        knowledge.description = format!("{}\n\n---\n\n{}", knowledge.description, description);
    }
    // Merge related nodes
    for node in related_nodes {
        if !knowledge.related_nodes.contains(node) {
            knowledge.related_nodes.push(node.clone());
        }
    }
    // Merge tags
    for tag in tags {
        if !knowledge.tags.contains(tag) {
            knowledge.tags.push(tag.clone());
        }
    }

    // Write vault note
    write_knowledge_note(&path, &knowledge)?;

    // Also add to graph.json if provided
    if let Some(gp) = graph_path {
        let _ = add_knowledge_to_graph(gp, &knowledge);
    }

    Ok(path)
}

/// Query knowledge by keyword, type, or tag.
pub fn query_knowledge(
    vault_dir: &Path,
    query: &str,
    type_filter: Option<&str>,
) -> Vec<Knowledge> {
    let all = load_knowledge(vault_dir);
    let query_lower = query.to_lowercase();

    all.into_iter()
        .filter(|k| {
            // Type filter
            if let Some(tf) = type_filter {
                if k.knowledge_type.to_string() != tf {
                    return false;
                }
            }
            // Keyword match
            if query.is_empty() {
                return true;
            }
            k.title.to_lowercase().contains(&query_lower)
                || k.description.to_lowercase().contains(&query_lower)
                || k.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
        })
        .collect()
}

/// Get a context summary for Claude: what has been learned so far.
///
/// Returns a compact text that Claude can read at the start of a session
/// to recall previously accumulated knowledge.
pub fn knowledge_context(vault_dir: &Path, max_items: usize) -> String {
    let items = load_knowledge(vault_dir);
    if items.is_empty() {
        return String::new();
    }

    let mut ctx = format!("## Accumulated Knowledge ({} items)\n\n", items.len());
    for k in items.iter().take(max_items) {
        let type_str = k.knowledge_type.to_string();
        let conf = (k.confidence * 100.0) as u32;
        ctx.push_str(&format!(
            "- **[{type_str}]** {} ({}% confidence, {} observations)\n",
            k.title, conf, k.observations
        ));
        // First line of description only
        if let Some(first_line) = k.description.lines().next() {
            if first_line.len() > 100 {
                ctx.push_str(&format!("  {:.100}...\n", first_line));
            } else {
                ctx.push_str(&format!("  {first_line}\n"));
            }
        }
    }
    ctx
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn write_knowledge_note(path: &Path, k: &Knowledge) -> crate::error::Result<()> {
    let wikilinks: Vec<String> = k.related_nodes.iter().map(|n| format!("[[{n}]]")).collect();
    let tags: Vec<String> = k.tags.iter().map(|t| format!("#{t}")).collect();
    let type_str = k.knowledge_type.to_string();

    let md = format!(
        "---\n\
         type: knowledge\n\
         knowledge_type: {type_str}\n\
         confidence: {conf:.2}\n\
         observations: {obs}\n\
         first_seen: {first}\n\
         last_seen: {last}\n\
         tags: [{tags_csv}]\n\
         related_nodes: [{nodes_csv}]\n\
         ---\n\n\
         # {title}\n\n\
         {desc}\n\n\
         ## Related\n\n\
         {related}\n\n\
         {tags_inline}\n",
        conf = k.confidence,
        obs = k.observations,
        first = k.first_seen,
        last = k.last_seen,
        tags_csv = k.tags.join(", "),
        nodes_csv = k.related_nodes.join(", "),
        title = k.title,
        desc = k.description,
        related = if wikilinks.is_empty() {
            "(none)".to_string()
        } else {
            wikilinks.join(" ")
        },
        tags_inline = tags.join(" "),
    );

    std::fs::write(path, md)?;
    Ok(())
}

fn parse_knowledge_note(content: &str) -> Option<Knowledge> {
    let fm = parse_frontmatter(content);
    let knowledge_type = match fm.get("knowledge_type").map(|s| s.as_str()) {
        Some("pattern") => KnowledgeType::Pattern,
        Some("decision") => KnowledgeType::Decision,
        Some("convention") => KnowledgeType::Convention,
        Some("coupling") => KnowledgeType::Coupling,
        Some("preference") => KnowledgeType::Preference,
        Some("bug_pattern") => KnowledgeType::BugPattern,
        Some("domain") => KnowledgeType::Domain,
        _ => return None,
    };

    let title = extract_title(content)?;
    let description = extract_body(content);
    let confidence = fm.get("confidence").and_then(|s| s.parse().ok()).unwrap_or(0.5);
    let observations = fm.get("observations").and_then(|s| s.parse().ok()).unwrap_or(1);
    let first_seen = fm.get("first_seen").and_then(|s| s.parse().ok()).unwrap_or(0);
    let last_seen = fm.get("last_seen").and_then(|s| s.parse().ok()).unwrap_or(0);

    let related_nodes = fm
        .get("related_nodes")
        .map(|s| {
            s.trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let tags = fm
        .get("tags")
        .map(|s| {
            s.trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Some(Knowledge {
        knowledge_type,
        title,
        description,
        related_nodes,
        confidence,
        observations,
        tags,
        first_seen,
        last_seen,
    })
}

fn add_knowledge_to_graph(graph_path: &Path, k: &Knowledge) -> crate::error::Result<()> {
    if !graph_path.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(graph_path)?;
    let mut data: serde_json::Value = serde_json::from_str(&text)?;

    let node_id = make_id(&["knowledge", &k.title]);

    let nodes = data.get_mut("nodes").and_then(|v| v.as_array_mut());
    if let Some(nodes) = nodes {
        // Upsert
        nodes.retain(|n| n.get("id").and_then(|v| v.as_str()) != Some(&node_id));
        nodes.push(serde_json::json!({
            "id": node_id,
            "label": k.title,
            "file_type": "rationale",
            "source_file": format!("knowledge/{}", k.knowledge_type),
            "confidence": "INFERRED",
            "confidence_score": k.confidence,
            "knowledge_type": k.knowledge_type.to_string(),
            "observations": k.observations,
        }));
    }

    let links = data.get_mut("links").and_then(|v| v.as_array_mut());
    if let Some(links) = links {
        // Remove old edges from this knowledge node
        links.retain(|e| e.get("source").and_then(|v| v.as_str()) != Some(&node_id));
        // Add edges to related nodes
        for related in &k.related_nodes {
            links.push(serde_json::json!({
                "source": node_id,
                "target": related,
                "relation": format!("knowledge_{}", k.knowledge_type),
                "confidence": "INFERRED",
                "source_file": "knowledge",
                "confidence_score": k.confidence,
                "weight": k.confidence,
            }));
        }
    }

    let json = serde_json::to_string_pretty(&data)
        .map_err(|e| crate::error::GraphifyError::Other(e.to_string()))?;
    let tmp = graph_path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, graph_path).or_else(|_| {
        std::fs::copy(&tmp, graph_path)?;
        let _ = std::fs::remove_file(&tmp);
        Ok(())
    })
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
    for line in content[4..end].lines() {
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
        if let Some(title) = line.trim().strip_prefix("# ") {
            return Some(title.trim().to_string());
        }
    }
    None
}

fn extract_body(content: &str) -> String {
    let mut body = String::new();
    let mut past_title = false;
    let mut past_frontmatter = false;

    for line in content.lines() {
        if !past_frontmatter {
            if line.trim() == "---" {
                past_frontmatter = true;
                continue;
            }
            continue;
        }
        // Skip second ---
        if line.trim() == "---" && !past_title {
            continue;
        }
        if line.starts_with("# ") && !past_title {
            past_title = true;
            continue;
        }
        if past_title {
            if line.starts_with("## Related") || line.starts_with("## Tags") {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
    }
    body.trim().to_string()
}

fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_learn_and_load() {
        let dir = TempDir::new().unwrap();

        // First observation
        learn(
            dir.path(), None,
            KnowledgeType::Pattern,
            "Repository Pattern",
            "All data access goes through Repository classes",
            &["user_repo".to_string(), "order_repo".to_string()],
            &["architecture".to_string()],
        ).unwrap();

        let items = load_knowledge(dir.path());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Repository Pattern");
        assert_eq!(items[0].observations, 1);
        let conf1 = items[0].confidence;

        // Second observation — reinforces
        learn(
            dir.path(), None,
            KnowledgeType::Pattern,
            "Repository Pattern",
            "Confirmed: ProductRepo also follows this pattern",
            &["product_repo".to_string()],
            &[],
        ).unwrap();

        let items = load_knowledge(dir.path());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].observations, 2);
        assert!(items[0].confidence > conf1, "Confidence should increase");
        assert_eq!(items[0].related_nodes.len(), 3); // merged
    }

    #[test]
    fn test_query_knowledge() {
        let dir = TempDir::new().unwrap();

        learn(dir.path(), None, KnowledgeType::Pattern, "Singleton", "Global state", &[], &["design".to_string()]).unwrap();
        learn(dir.path(), None, KnowledgeType::Convention, "Error Handling", "Use AppError", &[], &["rust".to_string()]).unwrap();
        learn(dir.path(), None, KnowledgeType::Decision, "JWT Auth", "Chose JWT for stateless", &[], &["auth".to_string()]).unwrap();

        let all = query_knowledge(dir.path(), "", None);
        assert_eq!(all.len(), 3);

        let patterns = query_knowledge(dir.path(), "", Some("pattern"));
        assert_eq!(patterns.len(), 1);

        let auth = query_knowledge(dir.path(), "auth", None);
        assert_eq!(auth.len(), 1);
    }

    #[test]
    fn test_knowledge_context() {
        let dir = TempDir::new().unwrap();

        learn(dir.path(), None, KnowledgeType::Pattern, "Observer", "Event-driven", &[], &[]).unwrap();
        learn(dir.path(), None, KnowledgeType::Preference, "Functional Style", "User prefers FP", &[], &[]).unwrap();

        let ctx = knowledge_context(dir.path(), 10);
        assert!(ctx.contains("Observer"));
        assert!(ctx.contains("Functional Style"));
        assert!(ctx.contains("Accumulated Knowledge"));
    }
}
