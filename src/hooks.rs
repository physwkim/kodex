use std::path::{Path, PathBuf};

const HOOK_MARKER: &str = "# >>> kodex >>>";
const HOOK_MARKER_END: &str = "# <<< kodex <<<";

/// Subdirectory under `~/.kodex/` for global git hooks.
const GLOBAL_HOOKS_SUBDIR: &str = "git-hooks";

const HOOK_SCRIPT: &str = r#"
# Auto-rebuild kodex graph if this project is registered (no-op otherwise).
if command -v kodex >/dev/null 2>&1; then
    kodex auto-update &
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

// --- Global hooks (single shared dir + git config core.hooksPath) ---

/// Path to the global hooks directory under `~/.kodex/`.
fn global_hooks_dir() -> PathBuf {
    crate::registry::kodex_home().join(GLOBAL_HOOKS_SUBDIR)
}

/// Read `git config --global <key>`. Returns `None` if unset or git unavailable.
fn git_config_global_get(key: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["config", "--global", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let v = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if v.is_empty() { None } else { Some(v) }
}

/// Set `git config --global <key> <value>`.
fn git_config_global_set(key: &str, value: &str) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(["config", "--global", key, value])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

/// Unset `git config --global <key>`.
fn git_config_global_unset(key: &str) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(["config", "--global", "--unset", key])
        .output()
        .map_err(|e| e.to_string())?;
    // exit code 5 = key not set; treat as success
    if !output.status.success() && output.status.code() != Some(5) {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

/// Expand a leading `~/` to the user's home directory.
fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

/// Install kodex hooks globally: writes scripts under `~/.kodex/git-hooks/` and
/// points `git config --global core.hooksPath` there. Refuses if a non-kodex
/// `core.hooksPath` is already configured (e.g. husky).
pub fn install_global() -> String {
    let kodex_dir = global_hooks_dir();
    let kodex_dir_str = kodex_dir.to_string_lossy().to_string();

    // Conflict check: refuse to overwrite a non-kodex core.hooksPath.
    if let Some(existing) = git_config_global_get("core.hooksPath") {
        let existing_path = expand_tilde(&existing);
        let kodex_canon = kodex_dir.canonicalize().unwrap_or_else(|_| kodex_dir.clone());
        let existing_canon = existing_path
            .canonicalize()
            .unwrap_or_else(|_| existing_path.clone());
        if existing_canon != kodex_canon {
            return format!(
                "Refused: git core.hooksPath is already set to '{existing}'.\n\
                 To use kodex global hooks, first run:\n  \
                   git config --global --unset core.hooksPath\n\
                 Or install per-project: `kodex hook install` inside each repo."
            );
        }
    }

    if let Err(e) = std::fs::create_dir_all(&kodex_dir) {
        return format!("Failed to create {}: {e}", kodex_dir.display());
    }

    let mut results = Vec::new();
    let script = format!(
        "#!/bin/sh\n{HOOK_MARKER}{HOOK_SCRIPT}{HOOK_MARKER_END}\n"
    );

    for hook_name in &["post-commit", "post-checkout"] {
        let hook_path = kodex_dir.join(hook_name);
        match std::fs::write(&hook_path, &script) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        &hook_path,
                        std::fs::Permissions::from_mode(0o755),
                    );
                }
                results.push(format!("{hook_name}: written"));
            }
            Err(e) => results.push(format!("{hook_name}: failed: {e}")),
        }
    }

    match git_config_global_set("core.hooksPath", &kodex_dir_str) {
        Ok(()) => results.push(format!("core.hooksPath: {kodex_dir_str}")),
        Err(e) => results.push(format!("core.hooksPath: failed to set: {e}")),
    }

    results.join("\n")
}

/// Uninstall global kodex hooks: removes hook files and unsets
/// `core.hooksPath` if it points to the kodex dir.
pub fn uninstall_global() -> String {
    let kodex_dir = global_hooks_dir();
    let mut results = Vec::new();

    for hook_name in &["post-commit", "post-checkout"] {
        let hook_path = kodex_dir.join(hook_name);
        if hook_path.exists() {
            match std::fs::remove_file(&hook_path) {
                Ok(()) => results.push(format!("{hook_name}: removed")),
                Err(e) => results.push(format!("{hook_name}: failed: {e}")),
            }
        } else {
            results.push(format!("{hook_name}: not installed"));
        }
    }

    if let Some(existing) = git_config_global_get("core.hooksPath") {
        let existing_path = expand_tilde(&existing);
        let kodex_canon = kodex_dir.canonicalize().unwrap_or_else(|_| kodex_dir.clone());
        let existing_canon = existing_path
            .canonicalize()
            .unwrap_or_else(|_| existing_path.clone());
        if existing_canon == kodex_canon {
            match git_config_global_unset("core.hooksPath") {
                Ok(()) => results.push("core.hooksPath: unset".to_string()),
                Err(e) => results.push(format!("core.hooksPath: failed to unset: {e}")),
            }
        } else {
            results.push(format!(
                "core.hooksPath: left unchanged (points elsewhere: {existing})"
            ));
        }
    }

    results.join("\n")
}

/// Check global hook installation status.
pub fn status_global() -> String {
    let kodex_dir = global_hooks_dir();
    let mut results = Vec::new();

    for hook_name in &["post-commit", "post-checkout"] {
        let hook_path = kodex_dir.join(hook_name);
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

    let core_hooks_path = git_config_global_get("core.hooksPath");
    let kodex_canon = kodex_dir.canonicalize().unwrap_or_else(|_| kodex_dir.clone());
    let active = core_hooks_path
        .as_deref()
        .map(|p| {
            let existing = expand_tilde(p);
            let existing_canon = existing.canonicalize().unwrap_or_else(|_| existing.clone());
            existing_canon == kodex_canon
        })
        .unwrap_or(false);

    let core_status = match core_hooks_path {
        Some(p) if active => format!("core.hooksPath: {p} (kodex active)"),
        Some(p) => format!("core.hooksPath: {p} (NOT kodex)"),
        None => "core.hooksPath: not set".to_string(),
    };
    results.push(core_status);

    results.join("\n")
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
