//! HDF5-based graph storage.
//!
//! File layout:
//! ```text
//! engram.h5
//! ├── /nodes/
//! │   ├── id          [string dataset, resizable]
//! │   ├── label       [string dataset]
//! │   ├── file_type   [string dataset]
//! │   ├── source_file [string dataset]
//! │   ├── confidence  [string dataset]
//! │   └── community   [u32 dataset]
//! ├── /edges/
//! │   ├── source      [string dataset]
//! │   ├── target      [string dataset]
//! │   ├── relation    [string dataset]
//! │   ├── confidence  [string dataset]
//! │   └── weight      [f64 dataset]
//! ├── /knowledge/
//! │   ├── title       [string dataset]
//! │   ├── type        [string dataset]
//! │   ├── confidence  [f64 dataset]
//! │   └── observations [u32 dataset]
//! └── metadata (attrs: node_count, edge_count, version)
//! ```

#[cfg(feature = "hdf5")]
use std::path::Path;

#[cfg(feature = "hdf5")]
use rust_hdf5::file::H5File;

#[cfg(feature = "hdf5")]
use crate::graph::EngramGraph;
#[cfg(feature = "hdf5")]
use crate::types::{Confidence, ExtractionResult, FileType};

/// Save a graph to HDF5 format.
#[cfg(feature = "hdf5")]
pub fn save_hdf5(
    graph: &EngramGraph,
    communities: &std::collections::HashMap<usize, Vec<String>>,
    path: &Path,
) -> crate::error::Result<()> {
    let file = H5File::create(path)
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 create error: {e}")))?;

    // Metadata attributes
    file.set_attr_string("version", "0.1.0")
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 attr error: {e}")))?;
    file.set_attr_numeric("node_count", &(graph.node_count() as u64))
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 attr error: {e}")))?;
    file.set_attr_numeric("edge_count", &(graph.edge_count() as u64))
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 attr error: {e}")))?;

    let comm_map = crate::export::node_community_map(communities);

    // Collect node data as parallel arrays
    let mut ids = Vec::new();
    let mut labels = Vec::new();
    let mut file_types = Vec::new();
    let mut source_files = Vec::new();
    let mut confidences = Vec::new();
    let mut community_ids: Vec<u32> = Vec::new();

    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(id) {
            ids.push(id.as_str());
            labels.push(node.label.as_str());
            file_types.push(node.file_type.to_string());
            source_files.push(node.source_file.as_str());
            confidences.push(
                node.confidence
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "EXTRACTED".to_string()),
            );
            community_ids.push(comm_map.get(id).copied().unwrap_or(0) as u32);
        }
    }

    // Write node datasets
    let nodes_grp = file.create_group("nodes")
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 group error: {e}")))?;

    write_strings(&nodes_grp, "id", &ids)?;
    write_strings(&nodes_grp, "label", &labels)?;
    let ft_strs: Vec<&str> = file_types.iter().map(|s| s.as_str()).collect();
    write_strings(&nodes_grp, "file_type", &ft_strs)?;
    write_strings(&nodes_grp, "source_file", &source_files)?;
    let conf_strs: Vec<&str> = confidences.iter().map(|s| s.as_str()).collect();
    write_strings(&nodes_grp, "confidence", &conf_strs)?;

    nodes_grp
        .new_dataset::<u32>()
        .shape(&[community_ids.len()])
        .create("community")
        .and_then(|ds| ds.write_raw(&community_ids))
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 write error: {e}")))?;

    // Collect edge data
    let mut e_sources = Vec::new();
    let mut e_targets = Vec::new();
    let mut e_relations = Vec::new();
    let mut e_confidences = Vec::new();
    let mut e_weights: Vec<f64> = Vec::new();

    for (src, tgt, edge) in graph.edges() {
        e_sources.push(src);
        e_targets.push(tgt);
        e_relations.push(edge.relation.as_str());
        e_confidences.push(edge.confidence.to_string());
        e_weights.push(edge.weight);
    }

    let edges_grp = file.create_group("edges")
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 group error: {e}")))?;

    write_strings(&edges_grp, "source", &e_sources)?;
    write_strings(&edges_grp, "target", &e_targets)?;
    write_strings(&edges_grp, "relation", &e_relations)?;
    let ec_strs: Vec<&str> = e_confidences.iter().map(|s| s.as_str()).collect();
    write_strings(&edges_grp, "confidence", &ec_strs)?;

    edges_grp
        .new_dataset::<f64>()
        .shape(&[e_weights.len()])
        .create("weight")
        .and_then(|ds| ds.write_raw(&e_weights))
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 write error: {e}")))?;

    file.close()
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 close error: {e}")))?;

    Ok(())
}

/// Load a graph from HDF5 format.
#[cfg(feature = "hdf5")]
pub fn load_hdf5(path: &Path) -> crate::error::Result<EngramGraph> {
    let file = H5File::open(path)
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 open error: {e}")))?;

    // Read node arrays
    let ids = read_strings(&file, "nodes/id")?;
    let labels = read_strings(&file, "nodes/label")?;
    let file_types = read_strings(&file, "nodes/file_type")?;
    let source_files = read_strings(&file, "nodes/source_file")?;
    let confidences = read_strings(&file, "nodes/confidence")?;

    let community_ds = file.dataset("nodes/community")
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 read error: {e}")))?;
    let community_ids: Vec<u32> = community_ds.read_raw()
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 read error: {e}")))?;

    let mut extraction = ExtractionResult::default();

    for i in 0..ids.len() {
        let file_type = FileType::from_str_loose(&file_types[i]).unwrap_or(FileType::Code);
        let confidence = Confidence::from_str_loose(&confidences[i]);

        extraction.nodes.push(crate::types::Node {
            id: ids[i].clone(),
            label: labels[i].clone(),
            file_type,
            source_file: source_files[i].clone(),
            source_location: None,
            confidence,
            confidence_score: confidence.map(|c| c.default_score()),
            community: Some(community_ids[i] as usize),
            norm_label: None,
            degree: None,
        });
    }

    // Read edges
    let e_sources = read_strings(&file, "edges/source")?;
    let e_targets = read_strings(&file, "edges/target")?;
    let e_relations = read_strings(&file, "edges/relation")?;
    let e_confidences = read_strings(&file, "edges/confidence")?;

    let weight_ds = file.dataset("edges/weight")
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 read error: {e}")))?;
    let e_weights: Vec<f64> = weight_ds.read_raw()
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 read error: {e}")))?;

    for i in 0..e_sources.len() {
        let confidence = Confidence::from_str_loose(&e_confidences[i])
            .unwrap_or(Confidence::EXTRACTED);

        extraction.edges.push(crate::types::Edge {
            source: e_sources[i].clone(),
            target: e_targets[i].clone(),
            relation: e_relations[i].clone(),
            confidence,
            source_file: String::new(),
            source_location: None,
            confidence_score: Some(confidence.default_score()),
            weight: e_weights.get(i).copied().unwrap_or(1.0),
            original_src: None,
            original_tgt: None,
        });
    }

    Ok(crate::graph::build_from_extraction(&extraction))
}

// --- Helpers ---

#[cfg(feature = "hdf5")]
fn write_strings(
    group: &rust_hdf5::group::H5Group,
    name: &str,
    strings: &[&str],
) -> crate::error::Result<()> {
    // Use the file-level vlen string writer via the group's parent file
    // For now, write as fixed-size padded strings
    // HDF5 vlen strings need the file handle
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
        .shape(&[strings.len(), max_len])
        .create(name)
        .and_then(|ds| ds.write_raw(&padded))
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 write strings error: {e}")))?;

    // Store actual string count as attribute for reading back
    group
        .new_dataset::<u64>()
        .shape(&[1])
        .create(&format!("{name}_count"))
        .and_then(|ds| ds.write_raw(&[strings.len() as u64]))
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 attr error: {e}")))?;

    Ok(())
}

#[cfg(feature = "hdf5")]
fn read_strings(file: &H5File, path: &str) -> crate::error::Result<Vec<String>> {
    let ds = file.dataset(path)
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 read error: {e}")))?;

    let shape = ds.shape();
    if shape.len() != 2 {
        return Err(crate::error::EngramError::Other(
            format!("Expected 2D dataset for strings at {path}, got {shape:?}")
        ));
    }

    let n_strings = shape[0];
    let max_len = shape[1];

    let raw: Vec<u8> = ds.read_raw()
        .map_err(|e| crate::error::EngramError::Other(format!("HDF5 read error: {e}")))?;

    let mut strings = Vec::with_capacity(n_strings);
    for i in 0..n_strings {
        let start = i * max_len;
        let end = start + max_len;
        let slice = &raw[start..end.min(raw.len())];
        // Trim null padding
        let trimmed = match slice.iter().position(|&b| b == 0) {
            Some(pos) => &slice[..pos],
            None => slice,
        };
        strings.push(String::from_utf8_lossy(trimmed).to_string());
    }

    Ok(strings)
}

// Stub when hdf5 feature is not enabled
#[cfg(not(feature = "hdf5"))]
pub fn save_hdf5(
    _graph: &crate::graph::EngramGraph,
    _communities: &std::collections::HashMap<usize, Vec<String>>,
    _path: &std::path::Path,
) -> crate::error::Result<()> {
    Err(crate::error::EngramError::Other(
        "HDF5 support requires --features hdf5".to_string(),
    ))
}

#[cfg(not(feature = "hdf5"))]
pub fn load_hdf5(_path: &std::path::Path) -> crate::error::Result<crate::graph::EngramGraph> {
    Err(crate::error::EngramError::Other(
        "HDF5 support requires --features hdf5".to_string(),
    ))
}
