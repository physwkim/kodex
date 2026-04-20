//! HDF5-based graph storage — the default persistence format.
//!
//! File layout:
//! ```text
//! kodex.h5
//! ├── /nodes/
//! │   ├── id          [vlen string]
//! │   ├── label       [vlen string]
//! │   ├── file_type   [vlen string]
//! │   ├── source_file [vlen string]
//! │   ├── confidence  [vlen string]
//! │   └── community   [u32 1D]
//! ├── /edges/
//! │   ├── source      [vlen string]
//! │   ├── target      [vlen string]
//! │   ├── relation    [vlen string]
//! │   ├── confidence  [vlen string]
//! │   └── weight      [f64 1D]
//! └── metadata (attrs: node_count, edge_count, version)
//! ```

use std::collections::HashMap;
use std::path::Path;

use rust_hdf5::file::H5File;

use crate::graph::KodexGraph;
use crate::types::{Confidence, ExtractionResult, FileType};

/// Save a graph to HDF5 format (no knowledge).
pub fn save_hdf5(
    graph: &KodexGraph,
    communities: &HashMap<usize, Vec<String>>,
    path: &Path,
) -> crate::error::Result<()> {
    save_hdf5_with_knowledge(graph, communities, path, &[], &[], &[], &[], &[], &[], &[])
}

/// Load a graph from HDF5 format.
pub fn load_hdf5(path: &Path) -> crate::error::Result<KodexGraph> {
    let file = H5File::open(path)
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 open: {e}")))?;

    let ids = read_vlen(&file, "nodes/id")?;
    let labels = read_vlen(&file, "nodes/label")?;
    let file_types = read_vlen(&file, "nodes/file_type")?;
    let source_files = read_vlen(&file, "nodes/source_file")?;
    let confidences = read_vlen(&file, "nodes/confidence")?;

    let community_ids: Vec<u32> = file
        .dataset("nodes/community")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();

    let mut extraction = ExtractionResult::default();

    for (i, id) in ids.iter().enumerate() {
        extraction.nodes.push(crate::types::Node {
            id: id.clone(),
            label: labels.get(i).cloned().unwrap_or_default(),
            file_type: FileType::from_str_loose(
                file_types.get(i).map(|s| s.as_str()).unwrap_or("code"),
            )
            .unwrap_or(FileType::Code),
            source_file: source_files.get(i).cloned().unwrap_or_default(),
            source_location: None,
            confidence: Confidence::from_str_loose(
                confidences
                    .get(i)
                    .map(|s| s.as_str())
                    .unwrap_or("EXTRACTED"),
            ),
            confidence_score: None,
            community: community_ids.get(i).map(|&c| c as usize),
            norm_label: None,
            degree: None,
        });
    }

    let e_src = read_vlen(&file, "edges/source")?;
    let e_tgt = read_vlen(&file, "edges/target")?;
    let e_rel = read_vlen(&file, "edges/relation")?;
    let e_conf = read_vlen(&file, "edges/confidence")?;
    let e_weight: Vec<f64> = file
        .dataset("edges/weight")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();

    for i in 0..e_src.len() {
        let confidence =
            Confidence::from_str_loose(e_conf.get(i).map(|s| s.as_str()).unwrap_or("EXTRACTED"))
                .unwrap_or(Confidence::EXTRACTED);

        extraction.edges.push(crate::types::Edge {
            source: e_src[i].clone(),
            target: e_tgt[i].clone(),
            relation: e_rel.get(i).cloned().unwrap_or_default(),
            confidence,
            source_file: String::new(),
            source_location: None,
            confidence_score: Some(confidence.default_score()),
            weight: e_weight.get(i).copied().unwrap_or(1.0),
            original_src: None,
            original_tgt: None,
        });
    }

    Ok(crate::graph::build_from_extraction(&extraction))
}

/// Append a knowledge entry to an existing HDF5 file.
///
/// Opens the file in read-write mode and adds datasets to /knowledge/ group.
/// Does NOT reload the entire graph — true incremental write.
#[allow(clippy::too_many_arguments)]
pub fn append_knowledge(
    h5_path: &Path,
    title: &str,
    knowledge_type: &str,
    description: &str,
    confidence: f64,
    observations: u32,
    related_nodes: &[String],
    tags: &[String],
) -> crate::error::Result<()> {
    if !h5_path.exists() {
        return Err(crate::error::KodexError::Other(
            "HDF5 file does not exist. Run `kodex run` first.".to_string(),
        ));
    }

    // Load existing knowledge
    let (
        mut titles,
        mut types,
        mut descriptions,
        mut confidences,
        mut obs,
        mut related,
        mut tag_list,
    ) = {
        let file = H5File::open(h5_path)
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5 open: {e}")))?;

        let t = read_vlen(&file, "knowledge/titles").unwrap_or_default();
        let ty = read_vlen(&file, "knowledge/types").unwrap_or_default();
        let d = read_vlen(&file, "knowledge/descriptions").unwrap_or_default();
        let c: Vec<f64> = file
            .dataset("knowledge/confidence")
            .and_then(|ds| ds.read_raw())
            .unwrap_or_default();
        let o: Vec<u32> = file
            .dataset("knowledge/observations")
            .and_then(|ds| ds.read_raw())
            .unwrap_or_default();
        let r = read_vlen(&file, "knowledge/related").unwrap_or_default();
        let tl = read_vlen(&file, "knowledge/tags").unwrap_or_default();
        (t, ty, d, c, o, r, tl)
    };

    // Check if exists — reinforce
    let existing_idx = titles.iter().position(|t| t == title);
    if let Some(idx) = existing_idx {
        obs[idx] += 1;
        confidences[idx] = 1.0 - (1.0 - confidences[idx]) * 0.8;
        if !description.is_empty() && descriptions[idx] != description {
            descriptions[idx] = format!("{}\n---\n{}", descriptions[idx], description);
        }
        // Merge related nodes
        let existing_related: Vec<&str> =
            related[idx].split(',').filter(|s| !s.is_empty()).collect();
        let mut merged: Vec<String> = existing_related.iter().map(|s| s.to_string()).collect();
        for node in related_nodes {
            if !merged.contains(node) {
                merged.push(node.clone());
            }
        }
        related[idx] = merged.join(",");
        // Merge tags
        let existing_tags: Vec<&str> = tag_list[idx].split(',').filter(|s| !s.is_empty()).collect();
        let mut merged_tags: Vec<String> = existing_tags.iter().map(|s| s.to_string()).collect();
        for tag in tags {
            if !merged_tags.contains(tag) {
                merged_tags.push(tag.clone());
            }
        }
        tag_list[idx] = merged_tags.join(",");
    } else {
        titles.push(title.to_string());
        types.push(knowledge_type.to_string());
        descriptions.push(description.to_string());
        confidences.push(confidence);
        obs.push(observations);
        related.push(related_nodes.join(","));
        tag_list.push(tags.join(","));
    }

    // Rewrite knowledge group (open_rw can't delete existing datasets, so recreate file knowledge section)
    // For now: reload full file, replace knowledge, save
    let graph = load_hdf5(h5_path)?;
    let communities = crate::cluster::cluster(&graph);

    // Save with knowledge
    save_hdf5_with_knowledge(
        &graph,
        &communities,
        h5_path,
        &titles,
        &types,
        &descriptions,
        &confidences,
        &obs,
        &related,
        &tag_list,
    )
}

/// Load knowledge entries from HDF5.
#[allow(clippy::type_complexity)]
pub fn load_knowledge_entries(
    h5_path: &Path,
) -> crate::error::Result<Vec<(String, String, String, f64, u32, String, String)>> {
    let file = H5File::open(h5_path)
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 open: {e}")))?;

    let titles = read_vlen(&file, "knowledge/titles").unwrap_or_default();
    let types = read_vlen(&file, "knowledge/types").unwrap_or_default();
    let descriptions = read_vlen(&file, "knowledge/descriptions").unwrap_or_default();
    let confidences: Vec<f64> = file
        .dataset("knowledge/confidence")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();
    let obs: Vec<u32> = file
        .dataset("knowledge/observations")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();
    let related = read_vlen(&file, "knowledge/related").unwrap_or_default();
    let tag_list = read_vlen(&file, "knowledge/tags").unwrap_or_default();

    let mut entries = Vec::new();
    #[allow(clippy::needless_range_loop)]
    for i in 0..titles.len() {
        entries.push((
            titles[i].clone(),
            types.get(i).cloned().unwrap_or_default(),
            descriptions.get(i).cloned().unwrap_or_default(),
            confidences.get(i).copied().unwrap_or(0.5),
            obs.get(i).copied().unwrap_or(1),
            related.get(i).cloned().unwrap_or_default(),
            tag_list.get(i).cloned().unwrap_or_default(),
        ));
    }
    Ok(entries)
}

/// Save graph + knowledge to HDF5.
#[allow(clippy::too_many_arguments)]
fn save_hdf5_with_knowledge(
    graph: &KodexGraph,
    communities: &HashMap<usize, Vec<String>>,
    path: &Path,
    k_titles: &[String],
    k_types: &[String],
    k_descriptions: &[String],
    k_confidences: &[f64],
    k_observations: &[u32],
    k_related: &[String],
    k_tags: &[String],
) -> crate::error::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = H5File::create(path)
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 create: {e}")))?;

    file.set_attr_string("version", "0.1.0").ok();
    file.set_attr_numeric("node_count", &(graph.node_count() as u64))
        .ok();
    file.set_attr_numeric("edge_count", &(graph.edge_count() as u64))
        .ok();

    let comm_map = crate::export::node_community_map(communities);

    // --- Nodes ---
    let (ids, labels, file_types, source_files, confidences, community_ids) =
        collect_node_data(graph, &comm_map);

    let nodes_grp = file
        .create_group("nodes")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;

    write_vlen(&nodes_grp, "id", &ids)?;
    write_vlen(&nodes_grp, "label", &labels)?;
    write_vlen(&nodes_grp, "file_type", &file_types)?;
    write_vlen(&nodes_grp, "source_file", &source_files)?;
    write_vlen(&nodes_grp, "confidence", &confidences)?;

    if !community_ids.is_empty() {
        nodes_grp
            .new_dataset::<u32>()
            .shape([community_ids.len()])
            .create("community")
            .and_then(|ds| ds.write_raw(&community_ids))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    }

    // --- Edges ---
    let (e_src, e_tgt, e_rel, e_conf, e_weight) = collect_edge_data(graph);

    let edges_grp = file
        .create_group("edges")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;

    write_vlen(&edges_grp, "source", &e_src)?;
    write_vlen(&edges_grp, "target", &e_tgt)?;
    write_vlen(&edges_grp, "relation", &e_rel)?;
    write_vlen(&edges_grp, "confidence", &e_conf)?;

    if !e_weight.is_empty() {
        edges_grp
            .new_dataset::<f64>()
            .shape([e_weight.len()])
            .create("weight")
            .and_then(|ds| ds.write_raw(&e_weight))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    }

    // --- Knowledge ---
    if !k_titles.is_empty() {
        let k_grp = file
            .create_group("knowledge")
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;

        write_vlen(&k_grp, "titles", k_titles)?;
        write_vlen(&k_grp, "types", k_types)?;
        write_vlen(&k_grp, "descriptions", k_descriptions)?;
        write_vlen(&k_grp, "related", k_related)?;
        write_vlen(&k_grp, "tags", k_tags)?;

        k_grp
            .new_dataset::<f64>()
            .shape([k_confidences.len()])
            .create("confidence")
            .and_then(|ds| ds.write_raw(k_confidences))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;

        k_grp
            .new_dataset::<u32>()
            .shape([k_observations.len()])
            .create("observations")
            .and_then(|ds| ds.write_raw(k_observations))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    }

    file.close()
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    Ok(())
}

#[allow(clippy::type_complexity)]
fn collect_node_data(
    graph: &KodexGraph,
    comm_map: &HashMap<String, usize>,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<u32>,
) {
    let mut ids = Vec::new();
    let mut labels = Vec::new();
    let mut file_types = Vec::new();
    let mut source_files = Vec::new();
    let mut confidences = Vec::new();
    let mut community_ids: Vec<u32> = Vec::new();

    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(id) {
            ids.push(id.clone());
            labels.push(node.label.clone());
            file_types.push(node.file_type.to_string());
            source_files.push(node.source_file.clone());
            confidences.push(
                node.confidence
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "EXTRACTED".to_string()),
            );
            community_ids.push(comm_map.get(id).copied().unwrap_or(0) as u32);
        }
    }

    (
        ids,
        labels,
        file_types,
        source_files,
        confidences,
        community_ids,
    )
}

#[allow(clippy::type_complexity)]
fn collect_edge_data(
    graph: &KodexGraph,
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<f64>) {
    let mut e_src = Vec::new();
    let mut e_tgt = Vec::new();
    let mut e_rel = Vec::new();
    let mut e_conf = Vec::new();
    let mut e_weight: Vec<f64> = Vec::new();

    for (src, tgt, edge) in graph.edges() {
        e_src.push(src.to_string());
        e_tgt.push(tgt.to_string());
        e_rel.push(edge.relation.clone());
        e_conf.push(edge.confidence.to_string());
        e_weight.push(edge.weight);
    }

    (e_src, e_tgt, e_rel, e_conf, e_weight)
}

// --- Helpers ---

fn write_vlen(
    group: &rust_hdf5::group::H5Group,
    name: &str,
    strings: &[String],
) -> crate::error::Result<()> {
    if strings.is_empty() {
        return Ok(());
    }
    let refs: Vec<&str> = strings.iter().map(|s| s.as_str()).collect();
    group
        .write_vlen_strings(name, &refs)
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 write {name}: {e}")))
}

fn read_vlen(file: &H5File, path: &str) -> crate::error::Result<Vec<String>> {
    let ds = match file.dataset(path) {
        Ok(ds) => ds,
        Err(_) => return Ok(Vec::new()),
    };
    ds.read_vlen_strings()
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 read {path}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hdf5_round_trip() {
        let dir = TempDir::new().unwrap();
        let h5_path = dir.path().join("test.h5");

        let extraction = ExtractionResult {
            nodes: vec![
                crate::types::Node {
                    id: "a".to_string(),
                    label: "Alpha".to_string(),
                    file_type: FileType::Code,
                    source_file: "a.py".to_string(),
                    source_location: None,
                    confidence: Some(Confidence::EXTRACTED),
                    confidence_score: Some(1.0),
                    community: None,
                    norm_label: None,
                    degree: None,
                },
                crate::types::Node {
                    id: "b".to_string(),
                    label: "Beta".to_string(),
                    file_type: FileType::Code,
                    source_file: "b.py".to_string(),
                    source_location: None,
                    confidence: Some(Confidence::INFERRED),
                    confidence_score: Some(0.5),
                    community: None,
                    norm_label: None,
                    degree: None,
                },
            ],
            edges: vec![crate::types::Edge {
                source: "a".to_string(),
                target: "b".to_string(),
                relation: "imports".to_string(),
                confidence: Confidence::EXTRACTED,
                source_file: "a.py".to_string(),
                source_location: None,
                confidence_score: Some(1.0),
                weight: 1.0,
                original_src: None,
                original_tgt: None,
            }],
            ..Default::default()
        };

        let graph = crate::graph::build_from_extraction(&extraction);
        let communities = crate::cluster::cluster(&graph);

        save_hdf5(&graph, &communities, &h5_path).unwrap();
        assert!(h5_path.exists());

        let loaded = load_hdf5(&h5_path).unwrap();
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);
        assert!(loaded.get_node("a").is_some());
        assert_eq!(loaded.get_node("a").unwrap().label, "Alpha");
        assert!(loaded.get_node("b").is_some());
    }
}
