//! Node fingerprinting and stable identity matching.
//!
//! Fingerprint = normalized_signature + body_hash
//! Used to match nodes across re-extractions even after renames/moves.

use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::types::Node;

/// Generate a fingerprint for a node based on its structural properties.
/// Combines: symbol kind + normalized signature + source context.
pub fn compute_fingerprint(
    _label: &str,
    file_type: &str,
    source_file: &str,
    source_location: Option<&str>,
    body_hint: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();

    // Symbol kind (class vs function vs module)
    hasher.update(file_type.as_bytes());
    hasher.update(b"|");

    // NOTE: label is intentionally excluded from fingerprint.
    // This allows renames to keep the same fingerprint.
    // Matching relies on file path + line proximity + body content.

    // File path (filename only, not full path — survives directory moves)
    let filename = source_file.rsplit('/').next().unwrap_or(source_file);
    hasher.update(filename.as_bytes());
    hasher.update(b"|");

    // Line number proximity hint
    if let Some(loc) = source_location {
        hasher.update(loc.as_bytes());
    }
    hasher.update(b"|");

    // Body content hash if available
    if let Some(body) = body_hint {
        // Normalize: strip whitespace for format-invariance
        let normalized: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        hasher.update(normalized.as_bytes());
    }

    format!("{:x}", hasher.finalize())[..16].to_string()
}

/// Generate a logical key: project/file.py::ClassName.method_name
pub fn logical_key(source_file: &str, label: &str) -> String {
    let file_part = source_file
        .rsplit('/')
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("/");
    format!("{file_part}::{}", label.trim_end_matches("()"))
}

/// Match score between two nodes for identity resolution.
/// Returns 0.0-1.0 where 1.0 = definite match.
pub fn match_score(old: &Node, new: &Node) -> f64 {
    let mut score = 0.0;
    let mut max_score = 0.0;

    // Fingerprint exact match (strongest signal)
    max_score += 40.0;
    if let (Some(old_fp), Some(new_fp)) = (&old.fingerprint, &new.fingerprint) {
        if old_fp == new_fp {
            score += 40.0;
        }
    }

    // Same file (strongest non-fingerprint signal)
    max_score += 25.0;
    if old.source_file == new.source_file {
        score += 25.0;
    } else {
        let old_name = old.source_file.rsplit('/').next().unwrap_or("");
        let new_name = new.source_file.rsplit('/').next().unwrap_or("");
        if !old_name.is_empty() && old_name == new_name {
            score += 15.0;
        }
    }

    // Line proximity (within 20 lines)
    max_score += 15.0;
    if let (Some(old_loc), Some(new_loc)) = (&old.source_location, &new.source_location) {
        let old_line: i64 = old_loc.trim_start_matches('L').parse().unwrap_or(0);
        let new_line: i64 = new_loc.trim_start_matches('L').parse().unwrap_or(0);
        let diff = (old_line - new_line).unsigned_abs();
        if diff == 0 {
            score += 15.0;
        } else if diff <= 5 {
            score += 12.0;
        } else if diff <= 20 {
            score += 6.0;
        }
    }

    // Same file_type
    max_score += 10.0;
    if old.file_type == new.file_type {
        score += 10.0;
    }

    // Label similarity (token overlap)
    max_score += 15.0;
    let old_tokens: Vec<&str> = old
        .label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let new_tokens: Vec<&str> = new
        .label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    if !old_tokens.is_empty() && !new_tokens.is_empty() {
        let common = old_tokens.iter().filter(|t| new_tokens.contains(t)).count();
        let total = old_tokens.len().max(new_tokens.len());
        score += 15.0 * (common as f64 / total as f64);
    }

    // Exact label match
    max_score += 10.0;
    if old.label == new.label {
        score += 10.0;
    }

    // Body hash match (strong signal — only counted when both sides have one)
    if old.body_hash.is_some() && new.body_hash.is_some() {
        max_score += 25.0;
        if old.body_hash == new.body_hash {
            score += 25.0;
        } else {
            // Bodies differ — actively penalize to prevent false matches.
            // Same file + similar position but different body = different entity.
            score -= 15.0;
        }
    }

    if max_score == 0.0 {
        return 0.0;
    }
    score / max_score
}

/// Threshold for considering two nodes the same entity.
const MATCH_THRESHOLD: f64 = 0.4;

/// Match new nodes against existing nodes, assigning UUIDs.
///
/// - Exact fingerprint match → reuse UUID
/// - Score above threshold → reuse UUID
/// - Below threshold → new UUID
pub fn assign_stable_ids(existing: &[Node], new_nodes: &mut [Node]) {
    // Build lookup by fingerprint and by id
    let mut fp_map: HashMap<String, &Node> = HashMap::new();
    let mut id_map: HashMap<String, &Node> = HashMap::new();

    for node in existing {
        if let Some(fp) = &node.fingerprint {
            fp_map.insert(fp.clone(), node);
        }
        id_map.insert(node.id.clone(), node);
    }

    for node in new_nodes.iter_mut() {
        // Generate fingerprint if not set
        if node.fingerprint.is_none() {
            node.fingerprint = Some(compute_fingerprint(
                &node.label,
                &node.file_type.to_string(),
                &node.source_file,
                node.source_location.as_deref(),
                node.body_hash.as_deref(),
            ));
        }

        // Generate logical_key if not set
        if node.logical_key.is_none() {
            node.logical_key = Some(logical_key(&node.source_file, &node.label));
        }

        // Try exact fingerprint match first
        if let Some(fp) = &node.fingerprint {
            if let Some(existing_node) = fp_map.get(fp) {
                if let Some(uuid) = &existing_node.uuid {
                    node.uuid = Some(uuid.clone());
                    continue;
                }
            }
        }

        // Try exact id match (backward compat)
        if let Some(existing_node) = id_map.get(&node.id) {
            if let Some(uuid) = &existing_node.uuid {
                node.uuid = Some(uuid.clone());
                continue;
            }
        }

        // Score-based matching
        let mut best_score = 0.0;
        let mut best_uuid: Option<String> = None;

        for existing_node in existing {
            let s = match_score(existing_node, node);
            if s > best_score && s >= MATCH_THRESHOLD {
                best_score = s;
                best_uuid = existing_node.uuid.clone();
            }
        }

        if let Some(uuid) = best_uuid {
            node.uuid = Some(uuid);
        } else {
            // New entity — assign fresh UUID
            node.uuid = Some(uuid::Uuid::new_v4().to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileType;

    fn make_node(label: &str, file: &str, loc: &str) -> Node {
        Node {
            id: crate::id::make_id(&[label]),
            label: label.to_string(),
            file_type: FileType::Code,
            source_file: file.to_string(),
            source_location: Some(loc.to_string()),
            confidence: None,
            confidence_score: None,
            community: None,
            norm_label: None,
            degree: None,
            uuid: Some(uuid::Uuid::new_v4().to_string()),
            fingerprint: Some(compute_fingerprint(label, "code", file, Some(loc), None)),
            logical_key: Some(logical_key(file, label)),
            body_hash: None,
        }
    }

    #[test]
    fn test_exact_fingerprint_match() {
        let old = make_node("authenticate()", "auth.py", "L42");
        let mut new_nodes = vec![make_node("authenticate()", "auth.py", "L42")];
        new_nodes[0].uuid = None; // clear so assign_stable_ids fills it

        assign_stable_ids(std::slice::from_ref(&old), &mut new_nodes);
        assert_eq!(new_nodes[0].uuid, old.uuid);
    }

    #[test]
    fn test_renamed_function_same_file() {
        let old = make_node("authenticate()", "auth.py", "L42");
        let mut new_nodes = vec![make_node("verify_token()", "auth.py", "L44")];
        new_nodes[0].uuid = None;
        new_nodes[0].fingerprint = None;

        assign_stable_ids(std::slice::from_ref(&old), &mut new_nodes);
        // same file(25) + close line(12) + same type(10) = 47/100 = 0.47 > 0.4
        // Rename in same file → UUID preserved
        assert_eq!(new_nodes[0].uuid, old.uuid);
    }

    #[test]
    fn test_new_function_gets_uuid() {
        let mut new_nodes = vec![make_node("brand_new()", "new.py", "L1")];
        new_nodes[0].uuid = None;

        assign_stable_ids(&[], &mut new_nodes);
        assert!(new_nodes[0].uuid.is_some());
    }

    #[test]
    fn test_logical_key() {
        assert_eq!(
            logical_key("project/src/auth.py", "authenticate()"),
            "src/auth.py::authenticate"
        );
    }
}
