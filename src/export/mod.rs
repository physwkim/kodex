mod json_export;
mod html;
mod obsidian;
mod cypher;
mod wiki;
mod graphml;
mod canvas;

pub use json_export::to_json;
pub use html::to_html;
pub use obsidian::to_obsidian;
pub use cypher::to_cypher;
pub use wiki::to_wiki;
pub use graphml::to_graphml;
pub use canvas::to_canvas;

use std::collections::{HashMap, HashSet};

pub const COMMUNITY_COLORS: &[&str] = &[
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F",
    "#EDC948", "#B07AA1", "#FF9DA7", "#9C755F", "#BAB0AC",
];

/// Strip diacritics from text (NFD normalize, remove combining marks).
pub fn strip_diacritics(text: &str) -> String {
    // Simple ASCII-folding approach: remove non-ASCII after normalization
    text.chars()
        .filter(|c| c.is_ascii() || c.is_alphanumeric())
        .collect()
}

/// Remove edges whose source or target is not in the node set.
pub fn prune_dangling_edges(
    nodes: &[serde_json::Value],
    edges: &mut Vec<serde_json::Value>,
) -> usize {
    let node_ids: HashSet<&str> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()))
        .collect();

    let before = edges.len();
    edges.retain(|e| {
        let src = e.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let tgt = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
        node_ids.contains(src) && node_ids.contains(tgt)
    });
    before - edges.len()
}

/// Build a node_id → community_id map.
pub fn node_community_map(communities: &HashMap<usize, Vec<String>>) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for (&cid, nodes) in communities {
        for nid in nodes {
            map.insert(nid.clone(), cid);
        }
    }
    map
}

/// Escape a string for Cypher single-quoted literals.
pub fn cypher_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Make JSON safe for embedding inside <script> tags.
pub fn js_safe(json: &str) -> String {
    json.replace("</", "<\\/")
}
