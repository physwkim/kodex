use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Start MCP stdio server. Connects to actor (starts it if needed).
/// The graph_path argument is unused — actor always uses ~/.kodex/kodex.db.
pub fn serve(_graph_path: &Path) {
    // Ensure actor is running
    if let Err(e) = kodex::actor::ensure_running() {
        eprintln!("Failed to start actor: {e}");
        return;
    }

    // Connect to actor socket
    let sock_path = kodex::actor::socket_path();
    let stream = match connect_with_retry(&sock_path, 10) {
        Some(s) => s,
        None => {
            eprintln!("Failed to connect to actor at {}", sock_path.display());
            return;
        }
    };

    let writer_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to clone stream: {e}");
            return;
        }
    };

    // Get CWD for project context
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .to_string_lossy()
        .to_string();

    // Proxy: stdin → actor, actor → stdout
    let mut actor_writer = writer_stream;
    let actor_reader = BufReader::new(stream);

    // Spawn reader thread: actor → stdout (wrap in MCP format)
    let reader_handle = std::thread::spawn(move || {
        for line in actor_reader.lines() {
            match line {
                Ok(l) => {
                    let wrapped = wrap_actor_response(&l);
                    println!("{wrapped}");
                    let _ = std::io::stdout().flush();
                }
                Err(_) => break,
            }
        }
    });

    // Main thread: stdin → actor (inject project_dir)
    // Intercept MCP protocol messages (initialize, tools/list) locally
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
                // Check if this is an MCP protocol message we handle locally
                if let Some(response) = handle_mcp_protocol(trimmed) {
                    if !response.is_empty() {
                        println!("{response}");
                        let _ = std::io::stdout().flush();
                    }
                    continue;
                }
                // Rewrite tools/call → direct method call for actor
                let forwarded = rewrite_tools_call(trimmed);
                let enriched = inject_project_dir(&forwarded, &cwd);
                if let Err(e) = writeln!(actor_writer, "{enriched}") {
                    eprintln!("[kodex-serve] actor write failed: {e}");

                    break;
                }
                let _ = actor_writer.flush();
            }
            Err(_) => break,
        }
    }

    let _ = reader_handle.join();
}

/// Connect to Unix socket with retries.
fn connect_with_retry(path: &Path, max_retries: u32) -> Option<std::os::unix::net::UnixStream> {
    for _ in 0..max_retries {
        if let Ok(stream) = std::os::unix::net::UnixStream::connect(path) {
            return Some(stream);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

/// Handle MCP protocol messages locally (initialize, tools/list, notifications).
/// Returns Some(response) if handled, None if should be forwarded to actor.
fn handle_mcp_protocol(input: &str) -> Option<String> {
    let req: serde_json::Value = serde_json::from_str(input).ok()?;
    let method = req.get("method")?.as_str()?;
    let id = req.get("id").cloned();

    match method {
        "initialize" => {
            let result = serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "kodex",
                    "version": env!("CARGO_PKG_VERSION")
                }
            });
            Some(format_response(&id, &result))
        }
        "notifications/initialized" | "notifications/cancelled" => {
            // Notifications: no response, don't forward to actor
            Some(String::new())
        }
        "tools/list" => {
            let result = serde_json::json!({ "tools": mcp_tool_definitions() });
            Some(format_response(&id, &result))
        }
        "tools/call" => None, // Handled by rewrite_tools_call in main loop
        _ => None,
    }
}

fn format_response(id: &Option<serde_json::Value>, result: &serde_json::Value) -> String {
    let id_val = id.as_ref().cloned().unwrap_or(serde_json::Value::Null);
    format!(
        r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#,
        serde_json::to_string(result).unwrap_or_default(),
        serde_json::to_string(&id_val).unwrap_or_default(),
    )
}

fn mcp_tool_definitions() -> Vec<serde_json::Value> {
    vec![
        tool_def(
            "learn",
            "Store or reinforce knowledge. Save when discovering a non-obvious pattern, fixing a tricky bug, or making a design decision; skip if the compiler/linter would catch it. Save policy by type:\n  - bug_pattern: fix is non-obvious AND likely to recur (skip one-off compiler errors)\n  - convention: project-wide rule a newcomer wouldn't infer from the code\n  - decision: a chosen approach with rationale that future readers need to understand the code\n  - architecture: cross-module structure that isn't visible in any single file\n  - lesson: outcome of a failed approach worth remembering\n  - tech_debt: known shortcut with explicit owner/condition for cleanup\n  - performance: measured optimization (skip speculative ones)\nPass context_uuid for chain of thought. Pass supersedes=<old_uuid> when this entry replaces an obsolete one — old entry is auto-obsoleted and back-references are written. Response includes merge_candidates if a similar entry exists.",
            &[
                ("title", "string", true),
                ("description", "string", true),
                ("type", "string", false),
                ("uuid", "string", false),
                ("context_uuid", "string", false),
                ("related_nodes", "array", false),
                ("tags", "array", false),
                ("supersedes", "string", false),
            ],
        ),
        tool_def(
            "recall",
            "Keyword search on title/description/tags. Best for exact identifiers (function/module names). For task-context retrieval prefer recall_for_task.",
            &[("query", "string", true), ("type", "string", false)],
        ),
        tool_def(
            "recall_for_task",
            "Ranked knowledge retrieval for current task. Prefer this over recall for natural-language queries. Optional `type` filters by knowledge type (e.g. bug_pattern, tech_debt).",
            &[
                ("question", "string", false),
                ("touched_files", "array", false),
                ("node_uuids", "array", false),
                ("max_items", "number", false),
                ("type", "string", false),
            ],
        ),
        tool_def(
            "recall_for_diff",
            "Recall knowledge relevant to a git diff. Pass `auto=true` to have the actor run `git diff <base_ref>` (default `HEAD`) in the project working tree and use that — saves the caller from pre-fetching diff output. Falls back to the supplied `diff` if git fails.",
            &[
                ("diff", "string", false),
                ("max_items", "number", false),
                ("auto", "boolean", false),
                ("base_ref", "string", false),
            ],
        ),
        tool_def(
            "get_task_context",
            "Full briefing with recommendations, warnings, conflicts.",
            &[
                ("question", "string", false),
                ("touched_files", "array", false),
                ("max_items", "number", false),
                ("format", "string", false),
                ("task_type", "string", false),
            ],
        ),
        tool_def(
            "knowledge_context",
            "All knowledge for session bootstrap. Set `inline_top_k=N` to inline full descriptions of the N highest-priority entries (saves a follow-up `recall` round-trip).",
            &[
                ("max_items", "number", false),
                ("inline_top_k", "number", false),
            ],
        ),
        tool_def(
            "query_graph",
            "BFS search over the code graph. Filters: `source_pattern` (limit to a path substring), `community` (limit to one community id), `exclude_hubs` (true → skip BFS expansion through degree>50 nodes; or pass a number for a custom threshold) — use these to cut noise from generic hubs like ok()/len(). Vague natural-language `question` paired with a precise `source_pattern` is fine — when fuzzy scoring finds nothing in scope, the tool falls back to seeding with the highest-degree nodes that pass the filter, so you still get an architectural overview. Set `format=mermaid` for a flowchart.",
            &[
                ("question", "string", true),
                ("depth", "number", false),
                ("token_budget", "number", false),
                ("format", "string", false),
                ("source_pattern", "string", false),
                ("community", "number", false),
                ("exclude_hubs", "boolean", false),
            ],
        ),
        tool_def(
            "get_node",
            "Get node details by label. Returns the top-N scored matches (default 3) with source_file, source_location, community, and degree — useful for disambiguating overloaded names. Set `top_n=1` for a single best match. Use `source_pattern` to scope to one repo (e.g. `pvxs/` vs `pva-rs/`) when the same identifier exists in both. Pass `expand=<relation>` (e.g. `contains`, `calls`) to also list the top candidate's outgoing neighbors via that relation, sorted by degree — use this to enumerate API surface (e.g. all methods of a class) without grep. When expanding, the candidate with the most matching outgoing edges is auto-selected (returned as `members_source`), so a file-level hub beats an empty stub class.",
            &[
                ("label", "string", true),
                ("top_n", "number", false),
                ("source_pattern", "string", false),
                ("expand", "string", false),
                ("expand_top_n", "number", false),
            ],
        ),
        tool_def(
            "god_nodes",
            "Most-connected entities. Filter to a domain with `pattern` (label substring), `source_pattern` (file path substring), or `min_degree` to skip generic hubs like ok()/len().",
            &[
                ("top_n", "number", false),
                ("pattern", "string", false),
                ("source_pattern", "string", false),
                ("min_degree", "number", false),
            ],
        ),
        tool_def(
            "compare_graphs",
            "**API parity / port-completeness check between two codebases.** Use this FIRST when working on porting, reimplementation, or feature parity — it surfaces which symbols exist in `left_pattern` files but have no normalized match in `right_pattern` files. Saves you from grep + manual diffing across upstream sources. Labels are normalized (camelCase ↔ snake_case ↔ scope qualifiers collapse) so naming-convention drift across languages doesn't generate false gaps. File-level / module hubs (e.g. `data`, `pvxs`, `evhelper`) are skipped by default — set `skip_file_nodes=false` to include them. Narrow with `pattern` (label substring, e.g. `pattern=\"connect\"` for connection-related gaps) or `min_degree`. Set `with_signature=true` to inline a few lines around each gap's source location (signature + preceding doc comment) so you can verify whether a gap is truly missing or just renamed/reshaped, without re-grepping the upstream. Pass `public_pattern` (e.g. `\"/include/\"`, `\"src/pvxs/\"`) to promote gaps in public-API headers above internal symbols — this typically cuts noise from internal scheduler/util functions by a large margin; `public_only=true` drops internals entirely. Recommended workflow: (1) `compare_graphs` with `with_signature=true` and `public_pattern` to find API-stable gaps with context → (2) `get_node` on each gap for full disambiguation → (3) `query_graph` with `source_pattern` to trace callers.",
            &[
                ("left_pattern", "string", true),
                ("right_pattern", "string", true),
                ("file_type", "string", false),
                ("min_norm_len", "number", false),
                ("top_n", "number", false),
                ("pattern", "string", false),
                ("min_degree", "number", false),
                ("skip_file_nodes", "boolean", false),
                ("with_signature", "boolean", false),
                ("signature_lines_above", "number", false),
                ("signature_lines_below", "number", false),
                ("signature_max_top", "number", false),
                ("public_pattern", "string", false),
                ("public_only", "boolean", false),
                ("internal_weight", "number", false),
                ("semantic_threshold", "number", false),
                ("semantic_top_per_gap", "number", false),
                ("compose_priority", "boolean", false),
            ],
        ),
        tool_def(
            "co_changes",
            "Find files that frequently co-change with a target file in git history. Reveals architectural seams that aren't visible in the static graph — when X is touched, Y typically needs review too. Scans the last `commit_limit` commits (default 200) and returns top files ranked by co-change count + weight (=co_commits / target_commits). Use after editing a file to surface hidden coupling, or to evaluate the blast radius of a planned change.",
            &[
                ("file", "string", true),
                ("commit_limit", "number", false),
                ("top_n", "number", false),
                ("min_weight", "number", false),
            ],
        ),
        tool_def(
            "list_communities",
            "Enumerate code communities (Louvain clusters) with top high-degree symbols and dominant source files. Use this first to pick a useful `community=N` value for `query_graph`. Defaults: top 3 symbols + 3 files per community, min size 3, return up to 20 communities.",
            &[
                ("top_per_community", "number", false),
                ("min_size", "number", false),
                ("limit", "number", false),
            ],
        ),
        tool_def("graph_stats", "Node/edge/community counts.", &[]),
        tool_def(
            "forget",
            "Delete knowledge.",
            &[
                ("title", "string", false),
                ("type", "string", false),
                ("project", "string", false),
                ("below_confidence", "number", false),
            ],
        ),
        tool_def(
            "update_knowledge",
            "Update knowledge fields.",
            &[
                ("uuid", "string", true),
                ("status", "string", false),
                ("scope", "string", false),
                ("applies_when", "string", false),
                ("superseded_by", "string", false),
                ("validate", "boolean", false),
            ],
        ),
        tool_def(
            "validate_knowledge",
            "Mark knowledge as validated.",
            &[("uuid", "string", true), ("note", "string", false)],
        ),
        tool_def(
            "mark_obsolete",
            "Mark knowledge as obsolete. Pass `uuids` (array) instead of `uuid` to obsolete several at once.",
            &[
                ("uuid", "string", false),
                ("uuids", "array", false),
                ("reason", "string", false),
            ],
        ),
        tool_def(
            "link_knowledge",
            "Connect knowledge to knowledge.",
            &[
                ("source_uuid", "string", true),
                ("target_uuid", "string", true),
                ("relation", "string", false),
                ("bidirectional", "boolean", false),
            ],
        ),
        tool_def(
            "link_knowledge_to_nodes",
            "Connect knowledge to code nodes.",
            &[
                ("uuid", "string", true),
                ("node_uuids", "array", true),
                ("relation", "string", false),
            ],
        ),
        tool_def(
            "thought_chain",
            "Trace reasoning chain from a knowledge entry.",
            &[("uuid", "string", true), ("format", "string", false)],
        ),
        tool_def(
            "knowledge_graph",
            "BFS traversal of knowledge graph.",
            &[
                ("uuid", "string", false),
                ("depth", "number", false),
                ("format", "string", false),
            ],
        ),
        tool_def(
            "detect_stale",
            "Find stale knowledge.",
            &[("detailed", "boolean", false)],
        ),
        tool_def(
            "find_duplicates",
            "Find similar knowledge.",
            &[("threshold", "number", false)],
        ),
        tool_def(
            "merge_knowledge",
            "Merge duplicate entries. Pass `merges: [{keep_uuid, absorb_uuid}, ...]` to merge several pairs in one call.",
            &[
                ("keep_uuid", "string", false),
                ("absorb_uuid", "string", false),
                ("merges", "array", false),
            ],
        ),
        tool_def("detect_conflicts", "Find conflicting knowledge.", &[]),
        tool_def("knowledge_health", "Knowledge base health metrics.", &[]),
        tool_def(
            "reason",
            "Graph reasoning: confidence propagation.",
            &[("uuids", "array", true), ("depth", "number", false)],
        ),
        tool_def("get_review_queue", "Get pending review items.", &[]),
        tool_def(
            "refresh_review_queue",
            "Auto-enqueue stale/conflict/duplicate items.",
            &[],
        ),
        tool_def(
            "complete_review",
            "Mark review item as done.",
            &[("uuid", "string", true)],
        ),
        tool_def(
            "save_insight",
            "Link nodes with a named pattern.",
            &[
                ("label", "string", true),
                ("description", "string", false),
                ("nodes", "array", true),
                ("pattern", "string", false),
            ],
        ),
        tool_def(
            "save_note",
            "Free-text memo.",
            &[
                ("title", "string", true),
                ("content", "string", true),
                ("related_nodes", "array", false),
            ],
        ),
        tool_def(
            "add_edge",
            "Add relationship between nodes.",
            &[
                ("source", "string", true),
                ("target", "string", true),
                ("relation", "string", false),
                ("description", "string", false),
            ],
        ),
    ]
}

fn tool_def(name: &str, desc: &str, params: &[(&str, &str, bool)]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for &(pname, ptype, req) in params {
        properties.insert(pname.to_string(), serde_json::json!({ "type": ptype }));
        if req {
            required.push(serde_json::Value::String(pname.to_string()));
        }
    }
    serde_json::json!({
        "name": name,
        "description": desc,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        }
    })
}

/// Rewrite MCP tools/call into direct JSON-RPC method call for actor.
fn rewrite_tools_call(input: &str) -> String {
    let req: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(_) => return input.to_string(),
    };
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    if method != "tools/call" {
        return input.to_string();
    }
    let params = match req.get("params") {
        Some(p) => p,
        None => return input.to_string(),
    };
    let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let rewritten = serde_json::json!({
        "jsonrpc": "2.0",
        "method": tool_name,
        "params": arguments,
        "id": id,
    });
    serde_json::to_string(&rewritten).unwrap_or_else(|_| input.to_string())
}

/// Wrap actor JSON-RPC response in MCP tools/call content format.
fn wrap_actor_response(line: &str) -> String {
    let resp: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return line.to_string(),
    };
    let id = resp.get("id").cloned().unwrap_or(serde_json::Value::Null);
    // Check if this looks like a raw actor response (has "result" but not MCP "content" format)
    if let Some(result) = resp.get("result") {
        if result.get("content").is_none() {
            let text = serde_json::to_string(result).unwrap_or_default();
            let mcp_result = serde_json::json!({
                "content": [{"type": "text", "text": text}],
            });
            return format!(
                r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#,
                serde_json::to_string(&mcp_result).unwrap_or_default(),
                serde_json::to_string(&id).unwrap_or_default(),
            );
        }
    }
    line.to_string()
}

/// Inject project_dir into JSON-RPC params so actor knows which db to use.
fn inject_project_dir(input: &str, cwd: &str) -> String {
    let mut req: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(_) => return input.to_string(),
    };

    if let Some(obj) = req.as_object_mut() {
        let params = obj.entry("params").or_insert_with(|| serde_json::json!({}));
        if let Some(p) = params.as_object_mut() {
            p.entry("project_dir")
                .or_insert_with(|| serde_json::json!(cwd));
        }
    }

    serde_json::to_string(&req).unwrap_or_else(|_| input.to_string())
}
