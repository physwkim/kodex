//! Import memories from Claude Code (~/.claude/memory/) into kodex.

use std::path::{Path, PathBuf};

/// Import all Claude memory files into kodex knowledge base.
/// Returns count of imported entries.
pub fn import_claude_memories(h5_path: &Path) -> crate::error::Result<usize> {
    let claude_dir = dirs::home_dir().unwrap_or_default().join(".claude");

    if !claude_dir.is_dir() {
        return Ok(0);
    }

    let mut count = 0;

    // Find all memory .md files (skip MEMORY.md index files)
    let memory_files = find_memory_files(&claude_dir);

    for (file_path, project_hint) in &memory_files {
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let (frontmatter, body) = parse_frontmatter_and_body(&content);

        let name = frontmatter.get("name").cloned().unwrap_or_default();
        let description = frontmatter.get("description").cloned().unwrap_or_default();
        let mem_type = frontmatter.get("type").cloned().unwrap_or_default();

        if name.is_empty() && description.is_empty() {
            continue;
        }

        let title = if !description.is_empty() {
            description.clone()
        } else {
            name.replace('_', " ")
        };

        let knowledge_type = match mem_type.as_str() {
            "feedback" => "preference",
            "project" => "context",
            "user" => "preference",
            "reference" => "api",
            other => other,
        };

        // Tag with source project if known
        let mut tags = vec!["imported".to_string(), "claude-memory".to_string()];
        if let Some(project) = project_hint {
            tags.push(format!("project:{project}"));
        }

        crate::storage::append_knowledge(
            h5_path,
            &title,
            knowledge_type,
            &body,
            0.7, // imported memories start at moderate confidence
            1,
            &[], // no node links for imported memories
            &tags,
        )?;

        count += 1;
    }

    Ok(count)
}

/// Find all memory .md files under ~/.claude/, with optional project hint.
fn find_memory_files(claude_dir: &Path) -> Vec<(PathBuf, Option<String>)> {
    let mut files = Vec::new();

    // Global memories: ~/.claude/memory/*.md
    let global_dir = claude_dir.join("memory");
    if global_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&global_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false)
                    && path.file_name().map(|n| n != "MEMORY.md").unwrap_or(false)
                {
                    files.push((path, None));
                }
            }
        }
    }

    // Project memories: ~/.claude/projects/*/memory/*.md
    let projects_dir = claude_dir.join("projects");
    if projects_dir.is_dir() {
        if let Ok(project_entries) = std::fs::read_dir(&projects_dir) {
            for project_entry in project_entries.flatten() {
                let project_path = project_entry.path();
                let project_name = project_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .trim_start_matches('-')
                    .replace("-Users-stevek-codes-", "")
                    .replace('-', "/");

                let memory_dir = project_path.join("memory");
                if memory_dir.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&memory_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.extension().map(|e| e == "md").unwrap_or(false)
                                && path.file_name().map(|n| n != "MEMORY.md").unwrap_or(false)
                            {
                                let hint = if project_name.is_empty() {
                                    None
                                } else {
                                    Some(project_name.clone())
                                };
                                files.push((path, hint));
                            }
                        }
                    }
                }
            }
        }
    }

    files
}

/// Parse YAML frontmatter and body from a markdown file.
fn parse_frontmatter_and_body(
    content: &str,
) -> (std::collections::HashMap<String, String>, String) {
    let mut map = std::collections::HashMap::new();

    if !content.starts_with("---") {
        return (map, content.to_string());
    }

    let rest = &content[3..];
    let end = match rest.find("\n---") {
        Some(pos) => pos,
        None => return (map, content.to_string()),
    };

    let fm = &rest[..end];
    for line in fm.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let k = key.trim().to_string();
            let v = value.trim().trim_matches('"').to_string();
            if !k.is_empty() && !v.is_empty() {
                map.insert(k, v);
            }
        }
    }

    let body = rest[end + 4..].trim().to_string();
    (map, body)
}
