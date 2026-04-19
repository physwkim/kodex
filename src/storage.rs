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

/// Save a graph to HDF5 format with vlen strings.
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

    // --- Collect node data ---
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

    // --- Write nodes ---
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

    // --- Collect edge data ---
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

    // --- Write edges ---
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

    file.close()
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    Ok(())
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

// --- Helpers ---

/// Write vlen strings via H5Group (not H5File — that doesn't resolve group paths).
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

/// Read vlen strings from a dataset path.
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
