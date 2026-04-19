use std::collections::HashMap;
use std::path::Path;

use crate::graph::EngramGraph;
use super::{node_community_map, COMMUNITY_COLORS};

/// Export graph as Obsidian Canvas JSON (infinite canvas with community groupings).
pub fn to_canvas(
    graph: &EngramGraph,
    communities: &HashMap<usize, Vec<String>>,
    output_path: &Path,
    community_labels: Option<&HashMap<usize, String>>,
) -> std::io::Result<()> {
    let _node_comm = node_community_map(communities);

    // Layout: arrange communities in a grid, nodes within each community in a column
    let mut canvas_nodes = Vec::new();
    let mut canvas_edges = Vec::new();

    let cols = (communities.len() as f64).sqrt().ceil() as usize;
    let community_spacing = 600;
    let node_spacing = 80;

    let mut sorted_cids: Vec<usize> = communities.keys().copied().collect();
    sorted_cids.sort();

    // Node ID → canvas position for edge routing
    let mut node_positions: HashMap<String, (i64, i64)> = HashMap::new();

    for (idx, &cid) in sorted_cids.iter().enumerate() {
        let nodes = match communities.get(&cid) {
            Some(n) => n,
            None => continue,
        };

        let col = idx % cols;
        let row = idx / cols;
        let base_x = (col * community_spacing) as i64;
        let base_y = (row * community_spacing) as i64;
        let color = COMMUNITY_COLORS[cid % COMMUNITY_COLORS.len()];

        let label = community_labels
            .and_then(|cl| cl.get(&cid))
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));

        // Group card
        let group_w = 500;
        let group_h = (nodes.len() * node_spacing + 80) as i64;
        canvas_nodes.push(serde_json::json!({
            "id": format!("group_{cid}"),
            "type": "group",
            "x": base_x,
            "y": base_y,
            "width": group_w,
            "height": group_h,
            "label": label,
            "color": color,
        }));

        // Node cards within group
        for (j, nid) in nodes.iter().enumerate() {
            let node = match graph.get_node(nid) {
                Some(n) => n,
                None => continue,
            };
            let x = base_x + 20;
            let y = base_y + 50 + (j * node_spacing) as i64;
            node_positions.insert(nid.clone(), (x, y));

            canvas_nodes.push(serde_json::json!({
                "id": nid,
                "type": "text",
                "x": x,
                "y": y,
                "width": 300,
                "height": 60,
                "text": format!("**{}**\n`{}`", node.label, node.source_file),
                "color": color,
            }));
        }
    }

    // Edges
    for (src, tgt, edge) in graph.edges() {
        if node_positions.contains_key(src) && node_positions.contains_key(tgt) {
            canvas_edges.push(serde_json::json!({
                "id": format!("edge_{}_{}", src, tgt),
                "fromNode": src,
                "toNode": tgt,
                "label": edge.relation,
            }));
        }
    }

    let canvas = serde_json::json!({
        "nodes": canvas_nodes,
        "edges": canvas_edges,
    });

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&canvas)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(output_path, json)
}
