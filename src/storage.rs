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
    let source_locations = read_vlen(&file, "nodes/source_location").unwrap_or_default();
    let uuids = read_vlen(&file, "nodes/uuid").unwrap_or_default();
    let fingerprints = read_vlen(&file, "nodes/fingerprint").unwrap_or_default();
    let logical_keys = read_vlen(&file, "nodes/logical_key").unwrap_or_default();

    let community_ids: Vec<u32> = file
        .dataset("nodes/community")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();

    let mut extraction = ExtractionResult::default();

    for (i, id) in ids.iter().enumerate() {
        let loc = source_locations.get(i).cloned().unwrap_or_default();
        extraction.nodes.push(crate::types::Node {
            id: id.clone(),
            label: labels.get(i).cloned().unwrap_or_default(),
            file_type: FileType::from_str_loose(
                file_types.get(i).map(|s| s.as_str()).unwrap_or("code"),
            )
            .unwrap_or(FileType::Code),
            source_file: source_files.get(i).cloned().unwrap_or_default(),
            source_location: if loc.is_empty() { None } else { Some(loc) },
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
            uuid: {
                let v = uuids.get(i).cloned().unwrap_or_default();
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            },
            fingerprint: {
                let v = fingerprints.get(i).cloned().unwrap_or_default();
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            },
            logical_key: {
                let v = logical_keys.get(i).cloned().unwrap_or_default();
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            },
        });
    }

    let e_src = read_vlen(&file, "edges/source")?;
    let e_tgt = read_vlen(&file, "edges/target")?;
    let e_rel = read_vlen(&file, "edges/relation")?;
    let e_conf = read_vlen(&file, "edges/confidence")?;
    let e_src_files = read_vlen(&file, "edges/source_file").unwrap_or_default();
    let e_src_locs = read_vlen(&file, "edges/source_location").unwrap_or_default();
    let e_weight: Vec<f64> = file
        .dataset("edges/weight")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();

    for i in 0..e_src.len() {
        let confidence =
            Confidence::from_str_loose(e_conf.get(i).map(|s| s.as_str()).unwrap_or("EXTRACTED"))
                .unwrap_or(Confidence::EXTRACTED);
        let sf = e_src_files.get(i).cloned().unwrap_or_default();
        let sl = e_src_locs.get(i).cloned().unwrap_or_default();

        extraction.edges.push(crate::types::Edge {
            source: e_src[i].clone(),
            target: e_tgt[i].clone(),
            relation: e_rel.get(i).cloned().unwrap_or_default(),
            confidence,
            source_file: sf,
            source_location: if sl.is_empty() { None } else { Some(sl) },
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
/// Knowledge entries have their own UUID (knowledge_uuid).
/// Links between knowledge and code nodes are stored in /links/ group.
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
        mut k_uuids,
        mut titles,
        mut types,
        mut descriptions,
        mut confidences,
        mut obs,
        mut tag_list,
        mut link_k_uuids,
        mut link_n_uuids,
        mut link_relations,
    ) = {
        let file = H5File::open(h5_path)
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5 open: {e}")))?;

        let ku = read_vlen(&file, "knowledge/uuid").unwrap_or_default();
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
        let tl = read_vlen(&file, "knowledge/tags").unwrap_or_default();
        let lk = read_vlen(&file, "links/knowledge_uuid").unwrap_or_default();
        let ln = read_vlen(&file, "links/node_uuid").unwrap_or_default();
        let lr = read_vlen(&file, "links/relation").unwrap_or_default();
        (ku, t, ty, d, c, o, tl, lk, ln, lr)
    };

    // Check if exists — reinforce by title match
    let existing_idx = titles.iter().position(|t| t == title);
    let knowledge_uuid = if let Some(idx) = existing_idx {
        obs[idx] += 1;
        confidences[idx] = 1.0 - (1.0 - confidences[idx]) * 0.8;
        if !description.is_empty() && descriptions[idx] != description {
            descriptions[idx] = format!("{}\n---\n{}", descriptions[idx], description);
        }
        // Merge tags
        let existing_tags: Vec<&str> = tag_list[idx].split(',').filter(|s| !s.is_empty()).collect();
        let mut merged_tags: Vec<String> = existing_tags.iter().map(|s| s.to_string()).collect();
        for tag in tags {
            if !merged_tags.contains(tag) {
                merged_tags.push(tag.clone());
            }
        }
        tag_list[idx] = merged_tags.join(",");
        k_uuids[idx].clone()
    } else {
        // New knowledge entry
        let new_uuid = uuid::Uuid::new_v4().to_string();
        k_uuids.push(new_uuid.clone());
        titles.push(title.to_string());
        types.push(knowledge_type.to_string());
        descriptions.push(description.to_string());
        confidences.push(confidence);
        obs.push(observations);
        tag_list.push(tags.join(","));
        new_uuid
    };

    // Add links: knowledge_uuid → node_uuid (use node id as fallback for node_uuid)
    for node_ref in related_nodes {
        if !link_k_uuids
            .iter()
            .zip(link_n_uuids.iter())
            .any(|(k, n)| k == &knowledge_uuid && n == node_ref)
        {
            link_k_uuids.push(knowledge_uuid.clone());
            link_n_uuids.push(node_ref.clone());
            link_relations.push("related_to".to_string());
        }
    }

    // Rewrite knowledge group (open_rw can't delete existing datasets, so recreate file knowledge section)
    // For now: reload full file, replace knowledge, save
    let graph = load_hdf5(h5_path)?;
    let communities = crate::cluster::cluster(&graph);

    // Save with knowledge (related moved to /links/ group)
    let empty_related: Vec<String> = Vec::new();
    save_hdf5_with_knowledge(
        &graph,
        &communities,
        h5_path,
        &k_uuids,
        &titles,
        &types,
        &descriptions,
        &confidences,
        &obs,
        &empty_related,
        &tag_list,
    )?;

    // Save links separately by reopening
    save_links(h5_path, &link_k_uuids, &link_n_uuids, &link_relations)
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

/// Public wrapper for save_hdf5_with_knowledge (used by registry workspace sync).
#[allow(clippy::too_many_arguments)]
pub fn save_hdf5_with_knowledge_pub(
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
    save_hdf5_with_knowledge(
        graph,
        communities,
        path,
        k_titles,
        k_types,
        k_descriptions,
        k_confidences,
        k_observations,
        k_related,
        k_tags,
    )
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
    let (
        ids,
        labels,
        file_types,
        source_files,
        confidences,
        source_locations,
        uuids,
        fingerprints,
        logical_keys,
        community_ids,
    ) = collect_node_data(graph, &comm_map);

    let nodes_grp = file
        .create_group("nodes")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;

    write_vlen(&nodes_grp, "id", &ids)?;
    write_vlen(&nodes_grp, "label", &labels)?;
    write_vlen(&nodes_grp, "file_type", &file_types)?;
    write_vlen(&nodes_grp, "source_file", &source_files)?;
    write_vlen(&nodes_grp, "confidence", &confidences)?;
    write_vlen(&nodes_grp, "source_location", &source_locations)?;
    write_vlen(&nodes_grp, "uuid", &uuids)?;
    write_vlen(&nodes_grp, "fingerprint", &fingerprints)?;
    write_vlen(&nodes_grp, "logical_key", &logical_keys)?;

    if !community_ids.is_empty() {
        nodes_grp
            .new_dataset::<u32>()
            .shape([community_ids.len()])
            .create("community")
            .and_then(|ds| ds.write_raw(&community_ids))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    }

    // --- Edges ---
    let (e_src, e_tgt, e_rel, e_conf, e_src_files, e_src_locs, e_weight) = collect_edge_data(graph);

    let edges_grp = file
        .create_group("edges")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;

    write_vlen(&edges_grp, "source", &e_src)?;
    write_vlen(&edges_grp, "target", &e_tgt)?;
    write_vlen(&edges_grp, "relation", &e_rel)?;
    write_vlen(&edges_grp, "confidence", &e_conf)?;
    write_vlen(&edges_grp, "source_file", &e_src_files)?;
    write_vlen(&edges_grp, "source_location", &e_src_locs)?;

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
    let mut uuids = Vec::new();
    let mut fingerprints = Vec::new();
    let mut logical_keys = Vec::new();
    let mut confidences = Vec::new();
    let mut source_locations = Vec::new();
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
            source_locations.push(node.source_location.clone().unwrap_or_default());
            uuids.push(node.uuid.clone().unwrap_or_default());
            fingerprints.push(node.fingerprint.clone().unwrap_or_default());
            logical_keys.push(node.logical_key.clone().unwrap_or_default());
            community_ids.push(comm_map.get(id).copied().unwrap_or(0) as u32);
        }
    }

    (
        ids,
        labels,
        file_types,
        source_files,
        confidences,
        source_locations,
        uuids,
        fingerprints,
        logical_keys,
        community_ids,
    )
}

#[allow(clippy::type_complexity)]
fn collect_edge_data(
    graph: &KodexGraph,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<f64>,
) {
    let mut e_src = Vec::new();
    let mut e_tgt = Vec::new();
    let mut e_rel = Vec::new();
    let mut e_conf = Vec::new();
    let mut e_src_files = Vec::new();
    let mut e_src_locs = Vec::new();
    let mut e_weight: Vec<f64> = Vec::new();

    for (src, tgt, edge) in graph.edges() {
        e_src.push(src.to_string());
        e_tgt.push(tgt.to_string());
        e_rel.push(edge.relation.clone());
        e_conf.push(edge.confidence.to_string());
        e_src_files.push(edge.source_file.clone());
        e_src_locs.push(edge.source_location.clone().unwrap_or_default());
        e_weight.push(edge.weight);
    }

    (
        e_src,
        e_tgt,
        e_rel,
        e_conf,
        e_src_files,
        e_src_locs,
        e_weight,
    )
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

/// Remove knowledge entries matching a filter.
/// Returns how many were removed.
pub fn forget_knowledge(
    h5_path: &Path,
    title_match: Option<&str>,
    type_match: Option<&str>,
    project_match: Option<&str>,
    below_confidence: Option<f64>,
) -> crate::error::Result<usize> {
    if !h5_path.exists() {
        return Ok(0);
    }

    let entries = load_knowledge_entries(h5_path)?;
    let before = entries.len();

    let mut keep_titles = Vec::new();
    let mut keep_types = Vec::new();
    let mut keep_descriptions = Vec::new();
    let mut keep_confidences: Vec<f64> = Vec::new();
    let mut keep_observations: Vec<u32> = Vec::new();
    let mut keep_related: Vec<String> = Vec::new();
    let mut keep_tags = Vec::new();

    for (title, ktype, desc, conf, obs, related, tags) in &entries {
        let should_remove = title_match.map(|m| title.contains(m)).unwrap_or(false)
            || type_match.map(|m| ktype == m).unwrap_or(false)
            || project_match.map(|m| related.contains(m)).unwrap_or(false)
            || below_confidence.map(|c| *conf < c).unwrap_or(false);

        if !should_remove {
            keep_titles.push(title.clone());
            keep_types.push(ktype.clone());
            keep_descriptions.push(desc.clone());
            keep_confidences.push(*conf);
            keep_observations.push(*obs);
            keep_related.push(related.clone());
            keep_tags.push(tags.clone());
        }
    }

    let removed = before - keep_titles.len();
    if removed == 0 {
        return Ok(0);
    }

    // Reload graph, re-save with filtered knowledge
    let graph = load_hdf5(h5_path)?;
    let communities = crate::cluster::cluster(&graph);
    save_hdf5_with_knowledge(
        &graph,
        &communities,
        h5_path,
        &keep_titles,
        &keep_types,
        &keep_descriptions,
        &keep_confidences,
        &keep_observations,
        &keep_related,
        &keep_tags,
    )?;

    Ok(removed)
}

/// Remove a project's code nodes/edges from the global h5.
pub fn forget_project(h5_path: &Path, project_path: &str) -> crate::error::Result<usize> {
    if !h5_path.exists() {
        return Ok(0);
    }

    let graph = load_hdf5(h5_path)?;
    let before = graph.node_count();

    // Find nodes belonging to this project
    let to_remove: Vec<String> = graph
        .node_ids()
        .filter(|id| {
            graph
                .get_node(id)
                .map(|n| n.source_file.starts_with(project_path))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    // Rebuild graph without those nodes
    let extraction = crate::types::ExtractionResult {
        nodes: graph
            .node_ids()
            .filter(|id| !to_remove.contains(id))
            .filter_map(|id| graph.get_node(id).cloned())
            .collect(),
        edges: graph
            .edges()
            .filter(|(src, tgt, _)| {
                !to_remove.contains(&src.to_string()) && !to_remove.contains(&tgt.to_string())
            })
            .map(|(_, _, e)| e.clone())
            .collect(),
        ..Default::default()
    };

    let new_graph = crate::graph::build_from_extraction(&extraction);
    let communities = crate::cluster::cluster(&new_graph);

    // Preserve knowledge
    let knowledge = load_knowledge_entries(h5_path).unwrap_or_default();
    let (kt, kty, kd, kc, ko, kr, ktg): (Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>) =
        knowledge.into_iter().fold(
            (vec![], vec![], vec![], vec![], vec![], vec![], vec![]),
            |(mut t, mut ty, mut d, mut c, mut o, mut r, mut tg), entry| {
                t.push(entry.0);
                ty.push(entry.1);
                d.push(entry.2);
                c.push(entry.3);
                o.push(entry.4);
                r.push(entry.5);
                tg.push(entry.6);
                (t, ty, d, c, o, r, tg)
            },
        );

    save_hdf5_with_knowledge(
        &new_graph,
        &communities,
        h5_path,
        &kt,
        &kty,
        &kd,
        &kc,
        &ko,
        &kr,
        &ktg,
    )?;

    Ok(before - new_graph.node_count())
}

/// Merge a project's extraction into the global h5.
/// Loads existing, removes old project nodes, adds new ones, re-saves.
pub fn merge_project(
    h5_path: &Path,
    project_name: &str,
    new_extraction: &ExtractionResult,
) -> crate::error::Result<()> {
    let _ = std::fs::create_dir_all(h5_path.parent().unwrap_or(Path::new(".")));

    // Load existing graph (if any)
    let mut existing_extraction = if h5_path.exists() {
        // Load as extraction (preserves all fields)
        let file = H5File::open(h5_path)
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5 open: {e}")))?;

        let mut ext = load_extraction_from_file(&file)?;

        // Remove nodes/edges belonging to this project
        ext.nodes
            .retain(|n| !n.source_file.starts_with(project_name));
        ext.edges
            .retain(|e| !e.source_file.starts_with(project_name));

        ext
    } else {
        ExtractionResult::default()
    };

    // Preserve knowledge from existing file
    let knowledge = if h5_path.exists() {
        load_knowledge_entries(h5_path).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Assign stable IDs to new nodes (match against existing by fingerprint/score)
    let mut new_nodes = new_extraction.nodes.clone();
    crate::fingerprint::assign_stable_ids(&existing_extraction.nodes, &mut new_nodes);

    // Merge
    existing_extraction.nodes.extend(new_nodes);
    existing_extraction
        .edges
        .extend(new_extraction.edges.clone());

    // Build unified graph
    let graph = crate::graph::build_from_extraction(&existing_extraction);
    let communities = crate::cluster::cluster(&graph);

    // Unpack knowledge
    let (kt, kty, kd, kc, ko, kr, ktg) = unpack_knowledge(knowledge);

    save_hdf5_with_knowledge(
        &graph,
        &communities,
        h5_path,
        &kt,
        &kty,
        &kd,
        &kc,
        &ko,
        &kr,
        &ktg,
    )
}

/// Load raw extraction data from an open HDF5 file (preserving all fields).
fn load_extraction_from_file(file: &H5File) -> crate::error::Result<ExtractionResult> {
    let ids = read_vlen(file, "nodes/id")?;
    let labels = read_vlen(file, "nodes/label")?;
    let file_types = read_vlen(file, "nodes/file_type")?;
    let source_files = read_vlen(file, "nodes/source_file")?;
    let confidences = read_vlen(file, "nodes/confidence")?;
    let source_locations = read_vlen(file, "nodes/source_location").unwrap_or_default();
    let uuids = read_vlen(file, "nodes/uuid").unwrap_or_default();
    let fingerprints = read_vlen(file, "nodes/fingerprint").unwrap_or_default();
    let logical_keys = read_vlen(file, "nodes/logical_key").unwrap_or_default();
    let community_ids: Vec<u32> = file
        .dataset("nodes/community")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();

    let mut ext = ExtractionResult::default();

    for (i, id) in ids.iter().enumerate() {
        let loc = source_locations.get(i).cloned().unwrap_or_default();
        ext.nodes.push(crate::types::Node {
            id: id.clone(),
            label: labels.get(i).cloned().unwrap_or_default(),
            file_type: FileType::from_str_loose(
                file_types.get(i).map(|s| s.as_str()).unwrap_or("code"),
            )
            .unwrap_or(FileType::Code),
            source_file: source_files.get(i).cloned().unwrap_or_default(),
            source_location: if loc.is_empty() { None } else { Some(loc) },
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
            uuid: {
                let v = uuids.get(i).cloned().unwrap_or_default();
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            },
            fingerprint: {
                let v = fingerprints.get(i).cloned().unwrap_or_default();
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            },
            logical_key: {
                let v = logical_keys.get(i).cloned().unwrap_or_default();
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            },
        });
    }

    let e_src = read_vlen(file, "edges/source")?;
    let e_tgt = read_vlen(file, "edges/target")?;
    let e_rel = read_vlen(file, "edges/relation")?;
    let e_conf = read_vlen(file, "edges/confidence")?;
    let e_src_files = read_vlen(file, "edges/source_file").unwrap_or_default();
    let e_src_locs = read_vlen(file, "edges/source_location").unwrap_or_default();
    let e_weight: Vec<f64> = file
        .dataset("edges/weight")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();

    for i in 0..e_src.len() {
        let confidence =
            Confidence::from_str_loose(e_conf.get(i).map(|s| s.as_str()).unwrap_or("EXTRACTED"))
                .unwrap_or(Confidence::EXTRACTED);
        let sf = e_src_files.get(i).cloned().unwrap_or_default();
        let sl = e_src_locs.get(i).cloned().unwrap_or_default();

        ext.edges.push(crate::types::Edge {
            source: e_src[i].clone(),
            target: e_tgt[i].clone(),
            relation: e_rel.get(i).cloned().unwrap_or_default(),
            confidence,
            source_file: sf,
            source_location: if sl.is_empty() { None } else { Some(sl) },
            confidence_score: Some(confidence.default_score()),
            weight: e_weight.get(i).copied().unwrap_or(1.0),
            original_src: None,
            original_tgt: None,
        });
    }

    Ok(ext)
}

#[allow(clippy::type_complexity)]
pub fn unpack_knowledge(
    entries: Vec<(String, String, String, f64, u32, String, String)>,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<f64>,
    Vec<u32>,
    Vec<String>,
    Vec<String>,
) {
    let mut t = Vec::new();
    let mut ty = Vec::new();
    let mut d = Vec::new();
    let mut c = Vec::new();
    let mut o = Vec::new();
    let mut r = Vec::new();
    let mut tg = Vec::new();
    for (title, ktype, desc, conf, obs, related, tags) in entries {
        t.push(title);
        ty.push(ktype);
        d.push(desc);
        c.push(conf);
        o.push(obs);
        r.push(related);
        tg.push(tags);
    }
    (t, ty, d, c, o, r, tg)
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
                    uuid: None,
                    fingerprint: None,
                    logical_key: None,
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
                    uuid: None,
                    fingerprint: None,
                    logical_key: None,
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
