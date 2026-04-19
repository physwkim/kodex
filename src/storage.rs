//! HDF5-based graph storage — the default persistence format.
//!
//! File layout:
//! ```text
//! kodex.h5
//! ├── /nodes/
//! │   ├── id          [u8 2D, padded strings]
//! │   ├── label       [u8 2D]
//! │   ├── file_type   [u8 2D]
//! │   ├── source_file [u8 2D]
//! │   ├── confidence  [u8 2D]
//! │   └── community   [u32 1D]
//! ├── /edges/
//! │   ├── source      [u8 2D]
//! │   ├── target      [u8 2D]
//! │   ├── relation    [u8 2D]
//! │   ├── confidence  [u8 2D]
//! │   └── weight      [f64 1D]
//! └── metadata (attrs: node_count, edge_count, version)
//! ```

use std::collections::HashMap;
use std::path::Path;

use rust_hdf5::file::H5File;

use crate::graph::KodexGraph;
use crate::types::{Confidence, ExtractionResult, FileType};

/// Save a graph to HDF5 format.
pub fn save_hdf5(
    graph: &KodexGraph,
    communities: &HashMap<usize, Vec<String>>,
    path: &Path,
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

    let nodes_grp = file
        .create_group("nodes")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;

    write_string_dataset(&nodes_grp, "id", &ids)?;
    write_string_dataset(&nodes_grp, "label", &labels)?;
    write_string_dataset(&nodes_grp, "file_type", &file_types)?;
    write_string_dataset(&nodes_grp, "source_file", &source_files)?;
    write_string_dataset(&nodes_grp, "confidence", &confidences)?;

    if !community_ids.is_empty() {
        nodes_grp
            .new_dataset::<u32>()
            .shape([community_ids.len()])
            .create("community")
            .and_then(|ds| ds.write_raw(&community_ids))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    }

    // --- Edges ---
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

    let edges_grp = file
        .create_group("edges")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;

    write_string_dataset(&edges_grp, "source", &e_src)?;
    write_string_dataset(&edges_grp, "target", &e_tgt)?;
    write_string_dataset(&edges_grp, "relation", &e_rel)?;
    write_string_dataset(&edges_grp, "confidence", &e_conf)?;

    if !e_weight.is_empty() {
        edges_grp
            .new_dataset::<f64>()
            .shape([e_weight.len()])
            .create("weight")
            .and_then(|ds| ds.write_raw(&e_weight))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    }

    file.close()
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    Ok(())
}

/// Load a graph from HDF5 format.
pub fn load_hdf5(path: &Path) -> crate::error::Result<KodexGraph> {
    let file = H5File::open(path)
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 open: {e}")))?;

    let ids = read_string_dataset(&file, "nodes/id")?;
    let labels = read_string_dataset(&file, "nodes/label")?;
    let file_types = read_string_dataset(&file, "nodes/file_type")?;
    let source_files = read_string_dataset(&file, "nodes/source_file")?;
    let confidences = read_string_dataset(&file, "nodes/confidence")?;

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

    let e_src = read_string_dataset(&file, "edges/source")?;
    let e_tgt = read_string_dataset(&file, "edges/target")?;
    let e_rel = read_string_dataset(&file, "edges/relation")?;
    let e_conf = read_string_dataset(&file, "edges/confidence")?;
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

// --- Helpers ---

fn write_string_dataset(
    group: &rust_hdf5::group::H5Group,
    name: &str,
    strings: &[String],
) -> crate::error::Result<()> {
    if strings.is_empty() {
        return Ok(());
    }
    let max_len = strings.iter().map(|s| s.len()).max().unwrap_or(1).max(1);
    let padded: Vec<u8> = strings
        .iter()
        .flat_map(|s| {
            let mut buf = s.as_bytes().to_vec();
            buf.resize(max_len, 0);
            buf
        })
        .collect();

    group
        .new_dataset::<u8>()
        .shape([strings.len(), max_len])
        .create(name)
        .and_then(|ds| ds.write_raw(&padded))
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 write {name}: {e}")))?;

    Ok(())
}

fn read_string_dataset(file: &H5File, path: &str) -> crate::error::Result<Vec<String>> {
    let ds = match file.dataset(path) {
        Ok(ds) => ds,
        Err(_) => return Ok(Vec::new()),
    };

    let shape = ds.shape();
    if shape.len() != 2 || shape[0] == 0 {
        return Ok(Vec::new());
    }

    let n = shape[0];
    let max_len = shape[1];

    let raw: Vec<u8> = ds
        .read_raw()
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 read {path}: {e}")))?;

    let mut strings = Vec::with_capacity(n);
    for i in 0..n {
        let start = i * max_len;
        let end = (start + max_len).min(raw.len());
        let slice = &raw[start..end];
        let trimmed = match slice.iter().position(|&b| b == 0) {
            Some(pos) => &slice[..pos],
            None => slice,
        };
        strings.push(String::from_utf8_lossy(trimmed).to_string());
    }

    Ok(strings)
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

        // Save
        save_hdf5(&graph, &communities, &h5_path).unwrap();
        assert!(h5_path.exists());

        // Load
        let loaded = load_hdf5(&h5_path).unwrap();
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);
        assert!(loaded.get_node("a").is_some());
        assert_eq!(loaded.get_node("a").unwrap().label, "Alpha");
        assert!(loaded.get_node("b").is_some());
    }
}
