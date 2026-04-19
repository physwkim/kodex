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

/// Default skill content embedded at compile time.
const SKILL_CONTENT: &str = r#"# kodex

Knowledge graph builder for code and documents.

## Usage

- `kodex .` — Build knowledge graph for current directory
- `kodex query "how does auth work"` — Search graph
- `kodex path "Client" "Database"` — Find shortest connection
- `kodex explain "ClassName"` — Show node details and neighbors
- `kodex update .` — Re-extract code (AST only, no LLM)
- `kodex watch .` — Auto-rebuild on file changes
- `kodex benchmark` — Measure token reduction

## Output

Results are saved to `kodex-out/`:
- `kodex.h5` — Knowledge graph (HDF5)
- `graph.html` — Interactive visualization (vis.js)
- `GRAPH_REPORT.md` — Analysis report
"#;

/// Install kodex skill to a platform's configuration directory.
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

    let skill_path = target_dir.join(rel_path);

    if let Some(parent) = skill_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return format!("Failed to create directory: {e}");
        }
    }

    // Don't overwrite if already exists
    if skill_path.exists() {
        let existing = std::fs::read_to_string(&skill_path).unwrap_or_default();
        if existing.contains("kodex") {
            return format!("Already installed at {}", skill_path.display());
        }
        // Append to existing file
        let mut content = existing;
        content.push_str("\n\n");
        content.push_str(SKILL_CONTENT);
        match std::fs::write(&skill_path, content) {
            Ok(()) => return format!("Appended to {}", skill_path.display()),
            Err(e) => return format!("Failed to write: {e}"),
        }
    }

    match std::fs::write(&skill_path, SKILL_CONTENT) {
        Ok(()) => format!("Installed to {}", skill_path.display()),
        Err(e) => format!("Failed to write: {e}"),
    }
}

/// Uninstall kodex skill from a platform.
pub fn uninstall(platform: Option<&str>, target_dir: &Path) -> String {
    let platform = platform.unwrap_or("claude");

    let rel_path = match PLATFORMS.iter().find(|(name, _)| *name == platform) {
        Some((_, path)) => *path,
        None => return format!("Unknown platform '{platform}'"),
    };

    let skill_path = target_dir.join(rel_path);
    if !skill_path.exists() {
        return format!("Not installed at {}", skill_path.display());
    }

    match std::fs::remove_file(&skill_path) {
        Ok(()) => format!("Uninstalled from {}", skill_path.display()),
        Err(e) => format!("Failed to remove: {e}"),
    }
}
