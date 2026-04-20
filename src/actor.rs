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

    let last_activity = std::sync::Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    let active_connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                *last_activity.lock().unwrap() = std::time::Instant::now();
                let la = last_activity.clone();
                let ac = active_connections.clone();
                ac.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                std::thread::spawn(move || {
                    handle_connection(stream);
                    ac.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    *la.lock().unwrap() = std::time::Instant::now();
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let idle = last_activity.lock().unwrap().elapsed().as_secs();
                let conns = active_connections.load(std::sync::atomic::Ordering::Relaxed);
                if conns == 0 && idle > IDLE_TIMEOUT_SECS {
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
    let _project_dir = params
        .get("project_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let h5_path = crate::registry::global_h5();
    // global h5 is the single source

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
            let knowledge_uuid = params.get("uuid").and_then(|v| v.as_str());
            let kt_str = params
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("pattern");
            let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let desc = params
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let related = extract_optional_string_array(&params, "related_nodes");
            let tags = extract_string_array(&params, "tags");
            let kt = parse_knowledge_type(kt_str);
            match crate::learn::learn_with_uuid(
                &h5_path,
                knowledge_uuid,
                kt,
                title,
                desc,
                related.as_deref(),
                &tags,
            ) {
                Ok(uuid) => {
                    serde_json::json!({"status": "learned", "uuid": uuid, "title": title})
                }
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "recall" => {
            let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let type_filter = params.get("type").and_then(|v| v.as_str());
            let results = crate::learn::query_knowledge(&h5_path, query, type_filter);
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|k| {
                    serde_json::json!({
                        "uuid": k.uuid, "title": k.title, "type": k.knowledge_type.to_string(),
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
            serde_json::json!(crate::learn::knowledge_context(&h5_path, max))
        }
        "forget" => {
            let title = params.get("title").and_then(|v| v.as_str());
            let ktype = params.get("type").and_then(|v| v.as_str());
            let project = params.get("project").and_then(|v| v.as_str());
            let below = params.get("below_confidence").and_then(|v| v.as_f64());
            match crate::storage::forget_knowledge(&h5_path, title, ktype, project, below) {
                Ok(n) => serde_json::json!({"status": "forgot", "removed": n}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "save_insight" => {
            let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let desc = params
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let nodes = extract_string_array(&params, "nodes");
            let pattern = params.get("pattern").and_then(|v| v.as_str());
            match crate::knowledge::save_insight(&h5_path, None, label, desc, &nodes, pattern) {
                Ok(()) => serde_json::json!({"status": "saved", "label": label}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "save_note" => {
            let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let related = extract_string_array(&params, "related_nodes");
            match crate::knowledge::save_note(&h5_path, None, title, content, &related) {
                Ok(()) => serde_json::json!({"status": "saved", "title": title}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "add_edge" => {
            let source = params.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let target = params.get("target").and_then(|v| v.as_str()).unwrap_or("");
            let relation = params
                .get("relation")
                .and_then(|v| v.as_str())
                .unwrap_or("related_to");
            let desc = params.get("description").and_then(|v| v.as_str());
            match crate::knowledge::add_edge(&h5_path, source, target, relation, desc) {
                Ok(()) => serde_json::json!({"status": "saved"}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "recall_for_task" => {
            let question = params.get("question").and_then(|v| v.as_str()).unwrap_or("");
            let touched_files = extract_string_array(&params, "touched_files");
            let node_uuids = extract_string_array(&params, "node_uuids");
            let max = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(5) as usize;
            let results = crate::learn::recall_for_task(
                &h5_path,
                question,
                &touched_files,
                &node_uuids,
                max,
            );
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|k| {
                    serde_json::json!({
                        "uuid": k.uuid, "title": k.title,
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
        "get_task_context" => {
            let question = params.get("question").and_then(|v| v.as_str()).unwrap_or("");
            let touched_files = extract_string_array(&params, "touched_files");
            let max = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            serde_json::json!(crate::learn::get_task_context(
                &h5_path,
                question,
                &touched_files,
                max,
            ))
        }
        "update_knowledge" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            let updates = crate::learn::KnowledgeUpdates {
                status: params.get("status").and_then(|v| v.as_str()).map(String::from),
                scope: params.get("scope").and_then(|v| v.as_str()).map(String::from),
                applies_when: params.get("applies_when").and_then(|v| v.as_str()).map(String::from),
                superseded_by: params.get("superseded_by").and_then(|v| v.as_str()).map(String::from),
                validate: params.get("validate").and_then(|v| v.as_bool()).unwrap_or(false),
            };
            match crate::learn::update_knowledge(&h5_path, uuid, &updates) {
                Ok(()) => serde_json::json!({"status": "updated", "uuid": uuid}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "link_knowledge_to_nodes" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            let node_uuids = extract_string_array(&params, "node_uuids");
            let relation = params.get("relation").and_then(|v| v.as_str()).unwrap_or("related_to");
            match crate::learn::link_knowledge_to_nodes(&h5_path, uuid, &node_uuids, relation) {
                Ok(()) => serde_json::json!({"status": "linked", "uuid": uuid, "nodes": node_uuids.len()}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "clear_knowledge_links" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            match crate::learn::clear_knowledge_links(&h5_path, uuid) {
                Ok(n) => serde_json::json!({"status": "cleared", "uuid": uuid, "removed": n}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "detect_stale" => {
            match crate::learn::detect_stale_knowledge(&h5_path) {
                Ok(n) => serde_json::json!({"status": "checked", "stale_count": n}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
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

/// Like extract_string_array but returns None when the key is absent.
/// This lets callers distinguish "key not provided" from "key = []".
fn extract_optional_string_array(params: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    params.get(key).and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    })
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
