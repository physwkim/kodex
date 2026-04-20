use std::path::Path;

/// Supported platforms for skill installation.
const PLATFORMS: &[(&str, &str)] = &[
    ("claude", ".claude/commands/kodex.md"),
    ("cursor", ".cursor/rules/kodex.md"),
    ("vscode", ".github/copilot-instructions.md"),
    ("codex", ".codex/skills/kodex.md"),
    ("opencode", ".config/opencode/skills/kodex.md"),
    ("aider", ".aider/skills/kodex.md"),
    ("kiro", ".kiro/steering/kodex.md"),
];

const SKILL_CONTENT: &str = r#"# kodex

AI knowledge graph with persistent memory.

## MCP Tools (auto-available via `kodex serve`)

### Knowledge lifecycle
- `learn` — save/reinforce a pattern/decision/convention (returns UUID)
- `recall` — search knowledge by keyword or type
- `recall_for_task` — ranked retrieval: question + touched_files + node_uuids → top-N
- `get_task_context` — full briefing for current task (question + files → ranked knowledge + stale warnings)
- `knowledge_context` — read all knowledge at session start
- `update_knowledge` — update status/scope/applies_when on existing entry
- `forget` — remove knowledge by title/type/confidence

### Knowledge linking
- `link_knowledge_to_nodes` — connect knowledge UUID to node UUIDs
- `clear_knowledge_links` — remove all links for a knowledge entry
- `save_insight` — link nodes with a named pattern
- `add_edge` — add relationship between code nodes

### Graph
- `query_graph` — BFS/DFS search over code graph
- `get_node` — node details by label
- `god_nodes` — most-connected entities

### Maintenance
- `detect_stale` — find knowledge linked to deleted/changed nodes

## CLI

- `kodex run .` — build knowledge graph
- `kodex query "how does auth work"` — search graph
- `kodex explain "ClassName"` — show node details
- `kodex update .` — re-extract code (AST only)
- `kodex import` — import Claude Code memories into kodex
- `kodex export` — export kodex knowledge to Claude Code memories
"#;

/// Install kodex: skill file + MCP server registration.
pub fn install(platform: Option<&str>, target_dir: &Path) -> String {
    let platform = platform.unwrap_or("claude");

    let rel_path = match PLATFORMS.iter().find(|(name, _)| *name == platform) {
        Some((_, path)) => *path,
        None => {
            let names: Vec<&str> = PLATFORMS.iter().map(|(n, _)| *n).collect();
            return format!(
                "Unknown platform '{platform}'. Supported: {}",
                names.join(", ")
            );
        }
    };

    let mut results = Vec::new();

    // 1. Install skill file
    let skill_path = target_dir.join(rel_path);
    if let Some(parent) = skill_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if skill_path.exists() {
        let existing = std::fs::read_to_string(&skill_path).unwrap_or_default();
        if existing.contains("kodex") {
            results.push(format!(
                "Skill: already installed at {}",
                skill_path.display()
            ));
        } else {
            let mut content = existing;
            content.push_str("\n\n");
            content.push_str(SKILL_CONTENT);
            match std::fs::write(&skill_path, content) {
                Ok(()) => results.push(format!("Skill: appended to {}", skill_path.display())),
                Err(e) => results.push(format!("Skill: failed: {e}")),
            }
        }
    } else {
        match std::fs::write(&skill_path, SKILL_CONTENT) {
            Ok(()) => results.push(format!("Skill: installed to {}", skill_path.display())),
            Err(e) => results.push(format!("Skill: failed: {e}")),
        }
    }

    // 2. Register MCP server (platform-specific)
    let mcp_result = match platform {
        "claude" => install_mcp_claude(target_dir),
        "cursor" => install_mcp_cursor(target_dir),
        _ => "MCP: not supported for this platform (manual setup needed)".to_string(),
    };
    results.push(mcp_result);

    results.join("\n")
}

/// Uninstall kodex skill + MCP registration.
pub fn uninstall(platform: Option<&str>, target_dir: &Path) -> String {
    let platform = platform.unwrap_or("claude");

    let rel_path = match PLATFORMS.iter().find(|(name, _)| *name == platform) {
        Some((_, path)) => *path,
        None => return format!("Unknown platform '{platform}'"),
    };

    let mut results = Vec::new();

    // Remove skill file
    let skill_path = target_dir.join(rel_path);
    if skill_path.exists() {
        match std::fs::remove_file(&skill_path) {
            Ok(()) => results.push(format!("Skill: removed {}", skill_path.display())),
            Err(e) => results.push(format!("Skill: failed: {e}")),
        }
    } else {
        results.push("Skill: not installed".to_string());
    }

    // Remove MCP registration
    let mcp_result = match platform {
        "claude" => uninstall_mcp_claude(target_dir),
        "cursor" => uninstall_mcp_cursor(target_dir),
        _ => "MCP: manual removal needed".to_string(),
    };
    results.push(mcp_result);

    results.join("\n")
}

// --- Claude Code MCP ---

fn install_mcp_claude(target_dir: &Path) -> String {
    let settings_path = target_dir.join(".claude/settings.json");
    if let Some(parent) = settings_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut settings = if settings_path.exists() {
        let text = std::fs::read_to_string(&settings_path).unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let obj = settings.as_object_mut().unwrap();

    // Check if already registered
    if let Some(servers) = obj.get("mcpServers").and_then(|v| v.as_object()) {
        if servers.contains_key("kodex") {
            return "MCP: already registered in .claude/settings.json".to_string();
        }
    }

    // Find kodex binary path
    let kodex_bin = find_kodex_binary();

    // Add MCP server entry
    let mcp_servers = obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(servers) = mcp_servers.as_object_mut() {
        servers.insert(
            "kodex".to_string(),
            serde_json::json!({
                "command": kodex_bin,
                "args": ["serve"]
            }),
        );
    }

    // Add hook: sync Claude memory writes to kodex
    let kodex_bin_clone = kodex_bin.clone();
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    if let Some(hooks_obj) = hooks.as_object_mut() {
        if !hooks_obj.contains_key("PostToolUse") {
            hooks_obj.insert(
                "PostToolUse".to_string(),
                serde_json::json!([{
                    "matcher": "Write",
                    "hooks": [{
                        "type": "command",
                        "command": format!(
                            "if echo \"$TOOL_INPUT\" | grep -q '.claude/memory'; then {} import 2>/dev/null; fi",
                            kodex_bin_clone
                        )
                    }]
                }]),
            );
        }
    }

    match serde_json::to_string_pretty(&settings) {
        Ok(json) => match std::fs::write(&settings_path, json) {
            Ok(()) => format!("MCP + hook: registered in {}", settings_path.display()),
            Err(e) => format!("MCP: failed to write settings: {e}"),
        },
        Err(e) => format!("MCP: failed to serialize: {e}"),
    }
}

fn uninstall_mcp_claude(target_dir: &Path) -> String {
    let settings_path = target_dir.join(".claude/settings.json");
    if !settings_path.exists() {
        return "MCP: no settings file".to_string();
    }

    let text = match std::fs::read_to_string(&settings_path) {
        Ok(t) => t,
        Err(_) => return "MCP: failed to read settings".to_string(),
    };

    let mut settings: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return "MCP: invalid settings JSON".to_string(),
    };

    if let Some(servers) = settings
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
    {
        if servers.remove("kodex").is_some() {
            if let Ok(json) = serde_json::to_string_pretty(&settings) {
                let _ = std::fs::write(&settings_path, json);
            }
            return "MCP: removed from .claude/settings.json".to_string();
        }
    }

    "MCP: not registered".to_string()
}

// --- Cursor MCP ---

fn install_mcp_cursor(target_dir: &Path) -> String {
    let settings_path = target_dir.join(".cursor/mcp.json");
    if let Some(parent) = settings_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut settings = if settings_path.exists() {
        let text = std::fs::read_to_string(&settings_path).unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or_else(|_| serde_json::json!({"mcpServers": {}}))
    } else {
        serde_json::json!({"mcpServers": {}})
    };

    let obj = settings.as_object_mut().unwrap();

    if let Some(servers) = obj.get("mcpServers").and_then(|v| v.as_object()) {
        if servers.contains_key("kodex") {
            return "MCP: already registered in .cursor/mcp.json".to_string();
        }
    }

    let kodex_bin = find_kodex_binary();

    let mcp_servers = obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(servers) = mcp_servers.as_object_mut() {
        servers.insert(
            "kodex".to_string(),
            serde_json::json!({
                "command": kodex_bin,
                "args": ["serve"]
            }),
        );
    }

    match serde_json::to_string_pretty(&settings) {
        Ok(json) => match std::fs::write(&settings_path, json) {
            Ok(()) => format!("MCP: registered in {}", settings_path.display()),
            Err(e) => format!("MCP: failed to write: {e}"),
        },
        Err(e) => format!("MCP: failed to serialize: {e}"),
    }
}

fn uninstall_mcp_cursor(target_dir: &Path) -> String {
    let settings_path = target_dir.join(".cursor/mcp.json");
    if !settings_path.exists() {
        return "MCP: no mcp.json".to_string();
    }

    let text = match std::fs::read_to_string(&settings_path) {
        Ok(t) => t,
        Err(_) => return "MCP: failed to read".to_string(),
    };

    let mut settings: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return "MCP: invalid JSON".to_string(),
    };

    if let Some(servers) = settings
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
    {
        if servers.remove("kodex").is_some() {
            if let Ok(json) = serde_json::to_string_pretty(&settings) {
                let _ = std::fs::write(&settings_path, json);
            }
            return "MCP: removed from .cursor/mcp.json".to_string();
        }
    }

    "MCP: not registered".to_string()
}

// --- Helpers ---

fn find_kodex_binary() -> String {
    // Try to find the kodex binary in common locations
    if let Ok(exe) = std::env::current_exe() {
        return exe.to_string_lossy().to_string();
    }
    // Fallback: assume it's in PATH
    "kodex".to_string()
}
