use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::graph::KodexGraph;
use super::{node_community_map, COMMUNITY_COLORS};

/// Export graph as an Obsidian vault (one .md per node + community overviews).
pub fn to_obsidian(
    graph: &KodexGraph,
    communities: &HashMap<usize, Vec<String>>,
    output_dir: &Path,
    community_labels: Option<&HashMap<usize, String>>,
    cohesion: Option<&HashMap<usize, f64>>,
) -> std::io::Result<usize> {
    std::fs::create_dir_all(output_dir)?;

    let node_comm = node_community_map(communities);
    let mut written = 0;

    // Deduplicate filenames
    let mut filenames: HashMap<String, String> = HashMap::new();
    let mut used_names: HashSet<String> = HashSet::new();
    for id in graph.node_ids() {
        let node = match graph.get_node(id) {
            Some(n) => n,
            None => continue,
        };
        let mut name = safe_name(&node.label);
        if used_names.contains(&name) {
            for i in 2..=10_000 {
                let candidate = format!("{name}_{i}");
                if !used_names.contains(&candidate) {
                    name = candidate;
                    break;
                }
            }
        }
        used_names.insert(name.clone());
        filenames.insert(id.clone(), name);
    }

    // Write node notes
    for id in graph.node_ids() {
        let node = match graph.get_node(id) {
            Some(n) => n,
            None => continue,
        };
        let filename = match filenames.get(id) {
            Some(f) => f,
            None => continue,
        };
        let cid = node_comm.get(id).copied().unwrap_or(0);
        let comm_label = community_labels
            .and_then(|cl| cl.get(&cid))
            .cloned()
            .unwrap_or_else(|| format!("Community_{cid}"));

        let mut md = String::new();
        // YAML frontmatter
        md.push_str("---\n");
        md.push_str(&format!("source_file: \"{}\"\n", node.source_file));
        md.push_str(&format!("type: {}\n", node.file_type));
        md.push_str(&format!("community: {cid}\n"));
        if let Some(loc) = &node.source_location {
            md.push_str(&format!("location: {loc}\n"));
        }
        md.push_str(&format!("tags: [kodex/{}, community/{}]\n", node.file_type, safe_name(&comm_label)));
        md.push_str("---\n\n");

        md.push_str(&format!("# {}\n\n", node.label));

        // Connections
        let neighbors = graph.neighbors(id);
        if !neighbors.is_empty() {
            md.push_str("## Connections\n\n");
            for nid in &neighbors {
                if let (Some(_nnode), Some(nfile)) = (graph.get_node(nid), filenames.get(nid)) {
                    // Find edge info
                    let edge_info = graph
                        .edges()
                        .find(|(s, t, _)| (*s == id && *t == *nid) || (*t == id && *s == *nid))
                        .map(|(_, _, e)| format!("{} [{}]", e.relation, e.confidence))
                        .unwrap_or_default();
                    md.push_str(&format!("- [[{}]] - {}\n", nfile, edge_info));
                }
            }
        }

        let path = output_dir.join(format!("{filename}.md"));
        std::fs::write(&path, md)?;
        written += 1;
    }

    // Write community overview notes
    let mut sorted_cids: Vec<usize> = communities.keys().copied().collect();
    sorted_cids.sort();
    for cid in sorted_cids {
        let nodes = match communities.get(&cid) {
            Some(n) => n,
            None => continue,
        };
        let label = community_labels
            .and_then(|cl| cl.get(&cid))
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));
        let coh = cohesion.and_then(|c| c.get(&cid)).copied().unwrap_or(0.0);

        let mut md = String::new();
        md.push_str("---\n");
        md.push_str("type: community\n");
        md.push_str(&format!("cohesion: {coh:.2}\n"));
        md.push_str(&format!("members: {}\n", nodes.len()));
        md.push_str("---\n\n");
        md.push_str(&format!("# {label}\n\n"));
        md.push_str(&format!("> {} nodes · cohesion {coh:.2}\n\n", nodes.len()));

        md.push_str("## Members\n\n");
        for nid in nodes {
            if let (Some(node), Some(fname)) = (graph.get_node(nid), filenames.get(nid)) {
                md.push_str(&format!(
                    "- [[{}]] — {} · `{}`\n",
                    fname, node.file_type, node.source_file
                ));
            }
        }

        // Dataview query
        let safe_label = safe_name(&label);
        md.push_str(&format!(
            "\n## Dataview\n\n```dataview\nTABLE source_file, type\nFROM #community/{safe_label}\n```\n"
        ));

        // Inter-community edge counts
        let node_set: std::collections::HashSet<&str> =
            nodes.iter().map(|s| s.as_str()).collect();
        let mut cross_comm_counts: HashMap<usize, usize> = HashMap::new();
        for (src, tgt, _) in graph.edges() {
            let src_in = node_set.contains(src);
            let tgt_in = node_set.contains(tgt);
            if src_in && !tgt_in {
                if let Some(&other_cid) = node_comm.get(tgt) {
                    if other_cid != cid {
                        *cross_comm_counts.entry(other_cid).or_default() += 1;
                    }
                }
            } else if tgt_in && !src_in {
                if let Some(&other_cid) = node_comm.get(src) {
                    if other_cid != cid {
                        *cross_comm_counts.entry(other_cid).or_default() += 1;
                    }
                }
            }
        }
        if !cross_comm_counts.is_empty() {
            md.push_str("\n## Cross-Community Connections\n\n");
            let mut sorted: Vec<_> = cross_comm_counts.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            for (other_cid, count) in sorted {
                let other_label = community_labels
                    .and_then(|cl| cl.get(&other_cid))
                    .cloned()
                    .unwrap_or_else(|| format!("Community {other_cid}"));
                let other_safe = safe_name(&other_label);
                md.push_str(&format!(
                    "- [[_COMMUNITY_{other_safe}|{other_label}]] — {count} shared edges\n"
                ));
            }
        }

        // Bridge nodes — top 5 nodes that connect to most other communities
        let mut bridge_scores: Vec<(&str, usize)> = Vec::new();
        for nid in nodes {
            let mut reached_communities: std::collections::HashSet<usize> = std::collections::HashSet::new();
            for neighbor in graph.neighbors(nid) {
                if let Some(&nc) = node_comm.get(&neighbor) {
                    if nc != cid {
                        reached_communities.insert(nc);
                    }
                }
            }
            if !reached_communities.is_empty() {
                bridge_scores.push((nid.as_str(), reached_communities.len()));
            }
        }
        bridge_scores.sort_by(|a, b| b.1.cmp(&a.1));
        if !bridge_scores.is_empty() {
            md.push_str("\n## Bridge Nodes\n\n");
            for (nid, reach) in bridge_scores.iter().take(5) {
                if let Some(fname) = filenames.get(*nid) {
                    let deg = graph.degree(nid);
                    md.push_str(&format!(
                        "- [[{fname}]] — {deg} connections, reaches {reach} other communities\n"
                    ));
                }
            }
        }

        let filename = format!("_COMMUNITY_{}", safe_name(&label));
        let path = output_dir.join(format!("{filename}.md"));
        std::fs::write(&path, md)?;
        written += 1;
    }

    // Write .obsidian/graph.json for community colors
    let obsidian_dir = output_dir.join(".obsidian");
    std::fs::create_dir_all(&obsidian_dir)?;
    let mut color_groups = Vec::new();
    for cid in communities.keys() {
        let label = community_labels
            .and_then(|cl| cl.get(cid))
            .cloned()
            .unwrap_or_else(|| format!("Community_{cid}"));
        let color = COMMUNITY_COLORS[cid % COMMUNITY_COLORS.len()];
        color_groups.push(serde_json::json!({
            "query": format!("tag:#community/{}", safe_name(&label)),
            "color": { "a": 1, "rgb": color_to_rgb_int(color) },
        }));
    }
    let graph_json = serde_json::json!({ "colorGroups": color_groups });
    std::fs::write(
        obsidian_dir.join("graph.json"),
        serde_json::to_string_pretty(&graph_json).unwrap_or_default(),
    )?;

    Ok(written)
}

fn safe_name(s: &str) -> String {
    s.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ' '], "_")
        .trim_end_matches(".md")
        .trim_end_matches(".mdx")
        .to_string()
}

fn color_to_rgb_int(hex: &str) -> u32 {
    u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0)
}
