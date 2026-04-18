use std::collections::HashMap;

use crate::graph::GraphifyGraph;
use crate::types::Confidence;
use super::god_nodes;

/// A suggested question the graph is positioned to answer.
#[derive(Debug, Clone)]
pub struct SuggestedQuestion {
    pub question: String,
    pub question_type: String,
    pub node_id: Option<String>,
}

/// Generate questions the graph is uniquely positioned to answer.
pub fn suggest_questions(
    graph: &GraphifyGraph,
    _communities: Option<&HashMap<usize, Vec<String>>>,
    top_n: usize,
) -> Vec<SuggestedQuestion> {
    let mut questions = Vec::new();

    // 1. Ambiguous edges needing review
    for (src, tgt, edge) in graph.edges() {
        if edge.confidence == Confidence::AMBIGUOUS {
            let src_label = graph.get_node(src).map(|n| n.label.as_str()).unwrap_or(src);
            let tgt_label = graph.get_node(tgt).map(|n| n.label.as_str()).unwrap_or(tgt);
            questions.push(SuggestedQuestion {
                question: format!(
                    "Is the {} relationship between {} and {} correct?",
                    edge.relation, src_label, tgt_label
                ),
                question_type: "ambiguous_edge".to_string(),
                node_id: Some(src.to_string()),
            });
            if questions.len() >= top_n {
                return questions;
            }
        }
    }

    // 2. God nodes with many INFERRED edges
    let gods = god_nodes::god_nodes(graph, 5);
    for god in &gods {
        let inferred_count = graph
            .edges()
            .filter(|(s, t, e)| {
                e.confidence == Confidence::INFERRED
                    && (*s == god.id || *t == god.id)
            })
            .count();
        if inferred_count >= 3 {
            questions.push(SuggestedQuestion {
                question: format!(
                    "How does {} connect to {} other entities (via inferred edges)?",
                    god.label, inferred_count
                ),
                question_type: "verify_inferred".to_string(),
                node_id: Some(god.id.clone()),
            });
        }
    }

    // 3. Isolated nodes (degree <= 1)
    for node_id in graph.node_ids() {
        if graph.degree(node_id) <= 1 {
            if let Some(node) = graph.get_node(node_id) {
                questions.push(SuggestedQuestion {
                    question: format!(
                        "Why is {} isolated? Is it missing connections?",
                        node.label
                    ),
                    question_type: "isolated_nodes".to_string(),
                    node_id: Some(node_id.clone()),
                });
            }
        }
        if questions.len() >= top_n {
            break;
        }
    }

    questions.truncate(top_n);
    questions
}
