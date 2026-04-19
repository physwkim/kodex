use std::path::Path;

pub fn query(question: &str, use_dfs: bool, budget: usize, graph_path: &Path) {
    let graph = match super::load_graph(graph_path) {
        Some(g) => g,
        None => return,
    };

    let terms: Vec<String> = question
        .split_whitespace()
        .filter(|t| t.len() > 2)
        .map(|t| t.to_lowercase())
        .collect();

    let scored = kodex::serve::score_nodes(&graph, &terms);
    let start_nodes: Vec<String> = scored.into_iter().take(3).map(|(_, id)| id).collect();

    if start_nodes.is_empty() {
        println!("No matching nodes found for: {question}");
        return;
    }

    let (visited, edges) = if use_dfs {
        kodex::serve::dfs(&graph, &start_nodes, 3)
    } else {
        kodex::serve::bfs(&graph, &start_nodes, 3)
    };

    let text = kodex::serve::subgraph_to_text(&graph, &visited, &edges, budget);
    println!("{text}");
}
