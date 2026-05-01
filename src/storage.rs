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

/// Set `evidence` for a knowledge entry, but only if currently empty. Used by `learn`
/// to record provenance without overwriting evidence the caller passed explicitly.
pub fn set_evidence_if_empty(path: &Path, uuid: &str, evidence: &str) -> crate::error::Result<()> {
    if !path.exists() || evidence.is_empty() {
        return Ok(());
    }
    let conn = open_db(path)?;
    conn.execute(
        "UPDATE knowledge SET evidence = ?1 WHERE uuid = ?2 AND (evidence IS NULL OR evidence = '')",
        params![evidence, uuid],
    )
    .map_err(|e| crate::error::KodexError::Other(format!("SQLite set_evidence: {e}")))?;
    cache_remove(path);
    Ok(())
}

/// Increment fetch_count + last_fetched + nudge confidence for the given UUIDs.
/// Direct UPDATE (no full rewrite) — cheap to call on every recall. Confidence
/// bump is small (+0.005) and capped at 0.95. Cache is invalidated.
pub fn bump_fetch_counters(path: &Path, uuids: &[String]) -> crate::error::Result<()> {
    if uuids.is_empty() || !path.exists() {
        return Ok(());
    }
    let conn = open_db(path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut stmt = conn
        .prepare(
            "UPDATE knowledge SET fetch_count = fetch_count + 1, last_fetched = ?1, \
             confidence = MIN(0.95, confidence + 0.005) WHERE uuid = ?2",
        )
        .map_err(|e| crate::error::KodexError::Other(format!("SQLite prepare bump: {e}")))?;
    for uuid in uuids {
        stmt.execute(params![now, uuid])
            .map_err(|e| crate::error::KodexError::Other(format!("SQLite bump: {e}")))?;
    }
    cache_remove(path);
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
            fetch_count: 0,
            last_fetched: 0,
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
    // Auto-link to similar existing knowledge entries (cluster discovery).
    if knowledge_uuid.is_none() {
        let kk_links = auto_link_knowledge_to_knowledge(
            &data,
            &k_uuid,
            title,
            description,
            knowledge_type,
            tags,
            now,
        );
        if !kk_links.is_empty() {
            data.links.extend(kk_links);
        }
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

/// Minimum existing-node count for the shrink guard to fire. Below this, a
/// project is considered "early-growth" and may shrink freely (typical for
/// scaffolding / first commits). Heuristic — tune if false positives appear.
const SHRINK_GUARD_MIN_OLD_NODES: usize = 50;

/// Refuse a merge when retained < this fraction of the old project node count.
/// 0.5 = "must keep at least half". Heuristic — chosen to catch catastrophic
/// extraction failures (parser crash, partial run) without flagging routine
/// refactors. Tune via `merge_project_force` if real-world signal differs.
const SHRINK_GUARD_MIN_RETAINED_RATIO: f64 = 0.5;

pub fn merge_project(
    db_path: &Path,
    project_name: &str,
    new_extraction: &ExtractionResult,
) -> crate::error::Result<()> {
    // Public entry: read `KODEX_FORCE_SHRINK` from env for the runtime
    // override, then delegate. Tests use `merge_project_force` directly so
    // they don't have to touch the global env.
    let force = std::env::var("KODEX_FORCE_SHRINK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    merge_project_force(db_path, project_name, new_extraction, force)
}

/// Same as [`merge_project`] but with an explicit `force_shrink` flag instead
/// of the `KODEX_FORCE_SHRINK` env var. Use from tests and from any callsite
/// that already knows the user opted into shrinkage (e.g. a future CLI flag
/// `--force-shrink`).
pub fn merge_project_force(
    db_path: &Path,
    project_name: &str,
    new_extraction: &ExtractionResult,
    force_shrink: bool,
) -> crate::error::Result<()> {
    // No early `cache_remove` here. Two reasons:
    // (a) on success, the trailing `save(...)` calls `cache_put` which fully
    //     overwrites whatever was in the cache — pre-clearing is redundant.
    // (b) on early-return paths (shrink-guard rejection below), pre-clearing
    //     evicts a still-valid cache entry, forcing the next reader to take
    //     a disk hit for state that didn't actually change.
    // Cache invalidation belongs in functions that mutate via raw SQL without
    // going through `load → mutate → save` (e.g. `bump_fetch_counters`,
    // `set_evidence_if_empty`), not here.
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

    // Shrink guard (mirrors graphify's pre-write check): if the new extraction
    // retains less than `SHRINK_GUARD_MIN_RETAINED_RATIO` of the previously-
    // stored nodes for this project, refuse the save. Massive drops almost
    // always signal an extraction failure (parser crash, dep change, partial
    // run) rather than a real codebase shrink — better surfaced than silently
    // overwriting a healthy graph. Pass `force_shrink=true` (or set
    // `KODEX_FORCE_SHRINK=1` for the env-driven entry) for legitimate cases:
    // mass file deletion, project rename, intentional pruning.
    let old_n = old_project_nodes.len();
    let new_n = new_extraction.nodes.len();
    if !force_shrink && old_n > SHRINK_GUARD_MIN_OLD_NODES {
        let retained = new_n as f64 / old_n.max(1) as f64;
        if retained < SHRINK_GUARD_MIN_RETAINED_RATIO {
            let max_loss_pct = ((1.0 - SHRINK_GUARD_MIN_RETAINED_RATIO) * 100.0) as u32;
            return Err(crate::error::KodexError::Other(format!(
                "shrink guard: project '{project_name}' would drop from {old_n} → {new_n} \
                 nodes (>{max_loss_pct}% loss). Refusing to overwrite. Re-run with \
                 KODEX_FORCE_SHRINK=1 if this is intentional (e.g. you deleted source files); \
                 otherwise investigate extraction errors before saving."
            )));
        }
    }

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
// Node embeddings
// ---------------------------------------------------------------------------

/// One stored embedding row (always available regardless of `embeddings`
/// feature — the BLOB is opaque bytes here; encoding/decoding is in the
/// `embedding` module).
#[derive(Debug, Clone)]
pub struct StoredEmbedding {
    pub node_id: String,
    pub model: String,
    pub dim: usize,
    pub vec: Vec<u8>,
}

/// Upsert one embedding row.
pub fn store_embedding(
    db_path: &Path,
    node_id: &str,
    model: &str,
    dim: usize,
    vec: &[u8],
) -> crate::error::Result<()> {
    let conn = open_db(db_path)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO node_embeddings (node_id, model, dim, vec, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(node_id) DO UPDATE SET model=excluded.model, dim=excluded.dim, vec=excluded.vec, updated_at=excluded.updated_at",
        rusqlite::params![node_id, model, dim as i64, vec, ts],
    )
    .map_err(|e| crate::error::KodexError::Other(format!("store_embedding: {e}")))?;
    Ok(())
}

/// Bulk upsert. Wraps everything in a single transaction for speed.
pub fn store_embeddings_bulk(db_path: &Path, rows: &[StoredEmbedding]) -> crate::error::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut conn = open_db(db_path)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let tx = conn
        .transaction()
        .map_err(|e| crate::error::KodexError::Other(format!("tx: {e}")))?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO node_embeddings (node_id, model, dim, vec, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(node_id) DO UPDATE SET model=excluded.model, dim=excluded.dim, vec=excluded.vec, updated_at=excluded.updated_at",
            )
            .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
        for r in rows {
            stmt.execute(rusqlite::params![
                r.node_id,
                r.model,
                r.dim as i64,
                &r.vec,
                ts
            ])
            .map_err(|e| crate::error::KodexError::Other(format!("exec: {e}")))?;
        }
    }
    tx.commit()
        .map_err(|e| crate::error::KodexError::Other(format!("commit: {e}")))?;
    Ok(())
}

/// Load all embeddings as `(node_id, vec_bytes)`. Caller decodes via
/// `embedding::bytes_to_vec` if the `embeddings` feature is enabled.
pub fn load_all_embeddings(db_path: &Path) -> crate::error::Result<Vec<StoredEmbedding>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn
        .prepare("SELECT node_id, model, dim, vec FROM node_embeddings")
        .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(StoredEmbedding {
                node_id: row.get(0)?,
                model: row.get(1)?,
                dim: row.get::<_, i64>(2)? as usize,
                vec: row.get(3)?,
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| crate::error::KodexError::Other(format!("row: {e}")))?);
    }
    Ok(out)
}

/// Count embedded nodes (status indicator for the `kodex embed` command).
pub fn count_embeddings(db_path: &Path) -> crate::error::Result<usize> {
    let conn = open_db(db_path)?;
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM node_embeddings", [], |r| r.get(0))
        .map_err(|e| crate::error::KodexError::Other(format!("count: {e}")))?;
    Ok(n as usize)
}

// ---------------------------------------------------------------------------
// Chunks (sub-node units for chunk-level semantic retrieval)
// ---------------------------------------------------------------------------

/// One stored chunk row. A chunk is a 50-line / 5-line-overlap window of
/// source text. `node_id` is best-effort — set when a graph node's start
/// line falls inside the chunk's line range, NULL otherwise (e.g. chunks
/// of markdown / configs / regions outside any extracted node).
#[derive(Debug, Clone)]
pub struct StoredChunk {
    pub id: String,
    pub node_id: Option<String>,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub language: Option<String>,
    pub content: String,
    pub content_hash: String,
}

/// Bulk upsert chunks. Wraps in a single transaction.
pub fn store_chunks_bulk(db_path: &Path, rows: &[StoredChunk]) -> crate::error::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut conn = open_db(db_path)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let tx = conn
        .transaction()
        .map_err(|e| crate::error::KodexError::Other(format!("tx: {e}")))?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO chunks (id, node_id, file_path, start_line, end_line, language, content, content_hash, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                    node_id=excluded.node_id,
                    file_path=excluded.file_path,
                    start_line=excluded.start_line,
                    end_line=excluded.end_line,
                    language=excluded.language,
                    content=excluded.content,
                    content_hash=excluded.content_hash,
                    updated_at=excluded.updated_at",
            )
            .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
        for r in rows {
            stmt.execute(rusqlite::params![
                r.id,
                r.node_id,
                r.file_path,
                r.start_line,
                r.end_line,
                r.language,
                r.content,
                r.content_hash,
                ts,
            ])
            .map_err(|e| crate::error::KodexError::Other(format!("exec: {e}")))?;
        }
    }
    tx.commit()
        .map_err(|e| crate::error::KodexError::Other(format!("commit: {e}")))?;
    Ok(())
}

/// Lightweight chunk record without `content` / `content_hash` — used by
/// `semantic_search` for the cosine scoring pass so a multi-hundred-MB
/// content blob doesn't get materialized just to filter / rank.
#[derive(Debug, Clone)]
pub struct ChunkMetadata {
    pub id: String,
    pub node_id: Option<String>,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub language: Option<String>,
}

/// Load all chunk metadata (no content) — paired with
/// `load_chunks_by_ids` for the top-K result enrichment.
pub fn load_chunk_metadata(db_path: &Path) -> crate::error::Result<Vec<ChunkMetadata>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn
        .prepare("SELECT id, node_id, file_path, start_line, end_line, language FROM chunks")
        .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ChunkMetadata {
                id: row.get(0)?,
                node_id: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                language: row.get(5)?,
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| crate::error::KodexError::Other(format!("row: {e}")))?);
    }
    Ok(out)
}

/// Load full `StoredChunk` rows for a specific list of chunk ids. Used by
/// `semantic_search` to fetch content for only the top-K survivors of the
/// cosine pass. Empty `ids` returns an empty vec.
pub fn load_chunks_by_ids(
    db_path: &Path,
    ids: &[String],
) -> crate::error::Result<Vec<StoredChunk>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let conn = open_db(db_path)?;
    // SQLite max parameter count is 999 by default; chunk in case top_k
    // is raised in the future.
    let mut out = Vec::with_capacity(ids.len());
    for batch in ids.chunks(900) {
        let placeholders: String = std::iter::repeat_n("?", batch.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, node_id, file_path, start_line, end_line, language, content, content_hash \
             FROM chunks WHERE id IN ({placeholders})",
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
        let params_iter = batch.iter().map(|s| s as &dyn rusqlite::ToSql);
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_iter), |row| {
                Ok(StoredChunk {
                    id: row.get(0)?,
                    node_id: row.get(1)?,
                    file_path: row.get(2)?,
                    start_line: row.get(3)?,
                    end_line: row.get(4)?,
                    language: row.get(5)?,
                    content: row.get(6)?,
                    content_hash: row.get(7)?,
                })
            })
            .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?;
        for r in rows {
            out.push(r.map_err(|e| crate::error::KodexError::Other(format!("row: {e}")))?);
        }
    }
    Ok(out)
}

/// Load all chunks (full content + metadata). Used by the embed step and by
/// `semantic_search` to enrich top-K hits before returning them.
pub fn load_all_chunks(db_path: &Path) -> crate::error::Result<Vec<StoredChunk>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, node_id, file_path, start_line, end_line, language, content, content_hash FROM chunks",
        )
        .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(StoredChunk {
                id: row.get(0)?,
                node_id: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                language: row.get(5)?,
                content: row.get(6)?,
                content_hash: row.get(7)?,
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| crate::error::KodexError::Other(format!("row: {e}")))?);
    }
    Ok(out)
}

/// Load `(chunk_id, content_hash)` for fast skip-existing checks during
/// `kodex embed` — avoids materializing full chunk content.
pub fn load_chunk_hashes(
    db_path: &Path,
) -> crate::error::Result<std::collections::HashMap<String, String>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn
        .prepare("SELECT id, content_hash FROM chunks")
        .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let h: String = row.get(1)?;
            Ok((id, h))
        })
        .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?;
    let mut out = std::collections::HashMap::new();
    for r in rows {
        let (id, h) = r.map_err(|e| crate::error::KodexError::Other(format!("row: {e}")))?;
        out.insert(id, h);
    }
    Ok(out)
}

/// One knowledge entry attached to a graph node, in the shape callers want
/// for response payloads. Subset of `KnowledgeEntry` — drops fields like
/// `description`, `evidence`, `tags` that are too verbose for inline use.
#[derive(Debug, Clone)]
pub struct KnowledgeAttachment {
    pub uuid: String,
    pub title: String,
    pub knowledge_type: String,
    pub confidence: f64,
    pub relation: String,
}

/// For each node id in `node_ids`, return the list of knowledge entries
/// linked to that node (id-keyed map). Bridges `node.id` (chunk's foreign
/// key) → `node.uuid` (link table's foreign key) via a narrow SQL lookup,
/// then in-memory joins links + knowledge for the wanted set. Obsolete
/// knowledge is filtered out.
///
/// This is the data-only sibling of `actor::attach_knowledge_for_nodes`,
/// which adds JSON shaping and is gated to the `embeddings` feature. Pull
/// this directly when writing tests or non-MCP callers.
pub fn knowledge_for_node_ids(
    db_path: &Path,
    node_ids: &[String],
) -> crate::error::Result<std::collections::HashMap<String, Vec<KnowledgeAttachment>>> {
    use std::collections::HashMap;

    let id_to_uuid = load_node_uuids_for_ids(db_path, node_ids)?;
    if id_to_uuid.is_empty() {
        return Ok(HashMap::new());
    }
    let uuid_to_id: HashMap<String, String> = id_to_uuid
        .iter()
        .map(|(id, u)| (u.clone(), id.clone()))
        .collect();

    let kdata = load_knowledge_only(db_path)?;
    let kmap: HashMap<&str, &KnowledgeEntry> = kdata
        .knowledge
        .iter()
        .map(|k| (k.uuid.as_str(), k))
        .collect();

    let mut out: HashMap<String, Vec<KnowledgeAttachment>> = HashMap::new();
    for link in &kdata.links {
        let Some(node_id) = uuid_to_id.get(link.node_uuid.as_str()) else {
            continue;
        };
        let Some(k) = kmap.get(link.knowledge_uuid.as_str()) else {
            continue;
        };
        if k.status.eq_ignore_ascii_case("obsolete") {
            continue;
        }
        out.entry(node_id.clone())
            .or_default()
            .push(KnowledgeAttachment {
                uuid: k.uuid.clone(),
                title: k.title.clone(),
                knowledge_type: k.knowledge_type.clone(),
                confidence: k.confidence,
                relation: link.relation.clone(),
            });
    }
    Ok(out)
}

/// Look up `(node.id, node.uuid)` pairs for a specific list of node ids.
/// Cheaper alternative to `load_graph_smart` when callers only need the
/// id↔uuid bridge (e.g. `attach_knowledge_for_nodes` in the actor).
/// Returns a map omitting any node whose uuid is empty or missing.
pub fn load_node_uuids_for_ids(
    db_path: &Path,
    ids: &[String],
) -> crate::error::Result<std::collections::HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let conn = open_db(db_path)?;
    let mut out = std::collections::HashMap::new();
    for batch in ids.chunks(900) {
        let placeholders: String = std::iter::repeat_n("?", batch.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT id, uuid FROM nodes WHERE id IN ({placeholders})");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
        let params_iter = batch.iter().map(|s| s as &dyn rusqlite::ToSql);
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_iter), |row| {
                let id: String = row.get(0)?;
                let uuid: String = row.get(1)?;
                Ok((id, uuid))
            })
            .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?;
        for r in rows {
            let (id, uuid) = r.map_err(|e| crate::error::KodexError::Other(format!("row: {e}")))?;
            if !uuid.is_empty() {
                out.insert(id, uuid);
            }
        }
    }
    Ok(out)
}

/// Per-project GC: delete chunks belonging to `project_name` whose `id` is
/// not in `keep`. Project membership is tested by `file_path.starts_with(
/// "{project_name}/")` in Rust to avoid SQL LIKE wildcard collisions —
/// names like `foo_bar` would otherwise match `fooXbar` because `_` is a
/// single-char wildcard in `LIKE`. Wrapped in a single transaction so a
/// large prune is one fsync, not N.
pub fn prune_chunks_for_project(
    db_path: &Path,
    project_name: &str,
    keep: &std::collections::HashSet<String>,
) -> crate::error::Result<usize> {
    let prefix = format!("{project_name}/");
    let mut conn = open_db(db_path)?;
    let tx = conn
        .transaction()
        .map_err(|e| crate::error::KodexError::Other(format!("tx: {e}")))?;
    let mut deleted = 0usize;
    {
        let mut stmt = tx
            .prepare("SELECT id, file_path FROM chunks")
            .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let fp: String = row.get(1)?;
                Ok((id, fp))
            })
            .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?
            .filter_map(|r| r.ok())
            .collect();
        let mut del_chunk = tx
            .prepare("DELETE FROM chunks WHERE id = ?1")
            .map_err(|e| crate::error::KodexError::Other(format!("prep del: {e}")))?;
        let mut del_emb = tx
            .prepare("DELETE FROM chunk_embeddings WHERE chunk_id = ?1")
            .map_err(|e| crate::error::KodexError::Other(format!("prep del_emb: {e}")))?;
        for (id, file_path) in rows {
            if !file_path.starts_with(&prefix) {
                continue;
            }
            if keep.contains(&id) {
                continue;
            }
            del_chunk
                .execute(rusqlite::params![id])
                .map_err(|e| crate::error::KodexError::Other(format!("del: {e}")))?;
            del_emb
                .execute(rusqlite::params![id])
                .map_err(|e| crate::error::KodexError::Other(format!("del_emb: {e}")))?;
            deleted += 1;
        }
    }
    tx.commit()
        .map_err(|e| crate::error::KodexError::Other(format!("commit: {e}")))?;
    Ok(deleted)
}

/// Delete chunk rows whose `id` is not in `keep`. Used to garbage-collect
/// stale chunks after a re-ingest where a file shrank or was removed.
pub fn prune_chunks_not_in(
    db_path: &Path,
    keep: &std::collections::HashSet<String>,
) -> crate::error::Result<usize> {
    let mut conn = open_db(db_path)?;
    let tx = conn
        .transaction()
        .map_err(|e| crate::error::KodexError::Other(format!("tx: {e}")))?;
    let mut deleted = 0usize;
    {
        // Pull all IDs first to avoid mutating-while-iterating; SQLite would
        // tolerate it but rusqlite's borrow rules wouldn't.
        let mut stmt = tx
            .prepare("SELECT id FROM chunks")
            .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?
            .filter_map(|r| r.ok())
            .collect();
        let mut del_chunk = tx
            .prepare("DELETE FROM chunks WHERE id = ?1")
            .map_err(|e| crate::error::KodexError::Other(format!("prep del: {e}")))?;
        let mut del_emb = tx
            .prepare("DELETE FROM chunk_embeddings WHERE chunk_id = ?1")
            .map_err(|e| crate::error::KodexError::Other(format!("prep del_emb: {e}")))?;
        for id in ids {
            if !keep.contains(&id) {
                del_chunk
                    .execute(rusqlite::params![id])
                    .map_err(|e| crate::error::KodexError::Other(format!("del: {e}")))?;
                del_emb
                    .execute(rusqlite::params![id])
                    .map_err(|e| crate::error::KodexError::Other(format!("del_emb: {e}")))?;
                deleted += 1;
            }
        }
    }
    tx.commit()
        .map_err(|e| crate::error::KodexError::Other(format!("commit: {e}")))?;
    Ok(deleted)
}

/// One stored chunk-embedding row.
#[derive(Debug, Clone)]
pub struct StoredChunkEmbedding {
    pub chunk_id: String,
    pub model: String,
    pub dim: usize,
    pub vec: Vec<u8>,
}

/// Bulk upsert chunk embeddings. Same shape as `store_embeddings_bulk` but
/// targets `chunk_embeddings` keyed by `chunk_id`.
pub fn store_chunk_embeddings_bulk(
    db_path: &Path,
    rows: &[StoredChunkEmbedding],
) -> crate::error::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut conn = open_db(db_path)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let tx = conn
        .transaction()
        .map_err(|e| crate::error::KodexError::Other(format!("tx: {e}")))?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO chunk_embeddings (chunk_id, model, dim, vec, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(chunk_id) DO UPDATE SET
                    model=excluded.model, dim=excluded.dim, vec=excluded.vec, updated_at=excluded.updated_at",
            )
            .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
        for r in rows {
            stmt.execute(rusqlite::params![
                r.chunk_id,
                r.model,
                r.dim as i64,
                &r.vec,
                ts,
            ])
            .map_err(|e| crate::error::KodexError::Other(format!("exec: {e}")))?;
        }
    }
    tx.commit()
        .map_err(|e| crate::error::KodexError::Other(format!("commit: {e}")))?;
    Ok(())
}

/// Load all chunk embeddings.
pub fn load_all_chunk_embeddings(
    db_path: &Path,
) -> crate::error::Result<Vec<StoredChunkEmbedding>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn
        .prepare("SELECT chunk_id, model, dim, vec FROM chunk_embeddings")
        .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(StoredChunkEmbedding {
                chunk_id: row.get(0)?,
                model: row.get(1)?,
                dim: row.get::<_, i64>(2)? as usize,
                vec: row.get(3)?,
            })
        })
        .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| crate::error::KodexError::Other(format!("row: {e}")))?);
    }
    Ok(out)
}

/// Load only `(chunk_id, model)` for the embed step's skip-existing path.
pub fn load_chunk_embedding_models(
    db_path: &Path,
) -> crate::error::Result<std::collections::HashMap<String, String>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn
        .prepare("SELECT chunk_id, model FROM chunk_embeddings")
        .map_err(|e| crate::error::KodexError::Other(format!("prep: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let m: String = row.get(1)?;
            Ok((id, m))
        })
        .map_err(|e| crate::error::KodexError::Other(format!("query: {e}")))?;
    let mut out = std::collections::HashMap::new();
    for r in rows {
        let (id, m) = r.map_err(|e| crate::error::KodexError::Other(format!("row: {e}")))?;
        out.insert(id, m);
    }
    Ok(out)
}

/// Count chunks (status indicator).
pub fn count_chunks(db_path: &Path) -> crate::error::Result<usize> {
    let conn = open_db(db_path)?;
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
        .map_err(|e| crate::error::KodexError::Other(format!("count: {e}")))?;
    Ok(n as usize)
}

/// Count chunk embeddings.
pub fn count_chunk_embeddings(db_path: &Path) -> crate::error::Result<usize> {
    let conn = open_db(db_path)?;
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunk_embeddings", [], |r| r.get(0))
        .map_err(|e| crate::error::KodexError::Other(format!("count: {e}")))?;
    Ok(n as usize)
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
            updated_at INTEGER DEFAULT 0, author TEXT DEFAULT '', trigger TEXT DEFAULT '',
            fetch_count INTEGER DEFAULT 0, last_fetched INTEGER DEFAULT 0
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
        CREATE TABLE IF NOT EXISTS node_embeddings (
            node_id TEXT PRIMARY KEY,
            model TEXT NOT NULL,
            dim INTEGER NOT NULL,
            vec BLOB NOT NULL,
            updated_at INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS chunks (
            id TEXT PRIMARY KEY,
            node_id TEXT,
            file_path TEXT NOT NULL,
            start_line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            language TEXT,
            content TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            updated_at INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS chunk_embeddings (
            chunk_id TEXT PRIMARY KEY,
            model TEXT NOT NULL,
            dim INTEGER NOT NULL,
            vec BLOB NOT NULL,
            updated_at INTEGER DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_nodes_uuid ON nodes(uuid);
        CREATE INDEX IF NOT EXISTS idx_knowledge_title ON knowledge(title);
        CREATE INDEX IF NOT EXISTS idx_links_kuuid ON links(knowledge_uuid);
        CREATE INDEX IF NOT EXISTS idx_links_nuuid ON links(node_uuid);
        CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(file_path);
        CREATE INDEX IF NOT EXISTS idx_chunks_node ON chunks(node_id);
        ",
    )
    .map_err(|e| crate::error::KodexError::Other(format!("SQLite tables: {e}")))?;
    run_migrations(conn)?;
    Ok(())
}

/// Current schema version. Bump when a migration is added below; the
/// matching arm in `run_migrations` is what actually applies it.
///
/// History:
/// - v0: pre-tracking — schemas before this constant existed. Detected by
///   `PRAGMA user_version = 0` (SQLite default for fresh DBs *and* legacy
///   DBs alike), so the v0→v1 migration always probes for missing
///   knowledge.{fetch_count,last_fetched} columns.
/// - v1: knowledge.fetch_count + knowledge.last_fetched (added 2026-04 to
///   support recall ranking by usage).
/// - v2: chunks + chunk_embeddings tables (added 2026-05 for chunk-level
///   semantic retrieval). New tables only — `CREATE TABLE IF NOT EXISTS` in
///   `create_tables` covers fresh DBs and existing v1 DBs alike, so the
///   migration arm just bumps `user_version`.
const SCHEMA_VERSION: i64 = 2;

/// Apply any pending migrations from `current` up to `SCHEMA_VERSION`.
/// Idempotent — running twice on the same DB is a no-op.
fn run_migrations(conn: &Connection) -> crate::error::Result<()> {
    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| crate::error::KodexError::Other(format!("user_version read: {e}")))?;

    for next in (current + 1)..=SCHEMA_VERSION {
        match next {
            1 => migrate_v0_to_v1(conn)?,
            2 => { /* tables created in create_tables(); no ALTER needed */ }
            other => {
                return Err(crate::error::KodexError::Other(format!(
                    "no migration registered for schema v{other}"
                )));
            }
        }
        conn.pragma_update(None, "user_version", next)
            .map_err(|e| crate::error::KodexError::Other(format!("user_version write: {e}")))?;
    }
    Ok(())
}

/// v0 → v1: add `fetch_count` and `last_fetched` columns to `knowledge`
/// for recall-ranking by usage. Idempotent — probes `PRAGMA table_info`
/// before each ALTER so a fresh-from-CREATE-TABLE schema (which already
/// has the columns from the current `create_tables` SQL) skips the work.
fn migrate_v0_to_v1(conn: &Connection) -> crate::error::Result<()> {
    let existing: std::collections::HashSet<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(knowledge)")
            .map_err(|e| crate::error::KodexError::Other(format!("SQLite pragma: {e}")))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| crate::error::KodexError::Other(format!("SQLite pragma rows: {e}")))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let want = [
        ("fetch_count", "INTEGER DEFAULT 0"),
        ("last_fetched", "INTEGER DEFAULT 0"),
    ];
    for (col, decl) in want {
        if !existing.contains(col) {
            let sql = format!("ALTER TABLE knowledge ADD COLUMN {col} {decl}");
            conn.execute(&sql, [])
                .map_err(|e| crate::error::KodexError::Other(format!("ALTER {col}: {e}")))?;
        }
    }
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
            "INSERT INTO knowledge (uuid,title,knowledge_type,description,confidence,observations,tags,scope,status,source,last_validated_at,applies_when,supersedes,superseded_by,evidence,created_at,updated_at,author,trigger,fetch_count,last_fetched) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
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
            k.fetch_count,
            k.last_fetched as i64,
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
            "SELECT uuid,title,knowledge_type,description,confidence,observations,tags,scope,status,source,last_validated_at,applies_when,supersedes,superseded_by,evidence,created_at,updated_at,author,trigger,fetch_count,last_fetched FROM knowledge",
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
                fetch_count: row.get::<_, i64>(19)? as u32,
                last_fetched: row.get::<_, i64>(20)? as u64,
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

/// Auto-link a freshly saved knowledge entry to existing knowledge entries that
/// look related (same type + shared title/tag tokens). Caps at 3 links to avoid
/// cluster pollution and never duplicates an existing edge.
fn auto_link_knowledge_to_knowledge(
    data: &KodexData,
    new_uuid: &str,
    title: &str,
    description: &str,
    knowledge_type: &str,
    tags: &[String],
    now: u64,
) -> Vec<KnowledgeLink> {
    const MAX_AUTO_K_LINKS: usize = 3;
    const TOKEN_OVERLAP_MIN: f64 = 0.5;

    let new_title_tokens: std::collections::HashSet<String> = title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() > 2)
        .map(String::from)
        .collect();
    if new_title_tokens.is_empty() {
        return Vec::new();
    }
    let new_desc_first: String = description
        .lines()
        .next()
        .unwrap_or("")
        .to_lowercase()
        .chars()
        .take(200)
        .collect();
    let new_desc_tokens: std::collections::HashSet<String> = new_desc_first
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() > 2)
        .map(String::from)
        .collect();
    let new_tags: std::collections::HashSet<String> =
        tags.iter().map(|t| t.to_lowercase()).collect();

    let mut scored: Vec<(String, f64)> = Vec::new();
    for k in &data.knowledge {
        if k.uuid == new_uuid || k.status == "obsolete" {
            continue;
        }
        // Title token overlap
        let other_title_tokens: std::collections::HashSet<String> = k
            .title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| t.len() > 2)
            .map(String::from)
            .collect();
        let title_common = new_title_tokens
            .iter()
            .filter(|t| other_title_tokens.contains(*t))
            .count();
        let title_total = new_title_tokens.len().max(other_title_tokens.len()).max(1);
        let title_overlap = title_common as f64 / title_total as f64;

        // Description first-line overlap
        let other_desc_first: String = k
            .description
            .lines()
            .next()
            .unwrap_or("")
            .to_lowercase()
            .chars()
            .take(200)
            .collect();
        let other_desc_tokens: std::collections::HashSet<String> = other_desc_first
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| t.len() > 2)
            .map(String::from)
            .collect();
        let desc_common = new_desc_tokens
            .iter()
            .filter(|t| other_desc_tokens.contains(*t))
            .count();
        let desc_total = new_desc_tokens.len().max(other_desc_tokens.len()).max(1);
        let desc_overlap = desc_common as f64 / desc_total as f64;

        // Tag overlap
        let other_tags: std::collections::HashSet<String> =
            k.tags.iter().map(|t| t.to_lowercase()).collect();
        let tag_common = new_tags.iter().filter(|t| other_tags.contains(*t)).count();
        let tag_total = new_tags.len().max(other_tags.len()).max(1);
        let tag_overlap = tag_common as f64 / tag_total as f64;

        let same_type = if k.knowledge_type == knowledge_type {
            0.1
        } else {
            0.0
        };
        let score = 0.5 * title_overlap + 0.3 * desc_overlap + 0.2 * tag_overlap + same_type;
        if score >= TOKEN_OVERLAP_MIN {
            scored.push((k.uuid.clone(), score));
        }
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = Vec::new();
    for (target_uuid, _) in scored.into_iter().take(MAX_AUTO_K_LINKS) {
        let exists = data.links.iter().any(|l| {
            l.knowledge_uuid == new_uuid && l.node_uuid == target_uuid && l.is_knowledge_link()
        });
        if exists {
            continue;
        }
        out.push(KnowledgeLink {
            knowledge_uuid: new_uuid.to_string(),
            node_uuid: target_uuid,
            relation: "related_to".to_string(),
            target_type: "knowledge".to_string(),
            confidence: 0.6,
            created_at: now,
            source: "inferred".to_string(),
            ..Default::default()
        });
    }
    out
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

    /// Simulate a pre-v1 database (legacy ad-hoc migration era) by stripping
    /// the `fetch_count` / `last_fetched` columns and resetting `user_version`
    /// to 0. `open_db` should detect the old version and re-apply the v0→v1
    /// migration, leaving the columns + a bumped `user_version`.
    #[test]
    fn test_migration_v0_to_v1_idempotent() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("legacy.db");
        // First open creates a fresh schema and stamps user_version=1.
        {
            let _ = open_db(&db).expect("open fresh");
        }
        // Roll back: drop the columns and the version stamp.
        {
            let conn = Connection::open(&db).unwrap();
            // SQLite < 3.35 has no DROP COLUMN; re-create the table without
            // them. Production ALTERs are forward-only, so this rollback is
            // synthetic — just enough to flip the migration trigger.
            conn.execute_batch(
                "
                ALTER TABLE knowledge RENAME TO knowledge_old;
                CREATE TABLE knowledge (
                    uuid TEXT PRIMARY KEY, title TEXT, knowledge_type TEXT, description TEXT,
                    confidence REAL, observations INTEGER, tags TEXT,
                    scope TEXT DEFAULT '', status TEXT DEFAULT 'active', source TEXT DEFAULT '',
                    last_validated_at INTEGER DEFAULT 0, applies_when TEXT DEFAULT '',
                    supersedes TEXT DEFAULT '', superseded_by TEXT DEFAULT '',
                    evidence TEXT DEFAULT '', created_at INTEGER DEFAULT 0,
                    updated_at INTEGER DEFAULT 0, author TEXT DEFAULT '', trigger TEXT DEFAULT ''
                );
                INSERT INTO knowledge SELECT
                    uuid, title, knowledge_type, description, confidence, observations, tags,
                    scope, status, source, last_validated_at, applies_when, supersedes,
                    superseded_by, evidence, created_at, updated_at, author, trigger
                FROM knowledge_old;
                DROP TABLE knowledge_old;
                ",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 0_i64).unwrap();
            let v: i64 = conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(v, 0, "synthetic rollback failed");
        }
        // Second open should re-apply v0→v1.
        let _ = open_db(&db).expect("re-open after rollback");
        let conn = Connection::open(&db).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(
            v, SCHEMA_VERSION,
            "user_version should be bumped to current"
        );
        let cols: std::collections::HashSet<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(knowledge)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert!(cols.contains("fetch_count"), "fetch_count must be re-added");
        assert!(
            cols.contains("last_fetched"),
            "last_fetched must be re-added"
        );
        // Idempotency: a third open must not error or duplicate columns.
        let _ = open_db(&db).expect("idempotent re-open");
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

    #[test]
    fn test_chunks_roundtrip() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("chunks.db");
        let row = StoredChunk {
            id: "c-1".into(),
            node_id: Some("n-1".into()),
            file_path: "p/foo.rs".into(),
            start_line: 1,
            end_line: 50,
            language: Some("rust".into()),
            content: "fn foo() {}".into(),
            content_hash: "deadbeef".into(),
        };
        store_chunks_bulk(&db, std::slice::from_ref(&row)).unwrap();
        let loaded = load_all_chunks(&db).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "c-1");
        assert_eq!(loaded[0].node_id.as_deref(), Some("n-1"));
        assert_eq!(loaded[0].file_path, "p/foo.rs");
        assert_eq!(loaded[0].start_line, 1);
        assert_eq!(loaded[0].end_line, 50);
        assert_eq!(loaded[0].content, "fn foo() {}");

        // Upsert with same id but new content — must overwrite.
        let updated = StoredChunk {
            content: "fn foo() { bar() }".into(),
            content_hash: "feed".into(),
            ..row
        };
        store_chunks_bulk(&db, &[updated]).unwrap();
        let reloaded = load_all_chunks(&db).unwrap();
        assert_eq!(reloaded.len(), 1, "upsert must not duplicate");
        assert!(reloaded[0].content.contains("bar"));

        // Embedding side: roundtrip + dim preserved.
        let emb = StoredChunkEmbedding {
            chunk_id: "c-1".into(),
            model: "BGE-small-en-v1.5/chunk-v1".into(),
            dim: 384,
            vec: vec![0u8; 4 * 384],
        };
        store_chunk_embeddings_bulk(&db, &[emb]).unwrap();
        let embs = load_all_chunk_embeddings(&db).unwrap();
        assert_eq!(embs.len(), 1);
        assert_eq!(embs[0].dim, 384);
        assert_eq!(embs[0].vec.len(), 4 * 384);

        // Prune: keep the chunk in keep-set → no deletion. Drop from keep
        // set → chunk + its embedding are removed.
        let mut keep: std::collections::HashSet<String> = std::collections::HashSet::new();
        keep.insert("c-1".to_string());
        let pruned = prune_chunks_not_in(&db, &keep).unwrap();
        assert_eq!(pruned, 0);
        keep.clear();
        let pruned = prune_chunks_not_in(&db, &keep).unwrap();
        assert_eq!(pruned, 1);
        assert!(load_all_chunks(&db).unwrap().is_empty());
        assert!(load_all_chunk_embeddings(&db).unwrap().is_empty());
    }

    #[test]
    fn test_prune_chunks_does_not_cross_delete_similar_project_names() {
        // Regression: an earlier draft used `LIKE 'foo_bar/%'`, which
        // matches `fooXbar/...` because `_` is a single-char wildcard in
        // SQLite's LIKE. The Rust-side `starts_with` filter now in
        // `prune_chunks_for_project` must keep both projects' chunks
        // independent.
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("multi.db");

        let chunk = |id: &str, file_path: &str| StoredChunk {
            id: id.into(),
            node_id: None,
            file_path: file_path.into(),
            start_line: 1,
            end_line: 10,
            language: Some("rust".into()),
            content: "x".into(),
            content_hash: id.into(),
        };
        store_chunks_bulk(
            &db,
            &[
                chunk("c-foo-1", "foo/src/a.rs"),
                chunk("c-fooXbar-1", "fooXbar/src/a.rs"),
                chunk("c-foo_bar-1", "foo_bar/src/a.rs"),
            ],
        )
        .unwrap();

        // Re-ingest of project `foo` keeps c-foo-1, drops nothing else.
        let mut keep = std::collections::HashSet::new();
        keep.insert("c-foo-1".to_string());
        let pruned = prune_chunks_for_project(&db, "foo", &keep).unwrap();
        assert_eq!(pruned, 0, "no foo-prefixed chunk should be deleted");

        let after = load_all_chunks(&db).unwrap();
        let ids: std::collections::HashSet<&str> = after.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains("c-foo-1"));
        assert!(
            ids.contains("c-fooXbar-1"),
            "fooXbar must NOT be pruned by `foo` (would happen with raw LIKE)"
        );
        assert!(
            ids.contains("c-foo_bar-1"),
            "foo_bar must NOT be pruned by `foo` — different project"
        );

        // Now actually GC project `foo_bar` with empty keep — only its
        // chunk goes; the other two stay.
        let pruned =
            prune_chunks_for_project(&db, "foo_bar", &std::collections::HashSet::new()).unwrap();
        assert_eq!(pruned, 1);
        let after = load_all_chunks(&db).unwrap();
        let ids: std::collections::HashSet<&str> = after.iter().map(|c| c.id.as_str()).collect();
        assert!(!ids.contains("c-foo_bar-1"));
        assert!(ids.contains("c-foo-1"));
        assert!(ids.contains("c-fooXbar-1"));
    }

    #[test]
    fn test_load_node_uuids_for_ids_skips_empty_uuids() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("uuids.db");
        // Use the public save() path so node columns are populated correctly.
        let mut data = KodexData {
            extraction: ExtractionResult::default(),
            knowledge: vec![],
            links: vec![],
            review_queue: vec![],
        };
        data.extraction.nodes.push(crate::types::Node {
            id: "id-a".into(),
            label: "A".into(),
            file_type: crate::types::FileType::Code,
            source_file: "p/a.rs".into(),
            source_location: Some("L1".into()),
            confidence: Some(crate::types::Confidence::EXTRACTED),
            confidence_score: None,
            community: None,
            norm_label: None,
            degree: None,
            uuid: Some("uuid-a".into()),
            fingerprint: None,
            logical_key: None,
            body_hash: None,
        });
        data.extraction.nodes.push(crate::types::Node {
            id: "id-b".into(),
            label: "B".into(),
            file_type: crate::types::FileType::Code,
            source_file: "p/b.rs".into(),
            source_location: Some("L1".into()),
            confidence: Some(crate::types::Confidence::EXTRACTED),
            confidence_score: None,
            community: None,
            norm_label: None,
            degree: None,
            uuid: Some(String::new()), // empty uuid — must be filtered out
            fingerprint: None,
            logical_key: None,
            body_hash: None,
        });
        save(&db, &data).unwrap();

        let map = load_node_uuids_for_ids(
            &db,
            &[
                "id-a".to_string(),
                "id-b".to_string(),
                "id-missing".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(map.len(), 1, "only id-a has a non-empty uuid");
        assert_eq!(map.get("id-a"), Some(&"uuid-a".to_string()));
        assert!(!map.contains_key("id-b"), "empty uuid must be filtered");
        assert!(!map.contains_key("id-missing"), "absent id must be absent");
    }

    #[test]
    fn test_knowledge_for_node_ids_joins_and_filters_obsolete() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("kfnids.db");

        // Two nodes (id-a / uuid-a, id-b / uuid-b), three knowledge entries
        // (k-active, k-other, k-obsolete), and four links exercising:
        //  - normal active link node-a → k-active
        //  - duplicate-relation link node-a → k-other (different relation)
        //  - obsolete link must be filtered out
        //  - link to a node not in the wanted set must be ignored
        let mut data = KodexData {
            extraction: ExtractionResult::default(),
            knowledge: vec![
                KnowledgeEntry {
                    uuid: "k-active".into(),
                    title: "Active".into(),
                    knowledge_type: "bug_pattern".into(),
                    confidence: 0.7,
                    status: "active".into(),
                    ..Default::default()
                },
                KnowledgeEntry {
                    uuid: "k-other".into(),
                    title: "Other".into(),
                    knowledge_type: "decision".into(),
                    confidence: 0.5,
                    status: "active".into(),
                    ..Default::default()
                },
                KnowledgeEntry {
                    uuid: "k-obsolete".into(),
                    title: "Old".into(),
                    knowledge_type: "lesson".into(),
                    confidence: 0.4,
                    status: "obsolete".into(),
                    ..Default::default()
                },
            ],
            links: vec![
                KnowledgeLink {
                    knowledge_uuid: "k-active".into(),
                    node_uuid: "uuid-a".into(),
                    relation: "warns_about".into(),
                    ..Default::default()
                },
                KnowledgeLink {
                    knowledge_uuid: "k-other".into(),
                    node_uuid: "uuid-a".into(),
                    relation: "documents".into(),
                    ..Default::default()
                },
                KnowledgeLink {
                    knowledge_uuid: "k-obsolete".into(),
                    node_uuid: "uuid-a".into(),
                    relation: "warns_about".into(),
                    ..Default::default()
                },
                // Link to a node outside the wanted set — must not surface.
                KnowledgeLink {
                    knowledge_uuid: "k-active".into(),
                    node_uuid: "uuid-c-elsewhere".into(),
                    relation: "warns_about".into(),
                    ..Default::default()
                },
            ],
            review_queue: vec![],
        };
        data.extraction.nodes.push(crate::types::Node {
            id: "id-a".into(),
            label: "A".into(),
            file_type: crate::types::FileType::Code,
            source_file: "p/a.rs".into(),
            source_location: Some("L1".into()),
            confidence: Some(crate::types::Confidence::EXTRACTED),
            confidence_score: None,
            community: None,
            norm_label: None,
            degree: None,
            uuid: Some("uuid-a".into()),
            fingerprint: None,
            logical_key: None,
            body_hash: None,
        });
        data.extraction.nodes.push(crate::types::Node {
            id: "id-b".into(),
            label: "B".into(),
            file_type: crate::types::FileType::Code,
            source_file: "p/b.rs".into(),
            source_location: Some("L1".into()),
            confidence: Some(crate::types::Confidence::EXTRACTED),
            confidence_score: None,
            community: None,
            norm_label: None,
            degree: None,
            uuid: Some("uuid-b".into()),
            fingerprint: None,
            logical_key: None,
            body_hash: None,
        });
        save(&db, &data).unwrap();

        let map = knowledge_for_node_ids(
            &db,
            &[
                "id-a".to_string(),
                "id-b".to_string(),
                "id-missing".to_string(),
            ],
        )
        .unwrap();

        // id-a: 2 active links (k-active warns_about, k-other documents);
        // k-obsolete dropped; cross-node link ignored.
        let a = map.get("id-a").expect("id-a should have attachments");
        assert_eq!(a.len(), 2, "obsolete link must be filtered");
        let titles: std::collections::HashSet<&str> = a.iter().map(|x| x.title.as_str()).collect();
        assert!(titles.contains("Active"));
        assert!(titles.contains("Other"));
        assert!(!titles.contains("Old"), "obsolete must not surface");

        // id-b: no links → not present in the map (rather than empty Vec).
        assert!(!map.contains_key("id-b"));

        // id-missing: not in the nodes table → not present.
        assert!(!map.contains_key("id-missing"));

        // Empty input → empty output.
        let empty = knowledge_for_node_ids(&db, &[]).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_load_chunk_metadata_strips_content() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("meta.db");
        store_chunks_bulk(
            &db,
            &[StoredChunk {
                id: "c-1".into(),
                node_id: Some("n-1".into()),
                file_path: "p/x.rs".into(),
                start_line: 1,
                end_line: 50,
                language: Some("rust".into()),
                content: "huge body".repeat(1000),
                content_hash: "h".into(),
            }],
        )
        .unwrap();
        let meta = load_chunk_metadata(&db).unwrap();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].id, "c-1");
        assert_eq!(meta[0].node_id.as_deref(), Some("n-1"));
        assert_eq!(meta[0].start_line, 1);
        assert_eq!(meta[0].end_line, 50);
        assert_eq!(meta[0].language.as_deref(), Some("rust"));

        let by_id = load_chunks_by_ids(&db, &["c-1".to_string()]).unwrap();
        assert_eq!(by_id.len(), 1);
        assert!(by_id[0].content.contains("huge body"));

        let empty = load_chunks_by_ids(&db, &[]).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_migration_v1_to_v2_creates_chunk_tables() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("v1_legacy.db");
        // Fresh open creates everything at SCHEMA_VERSION (currently 2).
        {
            let _ = open_db(&db).unwrap();
        }
        // Synthetic rollback to v1: drop the chunk tables and stamp
        // user_version=1. Mirrors how a real v1 DB would look on disk
        // before the v2 migration runs.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "DROP TABLE IF EXISTS chunks; DROP TABLE IF EXISTS chunk_embeddings;",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1_i64).unwrap();
        }
        // Re-open: create_tables should restore the tables (idempotent), and
        // run_migrations should bump user_version to 2.
        let _ = open_db(&db).unwrap();
        let conn = Connection::open(&db).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let tables: std::collections::HashSet<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table'")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert!(tables.contains("chunks"));
        assert!(tables.contains("chunk_embeddings"));
    }
}
