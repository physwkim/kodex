use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Knowledge types that Claude can accumulate
// ---------------------------------------------------------------------------

/// Categories of learnable knowledge.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeType {
    Architecture,
    Pattern,
    Decision,
    Convention,
    Coupling,
    Domain,
    Preference,
    BugPattern,
    TechDebt,
    Ops,
    Api,
    Performance,
    Roadmap,
    Context,
    Lesson,
    /// Any type not in the enum — stored as-is
    #[serde(untagged)]
    Custom(String),
}

impl std::fmt::Display for KnowledgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Architecture => write!(f, "architecture"),
            Self::Pattern => write!(f, "pattern"),
            Self::Decision => write!(f, "decision"),
            Self::Convention => write!(f, "convention"),
            Self::Coupling => write!(f, "coupling"),
            Self::Domain => write!(f, "domain"),
            Self::Preference => write!(f, "preference"),
            Self::BugPattern => write!(f, "bug_pattern"),
            Self::TechDebt => write!(f, "tech_debt"),
            Self::Ops => write!(f, "ops"),
            Self::Api => write!(f, "api"),
            Self::Performance => write!(f, "performance"),
            Self::Roadmap => write!(f, "roadmap"),
            Self::Context => write!(f, "context"),
            Self::Lesson => write!(f, "lesson"),
            Self::Custom(s) => write!(f, "{s}"),
        }
    }
}

/// A piece of learned knowledge.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Knowledge {
    pub uuid: String,
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

/// Store or reinforce a piece of knowledge directly in HDF5.
///
/// HDF5 is the source of truth. If knowledge with the same title exists,
/// increments observations and raises confidence.
pub fn learn(
    h5_path: &Path,
    knowledge_type: KnowledgeType,
    title: &str,
    description: &str,
    related_nodes: &[String],
    tags: &[String],
) -> crate::error::Result<()> {
    let nodes = if related_nodes.is_empty() {
        None
    } else {
        Some(related_nodes)
    };
    learn_with_uuid(h5_path, None, knowledge_type, title, description, nodes, tags).map(|_| ())
}

/// Learn with explicit UUID. Returns the UUID of the created/updated entry.
/// - uuid=Some → update existing entry
/// - uuid=None → create new entry with fresh UUID
///
/// `related_nodes`:
/// - `None` → don't touch existing links
/// - `Some(&[])` → clear all links
/// - `Some(&[...])` → replace links with these nodes
pub fn learn_with_uuid(
    h5_path: &Path,
    knowledge_uuid: Option<&str>,
    knowledge_type: KnowledgeType,
    title: &str,
    description: &str,
    related_nodes: Option<&[String]>,
    tags: &[String],
) -> crate::error::Result<String> {
    crate::storage::append_knowledge_with_uuid(
        h5_path,
        knowledge_uuid,
        title,
        &knowledge_type.to_string(),
        description,
        0.6,
        related_nodes,
        tags,
    )
}

/// Query knowledge by keyword, type, or tag. Reads from HDF5.
pub fn query_knowledge(h5_path: &Path, query: &str, type_filter: Option<&str>) -> Vec<Knowledge> {
    let data = match crate::storage::load(h5_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let query_lower = query.to_lowercase();

    let links = data.links;
    data.knowledge
        .into_iter()
        .filter(|k| {
            if let Some(tf) = type_filter {
                if k.knowledge_type != tf {
                    return false;
                }
            }
            if query.is_empty() {
                return true;
            }
            k.title.to_lowercase().contains(&query_lower)
                || k.description.to_lowercase().contains(&query_lower)
                || k.tags
                    .iter()
                    .any(|t| t.to_lowercase().contains(&query_lower))
        })
        .map(|k| {
            let related: Vec<String> = links
                .iter()
                .filter(|l| l.knowledge_uuid == k.uuid)
                .map(|l| l.node_uuid.clone())
                .collect();
            Knowledge {
                uuid: k.uuid,
                knowledge_type: parse_knowledge_type(&k.knowledge_type),
                title: k.title,
                description: k.description,
                confidence: k.confidence,
                observations: k.observations,
                related_nodes: related,
                tags: k.tags,
                first_seen: 0,
                last_seen: 0,
            }
        })
        .collect()
}

fn parse_knowledge_type(s: &str) -> KnowledgeType {
    match s {
        "architecture" => KnowledgeType::Architecture,
        "pattern" => KnowledgeType::Pattern,
        "decision" => KnowledgeType::Decision,
        "convention" => KnowledgeType::Convention,
        "coupling" => KnowledgeType::Coupling,
        "domain" => KnowledgeType::Domain,
        "preference" => KnowledgeType::Preference,
        "bug_pattern" => KnowledgeType::BugPattern,
        "tech_debt" => KnowledgeType::TechDebt,
        "ops" => KnowledgeType::Ops,
        "api" => KnowledgeType::Api,
        "performance" => KnowledgeType::Performance,
        "roadmap" => KnowledgeType::Roadmap,
        "context" => KnowledgeType::Context,
        "lesson" => KnowledgeType::Lesson,
        other => KnowledgeType::Custom(other.to_string()),
    }
}

/// Get a knowledge context summary from HDF5 for Claude.
pub fn knowledge_context(h5_path: &Path, max_items: usize) -> String {
    let items = query_knowledge(h5_path, "", None);
    build_index_content(&items, max_items)
}

fn build_index_content(items: &[Knowledge], max_items: usize) -> String {
    if items.is_empty() {
        return "# Knowledge Index\n\nNo knowledge accumulated yet.\n".to_string();
    }

    let mut ctx = format!("# Knowledge Index ({} items)\n\n", items.len());
    ctx.push_str("> Auto-generated. Read this file at session start. Details in individual _KNOWLEDGE_*.md files.\n\n");

    // Group by type
    let mut by_type: HashMap<String, Vec<&Knowledge>> = HashMap::new();
    for k in items.iter().take(max_items) {
        by_type
            .entry(k.knowledge_type.to_string())
            .or_default()
            .push(k);
    }

    let mut types: Vec<_> = by_type.into_iter().collect();
    types.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    for (type_name, items) in types {
        ctx.push_str(&format!("## {type_name}\n\n"));
        for k in items {
            let conf = (k.confidence * 100.0) as u32;
            // One line per item: title + first sentence of description
            let summary = k.description.lines().next().unwrap_or("");
            let summary = if summary.len() > 80 {
                let end = floor_char_boundary(summary, 80);
                format!("{}...", &summary[..end])
            } else {
                summary.to_string()
            };
            ctx.push_str(&format!(
                "- **{}** ({conf}%, {}x) — {summary}\n",
                k.title, k.observations
            ));
        }
        ctx.push('\n');
    }

    ctx
}

fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ---------------------------------------------------------------------------
// Staleness detection
// ---------------------------------------------------------------------------

/// Check knowledge entries for staleness and mark them.
/// A knowledge is stale if all linked node UUIDs no longer exist.
/// Returns count of entries marked as needs_review.
pub fn detect_stale_knowledge(h5_path: &Path) -> crate::error::Result<usize> {
    let mut data = crate::storage::load(h5_path)?;

    let valid_node_uuids: std::collections::HashSet<&str> = data
        .extraction
        .nodes
        .iter()
        .filter_map(|n| n.uuid.as_deref())
        .collect();

    let mut stale_count = 0;

    for entry in &mut data.knowledge {
        if entry.status == "obsolete" || entry.status == "needs_review" {
            continue;
        }

        // Find links for this knowledge
        let linked_uuids: Vec<&str> = data
            .links
            .iter()
            .filter(|l| l.knowledge_uuid == entry.uuid)
            .map(|l| l.node_uuid.as_str())
            .collect();

        if linked_uuids.is_empty() {
            continue; // Unlinked knowledge can't be stale by node reference
        }

        // Check if ALL linked nodes are gone
        let all_gone = linked_uuids
            .iter()
            .all(|uuid| !valid_node_uuids.contains(uuid));

        if all_gone {
            entry.status = "needs_review".to_string();
            // Decay confidence slightly
            entry.confidence *= 0.9;
            stale_count += 1;
        }
    }

    if stale_count > 0 {
        crate::storage::save(h5_path, &data)?;
    }

    Ok(stale_count)
}

// ---------------------------------------------------------------------------
// Knowledge relevance scoring
// ---------------------------------------------------------------------------

/// Score a knowledge entry's relevance to the current task context.
fn relevance_score(
    k: &Knowledge,
    touched_files: &[String],
    related_node_uuids: &std::collections::HashSet<String>,
    query_tokens: &[String],
) -> f64 {
    let mut score = 0.0;

    // Base confidence weight (0-30 points)
    score += k.confidence * 30.0;

    // Observation frequency (0-15 points, log scale)
    score += (k.observations as f64).ln().min(3.0) * 5.0;

    // Node link overlap (0-25 points)
    if !related_node_uuids.is_empty() {
        let linked: std::collections::HashSet<&str> =
            k.related_nodes.iter().map(|s| s.as_str()).collect();
        let overlap = related_node_uuids
            .iter()
            .filter(|u| linked.contains(u.as_str()))
            .count();
        if overlap > 0 {
            score += 25.0 * (overlap as f64 / related_node_uuids.len().max(1) as f64).min(1.0);
        }
    }

    // File mention in description/tags (0-20 points)
    for file in touched_files {
        let filename = file.rsplit('/').next().unwrap_or(file);
        if k.title.contains(filename)
            || k.description.contains(filename)
            || k.tags.iter().any(|t| t.contains(filename))
        {
            score += 20.0;
            break;
        }
    }

    // Query keyword match (0-10 points)
    if !query_tokens.is_empty() {
        let title_lower = k.title.to_lowercase();
        let desc_lower = k.description.to_lowercase();
        let matches = query_tokens
            .iter()
            .filter(|t| title_lower.contains(t.as_str()) || desc_lower.contains(t.as_str()))
            .count();
        score += 10.0 * (matches as f64 / query_tokens.len() as f64);
    }

    score
}

/// Recall knowledge ranked by relevance to the current task.
///
/// Input signals:
/// - `question`: natural language query
/// - `touched_files`: currently edited files
/// - `node_uuids`: UUIDs of nodes in the current neighborhood
/// - `max_items`: top-N to return
pub fn recall_for_task(
    h5_path: &Path,
    question: &str,
    touched_files: &[String],
    node_uuids: &[String],
    max_items: usize,
) -> Vec<Knowledge> {
    let data = match crate::storage::load(h5_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let query_tokens: Vec<String> = question
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(String::from)
        .collect();

    let node_uuid_set: std::collections::HashSet<String> =
        node_uuids.iter().cloned().collect();

    let links = &data.links;

    let mut scored: Vec<(f64, Knowledge)> = data
        .knowledge
        .iter()
        .filter(|k| k.status != "obsolete")
        .map(|k| {
            let related: Vec<String> = links
                .iter()
                .filter(|l| l.knowledge_uuid == k.uuid)
                .map(|l| l.node_uuid.clone())
                .collect();
            let knowledge = Knowledge {
                uuid: k.uuid.clone(),
                knowledge_type: parse_knowledge_type(&k.knowledge_type),
                title: k.title.clone(),
                description: k.description.clone(),
                confidence: k.confidence,
                observations: k.observations,
                related_nodes: related,
                tags: k.tags.clone(),
                first_seen: 0,
                last_seen: 0,
            };
            let score = relevance_score(&knowledge, touched_files, &node_uuid_set, &query_tokens);
            (score, knowledge)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(max_items).map(|(_, k)| k).collect()
}

/// Build a task-specific briefing for the agent.
///
/// Returns structured context: relevant knowledge, stale warnings, related patterns.
pub fn get_task_context(
    h5_path: &Path,
    question: &str,
    touched_files: &[String],
    max_items: usize,
) -> String {
    let data = match crate::storage::load(h5_path) {
        Ok(d) => d,
        Err(_) => return "No knowledge base found.".to_string(),
    };

    // Find node UUIDs related to touched files
    let file_node_uuids: Vec<String> = data
        .extraction
        .nodes
        .iter()
        .filter(|n| {
            touched_files.iter().any(|f| {
                let filename = f.rsplit('/').next().unwrap_or(f);
                n.source_file.contains(filename)
            })
        })
        .filter_map(|n| n.uuid.clone())
        .collect();

    let items = recall_for_task(h5_path, question, touched_files, &file_node_uuids, max_items);

    if items.is_empty() {
        return "No relevant knowledge found for this task.".to_string();
    }

    let mut ctx = String::new();

    // Relevant knowledge
    ctx.push_str(&format!("## Relevant Knowledge ({} items)\n\n", items.len()));
    for k in &items {
        let conf = (k.confidence * 100.0) as u32;
        let status_tag = if conf < 50 { " [tentative]" } else { "" };
        let summary = k.description.lines().next().unwrap_or("");
        let summary = if summary.len() > 100 {
            let end = floor_char_boundary(summary, 100);
            format!("{}...", &summary[..end])
        } else {
            summary.to_string()
        };
        ctx.push_str(&format!(
            "- **{}** ({conf}%{status_tag}) — {summary}\n",
            k.title
        ));
    }
    ctx.push('\n');

    // Stale warnings
    let stale: Vec<&Knowledge> = items
        .iter()
        .filter(|k| k.confidence < 0.4)
        .collect();
    if !stale.is_empty() {
        ctx.push_str("## Stale/Low-Confidence Warnings\n\n");
        for k in stale {
            ctx.push_str(&format!(
                "- ⚠ **{}** ({}%) — may be outdated\n",
                k.title,
                (k.confidence * 100.0) as u32
            ));
        }
        ctx.push('\n');
    }

    ctx
}

// ---------------------------------------------------------------------------
// Knowledge update APIs
// ---------------------------------------------------------------------------

/// Update specific fields on an existing knowledge entry (by UUID).
pub fn update_knowledge(
    h5_path: &Path,
    knowledge_uuid: &str,
    updates: &KnowledgeUpdates,
) -> crate::error::Result<()> {
    let mut data = crate::storage::load(h5_path)?;

    let entry = data
        .knowledge
        .iter_mut()
        .find(|k| k.uuid == knowledge_uuid)
        .ok_or_else(|| {
            crate::error::KodexError::Other(format!("Knowledge UUID not found: {knowledge_uuid}"))
        })?;

    if let Some(status) = &updates.status {
        entry.status = status.clone();
    }
    if let Some(scope) = &updates.scope {
        entry.scope = scope.clone();
    }
    if let Some(applies_when) = &updates.applies_when {
        entry.applies_when = applies_when.clone();
    }
    if let Some(superseded_by) = &updates.superseded_by {
        entry.superseded_by = superseded_by.clone();
        entry.status = "obsolete".to_string();
    }
    if updates.validate {
        entry.last_validated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    crate::storage::save(h5_path, &data)
}

/// Partial update fields for update_knowledge.
#[derive(Debug, Default)]
pub struct KnowledgeUpdates {
    pub status: Option<String>,
    pub scope: Option<String>,
    pub applies_when: Option<String>,
    pub superseded_by: Option<String>,
    pub validate: bool,
}

/// Link knowledge to specific nodes (additive — doesn't remove existing links).
pub fn link_knowledge_to_nodes(
    h5_path: &Path,
    knowledge_uuid: &str,
    node_uuids: &[String],
    relation: &str,
) -> crate::error::Result<()> {
    let mut data = crate::storage::load(h5_path)?;

    // Verify knowledge exists
    if !data.knowledge.iter().any(|k| k.uuid == knowledge_uuid) {
        return Err(crate::error::KodexError::Other(format!(
            "Knowledge UUID not found: {knowledge_uuid}"
        )));
    }

    for node_uuid in node_uuids {
        // Don't add duplicate links
        let exists = data.links.iter().any(|l| {
            l.knowledge_uuid == knowledge_uuid
                && l.node_uuid == *node_uuid
                && l.relation == relation
        });
        if !exists {
            data.links.push(crate::types::KnowledgeLink {
                knowledge_uuid: knowledge_uuid.to_string(),
                node_uuid: node_uuid.clone(),
                relation: relation.to_string(),
                target_type: String::new(),
            });
        }
    }

    crate::storage::save(h5_path, &data)
}

/// Clear all links for a given knowledge entry.
pub fn clear_knowledge_links(
    h5_path: &Path,
    knowledge_uuid: &str,
) -> crate::error::Result<usize> {
    let mut data = crate::storage::load(h5_path)?;
    let before = data.links.len();
    data.links.retain(|l| l.knowledge_uuid != knowledge_uuid);
    let removed = before - data.links.len();
    if removed > 0 {
        crate::storage::save(h5_path, &data)?;
    }
    Ok(removed)
}

/// Remove a specific link by knowledge_uuid + target_uuid + relation.
pub fn remove_link(
    h5_path: &Path,
    knowledge_uuid: &str,
    target_uuid: &str,
    relation: Option<&str>,
) -> crate::error::Result<bool> {
    let mut data = crate::storage::load(h5_path)?;
    let before = data.links.len();
    data.links.retain(|l| {
        !(l.knowledge_uuid == knowledge_uuid
            && l.node_uuid == target_uuid
            && relation.is_none_or(|r| l.relation == r))
    });
    let removed = before != data.links.len();
    if removed {
        crate::storage::save(h5_path, &data)?;
    }
    Ok(removed)
}

/// Link two knowledge entries together (knowledge ↔ knowledge).
pub fn link_knowledge_to_knowledge(
    h5_path: &Path,
    source_uuid: &str,
    target_uuid: &str,
    relation: &str,
    bidirectional: bool,
) -> crate::error::Result<()> {
    let mut data = crate::storage::load(h5_path)?;

    // Verify both exist
    let source_exists = data.knowledge.iter().any(|k| k.uuid == source_uuid);
    let target_exists = data.knowledge.iter().any(|k| k.uuid == target_uuid);
    if !source_exists {
        return Err(crate::error::KodexError::Other(format!(
            "Source knowledge not found: {source_uuid}"
        )));
    }
    if !target_exists {
        return Err(crate::error::KodexError::Other(format!(
            "Target knowledge not found: {target_uuid}"
        )));
    }

    // Add forward link
    let exists = data.links.iter().any(|l| {
        l.knowledge_uuid == source_uuid
            && l.node_uuid == target_uuid
            && l.relation == relation
            && l.is_knowledge_link()
    });
    if !exists {
        data.links.push(crate::types::KnowledgeLink {
            knowledge_uuid: source_uuid.to_string(),
            node_uuid: target_uuid.to_string(),
            relation: relation.to_string(),
            target_type: "knowledge".to_string(),
        });
    }

    // Add reverse link if bidirectional
    if bidirectional {
        let reverse_rel = match relation {
            "supersedes" => "superseded_by",
            "superseded_by" => "supersedes",
            "depends_on" => "depended_by",
            "supports" => "supported_by",
            "contradicts" => "contradicts",
            other => other,
        };
        let rev_exists = data.links.iter().any(|l| {
            l.knowledge_uuid == target_uuid
                && l.node_uuid == source_uuid
                && l.relation == reverse_rel
                && l.is_knowledge_link()
        });
        if !rev_exists {
            data.links.push(crate::types::KnowledgeLink {
                knowledge_uuid: target_uuid.to_string(),
                node_uuid: source_uuid.to_string(),
                relation: reverse_rel.to_string(),
                target_type: "knowledge".to_string(),
            });
        }
    }

    crate::storage::save(h5_path, &data)
}

/// Get all knowledge entries connected to a given knowledge UUID.
pub fn knowledge_neighbors(h5_path: &Path, knowledge_uuid: &str) -> Vec<(String, String, String)> {
    let data = match crate::storage::load(h5_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    data.links
        .iter()
        .filter(|l| {
            l.is_knowledge_link()
                && (l.knowledge_uuid == knowledge_uuid || l.node_uuid == knowledge_uuid)
        })
        .map(|l| {
            let other = if l.knowledge_uuid == knowledge_uuid {
                l.node_uuid.clone()
            } else {
                l.knowledge_uuid.clone()
            };
            let direction = if l.knowledge_uuid == knowledge_uuid {
                "outgoing"
            } else {
                "incoming"
            };
            (other, l.relation.clone(), direction.to_string())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_h5(dir: &std::path::Path) -> std::path::PathBuf {
        let h5_path = dir.join("test.h5");
        // Create a minimal HDF5 with empty graph
        let extraction = crate::types::ExtractionResult::default();
        let graph = crate::graph::build_from_extraction(&extraction);
        let communities = crate::cluster::cluster(&graph);
        crate::storage::save_hdf5(&graph, &communities, &h5_path).unwrap();
        h5_path
    }

    #[test]
    fn test_learn_and_load() {
        let dir = TempDir::new().unwrap();
        let h5 = make_test_h5(dir.path());

        learn(
            &h5,
            KnowledgeType::Pattern,
            "Repository Pattern",
            "All data access goes through Repository classes",
            &["user_repo".to_string()],
            &["architecture".to_string()],
        )
        .unwrap();

        let items = query_knowledge(&h5, "", None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Repository Pattern");
        assert_eq!(items[0].observations, 1);
        let conf1 = items[0].confidence;
        let uuid = items[0].uuid.clone();

        // Reinforce using UUID
        learn_with_uuid(
            &h5,
            Some(&uuid),
            KnowledgeType::Pattern,
            "Repository Pattern",
            "Confirmed: ProductRepo also follows this",
            Some(&["product_repo".to_string()]),
            &[],
        )
        .unwrap();

        let items = query_knowledge(&h5, "", None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].observations, 2);
        assert!(items[0].confidence > conf1, "Confidence should increase");
    }

    #[test]
    fn test_query_knowledge() {
        let dir = TempDir::new().unwrap();
        let h5 = make_test_h5(dir.path());

        learn(
            &h5,
            KnowledgeType::Pattern,
            "Singleton",
            "Global state",
            &[],
            &["design".to_string()],
        )
        .unwrap();
        learn(
            &h5,
            KnowledgeType::Convention,
            "Error Handling",
            "Use AppError",
            &[],
            &["rust".to_string()],
        )
        .unwrap();
        learn(
            &h5,
            KnowledgeType::Decision,
            "JWT Auth",
            "Chose JWT",
            &[],
            &["auth".to_string()],
        )
        .unwrap();

        let all = query_knowledge(&h5, "", None);
        assert_eq!(all.len(), 3);

        let patterns = query_knowledge(&h5, "", Some("pattern"));
        assert_eq!(patterns.len(), 1);

        let auth = query_knowledge(&h5, "auth", None);
        assert_eq!(auth.len(), 1);
    }

    #[test]
    fn test_knowledge_context() {
        let dir = TempDir::new().unwrap();
        let h5 = make_test_h5(dir.path());

        learn(
            &h5,
            KnowledgeType::Pattern,
            "Observer",
            "Event-driven",
            &[],
            &[],
        )
        .unwrap();
        learn(
            &h5,
            KnowledgeType::Preference,
            "Functional Style",
            "User prefers FP",
            &[],
            &[],
        )
        .unwrap();

        let ctx = knowledge_context(&h5, 10);
        assert!(ctx.contains("Observer"));
        assert!(ctx.contains("Functional Style"));
        assert!(ctx.contains("Knowledge Index"));
    }
}
