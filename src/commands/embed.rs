//! `kodex embed`: precompute embeddings for code-symbol nodes.
//!
//! Walks the graph, builds an embedding text for each function/class/method
//! node (label augmented with the surrounding signature + docstring lines),
//! and stores the resulting vectors in the `node_embeddings` SQLite table.
//!
//! Used by `compare_graphs --semantic-embedding` as a **candidate
//! pre-filter** — it returns top-K right-side labels by cosine but doesn't
//! decide equivalence. The LLM caller is expected to read each candidate's
//! inlined signature (via `with_signature=true`) and judge equivalence
//! with its own reasoning. See `kodex::embedding` for the full positioning
//! note.

#![cfg(feature = "embeddings")]

use std::path::Path;

use kodex::analyze::helpers::{is_concept_node, is_file_node};
use kodex::embedding::{vec_to_bytes, Embedder};
use kodex::serve::load_graph_smart;
use kodex::source_lookup;
use kodex::storage;

/// Model identifier with schema version. `v1` was label-only; `v2` includes
/// the signature line plus 2 lines above (typically the doc comment). Bump
/// the suffix whenever the embedded text format changes — older rows will
/// be detected and re-embedded automatically.
const MODEL_ID: &str = "BGE-small-en-v1.5/v2";
const BATCH_SIZE: usize = 64;
/// Lines of source pulled above the signature — captures `///` /
/// `/* ... */` doc comments without dragging in unrelated context.
const SIG_LINES_ABOVE: usize = 2;
const SIG_LINES_BELOW: usize = 0;

/// Embed all eligible nodes in the global db. Returns the number of new
/// embeddings written.
pub fn embed_nodes(
    db_path: &Path,
    source_pattern: Option<&str>,
    skip_existing: bool,
) -> kodex::error::Result<usize> {
    let graph = load_graph_smart(db_path)?;

    // Skip rows that were embedded with the *current* MODEL_ID. Older rows
    // (different model or schema version) get re-embedded transparently — the
    // INSERT...ON CONFLICT path overwrites them.
    let existing: std::collections::HashSet<String> = if skip_existing {
        storage::load_all_embeddings(db_path)?
            .into_iter()
            .filter(|e| e.model == MODEL_ID)
            .map(|e| e.node_id)
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let pat = source_pattern.map(str::to_lowercase);

    // Build the (id, text) work list. Skip file/concept hubs since they
    // have no useful semantic content (label == filename).
    let mut work: Vec<(String, String)> = Vec::new();
    for id in graph.node_ids() {
        if existing.contains(id) {
            continue;
        }
        let node = match graph.get_node(id) {
            Some(n) => n,
            None => continue,
        };
        if is_file_node(&graph, id) || is_concept_node(&graph, id) {
            continue;
        }
        if let Some(p) = pat.as_deref() {
            if !node.source_file.to_lowercase().contains(p) {
                continue;
            }
        }
        // The embedded text is `label (basename)` plus a few lines around
        // the source_location. The snippet captures the function signature
        // (parameter names + types) and the preceding doc comment, both of
        // which carry far more semantic signal than the bare identifier.
        // Falls back to label-only when the file isn't on disk (e.g. when
        // the project root has moved since ingestion).
        let basename = std::path::Path::new(&node.source_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let header = if basename.is_empty() {
            node.label.clone()
        } else {
            format!("{}  ({basename})", node.label)
        };
        let snippet = source_lookup::snippet_for(
            &node.source_file,
            node.source_location.as_deref(),
            SIG_LINES_ABOVE,
            SIG_LINES_BELOW,
        );
        let text = match snippet {
            Some(s) if !s.trim().is_empty() => format!("{header}\n{s}"),
            _ => header,
        };
        work.push((id.clone(), text));
    }

    if work.is_empty() {
        println!("No new nodes to embed.");
        return Ok(0);
    }
    println!("Embedding {} nodes...", work.len());

    let embedder = Embedder::new()?;
    let dim = embedder.dim;
    let mut total_written = 0usize;
    for batch in work.chunks(BATCH_SIZE) {
        let texts: Vec<&str> = batch.iter().map(|(_, t)| t.as_str()).collect();
        let vecs = embedder.embed(texts)?;
        let rows: Vec<storage::StoredEmbedding> = batch
            .iter()
            .zip(vecs.iter())
            .map(|((id, _), v)| storage::StoredEmbedding {
                node_id: id.clone(),
                model: MODEL_ID.to_string(),
                dim,
                vec: vec_to_bytes(v),
            })
            .collect();
        storage::store_embeddings_bulk(db_path, &rows)?;
        total_written += rows.len();
        if total_written.is_multiple_of(256) || total_written == work.len() {
            println!("  embedded {total_written}/{}", work.len());
        }
    }
    Ok(total_written)
}

/// Model identifier for chunk embeddings. Versioned independently from
/// `MODEL_ID` (node embeddings) because the embedded text is different —
/// chunks embed the raw 50-line window, nodes embed `label + signature`.
const CHUNK_MODEL_ID: &str = "BGE-small-en-v1.5/chunk-v1";

/// Embed all chunks in the global db whose stored embedding is missing or
/// stale (different model id / dim). Returns the number of new embeddings
/// written. Pre-filter: chunks are loaded together with their content_hash
/// + existing embedding model so unchanged chunks are skipped without
/// re-reading the BLOB.
pub fn embed_chunks(db_path: &Path, skip_existing: bool) -> kodex::error::Result<usize> {
    let chunks = storage::load_all_chunks(db_path)?;
    if chunks.is_empty() {
        println!("No chunks in db — run `kodex run` first.");
        return Ok(0);
    }

    let existing_models: std::collections::HashMap<String, String> = if skip_existing {
        storage::load_chunk_embedding_models(db_path)?
    } else {
        std::collections::HashMap::new()
    };

    // Skip rows whose embedding already matches CHUNK_MODEL_ID. Chunks
    // missing from the map (or with a different model id) get re-embedded.
    let work: Vec<&storage::StoredChunk> = chunks
        .iter()
        .filter(|c| {
            existing_models
                .get(&c.id)
                .map(|m| m.as_str() != CHUNK_MODEL_ID)
                .unwrap_or(true)
        })
        .collect();

    if work.is_empty() {
        println!("No new chunks to embed.");
        return Ok(0);
    }
    println!("Embedding {} chunks...", work.len());

    let embedder = Embedder::new()?;
    let dim = embedder.dim;
    let mut total_written = 0usize;
    for batch in work.chunks(BATCH_SIZE) {
        let texts: Vec<&str> = batch.iter().map(|c| c.content.as_str()).collect();
        let vecs = embedder.embed(texts)?;
        let rows: Vec<storage::StoredChunkEmbedding> = batch
            .iter()
            .zip(vecs.iter())
            .map(|(c, v)| storage::StoredChunkEmbedding {
                chunk_id: c.id.clone(),
                model: CHUNK_MODEL_ID.to_string(),
                dim,
                vec: vec_to_bytes(v),
            })
            .collect();
        storage::store_chunk_embeddings_bulk(db_path, &rows)?;
        total_written += rows.len();
        if total_written.is_multiple_of(256) || total_written == work.len() {
            println!("  embedded {total_written}/{}", work.len());
        }
    }
    Ok(total_written)
}
