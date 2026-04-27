//! SQLite storage — single file for code graph + knowledge.
//!
//! All read/write goes through KodexData.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection};

use crate::graph::KodexGraph;
use crate::types::{
    Confidence, ExtractionResult, FileType, KnowledgeEntry, KnowledgeLink, KodexData,
};

// Core API

pub fn save(path: &Path, data: &KodexData) -> crate::error::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = open_db(path)?;
    conn.execute_batch("BEGIN")?;
    // Clear and rewrite everything
    conn.execute_batch(
        "DELETE FROM nodes; DELETE FROM edges; DELETE FROM hyperedges;
         DELETE FROM knowledge; DELETE FROM links; DELETE FROM review_queue;",
    )?;
    // Community detection
    let communities = {
        let graph = crate::graph::build_from_extraction(&data.extraction);
        crate::cluster::cluster(&graph)
    };
    let comm_map = crate::export::node_community_map(&communities);
    write_nodes(&conn, &data.extraction.nodes, &comm_map)?;
    write_edges(&conn, &data.extraction.edges)?;
    write_hyperedges(&conn, &data.extraction.hyperedges)?;
    write_knowledge(&conn, &data.knowledge)?;
    write_links(&conn, &data.links)?;
    write_review_queue(&conn, &data.review_queue)?;
    conn.execute(
        "INSERT OR REPLACE INTO meta(key,value) VALUES('version',?1)",
        params![CURRENT_VERSION],
    )?;
    conn.execute_batch("COMMIT")?;
    cache_put(path, data);
    Ok(())
}

/// Save only knowledge/links/review_queue — true incremental, no graph rebuild.
pub fn save_knowledge_only(path: &Path, data: &KodexData) -> crate::error::Result<()> {
    if !path.exists() {
        return save(path, data);
    }
    let conn = open_db(path)?;
    conn.execute_batch("BEGIN")?;
    conn.execute_batch("DELETE FROM knowledge; DELETE FROM links; DELETE FROM review_queue;")?;
    write_knowledge(&conn, &data.knowledge)?;
    write_links(&conn, &data.links)?;
    write_review_queue(&conn, &data.review_queue)?;
    conn.execute_batch("COMMIT")?;
    // Update cache
    if let Some(mut cached) = cache_get(path) {
        cached.knowledge = data.knowledge.clone();
        cached.links = data.links.clone();
        cached.review_queue = data.review_queue.clone();
        cache_put(path, &cached);
    }
    Ok(())
}

/// Load only knowledge/links/review_queue — skips nodes/edges for memory efficiency.
pub fn load_knowledge_only(path: &Path) -> crate::error::Result<KodexData> {
    let conn = open_db(path)?;
    Ok(KodexData {
        extraction: ExtractionResult::default(),
        knowledge: read_knowledge(&conn)?,
        links: read_links(&conn)?,
        review_queue: read_review_queue(&conn)?,
    })
}

/// Current storage format version.
const CURRENT_VERSION: &str = "0.5.0";

// ---------------------------------------------------------------------------
// Path-keyed in-memory cache
// ---------------------------------------------------------------------------

use std::sync::Mutex;

/// Max cache entries.
const MAX_CACHE_ENTRIES: usize = 2;
/// Max estimated cache size in bytes.
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

static CACHE: Mutex<Option<HashMap<std::path::PathBuf, KodexData>>> = Mutex::new(None);

fn cache_get(path: &Path) -> Option<KodexData> {
    CACHE.lock().ok()?.as_ref()?.get(path).cloned()
}

fn estimate_size(data: &KodexData) -> usize {
    data.extraction.nodes.len() * 256
        + data.extraction.edges.len() * 128
        + data.knowledge.len() * 512
        + data.links.len() * 128
}

fn cache_put(path: &Path, data: &KodexData) {
    if let Ok(mut guard) = CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        if map.len() >= MAX_CACHE_ENTRIES && !map.contains_key(path) {
            if let Some(oldest) = map.keys().next().cloned() {
                map.remove(&oldest);
            }
        }
        let new_size = estimate_size(data);
        let total: usize = map.values().map(estimate_size).sum();
        if total + new_size > MAX_CACHE_BYTES {
            map.clear();
        }
        map.insert(path.to_path_buf(), data.clone());
    }
}

/// Invalidate cache for a specific path (call after external modification).
pub fn cache_remove(path: &Path) {
    if let Ok(mut guard) = CACHE.lock() {
        if let Some(map) = guard.as_mut() {
            map.remove(path);
        }
    }
}

pub fn load(path: &Path) -> crate::error::Result<KodexData> {
    if let Some(cached) = cache_get(path) {
        return Ok(cached);
    }
    let data = load_from_disk(path)?;
    cache_put(path, &data);
    Ok(data)
}

fn load_from_disk(path: &Path) -> crate::error::Result<KodexData> {
    let conn = open_db(path)?;
    let data = KodexData {
        extraction: read_extraction(&conn)?,
        knowledge: read_knowledge(&conn)?,
        links: read_links(&conn)?,
        review_queue: read_review_queue(&conn)?,
    };
    Ok(data)
}

pub fn load_graph(path: &Path) -> crate::error::Result<KodexGraph> {
    let data = load(path)?;
    Ok(crate::graph::build_from_extraction(&data.extraction))
}

pub fn save_db(
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
        review_queue: existing
            .as_ref()
            .map(|d| d.review_queue.clone())
            .unwrap_or_default(),
    };
    save(path, &data)
}

pub fn load_db(path: &Path) -> crate::error::Result<KodexGraph> {
    load_graph(path)
}

// Knowledge operations

#[allow(clippy::too_many_arguments)]
pub fn append_knowledge(
    db_path: &Path,
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
        db_path,
        None,
        title,
        knowledge_type,
        description,
        confidence,
        nodes,
        tags,
    )
    .map(|_| ())
}

/// Core knowledge upsert. UUID is the only lookup key.
#[allow(clippy::too_many_arguments)]
pub fn append_knowledge_with_uuid(
    db_path: &Path,
    knowledge_uuid: Option<&str>,
    title: &str,
    knowledge_type: &str,
    description: &str,
    confidence: f64,
    related_nodes: Option<&[String]>,
    tags: &[String],
) -> crate::error::Result<String> {
    let mut data = if !db_path.exists() {
        return Err(crate::error::KodexError::Other(
            "Database does not exist. Run `kodex run` first.".to_string(),
        ));
    } else if related_nodes.is_some() || knowledge_uuid.is_some() {
        load_knowledge_only(db_path)?
    } else {
        load(db_path)?
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Some(uuid) = knowledge_uuid {
        if !data.knowledge.iter().any(|k| k.uuid == uuid) {
            return Err(crate::error::KodexError::Other(format!(
                "Knowledge UUID not found: {uuid}. Use uuid=None to create new entry."
            )));
        }
    }
    let existing =
        knowledge_uuid.and_then(|uuid| data.knowledge.iter_mut().find(|k| k.uuid == uuid));
    let k_uuid = if let Some(entry) = existing {
        entry.observations += 1;
        entry.confidence = 1.0 - (1.0 - entry.confidence) * 0.8;
        entry.updated_at = now;
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
        let inferred_source = if tags.iter().any(|t| t == "imported") {
            "imported"
        } else {
            "agent"
        };
        data.knowledge.push(KnowledgeEntry {
            uuid: new_uuid.clone(),
            title: title.to_string(),
            knowledge_type: knowledge_type.to_string(),
            description: description.to_string(),
            confidence,
            observations: 1,
            tags: tags.to_vec(),
            scope: String::new(),
            status: "active".to_string(),
            source: inferred_source.to_string(),
            last_validated_at: 0,
            applies_when: String::new(),
            supersedes: String::new(),
            superseded_by: String::new(),
            evidence: String::new(),
            created_at: now,
            updated_at: now,
            author: String::new(),
            trigger: String::new(),
        });
        new_uuid
    };

    // Auto-link
    let auto_linked = if related_nodes.is_none() && knowledge_uuid.is_none() {
        auto_link_knowledge(&data, &k_uuid, title, description, now)
    } else {
        vec![]
    };
    if !auto_linked.is_empty() {
        data.links.extend(auto_linked);
    }

    if let Some(nodes) = related_nodes {
        data.links
            .retain(|l| l.knowledge_uuid != k_uuid || l.is_knowledge_link());
        for node_ref in nodes {
            let linked_bh = data.node_body_hash(node_ref);
            let linked_lk = data.node_logical_key(node_ref);
            data.links.push(KnowledgeLink {
                knowledge_uuid: k_uuid.clone(),
                node_uuid: node_ref.clone(),
                relation: "related_to".to_string(),
                target_type: String::new(),
                confidence: 1.0,
                created_at: now,
                linked_body_hash: linked_bh,
                linked_logical_key: linked_lk,
                source: "agent".to_string(),
                ..Default::default()
            });
        }
    }
    save_knowledge_only(db_path, &data)?;
    Ok(k_uuid)
}

#[allow(clippy::type_complexity)]
pub fn load_knowledge_entries(
    db_path: &Path,
) -> crate::error::Result<Vec<(String, String, String, f64, u32, String, String)>> {
    let data = load_knowledge_only(db_path)?;
    Ok(data
        .knowledge
        .iter()
        .map(|k| {
            let related: Vec<&str> = data
                .links
                .iter()
                .filter(|l| l.knowledge_uuid == k.uuid && !l.is_knowledge_link())
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
    db_path: &Path,
    title_match: Option<&str>,
    type_match: Option<&str>,
    project_match: Option<&str>,
    below_confidence: Option<f64>,
) -> crate::error::Result<usize> {
    if !db_path.exists() {
        return Ok(0);
    }
    let mut data = load_knowledge_only(db_path)?;
    let before = data.knowledge.len();
    let remove_uuids: Vec<String> = data
        .knowledge
        .iter()
        .filter(|k| {
            title_match.map(|m| k.title.contains(m)).unwrap_or(true)
                && type_match.map(|m| k.knowledge_type == m).unwrap_or(true)
                && project_match
                    .map(|m| k.description.contains(m))
                    .unwrap_or(true)
                && below_confidence.map(|c| k.confidence < c).unwrap_or(true)
        })
        .filter(|_| {
            title_match.is_some()
                || type_match.is_some()
                || project_match.is_some()
                || below_confidence.is_some()
        })
        .map(|k| k.uuid.clone())
        .collect();
    if remove_uuids.is_empty() {
        return Ok(0);
    }
    data.knowledge.retain(|k| !remove_uuids.contains(&k.uuid));
    data.links.retain(|l| {
        let source_removed = remove_uuids.contains(&l.knowledge_uuid);
        let target_removed = l.is_knowledge_link() && remove_uuids.contains(&l.node_uuid);
        !source_removed && !target_removed
    });
    save_knowledge_only(db_path, &data)?;
    Ok(before - data.knowledge.len())
}

// Project operations

pub fn merge_project(
    db_path: &Path,
    project_name: &str,
    new_extraction: &ExtractionResult,
) -> crate::error::Result<()> {
    cache_remove(db_path);
    let mut data = if db_path.exists() {
        load(db_path)?
    } else {
        KodexData::default()
    };
    let old_project_nodes: Vec<_> = data
        .extraction
        .nodes
        .iter()
        .filter(|n| n.source_file.starts_with(project_name))
        .cloned()
        .collect();
    data.extraction
        .nodes
        .retain(|n| !n.source_file.starts_with(project_name));
    data.extraction
        .edges
        .retain(|e| !e.source_file.starts_with(project_name));
    let mut new_nodes = new_extraction.nodes.clone();
    crate::fingerprint::assign_stable_ids(&old_project_nodes, &mut new_nodes);
    data.extraction.nodes.extend(new_nodes);
    data.extraction.edges.extend(new_extraction.edges.clone());
    let valid_node_uuids: std::collections::HashSet<&str> = data
        .extraction
        .nodes
        .iter()
        .filter_map(|n| n.uuid.as_deref())
        .collect();
    data.links.retain(|l| {
        l.is_knowledge_link()
            || valid_node_uuids.contains(l.node_uuid.as_str())
            || l.node_uuid.is_empty()
    });
    save(db_path, &data)
}

pub fn forget_project(db_path: &Path, project_path: &str) -> crate::error::Result<usize> {
    if !db_path.exists() {
        return Ok(0);
    }
    let mut data = load(db_path)?;
    let before = data.extraction.nodes.len();
    data.extraction
        .nodes
        .retain(|n| !n.source_file.starts_with(project_path));
    data.extraction
        .edges
        .retain(|e| !e.source_file.starts_with(project_path));
    let valid_node_uuids: std::collections::HashSet<&str> = data
        .extraction
        .nodes
        .iter()
        .filter_map(|n| n.uuid.as_deref())
        .collect();
    data.links.retain(|l| {
        l.is_knowledge_link()
            || valid_node_uuids.contains(l.node_uuid.as_str())
            || l.node_uuid.is_empty()
    });
    save(db_path, &data)?;
    Ok(before - data.extraction.nodes.len())
}

// ---------------------------------------------------------------------------
// SQLite internals
// ---------------------------------------------------------------------------

fn open_db(path: &Path) -> crate::error::Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite open: {e}")))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite pragma: {e}")))?;
    create_tables(&conn)?;
    Ok(conn)
}

fn create_tables(conn: &Connection) -> crate::error::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
        CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY, label TEXT, file_type TEXT, source_file TEXT,
            source_location TEXT, confidence TEXT, uuid TEXT, fingerprint TEXT,
            logical_key TEXT, body_hash TEXT, community INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS edges (
            source TEXT, target TEXT, relation TEXT, confidence TEXT,
            source_file TEXT, source_location TEXT, weight REAL DEFAULT 1.0
        );
        CREATE TABLE IF NOT EXISTS hyperedges (
            id TEXT, label TEXT, nodes TEXT, confidence TEXT, source_file TEXT
        );
        CREATE TABLE IF NOT EXISTS knowledge (
            uuid TEXT PRIMARY KEY, title TEXT, knowledge_type TEXT, description TEXT,
            confidence REAL, observations INTEGER, tags TEXT,
            scope TEXT DEFAULT '', status TEXT DEFAULT 'active', source TEXT DEFAULT '',
            last_validated_at INTEGER DEFAULT 0, applies_when TEXT DEFAULT '',
            supersedes TEXT DEFAULT '', superseded_by TEXT DEFAULT '',
            evidence TEXT DEFAULT '', created_at INTEGER DEFAULT 0,
            updated_at INTEGER DEFAULT 0, author TEXT DEFAULT '', trigger TEXT DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS links (
            knowledge_uuid TEXT, node_uuid TEXT, relation TEXT, target_type TEXT DEFAULT '',
            confidence REAL DEFAULT 1.0, created_at INTEGER DEFAULT 0,
            linked_body_hash TEXT DEFAULT '', linked_logical_key TEXT DEFAULT '',
            reason TEXT DEFAULT '', source TEXT DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS review_queue (
            knowledge_uuid TEXT, reason TEXT, created_at INTEGER, priority INTEGER,
            completed INTEGER DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_nodes_uuid ON nodes(uuid);
        CREATE INDEX IF NOT EXISTS idx_knowledge_title ON knowledge(title);
        CREATE INDEX IF NOT EXISTS idx_links_kuuid ON links(knowledge_uuid);
        CREATE INDEX IF NOT EXISTS idx_links_nuuid ON links(node_uuid);
        ",
    )
    .map_err(|e| crate::error::KodexError::Other(format!("SQLite tables: {e}")))?;
    Ok(())
}

fn write_nodes(
    conn: &Connection,
    nodes: &[crate::types::Node],
    comm_map: &HashMap<String, usize>,
) -> crate::error::Result<()> {
    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO nodes (id,label,file_type,source_file,source_location,confidence,uuid,fingerprint,logical_key,body_hash,community) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        )
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    for n in nodes {
        stmt.execute(params![
            n.id,
            n.label,
            n.file_type.to_string(),
            n.source_file,
            n.source_location.as_deref().unwrap_or(""),
            n.confidence.map(|c| c.to_string()).unwrap_or_default(),
            n.uuid.as_deref().unwrap_or(""),
            n.fingerprint.as_deref().unwrap_or(""),
            n.logical_key.as_deref().unwrap_or(""),
            n.body_hash.as_deref().unwrap_or(""),
            n.community
                .or_else(|| comm_map.get(&n.id).copied())
                .unwrap_or(0) as i64,
        ])
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite insert node: {e}")))?;
    }
    Ok(())
}

fn write_edges(conn: &Connection, edges: &[crate::types::Edge]) -> crate::error::Result<()> {
    let mut stmt = conn
        .prepare("INSERT INTO edges (source,target,relation,confidence,source_file,source_location,weight) VALUES (?1,?2,?3,?4,?5,?6,?7)")
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    for e in edges {
        stmt.execute(params![
            e.source,
            e.target,
            e.relation,
            e.confidence.to_string(),
            e.source_file,
            e.source_location.as_deref().unwrap_or(""),
            e.weight,
        ])
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite insert edge: {e}")))?;
    }
    Ok(())
}

fn write_hyperedges(
    conn: &Connection,
    hyperedges: &[crate::types::Hyperedge],
) -> crate::error::Result<()> {
    if hyperedges.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare("INSERT INTO hyperedges (id,label,nodes,confidence,source_file) VALUES (?1,?2,?3,?4,?5)")
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    for h in hyperedges {
        stmt.execute(params![
            h.id,
            h.label,
            h.nodes.join(","),
            h.confidence.to_string(),
            h.source_file.as_deref().unwrap_or(""),
        ])
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite insert hyperedge: {e}")))?;
    }
    Ok(())
}

fn write_knowledge(conn: &Connection, knowledge: &[KnowledgeEntry]) -> crate::error::Result<()> {
    if knowledge.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(
            "INSERT INTO knowledge (uuid,title,knowledge_type,description,confidence,observations,tags,scope,status,source,last_validated_at,applies_when,supersedes,superseded_by,evidence,created_at,updated_at,author,trigger) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        )
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    for k in knowledge {
        stmt.execute(params![
            k.uuid,
            k.title,
            k.knowledge_type,
            k.description,
            k.confidence,
            k.observations,
            k.tags.join(","),
            k.scope,
            k.status,
            k.source,
            k.last_validated_at as i64,
            k.applies_when,
            k.supersedes,
            k.superseded_by,
            k.evidence,
            k.created_at as i64,
            k.updated_at as i64,
            k.author,
            k.trigger,
        ])
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite insert knowledge: {e}")))?;
    }
    Ok(())
}

fn write_links(conn: &Connection, links: &[KnowledgeLink]) -> crate::error::Result<()> {
    if links.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(
            "INSERT INTO links (knowledge_uuid,node_uuid,relation,target_type,confidence,created_at,linked_body_hash,linked_logical_key,reason,source) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        )
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    for l in links {
        stmt.execute(params![
            l.knowledge_uuid,
            l.node_uuid,
            l.relation,
            l.target_type,
            l.confidence,
            l.created_at as i64,
            l.linked_body_hash,
            l.linked_logical_key,
            l.reason,
            l.source,
        ])
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite insert link: {e}")))?;
    }
    Ok(())
}

fn write_review_queue(
    conn: &Connection,
    queue: &[crate::types::ReviewQueueItem],
) -> crate::error::Result<()> {
    if queue.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare("INSERT INTO review_queue (knowledge_uuid,reason,created_at,priority,completed) VALUES (?1,?2,?3,?4,?5)")
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    for q in queue {
        stmt.execute(params![
            q.knowledge_uuid,
            q.reason,
            q.created_at as i64,
            q.priority as i64,
            if q.completed { 1 } else { 0 },
        ])
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite insert queue: {e}")))?;
    }
    Ok(())
}

fn read_extraction(conn: &Connection) -> crate::error::Result<ExtractionResult> {
    let opt = |s: String| -> Option<String> {
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    let mut ext = ExtractionResult::default();

    // Nodes
    let mut stmt = conn
        .prepare("SELECT id,label,file_type,source_file,source_location,confidence,uuid,fingerprint,logical_key,body_hash,community FROM nodes")
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(crate::types::Node {
                id: row.get(0)?,
                label: row.get(1)?,
                file_type: FileType::from_str_loose(row.get::<_, String>(2)?.as_str())
                    .unwrap_or(FileType::Code),
                source_file: row.get(3)?,
                source_location: opt(row.get(4)?),
                confidence: Confidence::from_str_loose(row.get::<_, String>(5)?.as_str()),
                confidence_score: None,
                community: Some(row.get::<_, i64>(10)? as usize),
                norm_label: None,
                degree: None,
                uuid: opt(row.get(6)?),
                fingerprint: opt(row.get(7)?),
                logical_key: opt(row.get(8)?),
                body_hash: opt(row.get(9)?),
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite read nodes: {e}")))?;
    for row in rows {
        ext.nodes
            .push(row.map_err(|e| crate::error::KodexError::Other(format!("SQLite row: {e}")))?);
    }

    // Edges
    let mut stmt = conn
        .prepare("SELECT source,target,relation,confidence,source_file,source_location,weight FROM edges")
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            let c_str: String = row.get(3)?;
            let c = Confidence::from_str_loose(&c_str).unwrap_or(Confidence::EXTRACTED);
            Ok(crate::types::Edge {
                source: row.get(0)?,
                target: row.get(1)?,
                relation: row.get(2)?,
                confidence: c,
                source_file: row.get(4)?,
                source_location: {
                    let s: String = row.get(5)?;
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                },
                confidence_score: Some(c.default_score()),
                weight: row.get(6)?,
                original_src: None,
                original_tgt: None,
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite read edges: {e}")))?;
    for row in rows {
        ext.edges
            .push(row.map_err(|e| crate::error::KodexError::Other(format!("SQLite row: {e}")))?);
    }

    // Hyperedges
    let mut stmt = conn
        .prepare("SELECT id,label,nodes,confidence,source_file FROM hyperedges")
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            let nodes_csv: String = row.get(2)?;
            let c_str: String = row.get(3)?;
            Ok(crate::types::Hyperedge {
                id: row.get(0)?,
                label: row.get(1)?,
                nodes: nodes_csv
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect(),
                confidence: Confidence::from_str_loose(&c_str).unwrap_or(Confidence::EXTRACTED),
                confidence_score: None,
                source_file: {
                    let s: String = row.get(4)?;
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                },
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite read hyperedges: {e}")))?;
    for row in rows {
        ext.hyperedges
            .push(row.map_err(|e| crate::error::KodexError::Other(format!("SQLite row: {e}")))?);
    }

    Ok(ext)
}

fn read_knowledge(conn: &Connection) -> crate::error::Result<Vec<KnowledgeEntry>> {
    let mut stmt = conn
        .prepare(
            "SELECT uuid,title,knowledge_type,description,confidence,observations,tags,scope,status,source,last_validated_at,applies_when,supersedes,superseded_by,evidence,created_at,updated_at,author,trigger FROM knowledge",
        )
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            let tags_csv: String = row.get(6)?;
            let status: String = row.get(8)?;
            Ok(KnowledgeEntry {
                uuid: row.get(0)?,
                title: row.get(1)?,
                knowledge_type: row.get(2)?,
                description: row.get(3)?,
                confidence: row.get(4)?,
                observations: row.get::<_, i64>(5)? as u32,
                tags: tags_csv
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect(),
                scope: row.get(7)?,
                status: if status.is_empty() {
                    "active".to_string()
                } else {
                    status
                },
                source: row.get(9)?,
                last_validated_at: row.get::<_, i64>(10)? as u64,
                applies_when: row.get(11)?,
                supersedes: row.get(12)?,
                superseded_by: row.get(13)?,
                evidence: row.get(14)?,
                created_at: row.get::<_, i64>(15)? as u64,
                updated_at: row.get::<_, i64>(16)? as u64,
                author: row.get(17)?,
                trigger: row.get(18)?,
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite read knowledge: {e}")))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| crate::error::KodexError::Other(format!("SQLite row: {e}")))?);
    }
    Ok(result)
}

fn read_links(conn: &Connection) -> crate::error::Result<Vec<KnowledgeLink>> {
    let mut stmt = conn
        .prepare(
            "SELECT knowledge_uuid,node_uuid,relation,target_type,confidence,created_at,linked_body_hash,linked_logical_key,reason,source FROM links",
        )
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(KnowledgeLink {
                knowledge_uuid: row.get(0)?,
                node_uuid: row.get(1)?,
                relation: row.get(2)?,
                target_type: row.get(3)?,
                confidence: row.get(4)?,
                created_at: row.get::<_, i64>(5)? as u64,
                linked_body_hash: row.get(6)?,
                linked_logical_key: row.get(7)?,
                reason: row.get(8)?,
                source: row.get(9)?,
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite read links: {e}")))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| crate::error::KodexError::Other(format!("SQLite row: {e}")))?);
    }
    Ok(result)
}

fn read_review_queue(
    conn: &Connection,
) -> crate::error::Result<Vec<crate::types::ReviewQueueItem>> {
    let mut stmt = conn
        .prepare("SELECT knowledge_uuid,reason,created_at,priority,completed FROM review_queue")
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(crate::types::ReviewQueueItem {
                knowledge_uuid: row.get(0)?,
                reason: row.get(1)?,
                created_at: row.get::<_, i64>(2)? as u64,
                priority: row.get::<_, i64>(3)? as u8,
                completed: row.get::<_, i64>(4)? != 0,
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite read queue: {e}")))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| crate::error::KodexError::Other(format!("SQLite row: {e}")))?);
    }
    Ok(result)
}

fn graph_to_extraction(graph: &KodexGraph) -> ExtractionResult {
    ExtractionResult {
        nodes: graph
            .node_ids()
            .filter_map(|id| graph.get_node(id).cloned())
            .collect(),
        edges: graph.edges().map(|(_, _, e)| e.clone()).collect(),
        hyperedges: graph.hyperedges.clone(),
        ..Default::default()
    }
}

fn auto_link_knowledge(
    data: &KodexData,
    knowledge_uuid: &str,
    title: &str,
    description: &str,
    now: u64,
) -> Vec<KnowledgeLink> {
    let title_lower = title.to_lowercase();
    let desc_lower = description.to_lowercase();
    let desc_first = desc_lower.lines().next().unwrap_or("");
    let tokens: std::collections::HashSet<&str> = title_lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .chain(desc_first.split(|c: char| !c.is_alphanumeric() && c != '_'))
        .filter(|t| t.len() > 3)
        .collect();

    if tokens.is_empty() {
        return vec![];
    }

    let mut links = Vec::new();
    let mut matched = 0;
    const MAX_AUTO_LINKS: usize = 5;

    for node in &data.extraction.nodes {
        if matched >= MAX_AUTO_LINKS {
            break;
        }
        let label_lower = node.label.to_lowercase();
        let is_match = tokens.iter().any(|t| label_lower.contains(t));
        if is_match {
            if let Some(uuid) = &node.uuid {
                let exists = data
                    .links
                    .iter()
                    .any(|l| l.knowledge_uuid == knowledge_uuid && l.node_uuid == *uuid);
                if !exists {
                    links.push(KnowledgeLink {
                        knowledge_uuid: knowledge_uuid.to_string(),
                        node_uuid: uuid.clone(),
                        relation: "related_to".to_string(),
                        target_type: String::new(),
                        confidence: 0.7,
                        created_at: now,
                        linked_body_hash: node.body_hash.clone().unwrap_or_default(),
                        linked_logical_key: node.logical_key.clone().unwrap_or_default(),
                        source: "inferred".to_string(),
                        ..Default::default()
                    });
                    matched += 1;
                }
            }
        }
    }

    links
}

// Compat: convert rusqlite::Error to our error type
impl From<rusqlite::Error> for crate::error::KodexError {
    fn from(e: rusqlite::Error) -> Self {
        crate::error::KodexError::Other(format!("SQLite: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_sqlite_round_trip() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("test.db");
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
                        body_hash: None,
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
                        body_hash: None,
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
                ..Default::default()
            }],
            links: vec![KnowledgeLink {
                knowledge_uuid: "k-1".into(),
                node_uuid: "uuid-a".into(),
                relation: "related_to".into(),
                target_type: String::new(),
                ..Default::default()
            }],
            review_queue: vec![],
        };
        save(&db, &data).unwrap();
        cache_remove(&db);
        let loaded = load(&db).unwrap();
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
        let db = dir.path().join("test_links.db");
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
                ..Default::default()
            }],
            links: vec![KnowledgeLink {
                knowledge_uuid: "k-1".into(),
                node_uuid: "n-1".into(),
                relation: "related_to".into(),
                target_type: String::new(),
                ..Default::default()
            }],
            review_queue: vec![],
        };
        save(&db, &data).unwrap();

        append_knowledge_with_uuid(
            &db,
            Some("k-1"),
            "Pattern",
            "pattern",
            "more info",
            0.6,
            None,
            &[],
        )
        .unwrap();
        cache_remove(&db);
        let loaded = load(&db).unwrap();
        assert_eq!(loaded.links.len(), 1, "None should not touch links");

        append_knowledge_with_uuid(
            &db,
            Some("k-1"),
            "Pattern",
            "pattern",
            "",
            0.6,
            Some(&[]),
            &[],
        )
        .unwrap();
        cache_remove(&db);
        let loaded = load(&db).unwrap();
        assert_eq!(loaded.links.len(), 0, "Some(&[]) should clear links");

        append_knowledge_with_uuid(
            &db,
            Some("k-1"),
            "Pattern",
            "pattern",
            "",
            0.6,
            Some(&["n-2".to_string()]),
            &[],
        )
        .unwrap();
        cache_remove(&db);
        let loaded = load(&db).unwrap();
        assert_eq!(loaded.links.len(), 1);
        assert_eq!(loaded.links[0].node_uuid, "n-2");
    }
}
