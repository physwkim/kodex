//! Knowledge graph reasoning: transitive trust propagation.
//!
//! Propagates confidence through supports/contradicts/supersedes chains.
//! Used to adjust recall scoring based on knowledge graph structure.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::types::{KnowledgeEntry, KnowledgeLink};

/// Confidence adjustment from graph reasoning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReasoningResult {
    /// UUID → adjusted confidence delta
    pub adjustments: HashMap<String, f64>,
    /// Explanation paths
    pub paths: Vec<ReasoningPath>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReasoningPath {
    pub from_uuid: String,
    pub to_uuid: String,
    pub relation: String,
    pub effect: f64,
    pub explanation: String,
}

/// Propagate confidence through the knowledge graph.
///
/// Rules:
/// - `supports`: target gets +boost (decayed by distance)
/// - `contradicts`: target gets -penalty
/// - `supersedes`: superseded entry penalized, superseding boosted
/// - `depends_on`: if dependency is low-confidence, dependent is penalized
///
/// Returns per-UUID confidence adjustments.
pub fn propagate_confidence(
    knowledge: &[KnowledgeEntry],
    links: &[KnowledgeLink],
    seed_uuids: &[String],
    max_depth: usize,
) -> ReasoningResult {
    let conf_map: HashMap<&str, f64> = knowledge
        .iter()
        .map(|k| (k.uuid.as_str(), k.confidence))
        .collect();

    // Build adjacency for knowledge↔knowledge links
    let mut outgoing: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for link in links {
        if link.is_knowledge_link() {
            outgoing
                .entry(link.knowledge_uuid.as_str())
                .or_default()
                .push((link.node_uuid.as_str(), link.relation.as_str()));
        }
    }

    let mut adjustments: HashMap<String, f64> = HashMap::new();
    let mut paths = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize, f64)> = VecDeque::new();

    // Seed from given UUIDs
    for uuid in seed_uuids {
        if conf_map.contains_key(uuid.as_str()) {
            queue.push_back((uuid.clone(), 0, 1.0));
            visited.insert(uuid.clone());
        }
    }

    let decay = 0.7; // confidence decays per hop

    while let Some((current, depth, strength)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let current_conf = conf_map.get(current.as_str()).copied().unwrap_or(0.5);

        if let Some(edges) = outgoing.get(current.as_str()) {
            for &(target, relation) in edges {
                let effect = match relation {
                    "supports" | "supported_by" => {
                        // Boost target proportional to source confidence
                        current_conf * strength * decay * 0.3
                    }
                    "contradicts" => {
                        // Penalize target
                        -(current_conf * strength * decay * 0.4)
                    }
                    "supersedes" => {
                        // Superseded entry penalized
                        -(current_conf * strength * decay * 0.5)
                    }
                    "superseded_by" => {
                        // Superseding entry gets minor boost
                        current_conf * strength * decay * 0.2
                    }
                    "depends_on" => {
                        // If dependency is weak, dependent is penalized
                        let dep_conf = conf_map.get(target).copied().unwrap_or(0.5);
                        if dep_conf < 0.5 {
                            -(0.5 - dep_conf) * strength * decay
                        } else {
                            0.0
                        }
                    }
                    _ => 0.0,
                };

                if effect.abs() > 0.01 {
                    *adjustments.entry(target.to_string()).or_insert(0.0) += effect;

                    let from_title = knowledge
                        .iter()
                        .find(|k| k.uuid == current)
                        .map(|k| k.title.as_str())
                        .unwrap_or("?");
                    let to_title = knowledge
                        .iter()
                        .find(|k| k.uuid == target)
                        .map(|k| k.title.as_str())
                        .unwrap_or("?");

                    paths.push(ReasoningPath {
                        from_uuid: current.clone(),
                        to_uuid: target.to_string(),
                        relation: relation.to_string(),
                        effect,
                        explanation: format!("{from_title} {relation} {to_title}: {effect:+.2}"),
                    });
                }

                if visited.insert(target.to_string()) {
                    queue.push_back((target.to_string(), depth + 1, strength * decay));
                }
            }
        }
    }

    // Clamp adjustments to [-0.3, +0.3] to avoid extreme swings
    for adj in adjustments.values_mut() {
        *adj = adj.clamp(-0.3, 0.3);
    }

    ReasoningResult { adjustments, paths }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{KnowledgeEntry, KnowledgeLink};

    fn make_k(uuid: &str, title: &str, conf: f64) -> KnowledgeEntry {
        KnowledgeEntry {
            uuid: uuid.into(),
            title: title.into(),
            confidence: conf,
            status: "active".into(),
            ..Default::default()
        }
    }

    fn make_kk_link(from: &str, to: &str, relation: &str) -> KnowledgeLink {
        KnowledgeLink {
            knowledge_uuid: from.into(),
            node_uuid: to.into(),
            relation: relation.into(),
            target_type: "knowledge".into(),
            confidence: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn test_supports_boosts() {
        let knowledge = vec![
            make_k("A", "Auth Pattern", 0.9),
            make_k("B", "Token Pattern", 0.6),
        ];
        let links = vec![make_kk_link("A", "B", "supports")];

        let result = propagate_confidence(&knowledge, &links, &["A".into()], 3);
        let b_adj = result.adjustments.get("B").copied().unwrap_or(0.0);
        assert!(b_adj > 0.0, "supports should boost target: {b_adj}");
    }

    #[test]
    fn test_contradicts_penalizes() {
        let knowledge = vec![
            make_k("A", "JWT Auth", 0.9),
            make_k("B", "Session Auth", 0.7),
        ];
        let links = vec![make_kk_link("A", "B", "contradicts")];

        let result = propagate_confidence(&knowledge, &links, &["A".into()], 3);
        let b_adj = result.adjustments.get("B").copied().unwrap_or(0.0);
        assert!(b_adj < 0.0, "contradicts should penalize target: {b_adj}");
    }

    #[test]
    fn test_supersedes_chain() {
        let knowledge = vec![
            make_k("A", "Old Pattern", 0.8),
            make_k("B", "New Pattern", 0.9),
        ];
        let links = vec![make_kk_link("B", "A", "supersedes")];

        let result = propagate_confidence(&knowledge, &links, &["B".into()], 3);
        let a_adj = result.adjustments.get("A").copied().unwrap_or(0.0);
        assert!(a_adj < 0.0, "superseded entry should be penalized: {a_adj}");
    }
}
