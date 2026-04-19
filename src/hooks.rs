use std::path::{Path, PathBuf};

const HOOK_MARKER: &str = "# >>> kodex >>>";
const HOOK_MARKER_END: &str = "# <<< kodex <<<";

const HOOK_SCRIPT: &str = r#"
# Auto-rebuild graph for code-only changes (no LLM cost)
if command -v kodex >/dev/null 2>&1; then
    kodex update . &
fi
"#;

/// Find the nearest .git directory walking up from `path`.
fn git_root(path: &Path) -> Option<PathBuf> {
    let mut dir = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    loop {
        if dir.join(".git").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Get the hooks directory (respects core.hooksPath).
fn hooks_dir(root: &Path) -> PathBuf {
    // Check if core.hooksPath is set
    if let Ok(output) = std::process::Command::new("git")
        .args(["config", "core.hooksPath"])
        .current_dir(root)
        .output()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            let p = PathBuf::from(&path);
            if p.is_absolute() {
                return p;
            }
            return root.join(p);
        }
    }
    root.join(".git").join("hooks")
}

/// Install kodex post-commit and post-checkout hooks.
pub fn install(path: &Path) -> String {
    let root = match git_root(path) {
        Some(r) => r,
        None => return "Not a git repository".to_string(),
    };

    let dir = hooks_dir(&root);
    let _ = std::fs::create_dir_all(&dir);

    let mut results = Vec::new();
    for hook_name in &["post-commit", "post-checkout"] {
        let result = install_hook(&dir, hook_name);
        results.push(format!("{hook_name}: {result}"));
    }

    results.join("\n")
}

/// Uninstall kodex hooks.
pub fn uninstall(path: &Path) -> String {
    let root = match git_root(path) {
        Some(r) => r,
        None => return "Not a git repository".to_string(),
    };

    let dir = hooks_dir(&root);
    let mut results = Vec::new();
    for hook_name in &["post-commit", "post-checkout"] {
        let result = uninstall_hook(&dir, hook_name);
        results.push(format!("{hook_name}: {result}"));
    }

    results.join("\n")
}

/// Check if kodex hooks are installed.
pub fn status(path: &Path) -> String {
    let root = match git_root(path) {
        Some(r) => r,
        None => return "Not a git repository".to_string(),
    };

    let dir = hooks_dir(&root);
    let mut results = Vec::new();
    for hook_name in &["post-commit", "post-checkout"] {
        let hook_path = dir.join(hook_name);
        let installed = hook_path.is_file()
            && std::fs::read_to_string(&hook_path)
                .unwrap_or_default()
                .contains(HOOK_MARKER);
        let status = if installed {
            "installed"
        } else {
            "not installed"
        };
        results.push(format!("{hook_name}: {status}"));
    }

    results.join("\n")
}

fn install_hook(dir: &Path, name: &str) -> String {
    let hook_path = dir.join(name);
    let existing = std::fs::read_to_string(&hook_path).unwrap_or_default();

    if existing.contains(HOOK_MARKER) {
        return "already installed".to_string();
    }

    let mut content = if existing.is_empty() {
        "#!/bin/sh\n".to_string()
    } else {
        existing
    };

    content.push('\n');
    content.push_str(HOOK_MARKER);
    content.push_str(HOOK_SCRIPT);
    content.push_str(HOOK_MARKER_END);
    content.push('\n');

    if std::fs::write(&hook_path, &content).is_err() {
        return "failed to write".to_string();
    }

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755));
    }

    "installed".to_string()
}

fn uninstall_hook(dir: &Path, name: &str) -> String {
    let hook_path = dir.join(name);
    let content = match std::fs::read_to_string(&hook_path) {
        Ok(c) => c,
        Err(_) => return "not installed".to_string(),
    };

    if !content.contains(HOOK_MARKER) {
        return "not installed".to_string();
    }

    // Remove the kodex section
    let mut result = String::new();
    let mut in_section = false;
    for line in content.lines() {
        if line.contains(HOOK_MARKER) {
            in_section = true;
            continue;
        }
        if line.contains(HOOK_MARKER_END) {
            in_section = false;
            continue;
        }
        if !in_section {
            result.push_str(line);
            result.push('\n');
        }
    }

    let trimmed = result.trim();
    if trimmed == "#!/bin/sh" || trimmed.is_empty() {
        let _ = std::fs::remove_file(&hook_path);
    } else {
        let _ = std::fs::write(&hook_path, result);
    }

    "uninstalled".to_string()
}
