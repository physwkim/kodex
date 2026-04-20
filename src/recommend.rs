//! Action-oriented recommendations from knowledge.
//!
//! Transforms knowledge recall into actionable guidance:
//! - What rules to follow
//! - What risks to watch
//! - What conflicts to resolve

use crate::learn::{KnowledgeConflict, RecallResult};

/// A recommended action based on knowledge.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Recommendation {
    pub action: String,
    pub reason: String,
    pub knowledge_uuid: String,
    pub priority: u8,
    pub category: String,
}

/// Generate recommendations from recalled knowledge + conflicts.
pub fn compute_recommendations(
    recalled: &[RecallResult],
    conflicts: &[KnowledgeConflict],
    task_type: &str,
) -> Vec<Recommendation> {
    let mut recs = Vec::new();

    for item in recalled {
        let k = &item.knowledge;
        let conf = (k.confidence * 100.0) as u32;

        // Type-based recommendations
        match k.knowledge_type.to_string().as_str() {
            "convention" => {
                recs.push(Recommendation {
                    action: format!("Follow: {}", k.title),
                    reason: first_line(&k.description),
                    knowledge_uuid: k.uuid.clone(),
                    priority: if conf > 80 { 9 } else { 7 },
                    category: "rule".into(),
                });
            }
            "bug_pattern" => {
                recs.push(Recommendation {
                    action: format!("Watch for: {}", k.title),
                    reason: first_line(&k.description),
                    knowledge_uuid: k.uuid.clone(),
                    priority: 9,
                    category: "hazard".into(),
                });
                if task_type == "bugfix" || task_type == "review" {
                    recs.push(Recommendation {
                        action: format!("Add test for: {}", k.title),
                        reason: "Known bug pattern — regression test recommended".into(),
                        knowledge_uuid: k.uuid.clone(),
                        priority: 8,
                        category: "test".into(),
                    });
                }
            }
            "coupling" => {
                recs.push(Recommendation {
                    action: format!("Check coupling: {}", k.title),
                    reason: first_line(&k.description),
                    knowledge_uuid: k.uuid.clone(),
                    priority: 8,
                    category: "coupling".into(),
                });
            }
            "decision" => {
                if task_type == "refactor" {
                    recs.push(Recommendation {
                        action: format!("Respect decision: {}", k.title),
                        reason: first_line(&k.description),
                        knowledge_uuid: k.uuid.clone(),
                        priority: 7,
                        category: "constraint".into(),
                    });
                }
            }
            "architecture" => {
                recs.push(Recommendation {
                    action: format!("Architecture: {}", k.title),
                    reason: first_line(&k.description),
                    knowledge_uuid: k.uuid.clone(),
                    priority: 6,
                    category: "constraint".into(),
                });
            }
            "tech_debt" if task_type == "refactor" => {
                recs.push(Recommendation {
                    action: format!("Opportunity: address {}", k.title),
                    reason: first_line(&k.description),
                    knowledge_uuid: k.uuid.clone(),
                    priority: 5,
                    category: "opportunity".into(),
                });
            }
            "performance" => {
                recs.push(Recommendation {
                    action: format!("Perf: watch {}", k.title),
                    reason: first_line(&k.description),
                    knowledge_uuid: k.uuid.clone(),
                    priority: 6,
                    category: "hazard".into(),
                });
            }
            _ => {}
        }
    }

    // Conflict-based recommendations
    for conflict in conflicts {
        recs.push(Recommendation {
            action: format!(
                "Resolve conflict: {} vs {}",
                conflict.title_a, conflict.title_b
            ),
            reason: conflict.description.clone(),
            knowledge_uuid: conflict.uuid_a.clone(),
            priority: 10,
            category: "conflict".into(),
        });
    }

    recs.sort_by(|a, b| b.priority.cmp(&a.priority));
    recs
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}
