//! Bidirectional sync between Claude Code memories and kodex knowledge.

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

/// Export kodex knowledge to Claude Code memory format (~/.claude/memory/).
/// Returns count of exported entries.
pub fn export_to_claude_memories(h5_path: &Path) -> crate::error::Result<usize> {
    let data = crate::storage::load(h5_path)?;

    if data.knowledge.is_empty() {
        return Ok(0);
    }

    let memory_dir = dirs::home_dir().unwrap_or_default().join(".claude/memory");
    std::fs::create_dir_all(&memory_dir)?;

    let mut count = 0;

    for entry in &data.knowledge {
        // Skip already-imported entries (avoid circular sync)
        if entry
            .tags
            .iter()
            .any(|t| t == "imported" || t == "claude-memory")
        {
            continue;
        }

        let mem_type = match entry.knowledge_type.as_str() {
            "preference" => "feedback",
            "context" => "project",
            "api" => "reference",
            _ => "feedback",
        };

        let safe_name = entry
            .title
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '_', "_")
            .trim_matches('_')
            .to_string();

        let filename = format!("kodex_{safe_name}.md");
        let filepath = memory_dir.join(&filename);

        // Don't overwrite if already exists with same content
        if filepath.exists() {
            let existing = std::fs::read_to_string(&filepath).unwrap_or_default();
            if existing.contains(&entry.title) {
                continue;
            }
        }

        let md = format!(
            "---\n\
             name: kodex_{safe_name}\n\
             description: {title}\n\
             type: {mem_type}\n\
             ---\n\n\
             {desc}\n",
            title = entry.title,
            desc = entry.description,
        );

        std::fs::write(&filepath, md)?;
        count += 1;
    }

    // Update MEMORY.md index
    if count > 0 {
        update_memory_index(&memory_dir)?;
    }

    Ok(count)
}

/// Update ~/.claude/memory/MEMORY.md with kodex entries.
fn update_memory_index(memory_dir: &Path) -> crate::error::Result<()> {
    let index_path = memory_dir.join("MEMORY.md");
    let mut content = if index_path.exists() {
        std::fs::read_to_string(&index_path).unwrap_or_default()
    } else {
        String::new()
    };

    // Read all kodex_ files
    if let Ok(entries) = std::fs::read_dir(memory_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("kodex_") && name.ends_with(".md") {
                let link = format!("- [{}]({})", name.trim_end_matches(".md"), name);
                if !content.contains(&link) {
                    content.push_str(&format!("{link}\n"));
                }
            }
        }
    }

    std::fs::write(index_path, content)?;
    Ok(())
}
