//! Rename / refactor detection.
//!
//! Knowledge entries link to code via `KnowledgeLink::node_uuid`. When code
//! is refactored — function renamed, file moved, method extracted — the
//! link's target uuid no longer exists, the link goes "orphaned", and the
//! memory becomes silently disconnected.
//!
//! `detect_renames` finds these orphans and proposes replacement node uuids
//! using the breadcrumbs the link captured at creation time
//! (`linked_logical_key`, `linked_body_hash`) plus structural similarity to
//! current nodes.

use std::collections::HashMap;

use crate::analyze::compare::tokenize_label;
use crate::graph::KodexGraph;
use crate::types::KnowledgeLink;

/// One replacement candidate for an orphaned link.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RenameCandidate {
    pub node_uuid: String,
    pub label: String,
    pub logical_key: Option<String>,
    pub source_file: String,
    /// 0..1 — sums of weighted signals. >=0.7 is usually a clean rename.
    pub confidence: f32,
    pub signals: Vec<String>,
}

/// One orphaned link with replacement suggestions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OrphanedLink {
    pub knowledge_uuid: String,
    /// Title of the orphaned knowledge entry (when available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_title: Option<String>,
    pub lost_node_uuid: String,
    /// Logical key snapshot from the link table — usually
    /// `<source_file>::<scope>.<method>` form.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lost_logical_key: String,
    pub candidates: Vec<RenameCandidate>,
}

#[derive(Debug, Clone)]
pub struct DetectQuery {
    /// Cap on returned orphans (sorted by best-candidate confidence desc).
    pub top_n: usize,
    /// Cap on candidates returned per orphan.
    pub candidates_per_orphan: usize,
    /// Drop candidates below this confidence.
    pub min_confidence: f32,
    /// Optional source-file substring to scope the scan (only orphans whose
    /// `linked_logical_key` matches will be processed).
    pub source_pattern: Option<String>,
}

impl Default for DetectQuery {
    fn default() -> Self {
        Self {
            top_n: 50,
            candidates_per_orphan: 3,
            min_confidence: 0.3,
            source_pattern: None,
        }
    }
}

/// Run rename detection. Returns orphaned links with ranked replacement
/// candidates from `graph`. Knowledge titles come from `knowledge_titles`
/// (uuid → title map; pass an empty map to omit titles).
pub fn detect_renames(
    graph: &KodexGraph,
    links: &[KnowledgeLink],
    knowledge_titles: &HashMap<String, String>,
    query: &DetectQuery,
) -> Vec<OrphanedLink> {
    // Set of node uuids that are still in the graph.
    let valid_uuids: std::collections::HashSet<String> = graph
        .node_ids()
        .filter_map(|id| graph.get_node(id))
        .filter_map(|n| n.uuid.clone())
        .collect();

    let pat = query.source_pattern.as_deref().map(str::to_lowercase);

    // Build candidate pool — every node with a uuid, indexed by source_file
    // so the per-orphan loop can narrow quickly.
    let mut by_source: HashMap<String, Vec<NodeRef>> = HashMap::new();
    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(id) {
            if let Some(uuid) = &node.uuid {
                by_source
                    .entry(node.source_file.clone())
                    .or_default()
                    .push(NodeRef {
                        uuid: uuid.clone(),
                        label: node.label.clone(),
                        logical_key: node.logical_key.clone(),
                        source_file: node.source_file.clone(),
                        body_hash: node.body_hash.clone(),
                        tokens: tokenize_label(&node.label),
                    });
            }
        }
    }

    let mut orphans: Vec<OrphanedLink> = Vec::new();
    for link in links {
        if link.is_knowledge_link() {
            continue;
        }
        if valid_uuids.contains(&link.node_uuid) {
            continue;
        }
        if let Some(p) = pat.as_deref() {
            if !link.linked_logical_key.to_lowercase().contains(p) {
                continue;
            }
        }

        let (lost_source, lost_label) = parse_logical_key(&link.linked_logical_key);
        let lost_tokens = tokenize_label(&lost_label);

        let mut candidates: Vec<RenameCandidate> = Vec::new();
        let same_file_pool: Vec<&NodeRef> = if let Some(src) = lost_source.as_deref() {
            by_source.get(src).into_iter().flatten().collect()
        } else {
            Vec::new()
        };

        for nr in &same_file_pool {
            let mut signals: Vec<String> = vec!["same_source_file".into()];
            let mut score = 0.4_f32;
            // Token Jaccard between lost label and candidate.
            let j = label_jaccard(&lost_tokens, &nr.tokens);
            if j > 0.0 {
                signals.push(format!("token_jaccard_{:.2}", j));
                score += 0.4 * j;
            }
            // Body-hash equality is the strongest signal — same content,
            // different uuid means the node was re-extracted (e.g. logical
            // key changed because of a rename above).
            if !link.linked_body_hash.is_empty()
                && nr.body_hash.as_deref() == Some(link.linked_body_hash.as_str())
            {
                signals.push("body_hash_match".into());
                score = score.max(0.95);
            }
            if score >= query.min_confidence {
                candidates.push(RenameCandidate {
                    node_uuid: nr.uuid.clone(),
                    label: nr.label.clone(),
                    logical_key: nr.logical_key.clone(),
                    source_file: nr.source_file.clone(),
                    confidence: score.min(1.0),
                    signals,
                });
            }
        }

        // Cross-file body-hash fallback: find any node with matching body
        // hash even if the file moved. Skip when no body hash recorded.
        if !link.linked_body_hash.is_empty() {
            for (file, pool) in &by_source {
                if Some(file.as_str()) == lost_source.as_deref() {
                    continue;
                }
                for nr in pool {
                    if nr.body_hash.as_deref() == Some(link.linked_body_hash.as_str()) {
                        candidates.push(RenameCandidate {
                            node_uuid: nr.uuid.clone(),
                            label: nr.label.clone(),
                            logical_key: nr.logical_key.clone(),
                            source_file: nr.source_file.clone(),
                            confidence: 0.9,
                            signals: vec!["body_hash_match_cross_file".into()],
                        });
                    }
                }
            }
        }

        candidates.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(query.candidates_per_orphan.max(1));
        if candidates.is_empty() {
            continue;
        }

        orphans.push(OrphanedLink {
            knowledge_uuid: link.knowledge_uuid.clone(),
            knowledge_title: knowledge_titles.get(&link.knowledge_uuid).cloned(),
            lost_node_uuid: link.node_uuid.clone(),
            lost_logical_key: link.linked_logical_key.clone(),
            candidates,
        });
    }

    // Best-candidate confidence as the orphan ranking key.
    orphans.sort_by(|a, b| {
        let ac = a.candidates.first().map(|c| c.confidence).unwrap_or(0.0);
        let bc = b.candidates.first().map(|c| c.confidence).unwrap_or(0.0);
        bc.partial_cmp(&ac).unwrap_or(std::cmp::Ordering::Equal)
    });
    orphans.truncate(query.top_n);
    orphans
}

struct NodeRef {
    uuid: String,
    label: String,
    logical_key: Option<String>,
    source_file: String,
    body_hash: Option<String>,
    tokens: Vec<String>,
}

/// Split `<source_file>::<rest>` (where rest is the symbol path).
/// Returns `(Some(source_file), label_segment)`. `label_segment` is the
/// last `::` or `.` separated component of `rest` so token Jaccard can
/// focus on the leaf identifier.
fn parse_logical_key(lk: &str) -> (Option<String>, String) {
    if lk.is_empty() {
        return (None, String::new());
    }
    let (source, rest) = match lk.split_once("::") {
        Some((s, r)) => (Some(s.to_string()), r),
        None => return (None, lk.to_string()),
    };
    let leaf = rest
        .rsplit_once("::")
        .map(|(_, leaf)| leaf)
        .or_else(|| rest.rsplit_once('.').map(|(_, leaf)| leaf))
        .unwrap_or(rest);
    (source, leaf.to_string())
}

fn label_jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let sa: std::collections::HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let sb: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let i = sa.intersection(&sb).count();
    let u = sa.union(&sb).count();
    if u == 0 {
        0.0
    } else {
        i as f32 / u as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_from_extraction;
    use crate::types::{Confidence, ExtractionResult, FileType, Node};

    fn mk_node(id: &str, label: &str, source_file: &str, uuid: &str) -> Node {
        Node {
            id: id.into(),
            label: label.into(),
            file_type: FileType::Code,
            source_file: source_file.into(),
            source_location: Some("L1".into()),
            confidence: Some(Confidence::EXTRACTED),
            confidence_score: Some(1.0),
            community: None,
            norm_label: None,
            degree: None,
            uuid: Some(uuid.into()),
            fingerprint: None,
            logical_key: None,
            body_hash: None,
        }
    }

    #[test]
    fn parse_logical_key_extracts_source_and_leaf() {
        let (src, leaf) = parse_logical_key("src/foo.rs::Server.handle_request");
        assert_eq!(src.as_deref(), Some("src/foo.rs"));
        assert_eq!(leaf, "handle_request");
    }

    #[test]
    fn finds_rename_via_token_jaccard_in_same_file() {
        let extraction = ExtractionResult {
            nodes: vec![mk_node(
                "n1",
                "handle_search_request",
                "src/server.rs",
                "uuid-new",
            )],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);
        let links = vec![KnowledgeLink {
            knowledge_uuid: "k1".into(),
            node_uuid: "uuid-old".into(), // no longer exists
            relation: "documents".into(),
            target_type: String::new(),
            confidence: 1.0,
            created_at: 0,
            linked_body_hash: String::new(),
            reason: String::new(),
            source: String::new(),
            linked_logical_key: "src/server.rs::handle_search".into(),
        }];

        let orphans = detect_renames(&graph, &links, &HashMap::new(), &DetectQuery::default());
        assert_eq!(orphans.len(), 1);
        let orph = &orphans[0];
        assert_eq!(orph.lost_node_uuid, "uuid-old");
        assert!(!orph.candidates.is_empty());
        let c = &orph.candidates[0];
        assert_eq!(c.label, "handle_search_request");
        assert!(c.signals.iter().any(|s| s == "same_source_file"));
        assert!(c.signals.iter().any(|s| s.starts_with("token_jaccard")));
    }

    #[test]
    fn body_hash_match_dominates_even_across_files() {
        let mut moved = mk_node("n1", "moved_fn", "src/new_home.rs", "uuid-new");
        moved.body_hash = Some("hash-abc".into());
        let extraction = ExtractionResult {
            nodes: vec![moved],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);
        let links = vec![KnowledgeLink {
            knowledge_uuid: "k1".into(),
            node_uuid: "uuid-old".into(),
            relation: "documents".into(),
            target_type: String::new(),
            confidence: 1.0,
            created_at: 0,
            linked_body_hash: "hash-abc".into(),
            reason: String::new(),
            source: String::new(),
            linked_logical_key: "src/old_home.rs::different_name".into(),
        }];
        let orphans = detect_renames(&graph, &links, &HashMap::new(), &DetectQuery::default());
        assert_eq!(orphans.len(), 1);
        let c = &orphans[0].candidates[0];
        assert_eq!(c.label, "moved_fn");
        assert!(c.confidence >= 0.9);
        assert!(c.signals.iter().any(|s| s.contains("body_hash_match")));
    }

    #[test]
    fn skips_link_when_target_still_exists() {
        let extraction = ExtractionResult {
            nodes: vec![mk_node("n", "alive", "src/x.rs", "uuid-still-here")],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);
        let links = vec![KnowledgeLink {
            knowledge_uuid: "k1".into(),
            node_uuid: "uuid-still-here".into(),
            relation: "documents".into(),
            target_type: String::new(),
            confidence: 1.0,
            created_at: 0,
            linked_body_hash: String::new(),
            reason: String::new(),
            source: String::new(),
            linked_logical_key: "src/x.rs::alive".into(),
        }];
        assert!(
            detect_renames(&graph, &links, &HashMap::new(), &DetectQuery::default()).is_empty()
        );
    }
}
