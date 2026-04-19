pub mod god_nodes;
pub mod helpers;
pub mod questions;
pub mod surprising;

pub use god_nodes::god_nodes;
pub use helpers::{is_concept_node, is_file_node};
pub use questions::suggest_questions;
pub use surprising::surprising_connections;

use std::collections::HashMap;

/// Invert communities map: community_id → node_ids  ⟹  node_id → community_id.
pub fn node_community_map(communities: &HashMap<usize, Vec<String>>) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for (&cid, nodes) in communities {
        for node_id in nodes {
            map.insert(node_id.clone(), cid);
        }
    }
    map
}
