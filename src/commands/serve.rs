use std::path::Path;

pub fn serve(graph_path: &Path) {
    let graph = match super::load_graph(graph_path) {
        Some(g) => g,
        None => return,
    };

    println!(
        "MCP server ready: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );
    println!("Reading JSON-RPC from stdin...");

    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        match stdin.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let response = handle_jsonrpc(trimmed, &graph);
                println!("{response}");
            }
            Err(_) => break,
        }
    }
}

fn handle_jsonrpc(input: &str, graph: &kodex::graph::KodexGraph) -> String {
    let req: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => {
            return format!(
                r#"{{"jsonrpc":"2.0","error":{{"code":-32700,"message":"Parse error: {e}"}},"id":null}}"#
            )
        }
    };

    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(serde_json::json!({}));

    let result = match method {
        "query_graph" => {
            let question = params
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let budget = params
                .get("token_budget")
                .and_then(|v| v.as_u64())
                .unwrap_or(2000) as usize;
            let terms: Vec<String> = question
                .split_whitespace()
                .filter(|t| t.len() > 2)
                .map(|t| t.to_lowercase())
                .collect();
            let scored = kodex::serve::score_nodes(graph, &terms);
            let start: Vec<String> = scored.into_iter().take(3).map(|(_, id)| id).collect();
            let (visited, edges) = kodex::serve::bfs(graph, &start, depth);
            serde_json::json!(kodex::serve::subgraph_to_text(
                graph, &visited, &edges, budget
            ))
        }
        "get_node" => {
            let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let matches = kodex::serve::score_nodes(graph, &[label.to_lowercase()]);
            match matches.first() {
                Some((_, nid)) => {
                    let node = graph.get_node(nid);
                    let deg = graph.degree(nid);
                    serde_json::json!({"node": node, "degree": deg})
                }
                None => serde_json::json!({"error": "not found"}),
            }
        }
        "god_nodes" => {
            let top_n = params.get("top_n").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let gods = kodex::analyze::god_nodes(graph, top_n);
            let list: Vec<serde_json::Value> = gods.iter().map(|g| {
                serde_json::json!({"label": g.label, "degree": g.degree, "source_file": g.source_file})
            }).collect();
            serde_json::json!(list)
        }
        "graph_stats" => {
            let communities = kodex::serve::communities_from_graph(graph);
            serde_json::json!({
                "nodes": graph.node_count(),
                "edges": graph.edge_count(),
                "communities": communities.len(),
            })
        }
        "save_insight" => handle_save_insight(&params),
        "save_note" => handle_save_note(&params),
        "add_edge" => handle_add_edge(&params),
        "learn" => handle_learn(&params),
        "recall" => {
            let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let type_filter = params.get("type").and_then(|v| v.as_str());
            let h5 = std::path::Path::new("kodex-out/kodex.h5");
            let results = kodex::learn::query_knowledge(h5, query, type_filter);
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|k| {
                    serde_json::json!({
                        "title": k.title,
                        "type": k.knowledge_type.to_string(),
                        "description": k.description.lines().next().unwrap_or(""),
                        "confidence": (k.confidence * 100.0) as u32,
                        "observations": k.observations,
                        "related_nodes": k.related_nodes,
                    })
                })
                .collect();
            serde_json::json!(items)
        }
        "knowledge_context" => {
            let h5 = std::path::Path::new("kodex-out/kodex.h5");
            let max = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(20) as usize;
            serde_json::json!(kodex::learn::knowledge_context(h5, max))
        }
        _ => serde_json::json!({"error": format!("Unknown method: {method}")}),
    };

    format!(
        r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#,
        serde_json::to_string(&result).unwrap_or_default(),
        serde_json::to_string(&id).unwrap_or_default(),
    )
}

fn extract_string_array(params: &serde_json::Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn handle_save_insight(params: &serde_json::Value) -> serde_json::Value {
    let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");
    let desc = params
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let nodes = extract_string_array(params, "nodes");
    let pattern = params.get("pattern").and_then(|v| v.as_str());
    let gp = std::path::Path::new("kodex-out/kodex.h5");
    match kodex::knowledge::save_insight(gp, None, label, desc, &nodes, pattern) {
        Ok(()) => serde_json::json!({"status": "saved", "label": label, "nodes": nodes.len()}),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    }
}

fn handle_save_note(params: &serde_json::Value) -> serde_json::Value {
    let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let related = extract_string_array(params, "related_nodes");
    let gp = std::path::Path::new("kodex-out/kodex.h5");
    match kodex::knowledge::save_note(gp, None, title, content, &related) {
        Ok(()) => serde_json::json!({"status": "saved", "title": title}),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    }
}

fn handle_add_edge(params: &serde_json::Value) -> serde_json::Value {
    let source = params.get("source").and_then(|v| v.as_str()).unwrap_or("");
    let target = params.get("target").and_then(|v| v.as_str()).unwrap_or("");
    let relation = params
        .get("relation")
        .and_then(|v| v.as_str())
        .unwrap_or("related_to");
    let desc = params.get("description").and_then(|v| v.as_str());
    let gp = std::path::Path::new("kodex-out/kodex.h5");
    match kodex::knowledge::add_edge(gp, source, target, relation, desc) {
        Ok(()) => serde_json::json!({"status": "saved", "source": source, "target": target}),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    }
}

fn handle_learn(params: &serde_json::Value) -> serde_json::Value {
    let kt_str = params
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("pattern");
    let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let desc = params
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let related = extract_string_array(params, "related_nodes");
    let tags = extract_string_array(params, "tags");

    let kt = match kt_str {
        "architecture" => kodex::learn::KnowledgeType::Architecture,
        "pattern" => kodex::learn::KnowledgeType::Pattern,
        "decision" => kodex::learn::KnowledgeType::Decision,
        "convention" => kodex::learn::KnowledgeType::Convention,
        "coupling" => kodex::learn::KnowledgeType::Coupling,
        "domain" => kodex::learn::KnowledgeType::Domain,
        "preference" => kodex::learn::KnowledgeType::Preference,
        "bug_pattern" => kodex::learn::KnowledgeType::BugPattern,
        "tech_debt" => kodex::learn::KnowledgeType::TechDebt,
        "ops" => kodex::learn::KnowledgeType::Ops,
        "api" => kodex::learn::KnowledgeType::Api,
        "performance" => kodex::learn::KnowledgeType::Performance,
        "roadmap" => kodex::learn::KnowledgeType::Roadmap,
        "context" => kodex::learn::KnowledgeType::Context,
        "lesson" => kodex::learn::KnowledgeType::Lesson,
        other => kodex::learn::KnowledgeType::Custom(other.to_string()),
    };

    let h5 = std::path::Path::new("kodex-out/kodex.h5");
    match kodex::learn::learn(h5, kt, title, desc, &related, &tags) {
        Ok(_) => serde_json::json!({"status": "learned", "title": title}),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    }
}
