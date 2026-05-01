use std::collections::{BTreeMap, HashSet};

use super::helpers::{is_concept_node, is_file_node};
use crate::graph::KodexGraph;
use crate::types::FileType;

/// Query for [`compare_repos`].
#[derive(Debug, Clone)]
pub struct CompareQuery {
    /// Substring (case-insensitive) that the source_file must contain.
    pub left_pattern: String,
    pub right_pattern: String,
    /// Optional restriction to a single file_type (typically Code).
    pub file_type: Option<FileType>,
    /// Drop labels whose normalized form is shorter than this.
    /// Default 3 — keeps useful short names (`get`, `set`) out of generic noise.
    pub min_norm_len: usize,
    /// Cap on returned items.
    pub top_n: usize,
    /// Optional substring that must appear in the (lowercased) label.
    /// Use to narrow gaps to a domain (e.g. `pattern="search"`).
    pub label_pattern: Option<String>,
    /// Only return gaps whose representative node has at least this degree.
    pub min_degree: usize,
    /// Skip file-level / module-level / concept nodes (`data`, `type`, `pvxs`,
    /// `evhelper`). Default true — these are almost never the answer to
    /// "what's missing in repo Y".
    pub skip_file_nodes: bool,
    /// Optional path substring (e.g. `"/include/"`, `"src/pvxs/"`) marking
    /// public/exported headers. When set, gaps in matching files are kept;
    /// non-matching gaps are either dropped (`public_only=true`) or
    /// down-weighted by `internal_penalty` so the public surface dominates
    /// the top of the result list.
    pub public_pattern: Option<String>,
    pub public_only: bool,
    /// Multiplier applied to non-public gaps when `public_pattern` is set
    /// but `public_only` is false. Default 0 keeps internal gaps but pushes
    /// them below all public ones; set to e.g. 0.5 for a softer weighting.
    pub internal_weight: f32,
    /// Minimum identifier-token Jaccard for a right-side label to be flagged
    /// as a candidate semantic match for a gap. 0 disables the check.
    /// Recommended start: 0.4 (catches `tickSearch` ↔ `process_search` while
    /// skipping noise). Off by default — adds an O(left_gaps × right_nodes)
    /// pass.
    pub semantic_threshold: f32,
    /// Cap on candidate matches per gap. Default 3.
    pub semantic_top_per_gap: usize,
    /// Sort gaps by composite priority (`degree × public_boost × docstring_boost`)
    /// instead of raw degree. Off by default — preserves backward-compatible
    /// degree ranking. The composite is most useful when `with_signature=true`
    /// is also set (so docstring detection has source to inspect).
    pub compose_priority: bool,
}

impl Default for CompareQuery {
    fn default() -> Self {
        Self {
            left_pattern: String::new(),
            right_pattern: String::new(),
            file_type: None,
            min_norm_len: 3,
            top_n: 200,
            label_pattern: None,
            min_degree: 0,
            skip_file_nodes: true,
            public_pattern: None,
            public_only: false,
            internal_weight: 0.0,
            semantic_threshold: 0.0,
            semantic_top_per_gap: 3,
            compose_priority: false,
        }
    }
}

/// Split a label into lowercase identifier tokens (camelCase + snake_case +
/// scope qualifiers all unified). e.g. `tickSearch()` → ["tick", "search"];
/// `Server::handle_request` → ["server", "handle", "request"].
pub fn tokenize_label(label: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in label.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !current.is_empty() {
                // camelCase boundary: flush current
                tokens.push(current.to_lowercase());
                current = String::new();
            }
            current.push(c);
            prev_lower = c.is_lowercase();
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current).to_lowercase());
            }
            prev_lower = false;
        }
    }
    if !current.is_empty() {
        tokens.push(current.to_lowercase());
    }
    // Drop single-character tokens (loop vars, separators) — too noisy. Keep
    // 2-char tokens because `io`, `fs`, `os`, `id` etc. are meaningful in code.
    tokens.retain(|t| t.len() > 1);
    tokens
}

fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_a: std::collections::HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}

/// One label that appears in `left_pattern` files and has no normalized match
/// among `right_pattern` files.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompareGap {
    pub label: String,
    pub norm: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    pub degree: usize,
    /// True when the gap's source_file matches `CompareQuery::public_pattern`.
    /// Always `false` when no public_pattern was supplied.
    #[serde(default)]
    pub is_public: bool,
    /// Semantic-token candidates from the right side that share enough
    /// identifier tokens with this gap's label. Empty unless
    /// `CompareQuery::semantic_threshold > 0`. Use to spot "this gap is
    /// probably implemented in right under a different name" cases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_matches: Vec<CandidateMatch>,
    /// Composite priority — only meaningful when `compose_priority=true`.
    /// Higher = more architecturally significant.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub priority_score: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// One right-side label that overlaps the gap's tokens by Jaccard ≥ threshold.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CandidateMatch {
    pub label: String,
    pub source_file: String,
    pub jaccard: f32,
    /// Cosine similarity from the embedding pass. 0.0 when only the lexical
    /// (token-Jaccard) pass produced this match.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub cosine: f32,
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

/// Normalize a label to a comparable identifier form:
/// lowercase, stripped of all non-alphanumeric characters. This collapses
/// `hurryUp`, `hurry_up`, `HURRY_UP`, `hurry-up()` to the same form so that
/// cross-language naming conventions don't generate false gaps.
///
/// Returns `None` when the label has no alphanumeric content.
pub fn normalize_label(label: &str) -> Option<String> {
    let mut out = String::with_capacity(label.len());
    for c in label.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Find labels that appear in `left_pattern` files but have no normalized
/// match among `right_pattern` files. Useful for "what's in repo A that
/// repo B is missing" parity checks across different naming conventions.
pub fn compare_repos(graph: &KodexGraph, query: &CompareQuery) -> Vec<CompareGap> {
    let left_pat = query.left_pattern.to_lowercase();
    let right_pat = query.right_pattern.to_lowercase();
    let label_pat = query.label_pattern.as_deref().map(str::to_lowercase);
    let public_pat = query.public_pattern.as_deref().map(str::to_lowercase);

    // Collect right-side normalized labels regardless of file/concept status —
    // the goal is "is this name present anywhere in right?", not "is this a
    // first-class symbol in right?". Skip filter only applies to left-side.
    let mut right_norms: HashSet<String> = HashSet::new();
    let mut left_by_norm: BTreeMap<String, CompareGap> = BTreeMap::new();

    for id in graph.node_ids() {
        let node = match graph.get_node(id) {
            Some(n) => n,
            None => continue,
        };
        if let Some(ft) = query.file_type {
            if node.file_type != ft {
                continue;
            }
        }
        let norm = match normalize_label(&node.label) {
            Some(n) if n.len() >= query.min_norm_len => n,
            _ => continue,
        };
        let src = node.source_file.to_lowercase();
        let in_left = !left_pat.is_empty() && src.contains(&left_pat);
        let in_right = !right_pat.is_empty() && src.contains(&right_pat);

        if in_right {
            right_norms.insert(norm.clone());
        }
        if !in_left {
            continue;
        }

        // Left-side filtering: drop file/concept hubs, label pattern, etc.
        if query.skip_file_nodes && (is_file_node(graph, id) || is_concept_node(graph, id)) {
            continue;
        }
        if let Some(p) = label_pat.as_deref() {
            if !node.label.to_lowercase().contains(p) {
                continue;
            }
        }
        let degree = graph.degree(id);
        if degree < query.min_degree {
            continue;
        }
        let is_public = public_pat
            .as_deref()
            .is_some_and(|p| node.source_file.to_lowercase().contains(p));

        // Keep the highest-degree representative of each normalized label,
        // breaking ties in favour of the public-API occurrence so the user
        // sees `pvxs/include/...` paths before internal `.cpp` definitions.
        left_by_norm
            .entry(norm.clone())
            .and_modify(|existing| {
                let prefer_new = degree > existing.degree || (is_public && !existing.is_public);
                if prefer_new {
                    *existing = CompareGap {
                        label: node.label.clone(),
                        norm: norm.clone(),
                        source_file: node.source_file.clone(),
                        source_location: node.source_location.clone(),
                        degree,
                        is_public,
                        candidate_matches: Vec::new(),
                        priority_score: 0.0,
                    };
                }
            })
            .or_insert_with(|| CompareGap {
                label: node.label.clone(),
                norm: norm.clone(),
                source_file: node.source_file.clone(),
                source_location: node.source_location.clone(),
                degree,
                is_public,
                candidate_matches: Vec::new(),
                priority_score: 0.0,
            });
    }

    let public_active = public_pat.is_some();
    let mut gaps: Vec<CompareGap> = left_by_norm
        .into_iter()
        .filter_map(|(norm, gap)| {
            if right_norms.contains(&norm) {
                return None;
            }
            if public_active && query.public_only && !gap.is_public {
                return None;
            }
            Some(gap)
        })
        .collect();

    // Optional semantic-token Jaccard pass: for each gap, scan right-side
    // labels in `right_pattern` files and flag those that share enough
    // identifier tokens. Catches "the gap is implemented in right under a
    // different name" cases (`tickSearch` ↔ `process_search_request`).
    if query.semantic_threshold > 0.0 {
        let right_nodes: Vec<(&str, &str, Vec<String>)> = graph
            .node_ids()
            .filter_map(|id| {
                let node = graph.get_node(id)?;
                if !node.source_file.to_lowercase().contains(&right_pat) {
                    return None;
                }
                Some((
                    node.label.as_str(),
                    node.source_file.as_str(),
                    tokenize_label(&node.label),
                ))
            })
            .filter(|(_, _, toks)| !toks.is_empty())
            .collect();

        for gap in &mut gaps {
            let gap_tokens = tokenize_label(&gap.label);
            if gap_tokens.is_empty() {
                continue;
            }
            let mut matches: Vec<CandidateMatch> = right_nodes
                .iter()
                .filter_map(|(label, source, toks)| {
                    let j = jaccard(&gap_tokens, toks);
                    if j >= query.semantic_threshold {
                        Some(CandidateMatch {
                            label: label.to_string(),
                            source_file: source.to_string(),
                            jaccard: j,
                            cosine: 0.0,
                        })
                    } else {
                        None
                    }
                })
                .collect();
            matches.sort_by(|a, b| {
                b.jaccard
                    .partial_cmp(&a.jaccard)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            matches.truncate(query.semantic_top_per_gap.max(1));
            gap.candidate_matches = matches;
        }
    }

    // Composite priority: fan-in × public_boost. fan_in (incoming edges) is
    // a better "architectural importance" signal than raw degree because it
    // counts how many places reference the node, ignoring its own outgoing
    // calls. Only computed when explicitly requested — older callers that
    // sort on degree keep working unchanged.
    if query.compose_priority {
        for gap in &mut gaps {
            // The gap's representative node id was preserved across the
            // norm collapse only as label/source; look up by source_file +
            // label fingerprint via the graph's id index. Since gaps were
            // built from graph nodes, find the matching node id by label.
            let fan_in = graph
                .node_ids()
                .find_map(|id| {
                    let n = graph.get_node(id)?;
                    if n.label == gap.label && n.source_file == gap.source_file {
                        Some(graph.fan_in(id))
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            let public_boost = if gap.is_public { 2.0_f32 } else { 1.0_f32 };
            // +1 ensures unreferenced public symbols still score above zero.
            gap.priority_score = (fan_in as f32 + 1.0) * public_boost;
        }
    }

    // Sort: when public_pattern is set, public gaps come first regardless of
    // degree (they're the API stability surface). Within each tier, higher
    // priority/degree wins. `internal_weight` only affects ranking when
    // public_only=false — internal gaps are kept but pushed down.
    gaps.sort_by(|a, b| {
        if public_active && a.is_public != b.is_public {
            return b.is_public.cmp(&a.is_public);
        }
        let a_score = if query.compose_priority {
            a.priority_score
        } else {
            effective_score(a, query, public_active)
        };
        let b_score = if query.compose_priority {
            b.priority_score
        } else {
            effective_score(b, query, public_active)
        };
        b_score
            .partial_cmp(&a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.norm.cmp(&b.norm))
    });
    gaps.truncate(query.top_n);
    gaps
}

fn effective_score(g: &CompareGap, q: &CompareQuery, public_active: bool) -> f32 {
    let base = g.degree as f32;
    if public_active && !g.is_public {
        base * q.internal_weight
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_from_extraction;
    use crate::types::{Confidence, Edge, ExtractionResult, Node};

    fn mk_node(id: &str, label: &str, source_file: &str) -> Node {
        Node {
            id: id.into(),
            label: label.into(),
            file_type: FileType::Code,
            source_file: source_file.into(),
            source_location: Some("L1".into()),
            confidence: Some(Confidence::EXTRACTED),
            confidence_score: Some(1.0),
            community: None,
            norm_label: None,
            degree: None,
            uuid: None,
            fingerprint: None,
            logical_key: None,
            body_hash: None,
        }
    }

    fn mk_edge(src: &str, tgt: &str) -> Edge {
        Edge {
            source: src.into(),
            target: tgt.into(),
            relation: "calls".into(),
            confidence: Confidence::EXTRACTED,
            source_file: "x".into(),
            source_location: None,
            confidence_score: Some(1.0),
            weight: 1.0,
            original_src: None,
            original_tgt: None,
        }
    }

    #[test]
    fn normalize_collapses_naming_conventions() {
        assert_eq!(normalize_label("hurryUp"), Some("hurryup".into()));
        assert_eq!(normalize_label("hurry_up"), Some("hurryup".into()));
        assert_eq!(normalize_label("HURRY_UP()"), Some("hurryup".into()));
        assert_eq!(normalize_label("Server::close"), Some("serverclose".into()));
        assert_eq!(normalize_label("()"), None);
    }

    #[test]
    fn finds_labels_in_left_missing_from_right() {
        let extraction = ExtractionResult {
            nodes: vec![
                // pvxs side
                mk_node("p1", "hurryUp", "pvxs/src/server.cpp"),
                mk_node("p2", "close", "pvxs/src/server.cpp"),
                mk_node("p3", "ignoreGUIDs", "pvxs/src/client.cpp"),
                // pva-rs side: has hurry_up (snake) and close, but not ignoreGUIDs
                mk_node("r1", "hurry_up", "pva-rs/src/server.rs"),
                mk_node("r2", "close", "pva-rs/src/server.rs"),
            ],
            edges: vec![mk_edge("p1", "p2"), mk_edge("p3", "p1")],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);

        let q = CompareQuery {
            left_pattern: "pvxs/".into(),
            right_pattern: "pva-rs/".into(),
            file_type: Some(FileType::Code),
            min_norm_len: 3,
            top_n: 50,
            ..Default::default()
        };
        let gaps = compare_repos(&graph, &q);
        let labels: Vec<&str> = gaps.iter().map(|g| g.label.as_str()).collect();

        assert!(
            labels.contains(&"ignoreGUIDs"),
            "expected ignoreGUIDs gap, got {labels:?}"
        );
        assert!(
            !labels.contains(&"hurryUp"),
            "hurryUp/hurry_up should be matched across naming conventions: {labels:?}"
        );
        assert!(
            !labels.contains(&"close"),
            "close exists in both: {labels:?}"
        );
    }

    #[test]
    fn dedupes_and_keeps_highest_degree_representative() {
        let extraction = ExtractionResult {
            nodes: vec![
                mk_node("a", "hurryUp", "left/a.cpp"),
                mk_node("b", "hurry_up", "left/b.cpp"),
                mk_node("c", "other", "left/c.cpp"),
                mk_node("d", "irrelevant", "right/d.rs"),
            ],
            edges: vec![mk_edge("a", "c"), mk_edge("a", "d"), mk_edge("b", "c")],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);
        let q = CompareQuery {
            left_pattern: "left/".into(),
            right_pattern: "right/".into(),
            ..Default::default()
        };
        let gaps = compare_repos(&graph, &q);
        let hurry: Vec<&CompareGap> = gaps.iter().filter(|g| g.norm == "hurryup").collect();
        assert_eq!(hurry.len(), 1, "should dedupe by normalized form");
        // 'a' has 2 edges, 'b' has 1 → keep 'a'
        assert_eq!(hurry[0].label, "hurryUp");
    }

    #[test]
    fn skips_file_and_concept_nodes_by_default() {
        // Node "data" with source_file "left/data.cpp" → is_file_node (label==stem).
        // Node "modulehub" with source_file "modulehub" → is_concept_node (no ext, no /).
        // Real symbol "process()" should pass.
        let extraction = ExtractionResult {
            nodes: vec![
                mk_node("d", "data", "left/data.cpp"),
                mk_node("h", "modulehub", "modulehub"),
                mk_node("p", "process()", "left/server.cpp"),
            ],
            edges: vec![mk_edge("p", "d"), mk_edge("p", "h")],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);
        let q = CompareQuery {
            left_pattern: "left".into(),
            right_pattern: "right".into(),
            ..Default::default()
        };
        let gaps = compare_repos(&graph, &q);
        let labels: Vec<&str> = gaps.iter().map(|g| g.label.as_str()).collect();
        assert!(
            labels.contains(&"process()"),
            "process() should pass: {labels:?}"
        );
        assert!(
            !labels.contains(&"data"),
            "data is a file-node, should be filtered: {labels:?}"
        );
        assert!(
            !labels.contains(&"modulehub"),
            "modulehub is a concept-node, should be filtered: {labels:?}"
        );
    }

    #[test]
    fn label_pattern_narrows_to_domain() {
        let extraction = ExtractionResult {
            nodes: vec![
                mk_node("a", "tickSearch()", "pvxs/client.cpp"),
                mk_node("b", "from_wire()", "pvxs/evhelper.cpp"),
                mk_node("c", "tostring()", "pvxs/util.cpp"),
            ],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);
        let q = CompareQuery {
            left_pattern: "pvxs".into(),
            right_pattern: "pva-rs".into(),
            label_pattern: Some("search".into()),
            ..Default::default()
        };
        let gaps = compare_repos(&graph, &q);
        let labels: Vec<&str> = gaps.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, vec!["tickSearch()"]);
    }

    #[test]
    fn min_degree_filters_low_connection_gaps() {
        // a has 2 edges (to b, c). b/c each have 1.
        let extraction = ExtractionResult {
            nodes: vec![
                mk_node("a", "hub_func()", "left/a.cpp"),
                mk_node("b", "leaf_b()", "left/b.cpp"),
                mk_node("c", "leaf_c()", "left/c.cpp"),
                mk_node("d", "irrelevant", "right/d.rs"),
            ],
            edges: vec![mk_edge("a", "b"), mk_edge("a", "c")],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);
        let q = CompareQuery {
            left_pattern: "left".into(),
            right_pattern: "right".into(),
            min_degree: 2,
            ..Default::default()
        };
        let gaps = compare_repos(&graph, &q);
        let labels: Vec<&str> = gaps.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, vec!["hub_func()"]);
    }

    #[test]
    fn public_pattern_promotes_header_gaps_above_internal() {
        // Two gaps in pvxs but pva-rs has nothing: a low-degree public-header
        // symbol and a high-degree internal symbol. Without public_pattern,
        // degree wins (internal first). With public_pattern, header wins.
        let extraction = ExtractionResult {
            nodes: vec![
                mk_node("p", "publicSym()", "pvxs/include/pvxs/server.h"),
                mk_node("i", "internalSym()", "pvxs/src/internal.cpp"),
                mk_node("u1", "user1", "pvxs/src/internal.cpp"),
                mk_node("u2", "user2", "pvxs/src/internal.cpp"),
                mk_node("u3", "user3", "pvxs/src/internal.cpp"),
            ],
            edges: vec![mk_edge("i", "u1"), mk_edge("i", "u2"), mk_edge("i", "u3")],
            ..Default::default()
        };
        let g = build_from_extraction(&extraction);

        let no_pub = CompareQuery {
            left_pattern: "pvxs".into(),
            right_pattern: "pva-rs".into(),
            ..Default::default()
        };
        let r1 = compare_repos(&g, &no_pub);
        let labels1: Vec<&str> = r1.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(
            labels1[0], "internalSym()",
            "no public_pattern → degree wins"
        );

        let with_pub = CompareQuery {
            left_pattern: "pvxs".into(),
            right_pattern: "pva-rs".into(),
            public_pattern: Some("/include/".into()),
            ..Default::default()
        };
        let r2 = compare_repos(&g, &with_pub);
        let labels2: Vec<&str> = r2.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(
            labels2[0], "publicSym()",
            "public header gap should outrank internal high-degree: {labels2:?}"
        );
        assert!(r2[0].is_public);
        assert!(!r2[1].is_public);
    }

    #[test]
    fn public_only_drops_internal_gaps() {
        let extraction = ExtractionResult {
            nodes: vec![
                mk_node("p", "publicSym()", "pvxs/include/pvxs/server.h"),
                mk_node("i", "internalSym()", "pvxs/src/internal.cpp"),
            ],
            ..Default::default()
        };
        let g = build_from_extraction(&extraction);
        let q = CompareQuery {
            left_pattern: "pvxs".into(),
            right_pattern: "pva-rs".into(),
            public_pattern: Some("/include/".into()),
            public_only: true,
            ..Default::default()
        };
        let result = compare_repos(&g, &q);
        let labels: Vec<&str> = result.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, vec!["publicSym()"]);
    }

    #[test]
    fn tokenize_splits_camel_and_snake_case() {
        assert_eq!(tokenize_label("tickSearch()"), vec!["tick", "search"]);
        assert_eq!(
            tokenize_label("Server::handle_REQUEST"),
            vec!["server", "handle", "request"]
        );
        // 2-char tokens are kept (`io`, `fs` are meaningful); 1-char dropped.
        assert_eq!(tokenize_label("io_op"), vec!["io", "op"]);
        assert_eq!(tokenize_label("Foo::x"), vec!["foo"]);
    }

    #[test]
    fn semantic_check_flags_token_overlap_candidates() {
        // pvxs gap `tickSearch()` has no normalized match in pva-rs, but the
        // pva-rs side has `process_search_request()` — same `search` token.
        // Without semantic_threshold the gap is reported standalone; with
        // it we get a candidate_matches entry.
        let extraction = ExtractionResult {
            nodes: vec![
                mk_node("g", "tickSearch()", "pvxs/src/client.cpp"),
                mk_node("c", "process_search_request()", "pva-rs/src/client.rs"),
                mk_node("u", "unrelated_thing()", "pva-rs/src/util.rs"),
            ],
            ..Default::default()
        };
        let g = build_from_extraction(&extraction);

        let no_sem = CompareQuery {
            left_pattern: "pvxs".into(),
            right_pattern: "pva-rs".into(),
            ..Default::default()
        };
        let r1 = compare_repos(&g, &no_sem);
        assert!(
            r1.iter().all(|gap| gap.candidate_matches.is_empty()),
            "no semantic_threshold → no candidate_matches"
        );

        let with_sem = CompareQuery {
            left_pattern: "pvxs".into(),
            right_pattern: "pva-rs".into(),
            semantic_threshold: 0.2,
            ..Default::default()
        };
        let r2 = compare_repos(&g, &with_sem);
        let gap = r2
            .iter()
            .find(|x| x.label == "tickSearch()")
            .expect("gap missing");
        let cand_labels: Vec<&str> = gap
            .candidate_matches
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert!(
            cand_labels.contains(&"process_search_request()"),
            "expected token-overlap match: {cand_labels:?}"
        );
        assert!(
            !cand_labels.contains(&"unrelated_thing()"),
            "non-overlapping label must not match: {cand_labels:?}"
        );
    }

    #[test]
    fn compose_priority_uses_fan_in_not_total_degree() {
        // hub_called: 1 outgoing + 5 incoming → fan_in=5, degree=6
        // hub_callsmany: 5 outgoing + 1 incoming → fan_in=1, degree=6
        // Without compose_priority, both tie on degree and norm sort decides.
        // With compose_priority, hub_called must rank higher because fan_in
        // is a truer "architectural centrality" signal.
        let mut nodes = vec![
            mk_node("hc", "hub_called", "left/a.rs"),
            mk_node("hcm", "hub_callsmany", "left/b.rs"),
        ];
        for i in 0..5 {
            nodes.push(mk_node(
                &format!("c{i}"),
                &format!("caller_{i}"),
                "left/x.rs",
            ));
            nodes.push(mk_node(
                &format!("t{i}"),
                &format!("target_{i}"),
                "left/x.rs",
            ));
        }
        let mut edges: Vec<Edge> = (0..5).map(|i| mk_edge(&format!("c{i}"), "hc")).collect();
        edges.push(mk_edge("hc", "t0")); // hub_called: 1 outgoing
        for i in 0..5 {
            edges.push(mk_edge("hcm", &format!("t{i}"))); // hub_callsmany: 5 outgoing
        }
        edges.push(mk_edge("c0", "hcm")); // hub_callsmany: 1 incoming

        let g = build_from_extraction(&ExtractionResult {
            nodes,
            edges,
            ..Default::default()
        });

        let q = CompareQuery {
            left_pattern: "left".into(),
            right_pattern: "right".into(),
            compose_priority: true,
            top_n: 10,
            ..Default::default()
        };
        let result = compare_repos(&g, &q);
        // First two should be the hubs; check ordering by priority.
        let hub_idx = result.iter().position(|x| x.label == "hub_called");
        let other_idx = result.iter().position(|x| x.label == "hub_callsmany");
        assert!(hub_idx.is_some() && other_idx.is_some(), "{result:?}");
        assert!(
            hub_idx.unwrap() < other_idx.unwrap(),
            "hub_called (fan_in=5) must rank above hub_callsmany (fan_in=1): {result:?}"
        );
        assert!(result[hub_idx.unwrap()].priority_score > 0.0);
    }

    #[test]
    fn empty_pattern_finds_no_gaps() {
        let extraction = ExtractionResult {
            nodes: vec![mk_node("a", "foo", "x.rs")],
            ..Default::default()
        };
        let graph = build_from_extraction(&extraction);
        let q = CompareQuery {
            left_pattern: String::new(),
            right_pattern: String::new(),
            ..Default::default()
        };
        assert!(compare_repos(&graph, &q).is_empty());
    }
}
