//! Generate hierarchy nodes from file paths.
//!
//! Walks unique source_file paths and creates directory/crate/module nodes
//! with "contains" edges to form a navigable project tree.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::id::make_id;
use crate::types::{Confidence, Edge, ExtractionResult, FileType, Node};

/// Add hierarchy nodes (project → crate → module → file) to an extraction result.
/// Detects crate boundaries by checking for Cargo.toml in directories.
pub fn add_hierarchy(extraction: &mut ExtractionResult, project_root: &Path) {
    let mut seen_dirs: HashSet<String> = HashSet::new();
    let mut dir_nodes: Vec<Node> = Vec::new();
    let mut dir_edges: Vec<Edge> = Vec::new();

    // Collect all unique directory paths from existing nodes
    let source_files: Vec<String> = extraction
        .nodes
        .iter()
        .map(|n| n.source_file.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // Detect crate directories (contain Cargo.toml)
    let crate_dirs = detect_crate_dirs(project_root);

    // File node IDs for linking
    let file_node_ids: HashMap<String, String> = extraction
        .nodes
        .iter()
        .filter(|n| n.source_location.as_deref() == Some("L1"))
        .map(|n| (n.source_file.clone(), n.id.clone()))
        .collect();

    for source_file in &source_files {
        let parts: Vec<&str> = source_file.split('/').collect();
        if parts.len() < 2 {
            continue;
        }

        // Build hierarchy: project/crate/src/module/.../file
        let mut current_path = String::new();
        let mut parent_id: Option<String> = None;

        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;

            if is_last {
                // File level — link parent dir → existing file node
                if let Some(parent) = &parent_id {
                    if let Some(file_nid) = file_node_ids.get(source_file) {
                        dir_edges.push(make_contains_edge(parent, file_nid, source_file));
                    }
                }
                continue;
            }

            if !current_path.is_empty() {
                current_path.push('/');
            }
            current_path.push_str(part);

            // Skip "src" directories (noise)
            if *part == "src" {
                continue;
            }

            let dir_id = make_id(&[&current_path]);

            if seen_dirs.insert(dir_id.clone()) {
                // Determine node type
                let is_crate = crate_dirs.contains(&current_path);
                let label = part.to_string();
                let file_type = if i == 0 {
                    FileType::Document // project root
                } else {
                    FileType::Code
                };

                let mut node = Node {
                    id: dir_id.clone(),
                    label: label.clone(),
                    file_type,
                    source_file: current_path.clone(),
                    source_location: None,
                    confidence: Some(Confidence::EXTRACTED),
                    confidence_score: Some(1.0),
                    community: None,
                    norm_label: if is_crate {
                        Some("crate".to_string())
                    } else {
                        Some("module".to_string())
                    },
                    degree: None,
                    uuid: None,
                    fingerprint: None,
                    logical_key: Some(current_path.clone()),
                    body_hash: None,
                };

                // Generate fingerprint
                node.fingerprint = Some(crate::fingerprint::compute_fingerprint(
                    &label,
                    &node.file_type.to_string(),
                    &current_path,
                    None,
                    None,
                ));

                dir_nodes.push(node);
            }

            // Link parent → child
            if let Some(parent) = &parent_id {
                let edge_key = format!("{parent}→{dir_id}");
                if seen_dirs.insert(edge_key) {
                    dir_edges.push(make_contains_edge(parent, &dir_id, &current_path));
                }
            }

            parent_id = Some(dir_id);
        }
    }

    extraction.nodes.extend(dir_nodes);
    extraction.edges.extend(dir_edges);
}

fn make_contains_edge(source: &str, target: &str, source_file: &str) -> Edge {
    Edge {
        source: source.to_string(),
        target: target.to_string(),
        relation: "contains".to_string(),
        confidence: Confidence::EXTRACTED,
        source_file: source_file.to_string(),
        source_location: None,
        confidence_score: Some(1.0),
        weight: 1.0,
        original_src: None,
        original_tgt: None,
    }
}

/// Package marker files — any of these indicates a package/crate/module root.
const PACKAGE_MARKERS: &[&str] = &[
    // Rust
    "Cargo.toml",
    // Python
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    // JavaScript / TypeScript
    "package.json",
    // Go
    "go.mod",
    // Java
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    // C#
    // .csproj handled separately (glob)
    // Ruby
    "Gemfile",
    // PHP
    "composer.json",
    // Elixir
    "mix.exs",
    // Swift
    "Package.swift",
    // Dart
    "pubspec.yaml",
];

/// Find directories containing package markers (crate/module/package roots).
fn detect_crate_dirs(project_root: &Path) -> HashSet<String> {
    let mut packages = HashSet::new();
    let project_name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    walk_for_packages(project_root, project_root, project_name, &mut packages);
    packages
}

fn walk_for_packages(dir: &Path, root: &Path, project_name: &str, packages: &mut HashSet<String>) {
    // Check for any package marker file
    let has_marker = PACKAGE_MARKERS.iter().any(|m| dir.join(m).exists())
        || has_csproj(dir)
        || dir.join("__init__.py").exists();

    if has_marker {
        let rel = dir
            .strip_prefix(root)
            .unwrap_or(dir)
            .to_string_lossy()
            .to_string();
        let path = if rel.is_empty() {
            project_name.to_string()
        } else {
            format!("{project_name}/{rel}")
        };
        packages.insert(path);
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // Skip hidden dirs, build outputs, dependency dirs
                if !name.starts_with('.')
                    && name != "target"
                    && name != "node_modules"
                    && name != "__pycache__"
                    && name != "vendor"
                    && name != "build"
                    && name != "dist"
                    && name != "bin"
                    && name != "obj"
                {
                    walk_for_packages(&p, root, project_name, packages);
                }
            }
        }
    }
}

/// Check if directory contains a .csproj file (C# project).
fn has_csproj(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "csproj")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
