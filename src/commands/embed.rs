//! `kodex embed`: precompute embeddings for code-symbol nodes.
//!
//! Walks the graph, builds an embedding text for each function/class/method
//! node (label optionally augmented with source_location), and stores the
//! resulting vectors in the `node_embeddings` SQLite table. Used by
//! `compare_graphs --semantic-embedding` for cross-language semantic match.

#![cfg(feature = "embeddings")]

use std::path::Path;

use kodex::analyze::helpers::{is_concept_node, is_file_node};
use kodex::embedding::{vec_to_bytes, Embedder};
use kodex::serve::load_graph_smart;
use kodex::storage;

const MODEL_NAME: &str = "BGE-small-en-v1.5";
const BATCH_SIZE: usize = 64;

/// Embed all eligible nodes in the global db. Returns the number of new
/// embeddings written.
pub fn embed_nodes(
    db_path: &Path,
    source_pattern: Option<&str>,
    skip_existing: bool,
) -> kodex::error::Result<usize> {
    let graph = load_graph_smart(db_path)?;

    let existing: std::collections::HashSet<String> = if skip_existing {
        storage::load_all_embeddings(db_path)?
            .into_iter()
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
        // The text we embed is the label augmented with the file's basename
        // — this gives the model a tiny bit of context (e.g. `close()` in
        // `connection.cpp` should embed differently from `close()` in
        // `file_io.rs`).
        let basename = std::path::Path::new(&node.source_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let text = if basename.is_empty() {
            node.label.clone()
        } else {
            format!("{}  ({basename})", node.label)
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
                model: MODEL_NAME.to_string(),
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
