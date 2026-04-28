//! Co-change analysis from git history.
//!
//! Files that frequently appear together in commits often share an
//! architectural seam — touching one usually means the other should be
//! reviewed. This module derives a co-change weight for a target file by
//! scanning the last N commits and counting how often each other file
//! shares a commit with it.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// One file that frequently co-changes with the target.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CoChange {
    pub file: String,
    /// Number of commits in which `file` appears together with the target.
    pub commits: usize,
    /// Co-change weight: `commits / target_commits`. 1.0 means it always
    /// changes with the target.
    pub weight: f32,
}

/// Configuration for [`co_changes`].
#[derive(Debug, Clone)]
pub struct CoChangeQuery {
    /// Path to query (relative to repo root or absolute).
    pub file: String,
    /// Number of recent commits to scan. Default 200.
    pub commit_limit: usize,
    /// Cap on returned co-changing files. Default 20.
    pub top_n: usize,
    /// Only return files with weight ≥ this. Default 0.0.
    pub min_weight: f32,
}

impl Default for CoChangeQuery {
    fn default() -> Self {
        Self {
            file: String::new(),
            commit_limit: 200,
            top_n: 20,
            min_weight: 0.0,
        }
    }
}

/// Result of [`co_changes`]. `target_commits` is the number of commits in
/// which the target file appeared (out of `commit_limit`); useful to gauge
/// confidence in the weights.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CoChangeResult {
    pub target: String,
    pub target_commits: usize,
    pub scanned_commits: usize,
    pub co_changes: Vec<CoChange>,
}

/// Compute co-changes for `query.file` in the git repo at `repo_dir`.
pub fn co_changes(repo_dir: &Path, query: &CoChangeQuery) -> std::io::Result<CoChangeResult> {
    if query.file.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "co_changes: file is required",
        ));
    }
    let commits = scan_commits(repo_dir, query.commit_limit)?;
    let scanned_commits = commits.len();

    // The target may appear in the log under several path forms (renamed,
    // moved). Match by filename suffix as a last-resort fallback so the
    // user's relative path "src/foo.rs" still finds commits that reference
    // it as "kodex/src/foo.rs".
    let target_norm = normalize_path(&query.file);
    let target_basename = Path::new(&target_norm)
        .file_name()
        .map(|b| b.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut target_commits = 0usize;
    for files in &commits {
        let target_in = files.iter().any(|f| {
            let fnorm = normalize_path(f);
            fnorm == target_norm
                || fnorm.ends_with(&format!("/{target_norm}"))
                || (!target_basename.is_empty()
                    && Path::new(&fnorm)
                        .file_name()
                        .map(|b| b == target_basename.as_str())
                        .unwrap_or(false))
        });
        if !target_in {
            continue;
        }
        target_commits += 1;
        for f in files {
            let fnorm = normalize_path(f);
            if fnorm == target_norm {
                continue;
            }
            *counts.entry(fnorm).or_insert(0) += 1;
        }
    }

    let denom = target_commits.max(1) as f32;
    let mut co: Vec<CoChange> = counts
        .into_iter()
        .map(|(file, c)| CoChange {
            file,
            commits: c,
            weight: c as f32 / denom,
        })
        .filter(|c| c.weight >= query.min_weight)
        .collect();
    co.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.file.cmp(&b.file))
    });
    co.truncate(query.top_n);

    Ok(CoChangeResult {
        target: query.file.clone(),
        target_commits,
        scanned_commits,
        co_changes: co,
    })
}

/// Run `git log -n LIMIT --name-only --pretty=format:%H` and parse into a
/// vec of file lists, one per commit.
fn scan_commits(repo_dir: &Path, limit: usize) -> std::io::Result<Vec<Vec<String>>> {
    let output = Command::new("git")
        .arg("log")
        .arg(format!("-n{limit}"))
        .arg("--name-only")
        .arg("--pretty=format:__COMMIT__%H")
        .arg("--no-renames")
        .current_dir(repo_dir)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "git log exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let raw = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut commits: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in raw.lines() {
        if let Some(_sha) = line.strip_prefix("__COMMIT__") {
            if !current.is_empty() {
                commits.push(std::mem::take(&mut current));
            }
        } else if !line.trim().is_empty() {
            current.push(line.trim().to_string());
        }
    }
    if !current.is_empty() {
        commits.push(current);
    }
    Ok(commits)
}

fn normalize_path(p: &str) -> String {
    p.replace('\\', "/").trim_start_matches("./").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_relative_prefix_and_normalizes_slashes() {
        assert_eq!(normalize_path("./src/foo.rs"), "src/foo.rs");
        assert_eq!(normalize_path("src\\foo.rs"), "src/foo.rs");
        assert_eq!(normalize_path("src/foo.rs"), "src/foo.rs");
    }

    /// Run a real `git log` on the kodex repo itself — we know `Cargo.toml`
    /// has co-changed with `Cargo.lock` plenty of times, so this asserts the
    /// pipeline end-to-end without needing a fixture commit history.
    #[test]
    fn finds_real_co_changes_in_kodex_repo() {
        let cwd = std::env::current_dir().unwrap();
        let q = CoChangeQuery {
            file: "Cargo.toml".into(),
            commit_limit: 100,
            top_n: 10,
            min_weight: 0.0,
        };
        let result = match co_changes(&cwd, &q) {
            Ok(r) => r,
            Err(_) => return, // Skip when not in a git checkout (CI sandboxes).
        };
        if result.target_commits == 0 {
            return; // Nothing to assert in a fresh repo.
        }
        let files: Vec<&str> = result
            .co_changes
            .iter()
            .map(|c| c.file.as_str())
            .collect();
        assert!(
            files.contains(&"Cargo.lock"),
            "Cargo.lock should co-change with Cargo.toml: {files:?}"
        );
    }
}
