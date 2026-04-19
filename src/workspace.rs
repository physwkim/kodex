use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{EngramError, Result};
use crate::types::{Edge, ExtractionResult};

const CONFIG_FILE: &str = "engram-workspace.yaml";

/// Workspace configuration.
#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    pub projects: Vec<PathBuf>,
    pub output: PathBuf,
    pub vault: Option<PathBuf>,
}

/// Parse a workspace config from YAML-like format.
/// Keeps it simple — no serde_yaml dependency.
pub fn load_config(path: &Path) -> Result<WorkspaceConfig> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| EngramError::Other(format!("Failed to read {}: {e}", path.display())))?;

    let mut projects = Vec::new();
    let mut output = None;
    let mut vault = None;
    let mut in_projects = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with("projects:") {
            in_projects = true;
            continue;
        }
        if in_projects && trimmed.starts_with("- ") {
            let p = trimmed.trim_start_matches("- ").trim();
            let expanded = shellexpand(p);
            projects.push(PathBuf::from(expanded));
            continue;
        }
        if in_projects && !trimmed.starts_with('-') {
            in_projects = false;
        }

        if let Some(val) = trimmed.strip_prefix("output:") {
            output = Some(PathBuf::from(shellexpand(val.trim())));
        }
        if let Some(val) = trimmed.strip_prefix("vault:") {
            vault = Some(PathBuf::from(shellexpand(val.trim())));
        }
    }

    if projects.is_empty() {
        return Err(EngramError::Other(
            "No projects listed in workspace config".to_string(),
        ));
    }

    Ok(WorkspaceConfig {
        projects,
        output: output.unwrap_or_else(|| PathBuf::from("engram-workspace")),
        vault,
    })
}

/// Generate a default workspace config file.
pub fn init(dir: &Path) -> Result<PathBuf> {
    let config_path = dir.join(CONFIG_FILE);
    if config_path.exists() {
        return Err(EngramError::Other(format!(
            "{CONFIG_FILE} already exists at {}",
            config_path.display()
        )));
    }

    // Auto-detect projects: subdirectories with .git
    let mut projects = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join(".git").is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    projects.push(format!("./{name}"));
                }
            }
        }
    }
    projects.sort();

    let projects_yaml = if projects.is_empty() {
        "  - ./project-a\n  - ./project-b".to_string()
    } else {
        projects
            .iter()
            .map(|p| format!("  - {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let content = format!(
        "# engram workspace configuration\n\
         \n\
         projects:\n\
         {projects_yaml}\n\
         \n\
         # Where to write merged graph.json, graph.html, report\n\
         output: ./engram-workspace\n\
         \n\
         # Where to write the unified Obsidian vault (optional)\n\
         # vault: ~/obsidian-vault/dev-knowledge\n"
    );

    std::fs::write(&config_path, content)?;
    Ok(config_path)
}

/// Find workspace config by walking up from the given directory.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(CONFIG_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Run the workspace: build each project, merge graphs, export.
pub fn run(
    config: &WorkspaceConfig,
    vault_override: Option<&Path>,
) -> Result<()> {
    let vault_path = vault_override
        .map(PathBuf::from)
        .or_else(|| config.vault.clone());

    std::fs::create_dir_all(&config.output)?;

    println!("engram workspace: {} projects", config.projects.len());

    // Step 1: Build each project
    let mut all_extractions: Vec<(String, ExtractionResult)> = Vec::new();

    for project_path in &config.projects {
        if !project_path.is_dir() {
            eprintln!("  skip: {} (not a directory)", project_path.display());
            continue;
        }

        let project_name = project_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        println!("  building: {project_name}...");

        // Detect
        let detection = crate::detect::detect(project_path, false);
        let code_paths: Vec<PathBuf> = detection.files.code.iter().map(PathBuf::from).collect();

        if code_paths.is_empty() {
            println!("    no code files, skipping");
            continue;
        }

        // Extract
        #[cfg(feature = "extract")]
        {
            let mut extraction = crate::extract::extract(&code_paths, Some(project_path));

            // Tag nodes with project name
            for node in &mut extraction.nodes {
                node.source_file = format!("{project_name}/{}", node.source_file
                    .strip_prefix(project_path.to_str().unwrap_or(""))
                    .unwrap_or(&node.source_file)
                    .trim_start_matches('/'));
            }
            for edge in &mut extraction.edges {
                edge.source_file = format!("{project_name}/{}", edge.source_file
                    .strip_prefix(project_path.to_str().unwrap_or(""))
                    .unwrap_or(&edge.source_file)
                    .trim_start_matches('/'));
            }

            println!(
                "    {} nodes, {} edges",
                extraction.nodes.len(),
                extraction.edges.len()
            );
            all_extractions.push((project_name, extraction));
        }

        #[cfg(not(feature = "extract"))]
        {
            println!("    extract feature not enabled");
        }
    }

    if all_extractions.is_empty() {
        return Err(EngramError::Other("No extractions produced".to_string()));
    }

    // Step 2: Merge into unified graph
    println!("\n  merging {} projects...", all_extractions.len());

    let mut merged = ExtractionResult::default();
    let mut cross_project_edges: Vec<Edge> = Vec::new();

    // Collect all nodes, detect shared names across projects
    let mut name_to_projects: HashMap<String, Vec<(String, String)>> = HashMap::new(); // label → [(project, node_id)]

    for (project_name, extraction) in &all_extractions {
        for node in &extraction.nodes {
            // Skip file-level nodes for cross-project matching
            if !node.label.ends_with("()") && !node.label.contains('.') {
                name_to_projects
                    .entry(node.label.clone())
                    .or_default()
                    .push((project_name.clone(), node.id.clone()));
            }
        }
        merged.nodes.extend(extraction.nodes.clone());
        merged.edges.extend(extraction.edges.clone());
    }

    // Create cross-project edges for shared names
    for (_label, occurrences) in &name_to_projects {
        if occurrences.len() < 2 {
            continue;
        }
        // Connect first occurrence to all others
        let (ref first_project, ref first_id) = occurrences[0];
        for (project, id) in &occurrences[1..] {
            cross_project_edges.push(Edge {
                source: first_id.clone(),
                target: id.clone(),
                relation: "shared_across_projects".to_string(),
                confidence: crate::types::Confidence::INFERRED,
                source_file: format!("{first_project} ↔ {project}"),
                source_location: None,
                confidence_score: Some(0.7),
                weight: 0.7,
                original_src: None,
                original_tgt: None,
            });
        }
    }

    let cross_count = cross_project_edges.len();
    merged.edges.extend(cross_project_edges);

    // Step 3: Build unified graph
    let graph = crate::graph::build_from_extraction(&merged);
    let communities = crate::cluster::cluster(&graph);
    let cohesion = crate::cluster::score_all(&graph, &communities);
    let gods = crate::analyze::god_nodes(&graph, 10);
    let surprises = crate::analyze::surprising_connections(&graph, Some(&communities), 5);
    let questions = crate::analyze::suggest_questions(&graph, Some(&communities), 7);

    let community_labels: HashMap<usize, String> = communities
        .iter()
        .map(|(&cid, nodes)| {
            let label = nodes
                .first()
                .and_then(|nid| graph.get_node(nid))
                .map(|n| n.label.clone())
                .unwrap_or_else(|| format!("Community {cid}"));
            (cid, label)
        })
        .collect();

    println!(
        "  merged: {} nodes, {} edges ({cross_count} cross-project), {} communities",
        graph.node_count(),
        graph.edge_count(),
        communities.len()
    );

    // Step 4: Export
    let _ = crate::export::to_json(&graph, &communities, &config.output.join("graph.json"));
    let _ = crate::export::to_html(
        &graph,
        &communities,
        &config.output.join("graph.html"),
        Some(&community_labels),
    );

    let detection = crate::types::DetectionResult::default();
    let report = crate::report::generate(
        &graph,
        &communities,
        &cohesion,
        &community_labels,
        &gods,
        &surprises,
        &detection,
        0,
        0,
        "workspace",
        Some(&questions),
    );
    let _ = std::fs::write(config.output.join("GRAPH_REPORT.md"), &report);

    println!("  output: {}", config.output.display());

    // Step 5: Export Obsidian vault (if configured)
    if let Some(vault_dir) = &vault_path {
        std::fs::create_dir_all(vault_dir)?;
        let count = crate::export::to_obsidian(
            &graph,
            &communities,
            vault_dir,
            Some(&community_labels),
            Some(&cohesion),
        )?;
        println!("  vault: {} ({count} notes)", vault_dir.display());
    }

    // Step 6: Collect knowledge from all projects into unified vault
    let unified_vault = vault_path.as_deref().unwrap_or(&config.output);
    let collected = collect_knowledge(&config.projects, unified_vault)?;
    if collected > 0 {
        println!("  knowledge: collected {collected} items from projects into unified vault");
    }

    // Step 7: Distribute unified knowledge back to each project
    let distributed = distribute_knowledge(unified_vault, &config.projects)?;
    if distributed > 0 {
        println!("  knowledge: distributed {distributed} items back to projects");
    }

    println!("  done!");
    Ok(())
}

/// Collect _KNOWLEDGE_*.md from each project's engram-out/ into the unified vault.
/// Files are prefixed with project name to avoid collisions.
fn collect_knowledge(projects: &[PathBuf], unified_vault: &Path) -> Result<usize> {
    let mut count = 0;

    for project_path in projects {
        let project_name = project_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let source_dir = project_path.join("engram-out");
        if !source_dir.is_dir() {
            continue;
        }

        let entries = match std::fs::read_dir(&source_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // Collect _KNOWLEDGE_*, _INSIGHT_*, _NOTE_* files
            if !filename.starts_with("_KNOWLEDGE_")
                && !filename.starts_with("_INSIGHT_")
                && !filename.starts_with("_NOTE_")
            {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Add project origin tag to frontmatter if not present
            let tagged_content = if content.contains(&format!("origin: {project_name}")) {
                content
            } else {
                inject_frontmatter_field(&content, "origin", project_name)
            };

            // Write to unified vault (project-prefixed to avoid collision)
            let dest_filename = if filename.contains(project_name) {
                filename
            } else {
                // Insert project name: _KNOWLEDGE_Foo.md → _KNOWLEDGE_project_Foo.md
                let prefix_end = filename.find('_').unwrap_or(0);
                let second_underscore = filename[prefix_end + 1..].find('_').map(|p| p + prefix_end + 1);
                match second_underscore {
                    Some(pos) => format!("{}{}_{}", &filename[..pos + 1], project_name, &filename[pos..]),
                    None => format!("{project_name}_{filename}"),
                }
            };

            let dest = unified_vault.join(&dest_filename);

            // Only copy if source is newer or dest doesn't exist
            let should_copy = if dest.exists() {
                let src_mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let dst_mtime = std::fs::metadata(&dest)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                src_mtime > dst_mtime
            } else {
                true
            };

            if should_copy {
                std::fs::write(&dest, tagged_content)?;
                count += 1;
            }
        }
    }

    Ok(count)
}

/// Distribute knowledge from the unified vault back to each project.
/// Only distributes knowledge that originated from OTHER projects,
/// so each project gains cross-project knowledge.
fn distribute_knowledge(unified_vault: &Path, projects: &[PathBuf]) -> Result<usize> {
    let mut count = 0;

    // Read all knowledge files from unified vault
    let entries = match std::fs::read_dir(unified_vault) {
        Ok(e) => e,
        Err(_) => return Ok(0),
    };

    let mut knowledge_files: Vec<(String, String, String)> = Vec::new(); // (filename, content, origin)

    for entry in entries.flatten() {
        let path = entry.path();
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if !filename.starts_with("_KNOWLEDGE_")
            && !filename.starts_with("_INSIGHT_")
            && !filename.starts_with("_NOTE_")
        {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Extract origin from frontmatter
        let origin = extract_frontmatter_field(&content, "origin")
            .unwrap_or_default();

        knowledge_files.push((filename, content, origin));
    }

    // Distribute to each project
    for project_path in projects {
        let project_name = project_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let dest_dir = project_path.join("engram-out");
        if !dest_dir.is_dir() {
            let _ = std::fs::create_dir_all(&dest_dir);
        }

        for (filename, content, origin) in &knowledge_files {
            // Don't distribute knowledge back to its own project
            if origin == project_name {
                continue;
            }

            let dest = dest_dir.join(filename);
            if !dest.exists() {
                std::fs::write(&dest, content)?;
                count += 1;
            }
        }
    }

    Ok(count)
}

fn inject_frontmatter_field(content: &str, key: &str, value: &str) -> String {
    if !content.starts_with("---") {
        return format!("---\n{key}: {value}\n---\n\n{content}");
    }
    // Insert field before closing ---
    if let Some(end) = content[3..].find("\n---") {
        let insert_pos = 3 + end;
        let mut result = String::new();
        result.push_str(&content[..insert_pos]);
        result.push_str(&format!("\n{key}: {value}"));
        result.push_str(&content[insert_pos..]);
        return result;
    }
    content.to_string()
}

fn extract_frontmatter_field(content: &str, key: &str) -> Option<String> {
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("\n---")?;
    let fm = &content[4..3 + end];
    for line in fm.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim() == key {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Expand ~ to home directory.
fn shellexpand(s: &str) -> String {
    if s.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &s[1..]);
        }
    }
    s.to_string()
}
