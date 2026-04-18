use std::collections::HashSet;

use crate::graph::GraphifyGraph;

const CHARS_PER_TOKEN: usize = 4;

const SAMPLE_QUESTIONS: &[&str] = &[
    "How does authentication work?",
    "What are the main data models?",
    "How is error handling implemented?",
    "What external APIs are used?",
    "How does the build pipeline work?",
];

/// Benchmark result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BenchmarkResult {
    pub corpus_tokens: usize,
    pub corpus_words: usize,
    pub nodes: usize,
    pub edges: usize,
    pub avg_query_tokens: usize,
    pub reduction_ratio: f64,
    pub per_question: Vec<QuestionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QuestionResult {
    pub question: String,
    pub query_tokens: usize,
    pub reduction: f64,
}

/// Measure token reduction vs naive full-corpus approach.
pub fn run_benchmark(
    graph: &GraphifyGraph,
    corpus_words: Option<usize>,
    questions: Option<&[&str]>,
) -> BenchmarkResult {
    let words = corpus_words.unwrap_or_else(|| graph.node_count() * 50);
    let corpus_tokens = words * 100 / 75; // ~133 tokens per 100 words

    let qs = questions.unwrap_or(SAMPLE_QUESTIONS);
    let mut per_question = Vec::new();

    for &q in qs {
        let qt = query_subgraph_tokens(graph, q, 3);
        if qt > 0 {
            per_question.push(QuestionResult {
                question: q.to_string(),
                query_tokens: qt,
                reduction: (corpus_tokens as f64 / qt as f64 * 10.0).round() / 10.0,
            });
        }
    }

    if per_question.is_empty() {
        return BenchmarkResult {
            corpus_tokens,
            corpus_words: words,
            nodes: graph.node_count(),
            edges: graph.edge_count(),
            avg_query_tokens: 0,
            reduction_ratio: 0.0,
            per_question,
            error: Some("No matching nodes found for any question".to_string()),
        };
    }

    let avg_query_tokens = per_question.iter().map(|p| p.query_tokens).sum::<usize>()
        / per_question.len();
    let reduction_ratio = if avg_query_tokens > 0 {
        (corpus_tokens as f64 / avg_query_tokens as f64 * 10.0).round() / 10.0
    } else {
        0.0
    };

    BenchmarkResult {
        corpus_tokens,
        corpus_words: words,
        nodes: graph.node_count(),
        edges: graph.edge_count(),
        avg_query_tokens,
        reduction_ratio,
        per_question,
        error: None,
    }
}

/// Print benchmark results in human-readable format.
pub fn print_benchmark(result: &BenchmarkResult) {
    if let Some(error) = &result.error {
        println!("Benchmark error: {error}");
        return;
    }

    println!("\ngraphify token reduction benchmark");
    println!("{}", "\u{2500}".repeat(50));
    println!("  Corpus:          {} words \u{2192} ~{} tokens (naive)", result.corpus_words, result.corpus_tokens);
    println!("  Graph:           {} nodes, {} edges", result.nodes, result.edges);
    println!("  Avg query cost:  ~{} tokens", result.avg_query_tokens);
    println!("  Reduction:       {}x fewer tokens per query", result.reduction_ratio);
    println!("\n  Per question:");
    for p in &result.per_question {
        let display = if p.question.len() > 55 {
            &p.question[..55]
        } else {
            &p.question
        };
        println!("    [{:.1}x] {display}", p.reduction);
    }
    println!();
}

/// Estimate tokens for a BFS subgraph query.
fn query_subgraph_tokens(graph: &GraphifyGraph, question: &str, depth: usize) -> usize {
    // Extract search terms
    let terms: Vec<String> = question
        .split_whitespace()
        .filter(|t| t.len() > 2)
        .map(|t| t.to_lowercase())
        .collect();

    if terms.is_empty() {
        return 0;
    }

    // Score nodes by label match
    let mut scored: Vec<(usize, &str)> = graph
        .node_ids()
        .filter_map(|id| {
            let node = graph.get_node(id)?;
            let label = node.label.to_lowercase();
            let score = terms.iter().filter(|t| label.contains(t.as_str())).count();
            if score > 0 {
                Some((score, id.as_str()))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    let start_nodes: Vec<String> = scored.iter().take(3).map(|(_, id)| id.to_string()).collect();

    if start_nodes.is_empty() {
        return 0;
    }

    // BFS with owned Strings (no lifetime issues)
    let mut visited: HashSet<String> = start_nodes.iter().cloned().collect();
    let mut frontier: HashSet<String> = visited.clone();

    for _ in 0..depth {
        let mut next_frontier = HashSet::new();
        for nid in &frontier {
            for neighbor in graph.neighbors(nid) {
                if !visited.contains(&neighbor) {
                    next_frontier.insert(neighbor);
                }
            }
        }
        visited.extend(next_frontier.iter().cloned());
        frontier = next_frontier;
    }

    // Estimate tokens from text
    let mut text_len = 0;
    for nid in &visited {
        if let Some(node) = graph.get_node(nid) {
            text_len += format!(
                "NODE {} src={} loc={}\n",
                node.label,
                node.source_file,
                node.source_location.as_deref().unwrap_or("")
            )
            .len();
        }
    }

    for (src, tgt, edge) in graph.edges() {
        if visited.contains(&src.to_string()) && visited.contains(&tgt.to_string()) {
            let src_label = graph
                .get_node(src)
                .map(|n| n.label.as_str())
                .unwrap_or(src);
            let tgt_label = graph
                .get_node(tgt)
                .map(|n| n.label.as_str())
                .unwrap_or(tgt);
            text_len += format!("EDGE {src_label} --{}--> {tgt_label}\n", edge.relation).len();
        }
    }

    (text_len / CHARS_PER_TOKEN).max(1)
}
