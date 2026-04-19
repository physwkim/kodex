use std::collections::HashMap;
use std::fmt::Write;

use crate::analyze::god_nodes::GodNode;
use crate::analyze::surprising::SurprisingConnection;
use crate::analyze::questions::SuggestedQuestion;
use crate::analyze::helpers::{is_file_node, is_concept_node};
use crate::graph::KodexGraph;
use crate::types::{Confidence, DetectionResult};

/// Generate GRAPH_REPORT.md content.
#[allow(clippy::too_many_arguments)]
pub fn generate(
    graph: &KodexGraph,
    communities: &HashMap<usize, Vec<String>>,
    cohesion_scores: &HashMap<usize, f64>,
    community_labels: &HashMap<usize, String>,
    god_node_list: &[GodNode],
    surprise_list: &[SurprisingConnection],
    detection_result: &DetectionResult,
    input_tokens: u64,
    output_tokens: u64,
    root: &str,
    suggested_questions: Option<&[SuggestedQuestion]>,
) -> String {
    let mut md = String::new();
    let today = chrono_free_date();

    writeln!(md, "# Graph Report - {root}  ({today})").unwrap();
    writeln!(md).unwrap();

    // --- Corpus Check ---
    writeln!(md, "## Corpus Check\n").unwrap();
    writeln!(
        md,
        "- {} files \u{00b7} ~{} words",
        detection_result.total_files, detection_result.total_words
    )
    .unwrap();
    if let Some(warning) = &detection_result.warning {
        writeln!(md, "- \u{26a0}\u{fe0f} {warning}").unwrap();
    }
    writeln!(md).unwrap();

    // --- Summary ---
    let total_edges = graph.edge_count();
    let mut ext_count = 0usize;
    let mut inf_count = 0usize;
    let mut amb_count = 0usize;
    let mut inf_scores = Vec::new();

    for (_, _, edge) in graph.edges() {
        match edge.confidence {
            Confidence::EXTRACTED => ext_count += 1,
            Confidence::INFERRED => {
                inf_count += 1;
                inf_scores.push(edge.confidence_score.unwrap_or(0.5));
            }
            Confidence::AMBIGUOUS => amb_count += 1,
        }
    }

    let total = (ext_count + inf_count + amb_count).max(1) as f64;
    let ext_pct = ext_count as f64 / total * 100.0;
    let inf_pct = inf_count as f64 / total * 100.0;
    let amb_pct = amb_count as f64 / total * 100.0;
    let inf_avg = if inf_scores.is_empty() {
        0.0
    } else {
        inf_scores.iter().sum::<f64>() / inf_scores.len() as f64
    };

    writeln!(md, "## Summary\n").unwrap();
    writeln!(
        md,
        "- {} nodes \u{00b7} {} edges \u{00b7} {} communities detected",
        graph.node_count(),
        total_edges,
        communities.len()
    )
    .unwrap();
    writeln!(
        md,
        "- Extraction: {ext_pct:.0}% EXTRACTED \u{00b7} {inf_pct:.0}% INFERRED \u{00b7} {amb_pct:.0}% AMBIGUOUS"
    )
    .unwrap();
    if inf_count > 0 {
        writeln!(
            md,
            "- INFERRED: {inf_count} edges (avg confidence: {inf_avg:.2})"
        )
        .unwrap();
    }
    writeln!(
        md,
        "- Token cost: {input_tokens} input \u{00b7} {output_tokens} output"
    )
    .unwrap();
    writeln!(md).unwrap();

    // --- Community Hubs ---
    writeln!(md, "## Community Hubs\n").unwrap();
    let mut sorted_cids: Vec<usize> = communities.keys().copied().collect();
    sorted_cids.sort();
    for cid in &sorted_cids {
        let label = community_labels
            .get(cid)
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));
        let safe = safe_community_name(&label);
        writeln!(md, "- [[_COMMUNITY_{safe}|{label}]]").unwrap();
    }
    writeln!(md).unwrap();

    // --- God Nodes ---
    writeln!(md, "## God Nodes (most connected)\n").unwrap();
    for (i, god) in god_node_list.iter().enumerate() {
        writeln!(
            md,
            "{}. `{}` - {} edges",
            i + 1,
            god.label,
            god.degree
        )
        .unwrap();
    }
    writeln!(md).unwrap();

    // --- Surprising Connections ---
    if !surprise_list.is_empty() {
        writeln!(md, "## Surprising Connections\n").unwrap();
        for s in surprise_list {
            let src_label = graph
                .get_node(&s.source)
                .map(|n| n.label.as_str())
                .unwrap_or(&s.source);
            let tgt_label = graph
                .get_node(&s.target)
                .map(|n| n.label.as_str())
                .unwrap_or(&s.target);
            writeln!(
                md,
                "- `{src_label}` --{}-->  `{tgt_label}` [{}]",
                s.relation, s.confidence
            )
            .unwrap();
            if !s.reasons.is_empty() {
                writeln!(md, "  {}", s.reasons.join(", ")).unwrap();
            }
        }
        writeln!(md).unwrap();
    }

    // --- Hyperedges ---
    if !graph.hyperedges.is_empty() {
        writeln!(md, "## Hyperedges\n").unwrap();
        for h in &graph.hyperedges {
            let score = h
                .confidence_score
                .map(|s| format!(" [{s:.2}]"))
                .unwrap_or_default();
            writeln!(
                md,
                "- **{}** \u{2014} {}{score}",
                h.label,
                h.nodes.join(", ")
            )
            .unwrap();
        }
        writeln!(md).unwrap();
    }

    // --- Communities ---
    writeln!(md, "## Communities\n").unwrap();
    for cid in &sorted_cids {
        let nodes = match communities.get(cid) {
            Some(n) => n,
            None => continue,
        };
        let label = community_labels
            .get(cid)
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));
        let coh = cohesion_scores.get(cid).copied().unwrap_or(0.0);

        writeln!(md, "### Community {cid} - \"{label}\"\n").unwrap();
        writeln!(md, "Cohesion: {coh:.2}").unwrap();

        let display: Vec<&str> = nodes
            .iter()
            .take(8)
            .filter_map(|nid| graph.get_node(nid).map(|n| n.label.as_str()))
            .collect();
        let extra = if nodes.len() > 8 {
            format!(" +{} more", nodes.len() - 8)
        } else {
            String::new()
        };
        writeln!(
            md,
            "Nodes ({}): {}{extra}\n",
            nodes.len(),
            display.join(", ")
        )
        .unwrap();
    }

    // --- Ambiguous Edges ---
    let ambiguous: Vec<_> = graph
        .edges()
        .filter(|(_, _, e)| e.confidence == Confidence::AMBIGUOUS)
        .collect();
    if !ambiguous.is_empty() {
        writeln!(md, "## Ambiguous Edges - Review These\n").unwrap();
        for (src, tgt, edge) in &ambiguous {
            let src_label = graph
                .get_node(src)
                .map(|n| n.label.as_str())
                .unwrap_or(src);
            let tgt_label = graph
                .get_node(tgt)
                .map(|n| n.label.as_str())
                .unwrap_or(tgt);
            writeln!(
                md,
                "- `{src_label}` \u{2192} `{tgt_label}` [AMBIGUOUS]"
            )
            .unwrap();
            writeln!(
                md,
                "  {} \u{00b7} relation: {}",
                edge.source_file, edge.relation
            )
            .unwrap();
        }
        writeln!(md).unwrap();
    }

    // --- Knowledge Gaps ---
    let isolated: Vec<&str> = graph
        .node_ids()
        .filter(|id| {
            graph.degree(id) <= 1 && !is_file_node(graph, id) && !is_concept_node(graph, id)
        })
        .filter_map(|id| graph.get_node(id).map(|n| n.label.as_str()))
        .collect();
    let thin_communities: Vec<_> = communities
        .iter()
        .filter(|(_, nodes)| nodes.len() < 3)
        .collect();

    if !isolated.is_empty() || !thin_communities.is_empty() || amb_pct > 20.0 {
        writeln!(md, "## Knowledge Gaps\n").unwrap();
        if !isolated.is_empty() {
            let display: Vec<_> = isolated.iter().take(10).copied().collect();
            let extra = if isolated.len() > 10 {
                format!(" +{} more", isolated.len() - 10)
            } else {
                String::new()
            };
            writeln!(
                md,
                "- {} isolated node(s): {}{extra}",
                isolated.len(),
                display.join(", ")
            )
            .unwrap();
        }
        for (cid, nodes) in &thin_communities {
            let label = community_labels
                .get(cid)
                .cloned()
                .unwrap_or_else(|| format!("Community {cid}"));
            writeln!(
                md,
                "- Thin community `{label}` ({} nodes)",
                nodes.len()
            )
            .unwrap();
        }
        if amb_pct > 20.0 {
            writeln!(
                md,
                "- High ambiguity: {amb_pct:.0}% of edges are AMBIGUOUS"
            )
            .unwrap();
        }
        writeln!(md).unwrap();
    }

    // --- Suggested Questions ---
    if let Some(questions) = suggested_questions {
        if !questions.is_empty() {
            writeln!(md, "## Suggested Questions\n").unwrap();
            for q in questions {
                writeln!(md, "- **{}**", q.question).unwrap();
            }
            writeln!(md).unwrap();
        }
    }

    md
}

fn safe_community_name(label: &str) -> String {
    label
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ' '], "_")
        .trim_end_matches(".md")
        .trim_end_matches(".mdx")
        .trim_end_matches(".markdown")
        .to_string()
}

fn chrono_free_date() -> String {
    // Civil date from Unix timestamp without chrono dependency.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let days = (secs / 86400) as i32;
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y}-{m:02}-{d:02}")
}
