//! HDF5 storage — single file for code graph + knowledge.
//!
//! All read/write goes through KodexData.

use std::collections::HashMap;
use std::path::Path;

use rust_hdf5::file::H5File;

use crate::graph::KodexGraph;
use crate::types::{
    Confidence, ExtractionResult, FileType, KnowledgeEntry, KnowledgeLink, KodexData,
};

// Core API

pub fn save(path: &Path, data: &KodexData) -> crate::error::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = H5File::create(path)
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 create: {e}")))?;
    let graph = crate::graph::build_from_extraction(&data.extraction);
    let communities = crate::cluster::cluster(&graph);
    file.set_attr_string("version", CURRENT_VERSION).ok();
    file.set_attr_numeric("node_count", &(graph.node_count() as u64))
        .ok();
    file.set_attr_numeric("edge_count", &(graph.edge_count() as u64))
        .ok();
    write_nodes(&file, &graph, &communities)?;
    write_edges(&file, &graph)?;
    write_knowledge(&file, &data.knowledge)?;
    write_links(&file, &data.links)?;
    file.close()
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 close: {e}")))?;
    Ok(())
}

/// Current storage format version.
const CURRENT_VERSION: &str = "0.3.0";

/// Known versions and their migration paths.
#[allow(dead_code)]
const KNOWN_VERSIONS: &[&str] = &["0.1.0", "0.2.0", "0.3.0"];

pub fn load(path: &Path) -> crate::error::Result<KodexData> {
    let file = H5File::open(path)
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5 open: {e}")))?;

    let file_version = file
        .attr_string("version")
        .unwrap_or_else(|_| "0.1.0".to_string());

    let mut data = KodexData {
        extraction: read_extraction(&file)?,
        knowledge: read_knowledge(&file)?,
        links: read_links(&file)?,
    };

    // Auto-migrate if needed
    if file_version != CURRENT_VERSION {
        migrate(&mut data, &file_version);
        // Re-save with current version (upgrade in place)
        drop(file);
        let _ = save(path, &data);
    }

    Ok(data)
}

/// Migrate data from an older version to current.
fn migrate(data: &mut KodexData, from_version: &str) {
    // v0.1.0 → v0.2.0: no uuid/fingerprint/logical_key on nodes
    if from_version == "0.1.0" {
        for node in &mut data.extraction.nodes {
            if node.uuid.is_none() {
                node.uuid = Some(uuid::Uuid::new_v4().to_string());
            }
            if node.fingerprint.is_none() {
                node.fingerprint = Some(crate::fingerprint::compute_fingerprint(
                    &node.label,
                    &node.file_type.to_string(),
                    &node.source_file,
                    node.source_location.as_deref(),
                    None,
                ));
            }
            if node.logical_key.is_none() {
                node.logical_key = Some(crate::fingerprint::logical_key(
                    &node.source_file,
                    &node.label,
                ));
            }
        }
    }

    // v0.1.0/v0.2.0 → v0.3.0: knowledge entries need uuid
    if from_version == "0.1.0" || from_version == "0.2.0" {
        for entry in &mut data.knowledge {
            if entry.uuid.is_empty() {
                entry.uuid = uuid::Uuid::new_v4().to_string();
            }
        }
    }

    // Future migrations go here:
    // if from_version < "0.4.0" { ... }
}

pub fn load_graph(path: &Path) -> crate::error::Result<KodexGraph> {
    let data = load(path)?;
    Ok(crate::graph::build_from_extraction(&data.extraction))
}

pub fn save_hdf5(
    graph: &KodexGraph,
    _communities: &HashMap<usize, Vec<String>>,
    path: &Path,
) -> crate::error::Result<()> {
    let extraction = graph_to_extraction(graph);
    let existing = if path.exists() { load(path).ok() } else { None };
    let data = KodexData {
        extraction,
        knowledge: existing
            .as_ref()
            .map(|d| d.knowledge.clone())
            .unwrap_or_default(),
        links: existing
            .as_ref()
            .map(|d| d.links.clone())
            .unwrap_or_default(),
    };
    save(path, &data)
}

pub fn load_hdf5(path: &Path) -> crate::error::Result<KodexGraph> {
    load_graph(path)
}

// Knowledge operations

#[allow(clippy::too_many_arguments)]
pub fn append_knowledge(
    h5_path: &Path,
    title: &str,
    knowledge_type: &str,
    description: &str,
    confidence: f64,
    _observations: u32,
    related_nodes: &[String],
    tags: &[String],
) -> crate::error::Result<()> {
    let nodes = if related_nodes.is_empty() {
        None
    } else {
        Some(related_nodes)
    };
    append_knowledge_with_uuid(
        h5_path, None, title, knowledge_type, description, confidence, nodes, tags,
    )
    .map(|_| ())
}

/// Core knowledge upsert. UUID is the only lookup key.
/// If uuid is provided, finds and updates existing entry.
/// If uuid is None, creates new entry with fresh UUID.
/// Returns the UUID of the created or updated entry.
///
/// `related_nodes`:
/// - `None` → don't touch existing links
/// - `Some(&[])` → clear all links for this knowledge
/// - `Some(&[...])` → replace links with these nodes
#[allow(clippy::too_many_arguments)]
pub fn append_knowledge_with_uuid(
    h5_path: &Path,
    knowledge_uuid: Option<&str>,
    title: &str,
    knowledge_type: &str,
    description: &str,
    confidence: f64,
    related_nodes: Option<&[String]>,
    tags: &[String],
) -> crate::error::Result<String> {
    let mut data = if h5_path.exists() {
        load(h5_path)?
    } else {
        return Err(crate::error::KodexError::Other(
            "HDF5 file does not exist. Run `kodex run` first.".to_string(),
        ));
    };
    // UUID is the only lookup key. No title fallback.
    let existing =
        knowledge_uuid.and_then(|uuid| data.knowledge.iter_mut().find(|k| k.uuid == uuid));
    let k_uuid = if let Some(entry) = existing {
        // Update existing entry (UUID stays the same)
        entry.observations += 1;
        entry.confidence = 1.0 - (1.0 - entry.confidence) * 0.8;
        // Title can change — it's display-only, not a key
        if !title.is_empty() {
            entry.title = title.to_string();
        }
        if !knowledge_type.is_empty() {
            entry.knowledge_type = knowledge_type.to_string();
        }
        if !description.is_empty() && entry.description != description {
            entry.description = format!("{}\n---\n{}", entry.description, description);
        }
        for tag in tags {
            if !entry.tags.contains(tag) {
                entry.tags.push(tag.clone());
            }
        }
        entry.uuid.clone()
    } else {
        let new_uuid = uuid::Uuid::new_v4().to_string();
        data.knowledge.push(KnowledgeEntry {
            uuid: new_uuid.clone(),
            title: title.to_string(),
            knowledge_type: knowledge_type.to_string(),
            description: description.to_string(),
            confidence,
            observations: 1,
            tags: tags.to_vec(),
        });
        new_uuid
    };
    // Handle links: None = don't touch, Some([]) = clear, Some([...]) = replace
    if let Some(nodes) = related_nodes {
        data.links.retain(|l| l.knowledge_uuid != k_uuid);
        for node_ref in nodes {
            data.links.push(KnowledgeLink {
                knowledge_uuid: k_uuid.clone(),
                node_uuid: node_ref.clone(),
                relation: "related_to".to_string(),
            });
        }
    }
    save(h5_path, &data)?;
    Ok(k_uuid)
}

#[allow(clippy::type_complexity)]
pub fn load_knowledge_entries(
    h5_path: &Path,
) -> crate::error::Result<Vec<(String, String, String, f64, u32, String, String)>> {
    let data = load(h5_path)?;
    Ok(data
        .knowledge
        .iter()
        .map(|k| {
            let related: Vec<&str> = data
                .links
                .iter()
                .filter(|l| l.knowledge_uuid == k.uuid)
                .map(|l| l.node_uuid.as_str())
                .collect();
            (
                k.title.clone(),
                k.knowledge_type.clone(),
                k.description.clone(),
                k.confidence,
                k.observations,
                related.join(","),
                k.tags.join(","),
            )
        })
        .collect())
}

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
    let mut data = load(h5_path)?;
    let before = data.knowledge.len();
    let remove_uuids: Vec<String> = data
        .knowledge
        .iter()
        .filter(|k| {
            title_match.map(|m| k.title.contains(m)).unwrap_or(false)
                || type_match.map(|m| k.knowledge_type == m).unwrap_or(false)
                || project_match
                    .map(|m| k.description.contains(m))
                    .unwrap_or(false)
                || below_confidence.map(|c| k.confidence < c).unwrap_or(false)
        })
        .map(|k| k.uuid.clone())
        .collect();
    if remove_uuids.is_empty() {
        return Ok(0);
    }
    data.knowledge.retain(|k| !remove_uuids.contains(&k.uuid));
    data.links
        .retain(|l| !remove_uuids.contains(&l.knowledge_uuid));
    save(h5_path, &data)?;
    Ok(before - data.knowledge.len())
}

// Project operations

pub fn merge_project(
    h5_path: &Path,
    project_name: &str,
    new_extraction: &ExtractionResult,
) -> crate::error::Result<()> {
    let mut data = if h5_path.exists() {
        load(h5_path)?
    } else {
        KodexData::default()
    };
    // Save existing project nodes for UUID matching BEFORE removing them
    let old_project_nodes: Vec<_> = data
        .extraction
        .nodes
        .iter()
        .filter(|n| n.source_file.starts_with(project_name))
        .cloned()
        .collect();

    // Remove old project slice
    data.extraction
        .nodes
        .retain(|n| !n.source_file.starts_with(project_name));
    data.extraction
        .edges
        .retain(|e| !e.source_file.starts_with(project_name));

    // Match new nodes against old project nodes to preserve UUIDs
    let mut new_nodes = new_extraction.nodes.clone();
    crate::fingerprint::assign_stable_ids(&old_project_nodes, &mut new_nodes);
    data.extraction.nodes.extend(new_nodes);
    data.extraction.edges.extend(new_extraction.edges.clone());

    // Clean stale links — remove links to node UUIDs that no longer exist
    let valid_node_uuids: std::collections::HashSet<&str> = data
        .extraction
        .nodes
        .iter()
        .filter_map(|n| n.uuid.as_deref())
        .collect();
    data.links
        .retain(|l| valid_node_uuids.contains(l.node_uuid.as_str()) || l.node_uuid.is_empty());

    save(h5_path, &data)
}

pub fn forget_project(h5_path: &Path, project_path: &str) -> crate::error::Result<usize> {
    if !h5_path.exists() {
        return Ok(0);
    }
    let mut data = load(h5_path)?;
    let before = data.extraction.nodes.len();
    data.extraction
        .nodes
        .retain(|n| !n.source_file.starts_with(project_path));
    data.extraction
        .edges
        .retain(|e| !e.source_file.starts_with(project_path));

    // Clean stale links
    let valid_node_uuids: std::collections::HashSet<&str> = data
        .extraction
        .nodes
        .iter()
        .filter_map(|n| n.uuid.as_deref())
        .collect();
    data.links
        .retain(|l| valid_node_uuids.contains(l.node_uuid.as_str()) || l.node_uuid.is_empty());

    save(h5_path, &data)?;
    Ok(before - data.extraction.nodes.len())
}



// HDF5 internals

fn write_nodes(
    file: &H5File,
    graph: &KodexGraph,
    communities: &HashMap<usize, Vec<String>>,
) -> crate::error::Result<()> {
    let comm_map = crate::export::node_community_map(communities);
    let grp = file
        .create_group("nodes")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    let (mut ids, mut labels, mut ft, mut sf, mut conf, mut sl, mut uu, mut fp, mut lk) = (
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
    );
    let mut cids: Vec<u32> = vec![];
    for id in graph.node_ids() {
        if let Some(n) = graph.get_node(id) {
            ids.push(id.clone());
            labels.push(n.label.clone());
            ft.push(n.file_type.to_string());
            sf.push(n.source_file.clone());
            conf.push(
                n.confidence
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "EXTRACTED".to_string()),
            );
            sl.push(n.source_location.clone().unwrap_or_default());
            uu.push(n.uuid.clone().unwrap_or_default());
            fp.push(n.fingerprint.clone().unwrap_or_default());
            lk.push(n.logical_key.clone().unwrap_or_default());
            cids.push(comm_map.get(id).copied().unwrap_or(0) as u32);
        }
    }
    write_vlen(&grp, "id", &ids)?;
    write_vlen(&grp, "label", &labels)?;
    write_vlen(&grp, "file_type", &ft)?;
    write_vlen(&grp, "source_file", &sf)?;
    write_vlen(&grp, "confidence", &conf)?;
    write_vlen(&grp, "source_location", &sl)?;
    write_vlen(&grp, "uuid", &uu)?;
    write_vlen(&grp, "fingerprint", &fp)?;
    write_vlen(&grp, "logical_key", &lk)?;
    if !cids.is_empty() {
        grp.new_dataset::<u32>()
            .shape([cids.len()])
            .create("community")
            .and_then(|ds| ds.write_raw(&cids))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    }
    Ok(())
}

fn write_edges(file: &H5File, graph: &KodexGraph) -> crate::error::Result<()> {
    let grp = file
        .create_group("edges")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    let (mut s, mut t, mut r, mut c, mut sf, mut sl) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    let mut w: Vec<f64> = vec![];
    for (src, tgt, edge) in graph.edges() {
        s.push(src.to_string());
        t.push(tgt.to_string());
        r.push(edge.relation.clone());
        c.push(edge.confidence.to_string());
        sf.push(edge.source_file.clone());
        sl.push(edge.source_location.clone().unwrap_or_default());
        w.push(edge.weight);
    }
    write_vlen(&grp, "source", &s)?;
    write_vlen(&grp, "target", &t)?;
    write_vlen(&grp, "relation", &r)?;
    write_vlen(&grp, "confidence", &c)?;
    write_vlen(&grp, "source_file", &sf)?;
    write_vlen(&grp, "source_location", &sl)?;
    if !w.is_empty() {
        grp.new_dataset::<f64>()
            .shape([w.len()])
            .create("weight")
            .and_then(|ds| ds.write_raw(&w))
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    }
    Ok(())
}

fn write_knowledge(file: &H5File, knowledge: &[KnowledgeEntry]) -> crate::error::Result<()> {
    if knowledge.is_empty() {
        return Ok(());
    }
    let grp = file
        .create_group("knowledge")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    let uu: Vec<String> = knowledge.iter().map(|k| k.uuid.clone()).collect();
    let ti: Vec<String> = knowledge.iter().map(|k| k.title.clone()).collect();
    let ty: Vec<String> = knowledge.iter().map(|k| k.knowledge_type.clone()).collect();
    let de: Vec<String> = knowledge.iter().map(|k| k.description.clone()).collect();
    let tg: Vec<String> = knowledge.iter().map(|k| k.tags.join(",")).collect();
    let co: Vec<f64> = knowledge.iter().map(|k| k.confidence).collect();
    let ob: Vec<u32> = knowledge.iter().map(|k| k.observations).collect();
    write_vlen(&grp, "uuid", &uu)?;
    write_vlen(&grp, "titles", &ti)?;
    write_vlen(&grp, "types", &ty)?;
    write_vlen(&grp, "descriptions", &de)?;
    write_vlen(&grp, "tags", &tg)?;
    grp.new_dataset::<f64>()
        .shape([co.len()])
        .create("confidence")
        .and_then(|ds| ds.write_raw(&co))
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    grp.new_dataset::<u32>()
        .shape([ob.len()])
        .create("observations")
        .and_then(|ds| ds.write_raw(&ob))
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    Ok(())
}

fn write_links(file: &H5File, links: &[KnowledgeLink]) -> crate::error::Result<()> {
    if links.is_empty() {
        return Ok(());
    }
    let grp = file
        .create_group("links")
        .map_err(|e| crate::error::KodexError::Other(format!("HDF5: {e}")))?;
    let ku: Vec<String> = links.iter().map(|l| l.knowledge_uuid.clone()).collect();
    let nu: Vec<String> = links.iter().map(|l| l.node_uuid.clone()).collect();
    let re: Vec<String> = links.iter().map(|l| l.relation.clone()).collect();
    write_vlen(&grp, "knowledge_uuid", &ku)?;
    write_vlen(&grp, "node_uuid", &nu)?;
    write_vlen(&grp, "relation", &re)?;
    Ok(())
}

fn read_extraction(file: &H5File) -> crate::error::Result<ExtractionResult> {
    let ids = read_vlen(file, "nodes/id")?;
    let labels = read_vlen(file, "nodes/label")?;
    let ft = read_vlen(file, "nodes/file_type")?;
    let sf = read_vlen(file, "nodes/source_file")?;
    let conf = read_vlen(file, "nodes/confidence")?;
    let sl = read_vlen(file, "nodes/source_location").unwrap_or_default();
    let uu = read_vlen(file, "nodes/uuid").unwrap_or_default();
    let fp = read_vlen(file, "nodes/fingerprint").unwrap_or_default();
    let lk = read_vlen(file, "nodes/logical_key").unwrap_or_default();
    let cids: Vec<u32> = file
        .dataset("nodes/community")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();
    let opt =
        |v: &[String], i: usize| -> Option<String> { v.get(i).cloned().filter(|s| !s.is_empty()) };
    let mut ext = ExtractionResult::default();
    for (i, id) in ids.iter().enumerate() {
        ext.nodes.push(crate::types::Node {
            id: id.clone(),
            label: labels.get(i).cloned().unwrap_or_default(),
            file_type: FileType::from_str_loose(ft.get(i).map(|s| s.as_str()).unwrap_or("code"))
                .unwrap_or(FileType::Code),
            source_file: sf.get(i).cloned().unwrap_or_default(),
            source_location: opt(&sl, i),
            confidence: Confidence::from_str_loose(
                conf.get(i).map(|s| s.as_str()).unwrap_or("EXTRACTED"),
            ),
            confidence_score: None,
            community: cids.get(i).map(|&c| c as usize),
            norm_label: None,
            degree: None,
            uuid: opt(&uu, i),
            fingerprint: opt(&fp, i),
            logical_key: opt(&lk, i),
        });
    }
    let es = read_vlen(file, "edges/source")?;
    let et = read_vlen(file, "edges/target")?;
    let er = read_vlen(file, "edges/relation")?;
    let ec = read_vlen(file, "edges/confidence")?;
    let esf = read_vlen(file, "edges/source_file").unwrap_or_default();
    let esl = read_vlen(file, "edges/source_location").unwrap_or_default();
    let ew: Vec<f64> = file
        .dataset("edges/weight")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();
    for i in 0..es.len() {
        let c = Confidence::from_str_loose(ec.get(i).map(|s| s.as_str()).unwrap_or("EXTRACTED"))
            .unwrap_or(Confidence::EXTRACTED);
        ext.edges.push(crate::types::Edge {
            source: es[i].clone(),
            target: et[i].clone(),
            relation: er.get(i).cloned().unwrap_or_default(),
            confidence: c,
            source_file: esf.get(i).cloned().unwrap_or_default(),
            source_location: opt(&esl, i),
            confidence_score: Some(c.default_score()),
            weight: ew.get(i).copied().unwrap_or(1.0),
            original_src: None,
            original_tgt: None,
        });
    }
    Ok(ext)
}

fn read_knowledge(file: &H5File) -> crate::error::Result<Vec<KnowledgeEntry>> {
    let uu = read_vlen(file, "knowledge/uuid").unwrap_or_default();
    let ti = read_vlen(file, "knowledge/titles").unwrap_or_default();
    let ty = read_vlen(file, "knowledge/types").unwrap_or_default();
    let de = read_vlen(file, "knowledge/descriptions").unwrap_or_default();
    let tg = read_vlen(file, "knowledge/tags").unwrap_or_default();
    let co: Vec<f64> = file
        .dataset("knowledge/confidence")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();
    let ob: Vec<u32> = file
        .dataset("knowledge/observations")
        .and_then(|ds| ds.read_raw())
        .unwrap_or_default();
    Ok((0..ti.len())
        .map(|i| KnowledgeEntry {
            uuid: uu
                .get(i)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            title: ti[i].clone(),
            knowledge_type: ty.get(i).cloned().unwrap_or_default(),
            description: de.get(i).cloned().unwrap_or_default(),
            confidence: co.get(i).copied().unwrap_or(0.5),
            observations: ob.get(i).copied().unwrap_or(1),
            tags: tg
                .get(i)
                .map(|t| {
                    t.split(',')
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect())
}

fn read_links(file: &H5File) -> crate::error::Result<Vec<KnowledgeLink>> {
    let ku = read_vlen(file, "links/knowledge_uuid").unwrap_or_default();
    let nu = read_vlen(file, "links/node_uuid").unwrap_or_default();
    let re = read_vlen(file, "links/relation").unwrap_or_default();
    Ok((0..ku.len())
        .map(|i| KnowledgeLink {
            knowledge_uuid: ku[i].clone(),
            node_uuid: nu.get(i).cloned().unwrap_or_default(),
            relation: re.get(i).cloned().unwrap_or_default(),
        })
        .collect())
}

fn graph_to_extraction(graph: &KodexGraph) -> ExtractionResult {
    ExtractionResult {
        nodes: graph
            .node_ids()
            .filter_map(|id| graph.get_node(id).cloned())
            .collect(),
        edges: graph.edges().map(|(_, _, e)| e.clone()).collect(),
        ..Default::default()
    }
}

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
    match file.dataset(path) {
        Ok(ds) => ds
            .read_vlen_strings()
            .map_err(|e| crate::error::KodexError::Other(format!("HDF5 read {path}: {e}"))),
        Err(_) => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    #[test]
    fn test_hdf5_round_trip() {
        let dir = TempDir::new().unwrap();
        let h5 = dir.path().join("test.h5");
        let data = KodexData {
            extraction: ExtractionResult {
                nodes: vec![
                    crate::types::Node {
                        id: "a".into(),
                        label: "Alpha".into(),
                        file_type: FileType::Code,
                        source_file: "a.py".into(),
                        source_location: Some("L1".into()),
                        confidence: Some(Confidence::EXTRACTED),
                        confidence_score: Some(1.0),
                        community: None,
                        norm_label: None,
                        degree: None,
                        uuid: Some("uuid-a".into()),
                        fingerprint: Some("fp-a".into()),
                        logical_key: Some("a.py::Alpha".into()),
                    },
                    crate::types::Node {
                        id: "b".into(),
                        label: "Beta".into(),
                        file_type: FileType::Code,
                        source_file: "b.py".into(),
                        source_location: None,
                        confidence: Some(Confidence::INFERRED),
                        confidence_score: None,
                        community: None,
                        norm_label: None,
                        degree: None,
                        uuid: None,
                        fingerprint: None,
                        logical_key: None,
                    },
                ],
                edges: vec![crate::types::Edge {
                    source: "a".into(),
                    target: "b".into(),
                    relation: "imports".into(),
                    confidence: Confidence::EXTRACTED,
                    source_file: "a.py".into(),
                    source_location: Some("L2".into()),
                    confidence_score: Some(1.0),
                    weight: 1.0,
                    original_src: None,
                    original_tgt: None,
                }],
                ..Default::default()
            },
            knowledge: vec![KnowledgeEntry {
                uuid: "k-1".into(),
                title: "Test Pattern".into(),
                knowledge_type: "pattern".into(),
                description: "A test".into(),
                confidence: 0.6,
                observations: 1,
                tags: vec!["test".into()],
            }],
            links: vec![KnowledgeLink {
                knowledge_uuid: "k-1".into(),
                node_uuid: "uuid-a".into(),
                relation: "related_to".into(),
            }],
        };
        save(&h5, &data).unwrap();
        let loaded = load(&h5).unwrap();
        assert_eq!(loaded.extraction.nodes.len(), 2);
        assert_eq!(loaded.extraction.edges.len(), 1);
        assert_eq!(loaded.knowledge.len(), 1);
        assert_eq!(loaded.knowledge[0].title, "Test Pattern");
        assert_eq!(loaded.links.len(), 1);
        let na = loaded
            .extraction
            .nodes
            .iter()
            .find(|n| n.id == "a")
            .unwrap();
        assert_eq!(na.uuid.as_deref(), Some("uuid-a"));
        assert_eq!(na.source_location.as_deref(), Some("L1"));
        assert_eq!(loaded.extraction.edges[0].source_file, "a.py");
    }

    #[test]
    fn test_links_clear_vs_noop() {
        let dir = TempDir::new().unwrap();
        let h5 = dir.path().join("test_links.h5");
        // Create initial data with one knowledge + one link
        let data = KodexData {
            extraction: ExtractionResult::default(),
            knowledge: vec![KnowledgeEntry {
                uuid: "k-1".into(),
                title: "Pattern".into(),
                knowledge_type: "pattern".into(),
                description: "desc".into(),
                confidence: 0.6,
                observations: 1,
                tags: vec![],
            }],
            links: vec![KnowledgeLink {
                knowledge_uuid: "k-1".into(),
                node_uuid: "n-1".into(),
                relation: "related_to".into(),
            }],
        };
        save(&h5, &data).unwrap();

        // Update with None for related_nodes → links should be untouched
        append_knowledge_with_uuid(
            &h5, Some("k-1"), "Pattern", "pattern", "more info", 0.6, None, &[],
        )
        .unwrap();
        let loaded = load(&h5).unwrap();
        assert_eq!(loaded.links.len(), 1, "None should not touch links");

        // Update with Some(&[]) → links should be cleared
        append_knowledge_with_uuid(
            &h5,
            Some("k-1"),
            "Pattern",
            "pattern",
            "",
            0.6,
            Some(&[]),
            &[],
        )
        .unwrap();
        let loaded = load(&h5).unwrap();
        assert_eq!(loaded.links.len(), 0, "Some(&[]) should clear links");

        // Update with Some(&[new_node]) → should add link
        append_knowledge_with_uuid(
            &h5,
            Some("k-1"),
            "Pattern",
            "pattern",
            "",
            0.6,
            Some(&["n-2".to_string()]),
            &[],
        )
        .unwrap();
        let loaded = load(&h5).unwrap();
        assert_eq!(loaded.links.len(), 1);
        assert_eq!(loaded.links[0].node_uuid, "n-2");
    }
}
