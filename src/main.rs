use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "graphify",
    version,
    about = "A knowledge graph builder for code and documents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Target path to analyze (default command: run full pipeline)
    path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the full pipeline: detect → extract → build → cluster → analyze → report → export
    Run {
        /// Target directory to analyze
        path: PathBuf,
    },

    /// Query the knowledge graph
    Query {
        /// Question to search for
        question: String,
        /// Use DFS instead of BFS
        #[arg(long)]
        dfs: bool,
        /// Token budget for output
        #[arg(long, default_value_t = 2000)]
        budget: usize,
        /// Path to graph.json
        #[arg(long, default_value = "graphify-out/graph.json")]
        graph: PathBuf,
    },

    /// Find shortest path between two nodes
    Path {
        /// Source node label
        source: String,
        /// Target node label
        target: String,
        /// Path to graph.json
        #[arg(long, default_value = "graphify-out/graph.json")]
        graph: PathBuf,
    },

    /// Explain a node and its neighbors
    Explain {
        /// Node label to explain
        node: String,
        /// Path to graph.json
        #[arg(long, default_value = "graphify-out/graph.json")]
        graph: PathBuf,
    },

    /// Watch directory for changes and auto-rebuild
    Watch {
        /// Directory to watch
        path: PathBuf,
        /// Debounce interval in seconds
        #[arg(long, default_value_t = 3.0)]
        debounce: f64,
        /// Obsidian vault path to auto-update on changes
        #[arg(long)]
        vault: Option<PathBuf>,
    },

    /// Measure token reduction benchmark
    Benchmark {
        /// Path to graph.json
        #[arg(default_value = "graphify-out/graph.json")]
        graph: PathBuf,
    },

    /// Manage git hooks
    Hook {
        /// Action: install, uninstall, or status
        #[arg(default_value = "status")]
        action: String,
    },

    /// Re-extract code files only (AST, no LLM) without full rebuild
    Update {
        /// Target directory
        path: PathBuf,
    },

    /// Rerun clustering on existing graph.json
    ClusterOnly {
        /// Target directory containing graphify-out/
        path: PathBuf,
    },

    /// Fetch URL and add to corpus
    Add {
        /// URL to fetch (tweet, arXiv, PDF, web page)
        url: String,
        /// Author attribution
        #[arg(long)]
        author: Option<String>,
        /// Contributor attribution
        #[arg(long)]
        contributor: Option<String>,
        /// Target directory for saved content
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },

    /// Install graphify skill to AI editor platform
    Install {
        /// Platform: claude, cursor, vscode, codex, opencode, aider, kiro
        #[arg(default_value = "claude")]
        platform: String,
    },

    /// Manage multi-project workspace
    Workspace {
        /// Action: init, run, or watch
        action: String,
        /// Override vault output path
        #[arg(long)]
        vault: Option<PathBuf>,
    },

    /// Start MCP stdio server for graph queries
    Serve {
        /// Path to graph.json
        #[arg(default_value = "graphify-out/graph.json")]
        graph: PathBuf,
    },
}

fn dirs_or_cwd() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run { path }) => {
            run_pipeline(&path);
        }
        Some(Commands::Query { question, dfs, budget, graph }) => {
            cmd_query(&question, dfs, budget, &graph);
        }
        Some(Commands::Path { source, target, graph }) => {
            cmd_path(&source, &target, &graph);
        }
        Some(Commands::Explain { node, graph }) => {
            cmd_explain(&node, &graph);
        }
        Some(Commands::Watch { path, debounce, vault }) => {
            if let Err(e) = graphify::watch::watch(&path, debounce, vault.as_deref()) {
                eprintln!("Watch error: {e}");
            }
        }
        Some(Commands::Benchmark { graph }) => {
            cmd_benchmark(&graph);
        }
        Some(Commands::Hook { action }) => {
            let path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let result = match action.as_str() {
                "install" => graphify::hooks::install(&path),
                "uninstall" => graphify::hooks::uninstall(&path),
                _ => graphify::hooks::status(&path),
            };
            println!("{result}");
        }
        Some(Commands::Workspace { action, vault }) => {
            cmd_workspace(&action, vault.as_deref());
        }
        Some(Commands::Install { platform }) => {
            let home = dirs_or_cwd();
            let result = graphify::install::install(Some(&platform), &home);
            println!("{result}");
        }
        Some(Commands::Update { path }) => {
            cmd_update(&path);
        }
        Some(Commands::ClusterOnly { path }) => {
            cmd_cluster_only(&path);
        }
        Some(Commands::Add { url, author, contributor, dir }) => {
            cmd_add(&url, author.as_deref(), contributor.as_deref(), &dir);
        }
        Some(Commands::Serve { graph }) => {
            cmd_serve(&graph);
        }
        None => {
            // Default: treat positional path as `run`
            if let Some(path) = cli.path {
                run_pipeline(&path);
            } else {
                println!("graphify: no path specified. Use --help for usage.");
            }
        }
    }
}

fn run_pipeline(path: &std::path::Path) {
    println!("graphify: analyzing {}", path.display());

    // Step 1: Detect files
    let detection = graphify::detect::detect(path, false);
    println!(
        "  detected {} files ({} code, {} doc, {} paper, {} image, {} video)",
        detection.total_files,
        detection.files.code.len(),
        detection.files.document.len(),
        detection.files.paper.len(),
        detection.files.image.len(),
        detection.files.video.len(),
    );
    if !detection.skipped_sensitive.is_empty() {
        println!("  skipped {} sensitive file(s)", detection.skipped_sensitive.len());
    }

    // Step 2: Extract (code files only, requires extract feature)
    #[cfg(feature = "extract")]
    let extraction = {
        let code_paths: Vec<PathBuf> = detection
            .files
            .code
            .iter()
            .map(PathBuf::from)
            .collect();
        if code_paths.is_empty() {
            println!("  no code files to extract");
            graphify::types::ExtractionResult::default()
        } else {
            println!("  extracting {} code files...", code_paths.len());
            let result = graphify::extract::extract(&code_paths, Some(path));
            println!(
                "  extracted {} nodes, {} edges",
                result.nodes.len(),
                result.edges.len()
            );
            result
        }
    };

    #[cfg(not(feature = "extract"))]
    let extraction = {
        println!("  extract feature not enabled, skipping AST extraction");
        graphify::types::ExtractionResult::default()
    };

    // Step 3: Build graph
    let graph = graphify::graph::build_from_extraction(&extraction);
    println!(
        "  built graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // Step 4: Cluster
    let communities = graphify::cluster::cluster(&graph);
    println!("  detected {} communities", communities.len());

    // Step 5: Analyze
    let cohesion = graphify::cluster::score_all(&graph, &communities);
    let gods = graphify::analyze::god_nodes(&graph, 10);
    let surprises = graphify::analyze::surprising_connections(&graph, Some(&communities), 5);
    let questions = graphify::analyze::suggest_questions(&graph, Some(&communities), 7);

    // Generate community labels
    let community_labels: std::collections::HashMap<usize, String> = communities
        .iter()
        .map(|(&cid, nodes)| {
            let label = nodes
                .first()
                .and_then(|nid| graph.get_node(nid))
                .map(|n| n.label.clone())
                .unwrap_or_else(|| format!("Community {cid}"));
            (cid, label)
        })
        .collect();

    // Step 6: Export
    let out_dir = path.join("graphify-out");
    let _ = std::fs::create_dir_all(&out_dir);

    // JSON
    if let Err(e) = graphify::export::to_json(&graph, &communities, &out_dir.join("graph.json")) {
        eprintln!("  JSON export error: {e}");
    } else {
        println!("  exported graph.json");
    }

    // HTML
    match graphify::export::to_html(
        &graph,
        &communities,
        &out_dir.join("graph.html"),
        Some(&community_labels),
    ) {
        Ok(()) => println!("  exported graph.html"),
        Err(e) => eprintln!("  HTML export error: {e}"),
    }

    // Step 7: Report
    let report = graphify::report::generate(
        &graph,
        &communities,
        &cohesion,
        &community_labels,
        &gods,
        &surprises,
        &detection,
        0,
        0,
        &path.display().to_string(),
        Some(&questions),
    );
    let report_path = out_dir.join("GRAPH_REPORT.md");
    if let Err(e) = std::fs::write(&report_path, &report) {
        eprintln!("  Report error: {e}");
    } else {
        println!("  exported GRAPH_REPORT.md");
    }

    println!("  done! Output in {}", out_dir.display());
}

fn cmd_query(question: &str, use_dfs: bool, budget: usize, graph_path: &std::path::Path) {
    let graph = match graphify::serve::load_graph_smart(graph_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to load graph: {e}");
            return;
        }
    };

    let terms: Vec<String> = question
        .split_whitespace()
        .filter(|t| t.len() > 2)
        .map(|t| t.to_lowercase())
        .collect();

    let scored = graphify::serve::score_nodes(&graph, &terms);
    let start_nodes: Vec<String> = scored.into_iter().take(3).map(|(_, id)| id).collect();

    if start_nodes.is_empty() {
        println!("No matching nodes found for: {question}");
        return;
    }

    let (visited, edges) = if use_dfs {
        graphify::serve::dfs(&graph, &start_nodes, 3)
    } else {
        graphify::serve::bfs(&graph, &start_nodes, 3)
    };

    let text = graphify::serve::subgraph_to_text(&graph, &visited, &edges, budget);
    println!("{text}");
}

fn cmd_path(source: &str, target: &str, graph_path: &std::path::Path) {
    let graph = match graphify::serve::load_graph_smart(graph_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to load graph: {e}");
            return;
        }
    };

    // Find matching nodes
    let src_nodes = graphify::serve::score_nodes(&graph, &[source.to_lowercase()]);
    let tgt_nodes = graphify::serve::score_nodes(&graph, &[target.to_lowercase()]);

    match (src_nodes.first(), tgt_nodes.first()) {
        (Some((_, src_id)), Some((_, tgt_id))) => {
            // Use petgraph's built-in path finding
            let src_idx = graph.node_index.get(src_id);
            let tgt_idx = graph.node_index.get(tgt_id);
            if let (Some(&si), Some(&ti)) = (src_idx, tgt_idx) {
                if let Some(path) = petgraph::algo::astar(
                    &graph.inner,
                    si,
                    |n| n == ti,
                    |_| 1,
                    |_| 0,
                ) {
                    println!("Path ({} hops):", path.0);
                    for idx in &path.1 {
                        let node = &graph.inner[*idx];
                        println!("  -> {} ({})", node.label, node.source_file);
                    }
                } else {
                    println!("No path found between '{source}' and '{target}'");
                }
            }
        }
        _ => {
            println!("Could not find nodes matching '{source}' and/or '{target}'");
        }
    }
}

fn cmd_explain(node_label: &str, graph_path: &std::path::Path) {
    let graph = match graphify::serve::load_graph_smart(graph_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to load graph: {e}");
            return;
        }
    };

    let matches = graphify::serve::score_nodes(&graph, &[node_label.to_lowercase()]);
    if let Some((_, node_id)) = matches.first() {
        if let Some(node) = graph.get_node(node_id) {
            println!("Node: {}", node.label);
            println!("File: {}", node.source_file);
            println!("Type: {}", node.file_type);
            if let Some(loc) = &node.source_location {
                println!("Location: {loc}");
            }
            println!("Degree: {}", graph.degree(node_id));

            let neighbors = graph.neighbors(node_id);
            if !neighbors.is_empty() {
                println!("\nNeighbors ({}):", neighbors.len());
                for nid in &neighbors {
                    if let Some(n) = graph.get_node(nid) {
                        let edge_info = graph
                            .edges()
                            .find(|(s, t, _)| {
                                (*s == *node_id && *t == *nid) || (*t == *node_id && *s == *nid)
                            })
                            .map(|(_, _, e)| format!(" [{}] {}", e.confidence, e.relation))
                            .unwrap_or_default();
                        println!("  {} ({}){edge_info}", n.label, n.source_file);
                    }
                }
            }
        }
    } else {
        println!("No node found matching '{node_label}'");
    }
}

fn cmd_benchmark(graph_path: &std::path::Path) {
    let graph = match graphify::serve::load_graph_smart(graph_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to load graph: {e}");
            return;
        }
    };

    let result = graphify::benchmark::run_benchmark(&graph, None, None);
    graphify::benchmark::print_benchmark(&result);
}

fn cmd_update(path: &std::path::Path) {
    println!("graphify update: re-extracting code files in {}", path.display());

    let detection = graphify::detect::detect(path, false);
    let code_paths: Vec<PathBuf> = detection.files.code.iter().map(PathBuf::from).collect();

    if code_paths.is_empty() {
        println!("  no code files found");
        return;
    }

    #[cfg(feature = "extract")]
    {
        println!("  extracting {} code files...", code_paths.len());
        let extraction = graphify::extract::extract(&code_paths, Some(path));

        let graph = graphify::graph::build_from_extraction(&extraction);
        let communities = graphify::cluster::cluster(&graph);
        let community_labels: std::collections::HashMap<usize, String> = communities
            .iter()
            .map(|(&cid, nodes)| {
                let label = nodes.first()
                    .and_then(|nid| graph.get_node(nid))
                    .map(|n| n.label.clone())
                    .unwrap_or_else(|| format!("Community {cid}"));
                (cid, label)
            })
            .collect();

        let out_dir = path.join("graphify-out");
        let _ = std::fs::create_dir_all(&out_dir);
        let _ = graphify::export::to_json(&graph, &communities, &out_dir.join("graph.json"));
        let _ = graphify::export::to_html(&graph, &communities, &out_dir.join("graph.html"), Some(&community_labels));

        println!("  updated: {} nodes, {} edges", graph.node_count(), graph.edge_count());
    }

    #[cfg(not(feature = "extract"))]
    println!("  extract feature not enabled");
}

fn cmd_cluster_only(path: &std::path::Path) {
    let graph_path = path.join("graphify-out/graph.json");
    let graph = match graphify::serve::load_graph_smart(&graph_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to load graph: {e}");
            return;
        }
    };

    let communities = graphify::cluster::cluster(&graph);
    let cohesion = graphify::cluster::score_all(&graph, &communities);

    println!("Re-clustered: {} communities", communities.len());
    for (cid, nodes) in &communities {
        let coh = cohesion.get(cid).copied().unwrap_or(0.0);
        println!("  Community {cid}: {} nodes (cohesion {coh:.2})", nodes.len());
    }

    // Re-export with new communities
    let community_labels: std::collections::HashMap<usize, String> = communities
        .iter()
        .map(|(&cid, nodes)| {
            let label = nodes.first()
                .and_then(|nid| graph.get_node(nid))
                .map(|n| n.label.clone())
                .unwrap_or_else(|| format!("Community {cid}"));
            (cid, label)
        })
        .collect();

    let out_dir = path.join("graphify-out");
    let _ = graphify::export::to_json(&graph, &communities, &out_dir.join("graph.json"));
    let _ = graphify::export::to_html(&graph, &communities, &out_dir.join("graph.html"), Some(&community_labels));
    println!("  re-exported graph.json and graph.html");
}

#[allow(unused_variables)]
fn cmd_add(url: &str, author: Option<&str>, contributor: Option<&str>, dir: &std::path::Path) {
    let url_type = graphify::ingest::detect_url_type(url);
    println!("graphify add: fetching {url} (type: {url_type})");

    #[cfg(feature = "fetch")]
    {
        match graphify::ingest::ingest(url, dir, author, contributor) {
            Ok(path) => println!("  saved to {}", path.display()),
            Err(e) => eprintln!("  fetch failed: {e}"),
        }
        return;
    }

    #[cfg(not(feature = "fetch"))]
    {
        // Fallback: save stub without fetching
        if let Err(e) = graphify::security::validate_url(url) {
            eprintln!("URL validation failed: {e}");
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let safe_name: String = url.chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(50)
            .collect();
        let filename = format!("{url_type}_{safe_name}_{now}.md");
        let out_path = dir.join(&filename);
        let content = format!("---\nsource_url: \"{url}\"\ntype: {url_type}\ncaptured_at: {now}\n---\n\n# {url}\n\nFetch feature not enabled. Rebuild with --features fetch.\n");
        match std::fs::write(&out_path, &content) {
            Ok(()) => println!("  saved stub to {}", out_path.display()),
            Err(e) => eprintln!("  failed to save: {e}"),
        }
    }
}

fn cmd_serve(graph_path: &std::path::Path) {
    let graph = match graphify::serve::load_graph_smart(graph_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Failed to load graph: {e}");
            return;
        }
    };

    println!(
        "MCP server ready: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );
    println!("Reading JSON-RPC from stdin...");

    // Simple JSON-RPC loop over stdin
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        match stdin.read_line(&mut line) {
            Ok(0) => break, // EOF
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

fn handle_jsonrpc(input: &str, graph: &graphify::graph::GraphifyGraph) -> String {
    let req: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => return format!(r#"{{"jsonrpc":"2.0","error":{{"code":-32700,"message":"Parse error: {e}"}},"id":null}}"#),
    };

    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(serde_json::json!({}));

    let result = match method {
        "query_graph" => {
            let question = params.get("question").and_then(|v| v.as_str()).unwrap_or("");
            let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let budget = params.get("token_budget").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;
            let terms: Vec<String> = question.split_whitespace().filter(|t| t.len() > 2).map(|t| t.to_lowercase()).collect();
            let scored = graphify::serve::score_nodes(graph, &terms);
            let start: Vec<String> = scored.into_iter().take(3).map(|(_, id)| id).collect();
            let (visited, edges) = graphify::serve::bfs(graph, &start, depth);
            serde_json::json!(graphify::serve::subgraph_to_text(graph, &visited, &edges, budget))
        }
        "get_node" => {
            let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let matches = graphify::serve::score_nodes(graph, &[label.to_lowercase()]);
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
            let gods = graphify::analyze::god_nodes(graph, top_n);
            let list: Vec<serde_json::Value> = gods.iter().map(|g| serde_json::json!({"label": g.label, "degree": g.degree, "source_file": g.source_file})).collect();
            serde_json::json!(list)
        }
        "graph_stats" => {
            let communities = graphify::serve::communities_from_graph(graph);
            serde_json::json!({
                "nodes": graph.node_count(),
                "edges": graph.edge_count(),
                "communities": communities.len(),
            })
        }
        "save_insight" => {
            let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let description = params.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let node_ids: Vec<String> = params.get("nodes")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let pattern = params.get("pattern").and_then(|v| v.as_str());
            let graph_path = std::path::Path::new("graphify-out/graph.json");
            match graphify::knowledge::save_insight(graph_path, None, label, description, &node_ids, pattern) {
                Ok(()) => serde_json::json!({"status": "saved", "label": label, "nodes": node_ids.len()}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "save_note" => {
            let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let related: Vec<String> = params.get("related_nodes")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let graph_path = std::path::Path::new("graphify-out/graph.json");
            match graphify::knowledge::save_note(graph_path, None, title, content, &related) {
                Ok(()) => serde_json::json!({"status": "saved", "title": title}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "add_edge" => {
            let source = params.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let target = params.get("target").and_then(|v| v.as_str()).unwrap_or("");
            let relation = params.get("relation").and_then(|v| v.as_str()).unwrap_or("related_to");
            let description = params.get("description").and_then(|v| v.as_str());
            let graph_path = std::path::Path::new("graphify-out/graph.json");
            match graphify::knowledge::add_edge(graph_path, source, target, relation, description) {
                Ok(()) => serde_json::json!({"status": "saved", "source": source, "target": target}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "learn" => {
            let knowledge_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("pattern");
            let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let description = params.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let related: Vec<String> = params.get("related_nodes")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let tags: Vec<String> = params.get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let kt = match knowledge_type {
                "decision" => graphify::learn::KnowledgeType::Decision,
                "convention" => graphify::learn::KnowledgeType::Convention,
                "coupling" => graphify::learn::KnowledgeType::Coupling,
                "preference" => graphify::learn::KnowledgeType::Preference,
                "bug_pattern" => graphify::learn::KnowledgeType::BugPattern,
                "domain" => graphify::learn::KnowledgeType::Domain,
                _ => graphify::learn::KnowledgeType::Pattern,
            };
            let vault = std::path::Path::new("graphify-out");
            let graph_path = std::path::Path::new("graphify-out/graph.json");
            match graphify::learn::learn(vault, Some(graph_path), kt, title, description, &related, &tags) {
                Ok(_) => serde_json::json!({"status": "learned", "title": title}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "recall" => {
            let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let type_filter = params.get("type").and_then(|v| v.as_str());
            let vault = std::path::Path::new("graphify-out");
            let results = graphify::learn::query_knowledge(vault, query, type_filter);
            let items: Vec<serde_json::Value> = results.iter().map(|k| {
                serde_json::json!({
                    "title": k.title,
                    "type": k.knowledge_type.to_string(),
                    "description": k.description.lines().next().unwrap_or(""),
                    "confidence": (k.confidence * 100.0) as u32,
                    "observations": k.observations,
                    "related_nodes": k.related_nodes,
                })
            }).collect();
            serde_json::json!(items)
        }
        "knowledge_context" => {
            let vault = std::path::Path::new("graphify-out");
            let max = params.get("max_items").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            serde_json::json!(graphify::learn::knowledge_context(vault, max))
        }
        _ => serde_json::json!({"error": format!("Unknown method: {method}")}),
    };

    format!(r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#,
        serde_json::to_string(&result).unwrap_or_default(),
        serde_json::to_string(&id).unwrap_or_default(),
    )
}

fn cmd_workspace(action: &str, vault_override: Option<&std::path::Path>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    match action {
        "init" => {
            match graphify::workspace::init(&cwd) {
                Ok(path) => println!("Created {}", path.display()),
                Err(e) => eprintln!("Error: {e}"),
            }
        }
        "run" => {
            let config_path = match graphify::workspace::find_config(&cwd) {
                Some(p) => p,
                None => {
                    eprintln!("No graphify-workspace.yaml found. Run `graphify workspace init` first.");
                    return;
                }
            };
            let config = match graphify::workspace::load_config(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Config error: {e}");
                    return;
                }
            };
            if let Err(e) = graphify::workspace::run(&config, vault_override) {
                eprintln!("Workspace error: {e}");
            }
        }
        _ => {
            println!("Usage: graphify workspace <init|run> [--vault <path>]");
        }
    }
}
