use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{GraphifyError, Result};
use crate::types::{Edge, ExtractionResult};

const CONFIG_FILE: &str = "graphify-workspace.yaml";

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
        .map_err(|e| GraphifyError::Other(format!("Failed to read {}: {e}", path.display())))?;

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
        return Err(GraphifyError::Other(
            "No projects listed in workspace config".to_string(),
        ));
    }

    Ok(WorkspaceConfig {
        projects,
        output: output.unwrap_or_else(|| PathBuf::from("graphify-workspace")),
        vault,
    })
}

/// Generate a default workspace config file.
pub fn init(dir: &Path) -> Result<PathBuf> {
    let config_path = dir.join(CONFIG_FILE);
    if config_path.exists() {
        return Err(GraphifyError::Other(format!(
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
        "# graphify workspace configuration\n\
         \n\
         projects:\n\
         {projects_yaml}\n\
         \n\
         # Where to write merged graph.json, graph.html, report\n\
         output: ./graphify-workspace\n\
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

    println!("graphify workspace: {} projects", config.projects.len());

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
        return Err(GraphifyError::Other("No extractions produced".to_string()));
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

    println!("  done!");
    Ok(())
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
