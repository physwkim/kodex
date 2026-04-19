use std::path::Path;

/// Save an AI-discovered insight as a vault note.
///
/// An insight connects multiple nodes with a named pattern or concept.
/// The vault .md file is the source of truth; HDF5/JSON are rebuilt from vault on next load.
pub fn save_insight(
    _graph_path: &Path,
    vault_path: Option<&Path>,
    label: &str,
    description: &str,
    node_ids: &[String],
    pattern: Option<&str>,
) -> crate::error::Result<()> {
    let vault = vault_path.unwrap_or_else(|| Path::new("kodex-out/vault"));
    write_insight_note(vault, label, description, node_ids, pattern)
}

/// Save a free-text note to the vault.
pub fn save_note(
    _graph_path: &Path,
    vault_path: Option<&Path>,
    title: &str,
    content: &str,
    related_nodes: &[String],
) -> crate::error::Result<()> {
    let vault = vault_path.unwrap_or_else(|| Path::new("kodex-out/vault"));
    write_free_note(vault, title, content, related_nodes)
}

/// Add a single edge — writes as a vault note linking source to target.
pub fn add_edge(
    _graph_path: &Path,
    source: &str,
    target: &str,
    relation: &str,
    description: Option<&str>,
) -> crate::error::Result<()> {
    // Edge-only additions don't need a separate file — they'll be
    // captured as wikilinks in existing node notes, or as an insight note.
    let vault = Path::new("kodex-out/vault");
    let label = format!("{source} → {target}");
    let desc = description.unwrap_or(relation);
    write_insight_note(
        vault,
        &label,
        desc,
        &[source.to_string(), target.to_string()],
        Some(relation),
    )
}

// --- Internal writers ---

fn write_insight_note(
    vault_dir: &Path,
    label: &str,
    description: &str,
    node_ids: &[String],
    pattern: Option<&str>,
) -> crate::error::Result<()> {
    std::fs::create_dir_all(vault_dir)?;
    let safe_name = safe_filename(label);
    let path = vault_dir.join(format!("_INSIGHT_{safe_name}.md"));

    let wikilinks: Vec<String> = node_ids.iter().map(|n| format!("[[{n}]]")).collect();
    let pattern_tag = pattern.map(|p| format!("#pattern/{p}")).unwrap_or_default();

    let md = format!(
        "---\n\
         type: insight\n\
         pattern: {pattern}\n\
         created_by: ai\n\
         tags: [kodex/insight, {pattern_tag}]\n\
         ---\n\n\
         # {label}\n\n\
         {description}\n\n\
         ## Related\n\n\
         {wikilinks}\n",
        pattern = pattern.unwrap_or("observation"),
        wikilinks = wikilinks.join("\n"),
    );

    std::fs::write(&path, md)?;
    Ok(())
}

fn write_free_note(
    vault_dir: &Path,
    title: &str,
    content: &str,
    related_nodes: &[String],
) -> crate::error::Result<()> {
    std::fs::create_dir_all(vault_dir)?;
    let safe_name = safe_filename(title);
    let path = vault_dir.join(format!("_NOTE_{safe_name}.md"));

    let wikilinks: Vec<String> = related_nodes.iter().map(|n| format!("- [[{n}]]")).collect();

    let md = format!(
        "---\n\
         type: note\n\
         created_by: ai\n\
         tags: [kodex/note]\n\
         ---\n\n\
         # {title}\n\n\
         {content}\n\n\
         ## Related\n\n\
         {related}\n",
        related = if wikilinks.is_empty() {
            "(none)".to_string()
        } else {
            wikilinks.join("\n")
        },
    );

    std::fs::write(&path, md)?;
    Ok(())
}

fn safe_filename(s: &str) -> String {
    s.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ' '], "_")
}
