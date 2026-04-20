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
