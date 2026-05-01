//! kodex actor — single daemon process that owns all SQLite files.
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

    // Record the binary's modification time at startup so we can detect
    // when `cargo install` replaces the executable while we are running.
    let exe_path = std::env::current_exe().ok();
    let start_mtime = exe_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok());

    let last_activity = std::sync::Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    let active_connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted streams must be blocking for BufReader::lines()
                stream.set_nonblocking(false).ok();
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
                // Exit if the binary on disk was replaced (cargo install).
                // The next ensure_running() call will spawn a fresh actor.
                if let (Some(path), Some(mtime)) = (exe_path.as_ref(), start_mtime) {
                    let current = std::fs::metadata(path)
                        .ok()
                        .and_then(|m| m.modified().ok());
                    if current != Some(mtime) {
                        break;
                    }
                }
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

        let owned = trimmed.to_string();
        let response = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process_request(&owned)
        })) {
            Ok(r) => r,
            Err(_) => {
                let id = serde_json::from_str::<serde_json::Value>(trimmed)
                    .ok()
                    .and_then(|r| r.get("id").cloned())
                    .unwrap_or(serde_json::Value::Null);
                format!(
                    r#"{{"jsonrpc":"2.0","error":{{"code":-32000,"message":"Internal error (panic caught)"}},"id":{}}}"#,
                    serde_json::to_string(&id).unwrap_or_default(),
                )
            }
        };
        if writeln!(writer, "{response}").is_err() {
            break;
        }
        let _ = writer.flush();
    }
}

/// Process a JSON-RPC request — same logic as serve, but with db access.
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

    // Resolve project db path from params or CWD
    let project_dir = params
        .get("project_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let db_path = crate::registry::global_db();
    // global db is the single source

    let result = match method {
        "query_graph" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
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
            let format = params
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("text");

            let source_pattern = params
                .get("source_pattern")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let community = params
                .get("community")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            // exclude_hubs: bool (default threshold 50) OR explicit numeric threshold
            let hub_threshold = match params.get("exclude_hubs") {
                Some(serde_json::Value::Bool(true)) => Some(50usize),
                Some(serde_json::Value::Bool(false)) | None => None,
                Some(v) => v.as_u64().map(|n| n as usize),
            };
            let filter = crate::serve::TraversalFilter {
                source_pattern,
                community,
                hub_threshold,
            };

            let terms: Vec<String> = question
                .split_whitespace()
                .filter(|t| t.len() > 2)
                .map(|t| t.to_lowercase())
                .collect();
            let scored = crate::serve::score_nodes_filtered(&graph, &terms, &filter);
            let scored_count = scored.len();
            let mut start: Vec<String> =
                scored.into_iter().take(3).map(|(_, id)| id).collect();
            let mut used_fallback = false;
            // Fallback: vague question + precise filter (e.g. source_pattern) often
            // produces zero fuzzy hits because no labels in the scoped subset
            // match the question terms. Seed with high-degree nodes in scope so
            // the caller still sees something useful.
            if start.is_empty() && filter.is_active() {
                start = crate::serve::top_degree_in_filter(&graph, &filter, 3);
                used_fallback = !start.is_empty();
            }
            let (visited, edges) = crate::serve::bfs_filtered(&graph, &start, depth, &filter);
            // `format=json` returns a structured object so the caller can
            // iterate nodes/edges programmatically; `mermaid` and `text`
            // (default) keep their string-return shape for backward compat.
            if format == "json" {
                let mut payload = crate::serve::subgraph_to_json(&graph, &visited, &edges);
                if let Some(stale) = staleness_warning(project_dir) {
                    payload["stale"] = stale;
                }
                return format!(
                    r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#,
                    serde_json::to_string(&payload).unwrap_or_default(),
                    serde_json::to_string(&id).unwrap_or_default(),
                );
            }
            let mut rendered = match format {
                "mermaid" => crate::serve::subgraph_to_mermaid(&graph, &visited, &edges),
                _ => crate::serve::subgraph_to_text(&graph, &visited, &edges, budget),
            };
            // Surface staleness inline since query_graph returns a plain
            // string — agents reading the output will see the warning even
            // though older clients still parse the body unchanged.
            if let Some(stale) = staleness_warning(project_dir) {
                let hint = stale.get("hint").and_then(|v| v.as_str()).unwrap_or("");
                rendered = format!("[STALE: {hint}]\n{rendered}");
            }
            // Empty-result diagnostics: tell the caller why nothing came back so
            // they know whether to broaden the question, drop a filter, or raise
            // depth. Backward-compatible: non-empty returns the rendered string
            // unchanged.
            if rendered.trim().is_empty() {
                let mut reasons: Vec<String> = Vec::new();
                if terms.is_empty() {
                    reasons.push(
                        "question has no usable terms (>2 chars after split)".into(),
                    );
                } else if scored_count == 0 {
                    reasons.push(format!(
                        "no fuzzy hit for terms={terms:?} within filter (source_pattern={:?}, community={:?})",
                        filter.source_pattern, filter.community
                    ));
                }
                if !start.is_empty() && visited.len() <= start.len() {
                    reasons.push(format!(
                        "BFS expanded 0 from {} seed(s) at depth={} (try higher depth, or hub_threshold may have stopped expansion)",
                        start.len(),
                        depth
                    ));
                }
                if start.is_empty() && !filter.is_active() {
                    reasons.push("no seeds and no filter to fall back on".into());
                }
                let why = if reasons.is_empty() {
                    "empty result (no diagnostic available)".to_string()
                } else {
                    reasons.join("; ")
                };
                let fallback_note = if used_fallback {
                    " [fallback: seeded with top-degree filter-passing nodes]"
                } else {
                    ""
                };
                serde_json::json!(format!("(empty){fallback_note}: {why}"))
            } else {
                serde_json::json!(rendered)
            }
        }
        "get_node" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let top_n = params
                .get("top_n")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(3);
            let expand = params
                .get("expand")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let expand_top_n = params
                .get("expand_top_n")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(20);
            let source_pattern = params
                .get("source_pattern")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let filter = crate::serve::TraversalFilter {
                source_pattern,
                ..Default::default()
            };
            let matches = crate::serve::score_nodes_filtered(
                &graph,
                &[label.to_lowercase()],
                &filter,
            );
            if matches.is_empty() {
                serde_json::json!({"error": "not found", "candidates": []})
            } else {
                let top_matches: Vec<(usize, String)> =
                    matches.into_iter().take(top_n.max(1)).collect();

                // When expanding, the candidate with the highest fuzzy score
                // isn't always the one with the API surface — `SharedPV::Impl`
                // (single-class, deg 1) and `sharedpv` (file hub, deg 21) tie
                // on label match but only the file hub `contains` the methods.
                // Pick the candidate with the most matching outgoing edges.
                let expand_target: Option<String> = expand.as_deref().and_then(|rel| {
                    top_matches
                        .iter()
                        .map(|(_, id)| {
                            let count = graph
                                .edges()
                                .filter(|(s, _, e)| *s == id.as_str() && e.relation == rel)
                                .count();
                            (count, id.clone())
                        })
                        .max_by_key(|(c, _)| *c)
                        .filter(|(c, _)| *c > 0)
                        .map(|(_, id)| id)
                });

                let candidates: Vec<serde_json::Value> = top_matches
                    .iter()
                    .filter_map(|(score, nid)| {
                        let node = graph.get_node(nid)?;
                        let indices =
                            crate::serve::label_match_indices(&node.label, label);
                        let highlight = highlight_label(&node.label, &indices);
                        Some(serde_json::json!({
                            "score": score,
                            "id": nid,
                            "label": node.label,
                            "highlight": highlight,
                            "match_indices": indices,
                            "source_file": node.source_file,
                            "source_location": node.source_location,
                            "community": node.community,
                            "degree": graph.degree(nid),
                        }))
                    })
                    .collect();

                let mut response = serde_json::json!({ "candidates": candidates });
                if let (Some(rel), Some(tid)) = (expand.as_deref(), expand_target.as_deref()) {
                    let mut members: Vec<(usize, serde_json::Value)> = graph
                        .edges()
                        .filter(|(s, _, e)| *s == tid && e.relation == rel)
                        .filter_map(|(_, t, _)| {
                            let n = graph.get_node(t)?;
                            let deg = graph.degree(t);
                            Some((
                                deg,
                                serde_json::json!({
                                    "label": n.label,
                                    "source_file": n.source_file,
                                    "source_location": n.source_location,
                                    "degree": deg,
                                }),
                            ))
                        })
                        .collect();
                    members.sort_by(|a, b| b.0.cmp(&a.0));
                    let total = members.len();
                    let trimmed: Vec<serde_json::Value> = members
                        .into_iter()
                        .take(expand_top_n)
                        .map(|(_, v)| v)
                        .collect();
                    response["members"] = serde_json::json!(trimmed);
                    response["members_total"] = serde_json::json!(total);
                    response["members_relation"] = serde_json::json!(rel);
                    response["members_source"] = serde_json::json!(tid);
                }
                if let Some(stale) = staleness_warning(project_dir) {
                    response["stale"] = stale;
                }
                response
            }
        }
        "find_callers" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let top_n = params.get("top_n").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            call_direction_handler(&graph, &params, top_n, crate::serve::find_callers, "callers")
        }
        "find_callees" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let top_n = params.get("top_n").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            call_direction_handler(&graph, &params, top_n, crate::serve::find_callees, "callees")
        }
        "trace_call_path" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let from_label = params.get("from").and_then(|v| v.as_str()).unwrap_or("");
            let to_label = params.get("to").and_then(|v| v.as_str()).unwrap_or("");
            let max_depth =
                params.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(8) as usize;

            let from_ids = resolve_call_seeds(&graph, from_label, 3);
            let to_ids = resolve_call_seeds(&graph, to_label, 3);

            if from_ids.is_empty() {
                return error_response(
                    &id,
                    &format!("no match for from label: {from_label}"),
                );
            }
            if to_ids.is_empty() {
                return error_response(
                    &id,
                    &format!("no match for to label: {to_label}"),
                );
            }

            let raw_paths =
                crate::serve::trace_call_path(&graph, &from_ids, &to_ids, max_depth);

            let paths: Vec<serde_json::Value> = raw_paths
                .iter()
                .take(20)
                .map(|path| {
                    let steps: Vec<serde_json::Value> = path
                        .iter()
                        .map(|nid| {
                            if let Some(node) = graph.get_node(nid) {
                                serde_json::json!({
                                    "label": node.label,
                                    "source_file": node.source_file,
                                    "source_location": node.source_location,
                                })
                            } else {
                                serde_json::json!({"id": nid})
                            }
                        })
                        .collect();
                    let chain: Vec<&str> = path
                        .iter()
                        .map(|nid| {
                            graph
                                .get_node(nid)
                                .map(|n| n.label.as_str())
                                .unwrap_or(nid.as_str())
                        })
                        .collect();
                    serde_json::json!({
                        "chain": chain.join(" → "),
                        "length": path.len() - 1,
                        "steps": steps,
                    })
                })
                .collect();

            serde_json::json!({
                "from": from_label,
                "to": to_label,
                "paths_found": paths.len(),
                "paths": paths,
            })
        }
        "detect_cycles" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let source_pattern = params
                .get("source_pattern")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let default_relations = vec!["calls".to_string(), "imports".to_string()];
            let mut relations: Vec<String> = params
                .get("relations")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if relations.is_empty() {
                relations = default_relations;
            }
            let relation_strs: Vec<&str> = relations.iter().map(String::as_str).collect();

            let cycles =
                crate::serve::detect_cycles_in_graph(&graph, &relation_strs, source_pattern);
            let cycle_objs: Vec<serde_json::Value> = cycles
                .iter()
                .map(|group| {
                    let nodes: Vec<serde_json::Value> = group
                        .iter()
                        .map(|nid| {
                            if let Some(node) = graph.get_node(nid) {
                                serde_json::json!({
                                    "label": node.label,
                                    "source_file": node.source_file,
                                })
                            } else {
                                serde_json::json!({"id": nid})
                            }
                        })
                        .collect();
                    let labels: Vec<&str> = group
                        .iter()
                        .map(|nid| {
                            graph
                                .get_node(nid)
                                .map(|n| n.label.as_str())
                                .unwrap_or(nid.as_str())
                        })
                        .collect();
                    serde_json::json!({
                        "size": group.len(),
                        "summary": labels.join(" ↔ "),
                        "nodes": nodes,
                    })
                })
                .collect();
            serde_json::json!({
                "cycles_found": cycle_objs.len(),
                "relations": relations,
                "cycles": cycle_objs,
            })
        }
        "god_nodes" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let top_n = params.get("top_n").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let filter = crate::analyze::GodNodesFilter {
                pattern: params
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                source_pattern: params
                    .get("source_pattern")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                min_degree: params
                    .get("min_degree")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize),
            };
            let gods = crate::analyze::god_nodes_filtered(&graph, top_n, &filter);
            let list: Vec<serde_json::Value> = gods.iter().map(|g| {
                serde_json::json!({"label": g.label, "degree": g.degree, "source_file": g.source_file})
            }).collect();
            serde_json::json!(list)
        }
        "compare_graphs" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let left = params
                .get("left_pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let right = params
                .get("right_pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if left.is_empty() || right.is_empty() {
                return error_response(
                    &id,
                    "compare_graphs requires non-empty left_pattern and right_pattern",
                );
            }
            let file_type = params
                .get("file_type")
                .and_then(|v| v.as_str())
                .and_then(crate::types::FileType::from_str_loose);
            let min_norm_len = params
                .get("min_norm_len")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(3);
            let top_n = params
                .get("top_n")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(200);
            let label_pattern = params
                .get("pattern")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let min_degree = params
                .get("min_degree")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);
            let skip_file_nodes = params
                .get("skip_file_nodes")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let public_pattern = params
                .get("public_pattern")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let public_only = params
                .get("public_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let internal_weight = params
                .get("internal_weight")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32)
                .unwrap_or(0.0);
            let semantic_threshold = params
                .get("semantic_threshold")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32)
                .unwrap_or(0.0);
            let semantic_top_per_gap = params
                .get("semantic_top_per_gap")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(3);
            let compose_priority = params
                .get("compose_priority")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let q = crate::analyze::CompareQuery {
                left_pattern: left.to_string(),
                right_pattern: right.to_string(),
                file_type,
                min_norm_len,
                top_n,
                label_pattern,
                min_degree,
                skip_file_nodes,
                public_pattern,
                public_only,
                internal_weight,
                semantic_threshold,
                semantic_top_per_gap,
                compose_priority,
            };
            let gaps = crate::analyze::compare_repos(&graph, &q);
            let with_signature = params
                .get("with_signature")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let signature_lines_above = params
                .get("signature_lines_above")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(2);
            let signature_lines_below = params
                .get("signature_lines_below")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);
            let signature_max_top = params
                .get("signature_max_top")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(20);
            // Inline a few lines around each gap's source_location so callers
            // don't have to re-grep the upstream to verify whether a gap is
            // truly missing or just renamed/reshaped. Only enrich the top-K
            // gaps to keep the response size bounded.
            let enriched: Vec<serde_json::Value> = if with_signature {
                gaps.iter()
                    .enumerate()
                    .map(|(i, g)| {
                        let mut v = serde_json::to_value(g).unwrap_or(serde_json::Value::Null);
                        if i < signature_max_top {
                            if let Some(snippet) = crate::source_lookup::snippet_for(
                                &g.source_file,
                                g.source_location.as_deref(),
                                signature_lines_above,
                                signature_lines_below,
                            ) {
                                v["signature"] = serde_json::json!(snippet);
                            }
                        }
                        v
                    })
                    .collect()
            } else {
                gaps.iter()
                    .map(|g| serde_json::to_value(g).unwrap_or(serde_json::Value::Null))
                    .collect()
            };

            // Optional embedding-based semantic pass — cosine similarity
            // over precomputed sentence vectors. Requires `kodex embed` to
            // have been run, plus building with --features embeddings.
            let semantic_embedding = params
                .get("semantic_embedding")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let _embedding_threshold = params
                .get("embedding_threshold")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32)
                .unwrap_or(0.65);
            let _embedding_top_per_gap = params
                .get("embedding_top_per_gap")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(3);
            let final_enriched: Vec<serde_json::Value> = {
                #[cfg(feature = "embeddings")]
                {
                    if semantic_embedding {
                        match semantic_embedding_pass(
                            &graph,
                            enriched,
                            &right.to_lowercase(),
                            &db_path,
                            _embedding_threshold,
                            _embedding_top_per_gap,
                        ) {
                            Ok(v) => v,
                            Err(e) => {
                                return error_response(
                                    &id,
                                    &format!("semantic_embedding pass failed: {e}"),
                                );
                            }
                        }
                    } else {
                        enriched
                    }
                }
                #[cfg(not(feature = "embeddings"))]
                {
                    if semantic_embedding {
                        return error_response(
                            &id,
                            "semantic_embedding=true requires kodex built with --features embeddings",
                        );
                    }
                    enriched
                }
            };

            let mut response = serde_json::json!({
                "gaps": final_enriched,
                "count": gaps.len(),
            });
            if let Some(stale) = staleness_warning(project_dir) {
                response["stale"] = stale;
            }
            response
        }
        "semantic_search" => {
            #[cfg(not(feature = "embeddings"))]
            {
                let _ = params;
                return error_response(
                    &id,
                    "semantic_search requires kodex built with --features embeddings (and `kodex embed` to have been run)",
                );
            }
            #[cfg(feature = "embeddings")]
            {
                match semantic_search_handler(&params, &db_path) {
                    Ok(v) => v,
                    Err(e) => return error_response(&id, &e),
                }
            }
        }
        "analyze_change" => {
            // Orchestrator: combines recall_for_diff (knowledge memories) +
            // co_changes (architectural blast radius) into one call. Saves
            // the agent from making N+1 round-trips when verifying a diff.
            let max_items = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            let auto = params
                .get("auto")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let base_ref = params
                .get("base_ref")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("HEAD");
            let cochange_top_n = params
                .get("co_change_top_n")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(8);
            let cochange_commit_limit = params
                .get("co_change_commit_limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(200);
            let cochange_max_files = params
                .get("co_change_max_files")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(5);

            let auto_diff: Option<String> = if auto {
                run_git_diff(project_dir, base_ref).ok()
            } else {
                None
            };
            let supplied = params.get("diff").and_then(|v| v.as_str()).unwrap_or("");
            let diff_text: &str = auto_diff
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(supplied);
            if diff_text.trim().is_empty() {
                return error_response(
                    &id,
                    "analyze_change: empty diff (set auto=true or supply `diff`)",
                );
            }

            let (analysis, results) =
                crate::learn::recall_for_diff(&db_path, diff_text, max_items);
            let uuids: Vec<String> =
                results.iter().map(|r| r.knowledge.uuid.clone()).collect();
            let _ = crate::storage::bump_fetch_counters(&db_path, &uuids);
            let knowledge: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "knowledge": r.knowledge,
                        "score": r.score,
                    })
                })
                .collect();

            // Per-file co_changes — capped to keep the response bounded.
            let repo_dir =
                crate::registry::entry_for_dir(std::path::Path::new(project_dir))
                    .map(|e| e.path)
                    .unwrap_or_else(|| std::path::PathBuf::from(project_dir));
            let mut co_change_groups: Vec<serde_json::Value> = Vec::new();
            for file in analysis.changed_files.iter().take(cochange_max_files) {
                let q = crate::analyze::CoChangeQuery {
                    file: file.clone(),
                    commit_limit: cochange_commit_limit,
                    top_n: cochange_top_n,
                    min_weight: 0.0,
                };
                if let Ok(r) = crate::analyze::co_changes(&repo_dir, &q) {
                    if !r.co_changes.is_empty() {
                        co_change_groups.push(serde_json::json!({
                            "file": file,
                            "target_commits": r.target_commits,
                            "co_changes": r.co_changes,
                        }));
                    }
                }
            }

            let mut response = serde_json::json!({
                "diff_summary": {
                    "changed_files": analysis.changed_files,
                    "changed_node_uuids": analysis.changed_node_uuids,
                },
                "knowledge": knowledge,
                "co_changes": co_change_groups,
            });
            if auto {
                response["diff_source"] = serde_json::json!(if auto_diff.is_some() {
                    format!("git diff {base_ref}")
                } else {
                    "git failed; used supplied diff".to_string()
                });
            }
            if let Some(stale) = staleness_warning(project_dir) {
                response["stale"] = stale;
            }
            response
        }
        "detect_renames" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let data = match crate::storage::load(&db_path) {
                Ok(d) => d,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let titles: std::collections::HashMap<String, String> = data
                .knowledge
                .iter()
                .map(|k| (k.uuid.clone(), k.title.clone()))
                .collect();
            let q = crate::analyze::DetectQuery {
                top_n: params
                    .get("top_n")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(50),
                candidates_per_orphan: params
                    .get("candidates_per_orphan")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(3),
                min_confidence: params
                    .get("min_confidence")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32)
                    .unwrap_or(0.3),
                source_pattern: params
                    .get("source_pattern")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            };
            let orphans =
                crate::analyze::detect_renames(&graph, &data.links, &titles, &q);
            serde_json::json!({
                "orphans": orphans,
                "count": orphans.len(),
            })
        }
        "co_changes" => {
            let file = params
                .get("file")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if file.is_empty() {
                return error_response(&id, "co_changes: `file` is required");
            }
            let commit_limit = params
                .get("commit_limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(200);
            let top_n = params
                .get("top_n")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(20);
            let min_weight = params
                .get("min_weight")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32)
                .unwrap_or(0.0);
            let query = crate::analyze::CoChangeQuery {
                file: file.to_string(),
                commit_limit,
                top_n,
                min_weight,
            };
            // Resolve repo root from the project_dir the MCP layer injects;
            // fall back to the registry entry path so the caller can run from
            // a sub-directory.
            let repo_dir = crate::registry::entry_for_dir(std::path::Path::new(project_dir))
                .map(|e| e.path)
                .unwrap_or_else(|| std::path::PathBuf::from(project_dir));
            match crate::analyze::co_changes(&repo_dir, &query) {
                Ok(r) => serde_json::json!(r),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "list_communities" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
                Ok(g) => g,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let top_per_community = params
                .get("top_per_community")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(3);
            let min_size = params
                .get("min_size")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(3);
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(20);
            let mut summaries =
                crate::analyze::community_summaries(&graph, top_per_community, min_size);
            summaries.truncate(limit);
            serde_json::json!({
                "communities": summaries,
                "count": summaries.len(),
            })
        }
        "graph_stats" => {
            let graph = match crate::serve::load_graph_smart(&db_path) {
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
            let context_uuid = params.get("context_uuid").and_then(|v| v.as_str());
            let supersedes = params.get("supersedes").and_then(|v| v.as_str());
            let kt = parse_knowledge_type(kt_str);
            // `supersedes` is only meaningful when creating a new entry.
            let save_result = if knowledge_uuid.is_none() && supersedes.is_some_and(|s| !s.is_empty()) {
                crate::learn::learn_supersedes(
                    &db_path,
                    kt,
                    title,
                    desc,
                    related.as_deref(),
                    &tags,
                    supersedes.unwrap(),
                )
            } else {
                crate::learn::learn_with_uuid(
                    &db_path,
                    knowledge_uuid,
                    kt,
                    title,
                    desc,
                    related.as_deref(),
                    &tags,
                    context_uuid,
                )
            };
            match save_result {
                Ok(uuid) => {
                    // Provenance: capture cwd / git HEAD if not already set
                    if let Some(prov) =
                        crate::learn::auto_provenance(std::path::Path::new(project_dir))
                    {
                        let _ = crate::storage::set_evidence_if_empty(&db_path, &uuid, &prov);
                    }
                    let merge_candidates =
                        crate::learn::find_similar_to_uuid(&db_path, &uuid, 0.6);
                    let mut resp = serde_json::json!({
                        "status": "learned",
                        "uuid": uuid,
                        "title": title,
                    });
                    if !merge_candidates.is_empty() {
                        resp["merge_candidates"] = serde_json::json!(merge_candidates);
                        resp["hint"] = serde_json::json!(
                            "consider merge_knowledge(keep=<uuid>, absorb=<uuid>) to consolidate"
                        );
                    }
                    resp
                }
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "recall" => {
            let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let type_filter = params.get("type").and_then(|v| v.as_str());
            let results = crate::learn::query_knowledge(&db_path, query, type_filter);
            let uuids: Vec<String> = results.iter().map(|k| k.uuid.clone()).collect();
            let _ = crate::storage::bump_fetch_counters(&db_path, &uuids);
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
            let inline_top_k = params
                .get("inline_top_k")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            serde_json::json!(crate::learn::knowledge_context(
                &db_path,
                max,
                inline_top_k
            ))
        }
        "forget" => {
            let title = params.get("title").and_then(|v| v.as_str());
            let ktype = params.get("type").and_then(|v| v.as_str());
            let project = params.get("project").and_then(|v| v.as_str());
            let below = params.get("below_confidence").and_then(|v| v.as_f64());
            match crate::storage::forget_knowledge(&db_path, title, ktype, project, below) {
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
            match crate::knowledge::save_insight(&db_path, None, label, desc, &nodes, pattern) {
                Ok(()) => serde_json::json!({"status": "saved", "label": label}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "save_note" => {
            let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let related = extract_string_array(&params, "related_nodes");
            match crate::knowledge::save_note(&db_path, None, title, content, &related) {
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
            match crate::knowledge::add_edge(&db_path, source, target, relation, desc) {
                Ok(()) => serde_json::json!({"status": "saved"}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "recall_for_task" => {
            let question = params
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let touched_files = extract_string_array(&params, "touched_files");
            let node_uuids = extract_string_array(&params, "node_uuids");
            let max = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(5) as usize;
            let type_filter = params.get("type").and_then(|v| v.as_str());
            // Use structured recall so we can surface the score breakdown.
            let results = crate::learn::recall_for_task_structured(
                &db_path,
                question,
                &touched_files,
                &node_uuids,
                max,
                type_filter,
            );
            let uuids: Vec<String> =
                results.iter().map(|r| r.knowledge.uuid.clone()).collect();
            let _ = crate::storage::bump_fetch_counters(&db_path, &uuids);
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    let k = &r.knowledge;
                    serde_json::json!({
                        "uuid": k.uuid, "title": k.title,
                        "type": k.knowledge_type.to_string(),
                        "description": k.description.lines().next().unwrap_or(""),
                        "confidence": (k.confidence * 100.0) as u32,
                        "observations": k.observations,
                        "related_nodes": k.related_nodes,
                        "score_breakdown": {
                            "total": r.score.total,
                            "confidence": r.score.confidence,
                            "node_overlap": r.score.node_overlap,
                            "file_mention": r.score.file_mention,
                            "applies_when": r.score.applies_when,
                            "keyword_match": r.score.keyword_match,
                            "recency": r.score.recency,
                            "type_priority": r.score.type_priority,
                            "reasons": r.score.reasons,
                        },
                    })
                })
                .collect();
            serde_json::json!(items)
        }
        "get_task_context" => {
            let question = params
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let touched_files = extract_string_array(&params, "touched_files");
            let max = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            let format = params
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("markdown");
            let task_type = params
                .get("task_type")
                .and_then(|v| v.as_str())
                .unwrap_or("coding");
            if format == "json" {
                serde_json::json!(crate::learn::get_task_context_json(
                    &db_path,
                    question,
                    &touched_files,
                    max,
                    task_type,
                ))
            } else {
                serde_json::json!(crate::learn::get_task_context_md(
                    &db_path,
                    question,
                    &touched_files,
                    max,
                    task_type,
                ))
            }
        }
        "recall_for_task_structured" => {
            let question = params
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let touched_files = extract_string_array(&params, "touched_files");
            let node_uuids = extract_string_array(&params, "node_uuids");
            let max = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(5) as usize;
            let type_filter = params.get("type").and_then(|v| v.as_str());
            let results = crate::learn::recall_for_task_structured(
                &db_path,
                question,
                &touched_files,
                &node_uuids,
                max,
                type_filter,
            );
            serde_json::json!(results)
        }
        "validate_knowledge" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            let note = params.get("note").and_then(|v| v.as_str());
            match crate::learn::validate_knowledge(&db_path, uuid, note) {
                Ok(()) => serde_json::json!({"status": "validated", "uuid": uuid}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "mark_obsolete" => {
            // Accept either single `uuid` or array `uuids` for bulk operations.
            let uuids: Vec<String> = if let Some(arr) = params.get("uuids").and_then(|v| v.as_array()) {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            } else if let Some(u) = params.get("uuid").and_then(|v| v.as_str()) {
                vec![u.to_string()]
            } else {
                return error_response(&id, "uuid or uuids required");
            };
            if uuids.is_empty() {
                return error_response(&id, "uuid or uuids required");
            }
            let reason = params.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let mut succeeded = Vec::new();
            let mut errors = Vec::new();
            for u in &uuids {
                match crate::learn::mark_obsolete(&db_path, u, reason) {
                    Ok(()) => succeeded.push(u.clone()),
                    Err(e) => errors.push(serde_json::json!({"uuid": u, "error": e.to_string()})),
                }
            }
            serde_json::json!({
                "status": "obsoleted",
                "succeeded": succeeded,
                "errors": errors,
            })
        }
        "update_knowledge" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            let updates = crate::learn::KnowledgeUpdates {
                status: params
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                scope: params
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                applies_when: params
                    .get("applies_when")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                superseded_by: params
                    .get("superseded_by")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                validate: params
                    .get("validate")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            };
            match crate::learn::update_knowledge(&db_path, uuid, &updates) {
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
            let relation = params
                .get("relation")
                .and_then(|v| v.as_str())
                .unwrap_or("related_to");
            match crate::learn::link_knowledge_to_nodes(&db_path, uuid, &node_uuids, relation) {
                Ok(()) => {
                    serde_json::json!({"status": "linked", "uuid": uuid, "nodes": node_uuids.len()})
                }
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "clear_knowledge_links" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            match crate::learn::clear_knowledge_links(&db_path, uuid) {
                Ok(n) => serde_json::json!({"status": "cleared", "uuid": uuid, "removed": n}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "remove_link" => {
            let k_uuid = match params.get("knowledge_uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "knowledge_uuid required"),
            };
            let target = match params.get("target_uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "target_uuid required"),
            };
            let relation = params.get("relation").and_then(|v| v.as_str());
            match crate::learn::remove_link(&db_path, k_uuid, target, relation) {
                Ok(true) => serde_json::json!({"status": "removed"}),
                Ok(false) => serde_json::json!({"status": "not_found"}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "link_knowledge" => {
            let source = match params.get("source_uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "source_uuid required"),
            };
            let target = match params.get("target_uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "target_uuid required"),
            };
            let relation = params
                .get("relation")
                .and_then(|v| v.as_str())
                .unwrap_or("related_to");
            let bidirectional = params
                .get("bidirectional")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            match crate::learn::link_knowledge_to_knowledge(
                &db_path,
                source,
                target,
                relation,
                bidirectional,
            ) {
                Ok(()) => {
                    serde_json::json!({"status": "linked", "source": source, "target": target, "relation": relation})
                }
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "knowledge_neighbors" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            let neighbors = crate::learn::knowledge_neighbors(&db_path, uuid);
            let items: Vec<serde_json::Value> = neighbors
                .iter()
                .map(|(other, rel, dir)| serde_json::json!({"uuid": other, "relation": rel, "direction": dir}))
                .collect();
            serde_json::json!(items)
        }
        "thought_chain" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            let format = params
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("json");
            let chain = crate::learn::thought_chain(&db_path, uuid);
            if format == "markdown" {
                serde_json::json!(crate::learn::render_thought_chain(&chain))
            } else {
                serde_json::json!(chain)
            }
        }
        "knowledge_graph" => {
            let start = params.get("uuid").and_then(|v| v.as_str());
            let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let format = params
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("json");
            let nodes = crate::learn::traverse_knowledge_graph(&db_path, start, depth);
            if format == "markdown" {
                serde_json::json!(crate::learn::render_knowledge_graph(&nodes))
            } else {
                serde_json::json!(nodes)
            }
        }
        "detect_stale" => {
            let detailed = params
                .get("detailed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if detailed {
                match crate::learn::detect_stale_detailed(&db_path) {
                    Ok(entries) => serde_json::json!(entries),
                    Err(e) => serde_json::json!({"error": e.to_string()}),
                }
            } else {
                match crate::learn::detect_stale_knowledge(&db_path) {
                    Ok(n) => serde_json::json!({"status": "checked", "stale_count": n}),
                    Err(e) => serde_json::json!({"error": e.to_string()}),
                }
            }
        }
        "find_duplicates" => {
            let threshold = params
                .get("threshold")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.6);
            let candidates = crate::learn::find_duplicates(&db_path, threshold);
            serde_json::json!(candidates)
        }
        "merge_knowledge" => {
            // Bulk variant: pass `merges: [{keep_uuid, absorb_uuid}, ...]`.
            // Single variant: keep_uuid + absorb_uuid (legacy).
            if let Some(arr) = params.get("merges").and_then(|v| v.as_array()) {
                let mut succeeded = Vec::new();
                let mut errors = Vec::new();
                for m in arr {
                    let keep = m.get("keep_uuid").and_then(|v| v.as_str());
                    let absorb = m.get("absorb_uuid").and_then(|v| v.as_str());
                    match (keep, absorb) {
                        (Some(k), Some(a)) => match crate::learn::merge_knowledge(&db_path, k, a) {
                            Ok(()) => succeeded.push(serde_json::json!({"kept": k, "absorbed": a})),
                            Err(e) => errors.push(serde_json::json!({
                                "kept": k, "absorbed": a, "error": e.to_string()
                            })),
                        },
                        _ => errors.push(serde_json::json!({
                            "error": "each merge requires keep_uuid + absorb_uuid"
                        })),
                    }
                }
                serde_json::json!({
                    "status": "merged",
                    "succeeded": succeeded,
                    "errors": errors,
                })
            } else {
                let keep = match params.get("keep_uuid").and_then(|v| v.as_str()) {
                    Some(u) => u,
                    None => return error_response(&id, "keep_uuid or merges required"),
                };
                let absorb = match params.get("absorb_uuid").and_then(|v| v.as_str()) {
                    Some(u) => u,
                    None => return error_response(&id, "absorb_uuid required"),
                };
                match crate::learn::merge_knowledge(&db_path, keep, absorb) {
                    Ok(()) => {
                        serde_json::json!({"status": "merged", "kept": keep, "absorbed": absorb})
                    }
                    Err(e) => serde_json::json!({"error": e.to_string()}),
                }
            }
        }
        "detect_conflicts" => {
            let conflicts = crate::learn::detect_conflicts(&db_path);
            serde_json::json!(conflicts)
        }
        "knowledge_health" => {
            let health = crate::learn::knowledge_health(&db_path);
            serde_json::json!(health)
        }
        // Gen3: review queue
        "get_review_queue" => {
            let queue = crate::learn::get_review_queue(&db_path);
            serde_json::json!(queue)
        }
        "complete_review" => {
            let uuid = match params.get("uuid").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return error_response(&id, "uuid required"),
            };
            match crate::learn::complete_review(&db_path, uuid) {
                Ok(()) => serde_json::json!({"status": "completed", "uuid": uuid}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        "refresh_review_queue" => match crate::learn::refresh_review_queue(&db_path) {
            Ok(n) => serde_json::json!({"status": "refreshed", "enqueued": n}),
            Err(e) => serde_json::json!({"error": e.to_string()}),
        },
        // Gen3: diff-aware recall
        "recall_for_diff" => {
            let max = params
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            let auto = params
                .get("auto")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let base_ref = params
                .get("base_ref")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("HEAD");
            // When auto=true, derive the diff from the project's working tree
            // against `base_ref` (default HEAD) so the caller doesn't have to
            // pre-fetch git output. Falls through to user-supplied `diff` on
            // git failure so the call doesn't error out silently.
            let auto_diff: Option<String> = if auto {
                run_git_diff(project_dir, base_ref).ok()
            } else {
                None
            };
            let supplied = params.get("diff").and_then(|v| v.as_str()).unwrap_or("");
            let diff_text: &str = auto_diff
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(supplied);
            let (analysis, results) = crate::learn::recall_for_diff(&db_path, diff_text, max);
            let uuids: Vec<String> =
                results.iter().map(|r| r.knowledge.uuid.clone()).collect();
            let _ = crate::storage::bump_fetch_counters(&db_path, &uuids);
            // Synthesize matched_by signals so the caller can tell why each entry surfaced.
            let enriched: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    let mut signals: Vec<&str> = Vec::new();
                    if r.score.file_mention > 0.0 {
                        signals.push("filename_overlap");
                    }
                    if r.score.node_overlap > 0.0 {
                        signals.push("node_overlap");
                    }
                    if r.score.keyword_match > 0.0 {
                        signals.push("token_overlap");
                    }
                    if r.score.applies_when > 0.0 {
                        signals.push("applies_when");
                    }
                    serde_json::json!({
                        "knowledge": r.knowledge,
                        "score": r.score,
                        "matched_by": signals,
                    })
                })
                .collect();
            let mut response = serde_json::json!({
                "analysis": analysis,
                "relevant_knowledge": enriched,
            });
            if auto {
                response["diff_source"] = serde_json::json!(if auto_diff.is_some() {
                    format!("git diff {base_ref}")
                } else {
                    "git failed; used supplied diff".to_string()
                });
            }
            response
        }
        // Gen3: knowledge graph reasoning
        "reason" => {
            let uuids = extract_string_array(&params, "uuids");
            let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let data = match crate::storage::load(&db_path) {
                Ok(d) => d,
                Err(e) => return error_response(&id, &e.to_string()),
            };
            let result =
                crate::reasoning::propagate_confidence(&data.knowledge, &data.links, &uuids, depth);
            serde_json::json!(result)
        }
        _ => serde_json::json!({"error": format!("Unknown method: {method}")}),
    };

    format!(
        r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#,
        serde_json::to_string(&result).unwrap_or_default(),
        serde_json::to_string(&id).unwrap_or_default(),
    )
}

#[cfg(feature = "embeddings")]
struct Candidate {
    label: String,
    source_file: String,
    vec: Vec<f32>,
}

/// Embedding-based pass for `compare_graphs --semantic-embedding`. For each
/// gap object in `enriched`, embed the gap's label, compute cosine vs all
/// stored embeddings whose source_file matches `right_pattern`, and merge
/// top-K matches above the threshold into the gap's `candidate_matches`
/// array (de-duplicated by label). Returns the modified array.
/// `semantic_search`: NL → top-K code chunks via cosine over chunk
/// embeddings. Optional file/language filters. When `link_knowledge=true`
/// (default), each hit whose chunk maps to a graph node is enriched with
/// the knowledge entries linked to that node — the kodex-only differentiator
/// over plain vector retrieval.
#[cfg(feature = "embeddings")]
fn semantic_search_handler(
    params: &serde_json::Value,
    db_path: &std::path::Path,
) -> Result<serde_json::Value, String> {
    use crate::embedding::Embedder;
    use std::collections::HashMap;

    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if query.is_empty() {
        return Err("semantic_search requires a non-empty `query`".into());
    }
    let requested_top_k = params
        .get("top_k")
        .and_then(|v| v.as_u64())
        .unwrap_or(10);
    let top_k = requested_top_k.clamp(1, 500) as usize;
    let truncated = requested_top_k > 500;
    // Accept both `path_substring` (preferred — accurate name) and the
    // legacy `file_glob` for back-compat with any caller built against the
    // initial draft of this tool. Either is a plain substring match against
    // `chunk.file_path`, NOT a glob.
    let path_substring = params
        .get("path_substring")
        .or_else(|| params.get("file_glob"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let language = params
        .get("language")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let link_knowledge = params
        .get("link_knowledge")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Two-stage load: (1) lightweight metadata for the cosine pass — no
    // chunk content materialized — and (2) `load_chunks_by_ids` for the
    // top-K survivors. Avoids hundreds of MB of content allocation per
    // query on large repos.
    let metadata =
        crate::storage::load_chunk_metadata(db_path).map_err(|e| e.to_string())?;
    if metadata.is_empty() {
        return Err("no chunks in db — run `kodex run` to populate them first".into());
    }
    let embeddings =
        crate::storage::load_all_chunk_embeddings(db_path).map_err(|e| e.to_string())?;
    if embeddings.is_empty() {
        return Err(
            "no chunk embeddings — run `kodex embed` (with --features embeddings) first".into(),
        );
    }

    let meta_map: HashMap<&str, &crate::storage::ChunkMetadata> =
        metadata.iter().map(|c| (c.id.as_str(), c)).collect();

    let embedder = Embedder::new().map_err(|e| e.to_string())?;
    let q = embedder.embed_one(query).map_err(|e| e.to_string())?;

    let scored = rank_chunks(
        &q,
        &embeddings,
        &meta_map,
        path_substring.as_deref(),
        language.as_deref(),
        top_k,
    );

    // Fetch content for the top-K only.
    let top_ids: Vec<String> = scored.iter().map(|(_, m)| m.id.clone()).collect();
    let top_content =
        crate::storage::load_chunks_by_ids(db_path, &top_ids).map_err(|e| e.to_string())?;
    let content_map: HashMap<&str, &crate::storage::StoredChunk> =
        top_content.iter().map(|c| (c.id.as_str(), c)).collect();

    // Resolve attached knowledge for code chunks that mapped to a node. The
    // chunks store `node.id` (label-derived stable id), but `links` keys on
    // `node.uuid` — `attach_knowledge_for_nodes` does the bridge via a
    // narrow SQL lookup (no full graph load).
    let knowledge_by_node: HashMap<String, Vec<serde_json::Value>> = if link_knowledge {
        attach_knowledge_for_nodes(
            db_path,
            scored.iter().filter_map(|(_, m)| m.node_id.as_deref()),
        )
        .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let hits: Vec<serde_json::Value> = scored
        .into_iter()
        .map(|(score, m)| {
            // Content lookup may miss only if a row was deleted between the
            // metadata pass and the by-ids pass — rare race in concurrent
            // re-ingest. Fall back to empty content rather than dropping
            // the hit so the file_path / line range stay actionable.
            let content = content_map
                .get(m.id.as_str())
                .map(|c| c.content.as_str())
                .unwrap_or("");
            let mut hit = serde_json::json!({
                "score": score,
                "file_path": m.file_path,
                "start_line": m.start_line,
                "end_line": m.end_line,
                "content": content,
            });
            if let Some(lang) = &m.language {
                hit["language"] = serde_json::json!(lang);
            }
            if let Some(nid) = &m.node_id {
                hit["node_id"] = serde_json::json!(nid);
                if let Some(k) = knowledge_by_node.get(nid) {
                    hit["attached_knowledge"] = serde_json::json!(k);
                }
            }
            hit
        })
        .collect();

    let count = hits.len();
    let mut response = serde_json::json!({
        "query": query,
        "hits": hits,
        "count": count,
    });
    if truncated {
        response["truncated"] = serde_json::json!(true);
        response["requested_top_k"] = serde_json::json!(requested_top_k);
    }
    Ok(response)
}

/// Pure cosine ranking + filter pass, extracted from `semantic_search_handler`
/// so it's testable without a model. Filters by `path_substring` (substring
/// match against `file_path`) and `language` (exact match), then computes
/// cosine vs `query_vec` for each surviving embedding, sorts desc, truncates
/// to `top_k`. The `metadata` map is keyed by chunk id; embeddings whose
/// chunk id has no metadata entry are dropped (typically a race between
/// metadata read and an in-flight re-ingest).
#[cfg(feature = "embeddings")]
pub(crate) fn rank_chunks<'a>(
    query_vec: &[f32],
    embeddings: &'a [crate::storage::StoredChunkEmbedding],
    metadata: &'a std::collections::HashMap<&str, &'a crate::storage::ChunkMetadata>,
    path_substring: Option<&str>,
    language: Option<&str>,
    top_k: usize,
) -> Vec<(f32, &'a crate::storage::ChunkMetadata)> {
    use crate::embedding::{bytes_to_vec, cosine};
    let mut scored: Vec<(f32, &crate::storage::ChunkMetadata)> = embeddings
        .iter()
        .filter_map(|e| {
            let meta = metadata.get(e.chunk_id.as_str())?;
            if let Some(p) = path_substring {
                if !meta.file_path.contains(p) {
                    return None;
                }
            }
            if let Some(l) = language {
                if meta.language.as_deref() != Some(l) {
                    return None;
                }
            }
            let v = bytes_to_vec(&e.vec);
            if v.is_empty() {
                return None;
            }
            Some((cosine(query_vec, &v), *meta))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    scored
}

/// Thin wrapper over `storage::knowledge_for_node_ids` that JSON-shapes the
/// result for embedding into the `semantic_search` response. The data-only
/// path is in storage so non-MCP callers (and tests) can exercise it
/// without the `embeddings` feature.
#[cfg(feature = "embeddings")]
fn attach_knowledge_for_nodes<'a>(
    db_path: &std::path::Path,
    node_ids: impl Iterator<Item = &'a str>,
) -> crate::error::Result<std::collections::HashMap<String, Vec<serde_json::Value>>> {
    use std::collections::HashSet;

    let wanted: Vec<String> = node_ids
        .map(|s| s.to_string())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let raw = crate::storage::knowledge_for_node_ids(db_path, &wanted)?;
    Ok(raw
        .into_iter()
        .map(|(node_id, attachments)| {
            let json_list: Vec<serde_json::Value> = attachments
                .into_iter()
                .map(|a| {
                    serde_json::json!({
                        "uuid": a.uuid,
                        "title": a.title,
                        "type": a.knowledge_type,
                        "confidence": a.confidence,
                        "relation": a.relation,
                    })
                })
                .collect();
            (node_id, json_list)
        })
        .collect())
}

#[cfg(feature = "embeddings")]
fn semantic_embedding_pass(
    graph: &crate::graph::KodexGraph,
    mut enriched: Vec<serde_json::Value>,
    right_pattern: &str,
    db_path: &std::path::Path,
    threshold: f32,
    top_per_gap: usize,
) -> Result<Vec<serde_json::Value>, String> {
    use crate::embedding::{bytes_to_vec, cosine, Embedder};

    let stored = crate::storage::load_all_embeddings(db_path)
        .map_err(|e| e.to_string())?;
    if stored.is_empty() {
        return Err("no embeddings stored — run `kodex embed` first".into());
    }

    // Restrict the candidate pool to right-pattern files. Build a parallel
    // vec of (label, source_file, vector) so we don't re-look up the graph
    // node per gap.
    let candidates: Vec<Candidate> = stored
        .into_iter()
        .filter_map(|row| {
            let node = graph.get_node(&row.node_id)?;
            if !node.source_file.to_lowercase().contains(right_pattern) {
                return None;
            }
            let v = bytes_to_vec(&row.vec);
            if v.is_empty() {
                None
            } else {
                Some(Candidate {
                    label: node.label.clone(),
                    source_file: node.source_file.clone(),
                    vec: v,
                })
            }
        })
        .collect();
    if candidates.is_empty() {
        return Ok(enriched);
    }

    let embedder = Embedder::new().map_err(|e| e.to_string())?;
    for gap in enriched.iter_mut() {
        let label = match gap.get("label").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let q = match embedder.embed_one(&label) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let mut scored: Vec<(f32, &Candidate)> = candidates
            .iter()
            .map(|c| (cosine(&q, &c.vec), c))
            .filter(|(s, _)| *s >= threshold)
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_per_gap.max(1));
        if scored.is_empty() {
            continue;
        }
        // Merge with existing lexical candidate_matches; dedupe by label.
        let mut existing: Vec<serde_json::Value> = gap
            .get("candidate_matches")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for (cos, c) in scored {
            if let Some(item) = existing.iter_mut().find(|v| {
                v.get("label").and_then(|x| x.as_str()) == Some(c.label.as_str())
            }) {
                item["cosine"] = serde_json::json!(cos);
            } else {
                existing.push(serde_json::json!({
                    "label": c.label,
                    "source_file": c.source_file,
                    "jaccard": 0.0_f32,
                    "cosine": cos,
                }));
            }
        }
        // Re-sort the merged list by cosine desc (then jaccard desc).
        existing.sort_by(|a, b| {
            let ac = a
                .get("cosine")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let bc = b
                .get("cosine")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            bc.partial_cmp(&ac).unwrap_or(std::cmp::Ordering::Equal)
        });
        gap["candidate_matches"] = serde_json::json!(existing);
    }
    Ok(enriched)
}

/// Report whether the graph is behind the project's git HEAD. Returns a JSON
/// object describing the drift, or `None` when in sync / unknown.
fn staleness_warning(project_dir: &str) -> Option<serde_json::Value> {
    let dir = std::path::Path::new(project_dir);
    let entry = crate::registry::entry_for_dir(dir)?;
    let current = crate::registry::drift(&entry, &entry.path)?;
    Some(serde_json::json!({
        "indexed_commit": entry.last_indexed_commit,
        "current_commit": current,
        "project_path": entry.path.display().to_string(),
        "hint": format!(
            "graph is behind HEAD; run `kodex run {}` to refresh",
            entry.path.display()
        ),
    }))
}

/// Run `git diff <base_ref>` in `cwd` and return stdout. Used by
/// `recall_for_diff` when `auto=true` so the agent doesn't have to shell out
/// before retrieving knowledge for the working-tree state.
fn run_git_diff(cwd: &str, base_ref: &str) -> std::io::Result<String> {
    let output = std::process::Command::new("git")
        .arg("diff")
        .arg(base_ref)
        .current_dir(cwd)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "git diff exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Wrap matched characters in `[...]` so JSON consumers can render highlights
/// without parsing index arrays. e.g. `close_file` + indices [0,1,2] → `[clo]se_file`.
fn highlight_label(label: &str, indices: &[u32]) -> String {
    if indices.is_empty() {
        return label.to_string();
    }
    let mut out = String::with_capacity(label.len() + 4);
    let mut prev_in_match = false;
    for (i, c) in label.chars().enumerate() {
        let in_match = indices.binary_search(&(i as u32)).is_ok();
        if in_match && !prev_in_match {
            out.push('[');
        } else if !in_match && prev_in_match {
            out.push(']');
        }
        out.push(c);
        prev_in_match = in_match;
    }
    if prev_in_match {
        out.push(']');
    }
    out
}

/// Fuzzy-match `label` against the graph and return the top-N node IDs.
fn resolve_call_seeds(
    graph: &crate::graph::KodexGraph,
    label: &str,
    top_n: usize,
) -> Vec<String> {
    crate::serve::score_nodes_filtered(
        graph,
        &[label.to_lowercase()],
        &Default::default(),
    )
    .into_iter()
    .take(top_n)
    .map(|(_, id)| id)
    .collect()
}

/// Shared handler body for find_callers / find_callees.
///
/// `traverse` is either `crate::serve::find_callers` or `crate::serve::find_callees`.
/// `result_key` is `"callers"` or `"callees"`.
fn call_direction_handler(
    graph: &crate::graph::KodexGraph,
    params: &serde_json::Value,
    top_n: usize,
    traverse: fn(
        &crate::graph::KodexGraph,
        &[String],
        usize,
        Option<&str>,
    ) -> Vec<crate::serve::CallHit>,
    result_key: &str,
) -> serde_json::Value {
    let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");
    let depth = params.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
    let source_pattern = params
        .get("source_pattern")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    let seed_ids = resolve_call_seeds(graph, label, top_n);
    if seed_ids.is_empty() {
        return serde_json::json!({"error": "no matching node found", "label": label});
    }

    let seed_labels: Vec<String> = seed_ids
        .iter()
        .filter_map(|id| graph.get_node(id).map(|n| n.label.clone()))
        .collect();
    let hits = traverse(graph, &seed_ids, depth, source_pattern);
    let items: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            serde_json::json!({
                "label": h.label,
                "source_file": h.source_file,
                "call_location": h.call_location,
                "depth": h.depth,
            })
        })
        .collect();

    let mut resp = serde_json::json!({
        "target": seed_labels,
        "count": items.len(),
        "depth": depth,
    });
    resp[result_key] = serde_json::json!(items);
    resp
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

#[cfg(all(test, feature = "embeddings"))]
mod tests {
    use super::*;
    use crate::embedding::vec_to_bytes;
    use crate::storage::{ChunkMetadata, StoredChunkEmbedding};
    use std::collections::HashMap;

    /// Build a 384-dim unit vector pointing along axis `i` (zeros elsewhere).
    /// Cosine between two such vectors is `delta(i, j)` (1.0 if same axis,
    /// 0.0 otherwise) — perfect for asserting rank order without a real
    /// embedding model.
    fn axis_vec(i: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[i] = 1.0;
        v
    }

    fn meta(id: &str, file_path: &str, language: Option<&str>) -> ChunkMetadata {
        ChunkMetadata {
            id: id.to_string(),
            node_id: None,
            file_path: file_path.to_string(),
            start_line: 1,
            end_line: 50,
            language: language.map(str::to_string),
        }
    }

    fn emb(chunk_id: &str, axis: usize) -> StoredChunkEmbedding {
        StoredChunkEmbedding {
            chunk_id: chunk_id.to_string(),
            model: "test".to_string(),
            dim: 384,
            vec: vec_to_bytes(&axis_vec(axis)),
        }
    }

    #[test]
    fn rank_chunks_orders_by_cosine_descending() {
        let m = vec![
            meta("c-a", "src/a.rs", Some("rust")),
            meta("c-b", "src/b.rs", Some("rust")),
            meta("c-c", "src/c.rs", Some("rust")),
        ];
        let map: HashMap<&str, &ChunkMetadata> =
            m.iter().map(|x| (x.id.as_str(), x)).collect();

        // c-a aligned with axis 0; c-b axis 1; c-c axis 2.
        let embeddings = vec![emb("c-a", 0), emb("c-b", 1), emb("c-c", 2)];
        // Query along axis 0 → c-a wins with cos=1.0; others get 0.0.
        let q = axis_vec(0);

        let ranked = rank_chunks(&q, &embeddings, &map, None, None, 10);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].1.id, "c-a");
        assert!((ranked[0].0 - 1.0).abs() < 1e-5);
        assert!(ranked[1].0.abs() < 1e-5);
        assert!(ranked[2].0.abs() < 1e-5);
    }

    #[test]
    fn rank_chunks_top_k_truncates() {
        let m: Vec<ChunkMetadata> = (0..5)
            .map(|i| meta(&format!("c-{i}"), &format!("src/{i}.rs"), Some("rust")))
            .collect();
        let map: HashMap<&str, &ChunkMetadata> =
            m.iter().map(|x| (x.id.as_str(), x)).collect();
        let embeddings: Vec<StoredChunkEmbedding> =
            (0..5).map(|i| emb(&format!("c-{i}"), i)).collect();
        let q = axis_vec(0);

        let ranked = rank_chunks(&q, &embeddings, &map, None, None, 2);
        assert_eq!(ranked.len(), 2, "must truncate to top_k");
        assert_eq!(ranked[0].1.id, "c-0");
    }

    #[test]
    fn rank_chunks_path_substring_filters() {
        let m = vec![
            meta("c-rs", "src/foo/bar.rs", Some("rust")),
            meta("c-py", "src/foo/baz.py", Some("python")),
            meta("c-other", "tests/bar.rs", Some("rust")),
        ];
        let map: HashMap<&str, &ChunkMetadata> =
            m.iter().map(|x| (x.id.as_str(), x)).collect();
        let embeddings = vec![emb("c-rs", 0), emb("c-py", 0), emb("c-other", 0)];
        let q = axis_vec(0);

        let ranked = rank_chunks(&q, &embeddings, &map, Some("src/foo/"), None, 10);
        let ids: std::collections::HashSet<&str> =
            ranked.iter().map(|(_, m)| m.id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("c-rs"));
        assert!(ids.contains("c-py"));
        assert!(!ids.contains("c-other"), "tests/ must be excluded");
    }

    #[test]
    fn rank_chunks_language_filters() {
        let m = vec![
            meta("c-rs", "x.rs", Some("rust")),
            meta("c-py", "x.py", Some("python")),
            meta("c-none", "x.txt", None),
        ];
        let map: HashMap<&str, &ChunkMetadata> =
            m.iter().map(|x| (x.id.as_str(), x)).collect();
        let embeddings = vec![emb("c-rs", 0), emb("c-py", 0), emb("c-none", 0)];
        let q = axis_vec(0);

        let ranked = rank_chunks(&q, &embeddings, &map, None, Some("rust"), 10);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].1.id, "c-rs");
    }

    #[test]
    fn rank_chunks_drops_embeddings_with_no_matching_metadata() {
        // Race: an embedding row references a chunk that was just deleted —
        // metadata lookup fails, the embedding is skipped without panic.
        let m = vec![meta("c-keep", "x.rs", Some("rust"))];
        let map: HashMap<&str, &ChunkMetadata> =
            m.iter().map(|x| (x.id.as_str(), x)).collect();
        let embeddings = vec![emb("c-keep", 0), emb("c-stale", 0)];
        let q = axis_vec(0);

        let ranked = rank_chunks(&q, &embeddings, &map, None, None, 10);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].1.id, "c-keep");
    }

    #[test]
    fn rank_chunks_drops_dim_mismatched_vectors() {
        // Mixed-dim DB (e.g. mid-migration to a different model). The
        // mismatched embedding must be skipped, not poison the response.
        let m = vec![meta("c-good", "x.rs", Some("rust"))];
        let map: HashMap<&str, &ChunkMetadata> =
            m.iter().map(|x| (x.id.as_str(), x)).collect();
        let mut bad = emb("c-good", 0);
        bad.vec.truncate(7); // 7 bytes — not a multiple of 4 → bytes_to_vec
                             // returns empty.
        let embeddings = vec![bad];
        let q = axis_vec(0);

        let ranked = rank_chunks(&q, &embeddings, &map, None, None, 10);
        assert!(ranked.is_empty());
    }
}
