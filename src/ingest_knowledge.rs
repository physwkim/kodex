//! Ingest knowledge from external sources: git commits, README, docs.

use std::path::Path;

/// Ingest knowledge from git commit messages in the project directory.
/// Extracts decisions, lessons, and bug fixes from recent commits.
pub fn ingest_git_commits(
    h5_path: &Path,
    project_dir: &Path,
    max_commits: usize,
) -> crate::error::Result<usize> {
    let output = std::process::Command::new("git")
        .args(["log", "--oneline", &format!("-{max_commits}")])
        .current_dir(project_dir)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return Ok(0),
    };

    let project_name = project_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let mut count = 0;
    for line in output.lines() {
        if line.len() < 9 {
            continue;
        }
        let hash = &line[..7];
        let msg = line[8..].trim();

        // Classify commit message
        let (knowledge_type, should_save) = classify_commit(msg);
        if !should_save {
            continue;
        }

        // Check if already imported (by commit hash in tags)
        let tag = format!("commit:{hash}");
        let existing = crate::learn::query_knowledge(h5_path, hash, None);
        if existing.iter().any(|k| k.tags.iter().any(|t| t == &tag)) {
            continue;
        }

        crate::storage::append_knowledge(
            h5_path,
            msg,
            knowledge_type,
            &format!("From git commit {hash} in {project_name}"),
            0.5,
            1,
            &[],
            &[tag, format!("project:{project_name}"), "git-commit".into()],
        )?;
        count += 1;
    }

    Ok(count)
}

/// Classify a commit message into knowledge type.
/// Returns (type, should_save).
fn classify_commit(msg: &str) -> (&'static str, bool) {
    let lower = msg.to_lowercase();

    // Skip trivial commits
    if lower.starts_with("merge")
        || lower.starts_with("wip")
        || lower.starts_with("chore")
        || lower.len() < 15
    {
        return ("", false);
    }

    if lower.contains("fix") || lower.contains("bug") || lower.contains("crash") {
        return ("bug_pattern", true);
    }
    if lower.contains("refactor") || lower.contains("cleanup") || lower.contains("simplif") {
        return ("decision", true);
    }
    if lower.contains("add") && (lower.contains("feature") || lower.contains("support")) {
        return ("architecture", true);
    }
    if lower.contains("critical") || lower.contains("breaking") || lower.contains("important") {
        return ("lesson", true);
    }
    if lower.contains("convention") || lower.contains("style") || lower.contains("format") {
        return ("convention", true);
    }
    if lower.contains("perf") || lower.contains("optim") || lower.contains("speed") {
        return ("performance", true);
    }

    // Default: don't save generic commits
    ("", false)
}

/// Ingest knowledge from README.md in the project directory.
/// Extracts project description as architecture knowledge.
pub fn ingest_readme(h5_path: &Path, project_dir: &Path) -> crate::error::Result<usize> {
    let readme_path = project_dir.join("README.md");
    if !readme_path.exists() {
        return Ok(0);
    }

    let content = std::fs::read_to_string(&readme_path)?;
    if content.len() < 50 {
        return Ok(0);
    }

    let project_name = project_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let tag = format!("readme:{project_name}");
    let existing = crate::learn::query_knowledge(h5_path, &tag, None);
    if existing.iter().any(|k| k.tags.iter().any(|t| t == &tag)) {
        return Ok(0);
    }

    // Extract first paragraph (project summary)
    let summary: String = content
        .lines()
        .skip_while(|l| l.starts_with('#') || l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if summary.len() < 20 {
        return Ok(0);
    }

    let title = format!("{project_name} project overview");
    let desc = if summary.len() > 500 {
        format!("{}...", &summary[..500])
    } else {
        summary
    };

    crate::storage::append_knowledge(
        h5_path,
        &title,
        "architecture",
        &desc,
        0.6,
        1,
        &[],
        &[tag, format!("project:{project_name}"), "readme".into()],
    )?;

    Ok(1)
}

/// Ingest all external sources for a project.
pub fn ingest_project(
    h5_path: &Path,
    project_dir: &Path,
    max_commits: usize,
) -> crate::error::Result<usize> {
    let mut total = 0;
    total += ingest_readme(h5_path, project_dir)?;
    total += ingest_git_commits(h5_path, project_dir, max_commits)?;
    Ok(total)
}
