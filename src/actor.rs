//! kodex actor — single daemon process that owns all HDF5 files.
//!
//! Listens on ~/.kodex/kodex.sock (Unix) or localhost:19850 (fallback).
//! All kodex serve instances connect here as proxies.
//! Auto-exits after idle timeout (no connections for 5 minutes).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

const IDLE_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// Socket path for the actor.
pub fn socket_path() -> PathBuf {
    crate::registry::kodex_home().join("kodex.sock")
}

/// PID file to track running actor.
fn pid_path() -> PathBuf {
    crate::registry::kodex_home().join("kodex.pid")
}

/// Check if actor is already running.
pub fn is_running() -> bool {
    let sock = socket_path();
    if !sock.exists() {
        return false;
    }
    // Try connecting to verify it's alive
    std::os::unix::net::UnixStream::connect(&sock).is_ok()
}

/// Start actor in background (called by `kodex serve` if not running).
pub fn ensure_running() -> crate::error::Result<()> {
    if is_running() {
        return Ok(());
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kodex"));

    let _ = std::fs::create_dir_all(crate::registry::kodex_home());

    // Spawn detached process
    let child = std::process::Command::new(exe)
        .arg("actor")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| crate::error::KodexError::Other(format!("Failed to start actor: {e}")))?;

    // Write PID
    let _ = std::fs::write(pid_path(), child.id().to_string());

    // Wait briefly for socket to appear
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if socket_path().exists() {
            return Ok(());
        }
    }

    Ok(()) // may not be ready yet, serve will retry
}

/// Run the actor (foreground, called by `kodex actor` command).
pub fn run_actor() {
    let sock_path = socket_path();
    let _ = std::fs::remove_file(&sock_path); // clean stale socket
    let _ = std::fs::create_dir_all(crate::registry::kodex_home());

    let listener = match std::os::unix::net::UnixListener::bind(&sock_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("actor: failed to bind {}: {e}", sock_path.display());
            return;
        }
    };

    // Non-blocking for idle timeout
    listener
        .set_nonblocking(true)
        .expect("failed to set non-blocking");

    let mut last_activity = std::time::Instant::now();

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                last_activity = std::time::Instant::now();
                // Handle in current thread (sequential — ensures h5 safety)
                handle_connection(stream);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Check idle timeout
                if last_activity.elapsed().as_secs() > IDLE_TIMEOUT_SECS {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("actor: accept error: {e}");
                break;
            }
        }
    }

    // Cleanup
    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(pid_path());
}

/// Handle a single client connection (one JSON-RPC request per line).
fn handle_connection(stream: std::os::unix::net::UnixStream) {
    let reader_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let reader = BufReader::new(reader_stream);
    let mut writer = stream;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = process_request(trimmed);
        if writeln!(writer, "{response}").is_err() {
            break;
        }
        let _ = writer.flush();
    }
}

/// Process a JSON-RPC request — same logic as serve, but with h5 access.
fn process_request(input: &str) -> String {
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

    // Resolve project h5 path from params or CWD
    let project_dir = params
        .get("project_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let h5_path = std::path::Path::new(project_dir).join("kodex-out/kodex.h5");
    let ws_h5 = crate::registry::workspace_h5();

    let result = match method {
        "query_graph" => {
            let graph = match crate::serve::load_graph_smart(&h5_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
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
            let scored = crate::serve::score_nodes(&graph, &terms);
            let start: Vec<String> = scored.into_iter().take(3).map(|(_, id)| id).collect();
            let (visited, edges) = crate::serve::bfs(&graph, &start, depth);
            serde_json::json!(crate::serve::subgraph_to_text(
                &graph, &visited, &edges, budget
            ))
        }
        "get_node" => {
            let graph = match crate::serve::load_graph_smart(&h5_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let matches = crate::serve::score_nodes(&graph, &[label.to_lowercase()]);
            match matches.first() {
                Some((_, nid)) => {
                    serde_json::json!({"node": graph.get_node(nid), "degree": graph.degree(nid)})
                }
                None => serde_json::json!({"error": "not found"}),
            }
        }
        "god_nodes" => {
            let graph = match crate::serve::load_graph_smart(&h5_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let top_n = params.get("top_n").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let gods = crate::analyze::god_nodes(&graph, top_n);
            let list: Vec<serde_json::Value> = gods.iter().map(|g| {
                serde_json::json!({"label": g.label, "degree": g.degree, "source_file": g.source_file})
            }).collect();
            serde_json::json!(list)
        }
        "graph_stats" => {
            let graph = match crate::serve::load_graph_smart(&h5_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let communities = crate::serve::communities_from_graph(&graph);
            serde_json::json!({"nodes": graph.node_count(), "edges": graph.edge_count(), "communities": communities.len()})
        }
        "learn" => {
            let kt_str = params
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("pattern");
            let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let desc = params
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let related = extract_string_array(&params, "related_nodes");
            let tags = extract_string_array(&params, "tags");
            let kt = parse_knowledge_type(kt_str);
            match crate::learn::learn(&h5_path, kt, title, desc, &related, &tags) {
                Ok(()) => {
                    // Sync to workspace
                    let _ = crate::registry::register(std::path::Path::new(project_dir));
                    serde_json::json!({"status": "learned", "title": title})
                }
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "recall" => {
            let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let type_filter = params.get("type").and_then(|v| v.as_str());
            let mut results = crate::learn::query_knowledge(&h5_path, query, type_filter);
            if ws_h5.exists() {
                for r in crate::learn::query_knowledge(&ws_h5, query, type_filter) {
                    if !results.iter().any(|existing| existing.title == r.title) {
                        results.push(r);
                    }
                }
            }
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|k| {
                    serde_json::json!({
                        "title": k.title, "type": k.knowledge_type.to_string(),
                        "description": k.description.lines().next().unwrap_or(""),
                        "confidence": (k.confidence * 100.0) as u32,
                        "observations": k.observations, "related_nodes": k.related_nodes,
                    })
                })
                .collect();
            serde_json::json!(items)
        }
        "knowledge_context" => {
            let max = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(20) as usize;
            let local = crate::learn::knowledge_context(&h5_path, max);
            let global = if ws_h5.exists() {
                crate::learn::knowledge_context(&ws_h5, max)
            } else {
                String::new()
            };
            if global.is_empty() {
                serde_json::json!(local)
            } else {
                serde_json::json!(format!("{local}\n\n## Global Knowledge\n\n{global}"))
            }
        }
        _ => serde_json::json!({"error": format!("Unknown method: {method}")}),
    };

    format!(
        r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#,
        serde_json::to_string(&result).unwrap_or_default(),
        serde_json::to_string(&id).unwrap_or_default(),
    )
}

fn error_response(id: &serde_json::Value, msg: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","error":{{"code":-32000,"message":"{msg}"}},"id":{}}}"#,
        serde_json::to_string(id).unwrap_or_default(),
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

fn parse_knowledge_type(s: &str) -> crate::learn::KnowledgeType {
    match s {
        "architecture" => crate::learn::KnowledgeType::Architecture,
        "pattern" => crate::learn::KnowledgeType::Pattern,
        "decision" => crate::learn::KnowledgeType::Decision,
        "convention" => crate::learn::KnowledgeType::Convention,
        "coupling" => crate::learn::KnowledgeType::Coupling,
        "domain" => crate::learn::KnowledgeType::Domain,
        "preference" => crate::learn::KnowledgeType::Preference,
        "bug_pattern" => crate::learn::KnowledgeType::BugPattern,
        "tech_debt" => crate::learn::KnowledgeType::TechDebt,
        "ops" => crate::learn::KnowledgeType::Ops,
        "api" => crate::learn::KnowledgeType::Api,
        "performance" => crate::learn::KnowledgeType::Performance,
        "roadmap" => crate::learn::KnowledgeType::Roadmap,
        "context" => crate::learn::KnowledgeType::Context,
        "lesson" => crate::learn::KnowledgeType::Lesson,
        other => crate::learn::KnowledgeType::Custom(other.to_string()),
    }
}
