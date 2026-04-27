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

const CLAUDE_DIRECTIVE: &str = r#"# kodex
At session start, call the kodex `knowledge_context` MCP tool to load knowledge from previous sessions. Use `recall_for_task` when working on specific tasks.
When querying kodex, use specific code identifiers (function/class/module names) not natural language descriptions. Translate user questions into concrete keywords before calling query_graph or recall.
When you discover a pattern, fix a bug, or make a design decision, automatically call `learn` without asking. Use appropriate types: bug_pattern, convention, decision, architecture, coupling, lesson, tech_debt.
"#;

const SKILL_CONTENT: &str = r#"# kodex

AI knowledge graph with persistent memory.

## Usage
At session start, call `knowledge_context` to load knowledge from previous sessions. Use `recall_for_task` for task-relevant knowledge.
When querying kodex, use specific code identifiers (function/class/module names) not natural language descriptions. Translate user questions into concrete keywords before calling query_graph or recall.
When you discover a pattern, fix a bug, or make a design decision, automatically call `learn` without asking. Use appropriate types: bug_pattern, convention, decision, architecture, coupling, lesson, tech_debt.

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

    // 2. Add kodex directives to global CLAUDE.md
    if platform == "claude" {
        let claude_md = target_dir.join(".claude/CLAUDE.md");
        if let Some(parent) = claude_md.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let existing = std::fs::read_to_string(&claude_md).unwrap_or_default();
        if existing.contains("kodex") {
            results.push("CLAUDE.md: kodex directives already present".to_string());
        } else {
            let directive = format!("{CLAUDE_DIRECTIVE}\n{existing}");
            match std::fs::write(&claude_md, directive) {
                Ok(()) => results.push(format!(
                    "CLAUDE.md: added kodex directives to {}",
                    claude_md.display()
                )),
                Err(e) => results.push(format!("CLAUDE.md: failed: {e}")),
            }
        }
    }

    // 3. Register MCP server (platform-specific)
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
    let kodex_bin = find_kodex_binary();
    let mut results = Vec::new();

    // 1. Register MCP server in ~/.claude.json (user scope)
    let claude_json_path = target_dir.join(".claude.json");
    let mut claude_json = if claude_json_path.exists() {
        let text = std::fs::read_to_string(&claude_json_path).unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let cj_obj = claude_json.as_object_mut().unwrap();
    let already = cj_obj
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .map(|s| s.contains_key("kodex"))
        .unwrap_or(false);

    if already {
        results.push("MCP: already registered in .claude.json".to_string());
    } else {
        let mcp_servers = cj_obj
            .entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(servers) = mcp_servers.as_object_mut() {
            servers.insert(
                "kodex".to_string(),
                serde_json::json!({
                    "type": "stdio",
                    "command": kodex_bin,
                    "args": ["serve"]
                }),
            );
        }
        match serde_json::to_string_pretty(&claude_json) {
            Ok(json) => match std::fs::write(&claude_json_path, json) {
                Ok(()) => {
                    results.push(format!("MCP: registered in {}", claude_json_path.display()))
                }
                Err(e) => results.push(format!("MCP: failed to write: {e}")),
            },
            Err(e) => results.push(format!("MCP: failed to serialize: {e}")),
        }
    }

    // 2. Register hook in ~/.claude/settings.json
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

    // Remove stale mcpServers from settings.json (wrong location)
    if obj.contains_key("mcpServers") {
        obj.remove("mcpServers");
    }

    let kodex_bin_clone = kodex_bin.clone();
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let mut wrote_post = false;
    let mut wrote_start = false;
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
            wrote_post = true;
        }
        if !hooks_obj.contains_key("SessionStart") {
            // Inject knowledge context as `additionalContext` on every session start.
            // `inline_top_k=3` includes full bodies for the 3 highest-priority entries
            // so Claude doesn't need a follow-up `recall` to actually use them.
            hooks_obj.insert(
                "SessionStart".to_string(),
                serde_json::json!([{
                    "hooks": [{
                        "type": "command",
                        "command": format!("{} context --inline-top-k 3 2>/dev/null", kodex_bin_clone)
                    }]
                }]),
            );
            wrote_start = true;
        }
    }
    if wrote_post || wrote_start {
        match serde_json::to_string_pretty(&settings) {
            Ok(json) => match std::fs::write(&settings_path, &json) {
                Ok(()) => {
                    let mut parts = Vec::new();
                    if wrote_post {
                        parts.push("PostToolUse");
                    }
                    if wrote_start {
                        parts.push("SessionStart");
                    }
                    results.push(format!(
                        "Hook: registered {} in {}",
                        parts.join(" + "),
                        settings_path.display()
                    ));
                }
                Err(e) => results.push(format!("Hook: failed: {e}")),
            },
            Err(e) => results.push(format!("Hook: failed: {e}")),
        }
    } else {
        results.push("Hook: already registered".to_string());
    }

    // Always save settings.json (may have removed stale mcpServers)
    if let Ok(json) = serde_json::to_string_pretty(&settings) {
        let _ = std::fs::write(&settings_path, json);
    }

    results.join("\n")
}

fn uninstall_mcp_claude(target_dir: &Path) -> String {
    let mut results = Vec::new();

    // Remove from ~/.claude.json
    let claude_json_path = target_dir.join(".claude.json");
    if claude_json_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&claude_json_path) {
            if let Ok(mut cj) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(servers) = cj.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
                    if servers.remove("kodex").is_some() {
                        if let Ok(json) = serde_json::to_string_pretty(&cj) {
                            let _ = std::fs::write(&claude_json_path, json);
                        }
                        results.push("MCP: removed from .claude.json".to_string());
                    }
                }
            }
        }
    }

    // Also clean from settings.json (legacy location)
    let settings_path = target_dir.join(".claude/settings.json");
    if settings_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&settings_path) {
            if let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&text) {
                let mut changed = false;
                if let Some(servers) = settings
                    .get_mut("mcpServers")
                    .and_then(|v| v.as_object_mut())
                {
                    if servers.remove("kodex").is_some() {
                        changed = true;
                    }
                }
                if changed {
                    if let Ok(json) = serde_json::to_string_pretty(&settings) {
                        let _ = std::fs::write(&settings_path, json);
                    }
                    results.push("MCP: removed from .claude/settings.json (legacy)".to_string());
                }
            }
        }
    }

    if results.is_empty() {
        results.push("MCP: not registered".to_string());
    }

    results.join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_install_registers_session_start_hook() {
        let dir = TempDir::new().unwrap();
        let report = install_mcp_claude(dir.path());
        let settings_path = dir.path().join(".claude/settings.json");
        assert!(settings_path.exists(), "settings.json should be written");
        let text = std::fs::read_to_string(&settings_path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let session_start = json
            .pointer("/hooks/SessionStart")
            .expect("SessionStart hook should be registered");
        assert!(session_start.is_array(), "SessionStart should be an array");
        let cmd = json
            .pointer("/hooks/SessionStart/0/hooks/0/command")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(
            cmd.contains("context"),
            "SessionStart command should call `context`: {cmd}"
        );
        assert!(
            cmd.contains("inline-top-k"),
            "SessionStart command should pass --inline-top-k: {cmd}"
        );
        // Sanity: report should mention SessionStart
        assert!(
            report.contains("SessionStart"),
            "report should mention SessionStart: {report}"
        );
    }

    #[test]
    fn test_install_idempotent() {
        let dir = TempDir::new().unwrap();
        let _first = install_mcp_claude(dir.path());
        let second = install_mcp_claude(dir.path());
        assert!(
            second.contains("already registered"),
            "second install should be a no-op for hooks: {second}"
        );
    }
}
