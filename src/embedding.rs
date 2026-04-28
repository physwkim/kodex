//! Optional sentence embedding — a **candidate pre-filter** for
//! cross-language semantic similarity.
//!
//! ## Positioning
//!
//! This is not an equivalence judge. The 384-dim BGE-small vectors capture
//! surface co-occurrence patterns, not deep semantic understanding. The
//! intended use is to *narrow* a large right-side candidate pool down to
//! the top-K nearest neighbors so the LLM caller can read each one's
//! signature + docstring (via `with_signature=true`) and make the actual
//! equivalence call with its own native reasoning.
//!
//! For most workflows the lexical token-Jaccard pass (`semantic_threshold`,
//! v0.7.0) plus the LLM's own semantic understanding over inlined
//! signatures already outperforms cosine, and the embedding pipeline
//! (~33MB model, ORT native dep, ~5min first-time `kodex embed`) is
//! overhead. Reach for it only when scale demands it: hundreds of gaps to
//! classify, batch automation without an LLM in the loop, or contexts
//! where token-budget for direct LLM judgment is expensive.
//!
//! ## Mechanics
//!
//! Compiled only with the `embeddings` Cargo feature so the default build
//! stays free of ONNX runtime / model download weight. Two surfaces:
//!
//! - `kodex embed`: precompute embeddings for every code symbol and store
//!   them in SQLite (`node_embeddings` table).
//! - `compare_graphs --semantic-embedding`: cosine over precomputed
//!   right-side embeddings, returns top-K candidates merged into each
//!   gap's `candidate_matches` array.
//!
//! Default model: BAAI/bge-small-en-v1.5 (384-dim, ~33MB on disk after the
//! first-use download). Caches under `~/.cache/fastembed/` by default.

#![cfg(feature = "embeddings")]

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

/// Wrapper around `fastembed::TextEmbedding`. Hides the model-loading
/// boilerplate and offers a small ergonomic surface (`embed`, `embed_one`,
/// `cosine`) that matches kodex's needs.
pub struct Embedder {
    inner: TextEmbedding,
    pub dim: usize,
}

impl Embedder {
    /// Load (or download on first use) the default model.
    pub fn new() -> crate::error::Result<Self> {
        Self::with_model(EmbeddingModel::BGESmallENV15)
    }

    pub fn with_model(model: EmbeddingModel) -> crate::error::Result<Self> {
        let inner = TextEmbedding::try_new(InitOptions::new(model)).map_err(|e| {
            crate::error::KodexError::Other(format!("embedder init: {e}"))
        })?;
        // The model dimension is fixed per-model; for BGE-small-en-v1.5 this
        // is 384. We discover it by embedding a probe to keep the wrapper
        // model-agnostic.
        let probe = inner
            .embed(vec!["probe"], None)
            .map_err(|e| crate::error::KodexError::Other(format!("embed probe: {e}")))?;
        let dim = probe.first().map(|v| v.len()).unwrap_or(0);
        Ok(Self { inner, dim })
    }

    /// Embed a batch of texts. Each output vector is L2-normalized so a
    /// dot product == cosine similarity.
    pub fn embed(&self, texts: Vec<&str>) -> crate::error::Result<Vec<Vec<f32>>> {
        let raw = self
            .inner
            .embed(texts, None)
            .map_err(|e| crate::error::KodexError::Other(format!("embed: {e}")))?;
        Ok(raw.into_iter().map(|v| l2_normalize(&v)).collect())
    }

    /// Convenience: embed a single text and return one vector.
    pub fn embed_one(&self, text: &str) -> crate::error::Result<Vec<f32>> {
        let mut v = self.embed(vec![text])?;
        v.pop()
            .ok_or_else(|| crate::error::KodexError::Other("empty embedding".into()))
    }
}

/// Cosine similarity between two L2-normalized vectors. Returns 0.0 on
/// dimension mismatch instead of panicking — callers may pass mixed-model
/// vectors when the user re-embeds with a different default.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut acc = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        acc += x * y;
    }
    acc
}

fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

/// Encode an embedding as little-endian f32 bytes for SQLite BLOB storage.
pub fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Decode SQLite BLOB bytes back into an f32 vector. Returns an empty Vec
/// on a malformed (non-multiple-of-4) blob — callers typically treat that
/// as "no embedding" rather than an error.
pub fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
    if !bytes.len().is_multiple_of(4) {
        return Vec::new();
    }
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_of_identical_unit_vectors_is_one() {
        let v = vec![0.6_f32, 0.8];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_of_orthogonal_is_zero() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        assert!(cosine(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_dimension_mismatch_returns_zero() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn vec_bytes_round_trip() {
        let v = vec![1.0_f32, -0.5, 0.25, 1e-6];
        let bytes = vec_to_bytes(&v);
        assert_eq!(bytes.len(), 16);
        let back = bytes_to_vec(&bytes);
        assert_eq!(v, back);
    }

    #[test]
    fn malformed_bytes_return_empty() {
        assert!(bytes_to_vec(&[0u8, 1, 2]).is_empty());
    }
}
