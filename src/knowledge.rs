use std::path::Path;

use crate::id::make_id;

/// Save an AI-discovered insight as a hyperedge + note in the graph and vault.
///
/// An insight connects multiple nodes with a named pattern or concept.
/// Example: "Observer pattern" linking EventBus, Listener, Handler.
pub fn save_insight(
    graph_path: &Path,
    vault_path: Option<&Path>,
    label: &str,
    description: &str,
    node_ids: &[String],
    pattern: Option<&str>,
) -> crate::error::Result<()> {
    let mut data = load_graph_json(graph_path)?;

    let insight_id = make_id(&["insight", label]);
    let now = timestamp();

    // Add insight node
    let nodes = data
        .get_mut("nodes")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| crate::error::GraphifyError::Other("Invalid graph.json".to_string()))?;

    // Check if insight already exists
    let exists = nodes.iter().any(|n| {
        n.get("id").and_then(|v| v.as_str()) == Some(&insight_id)
    });

    if !exists {
        nodes.push(serde_json::json!({
            "id": insight_id,
            "label": label,
            "file_type": "rationale",
            "source_file": "ai_insight",
            "source_location": format!("T{now}"),
            "confidence": "INFERRED",
            "confidence_score": 0.9,
            "insight_type": pattern.unwrap_or("observation"),
            "description": description,
        }));
    }

    // Add edges from insight to each referenced node
    let links = data
        .get_mut("links")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| crate::error::GraphifyError::Other("Invalid graph.json".to_string()))?;

    for nid in node_ids {
        links.push(serde_json::json!({
            "source": insight_id,
            "target": nid,
            "relation": "insight_about",
            "confidence": "INFERRED",
            "source_file": "ai_insight",
            "confidence_score": 0.9,
            "weight": 0.9,
        }));
    }

    // Add as hyperedge too
    let hyperedges = data
        .get_mut("hyperedges")
        .and_then(|v| v.as_array_mut());
    if let Some(hyper) = hyperedges {
        hyper.push(serde_json::json!({
            "id": insight_id,
            "label": label,
            "nodes": node_ids,
            "confidence": "INFERRED",
            "confidence_score": 0.9,
            "description": description,
        }));
    }

    save_graph_json(graph_path, &data)?;

    // Write vault note if vault path provided
    if let Some(vp) = vault_path {
        write_insight_note(vp, label, description, node_ids, pattern)?;
    }

    Ok(())
}

/// Save a free-text note to the vault and link it to the graph.
pub fn save_note(
    graph_path: &Path,
    vault_path: Option<&Path>,
    title: &str,
    content: &str,
    related_nodes: &[String],
) -> crate::error::Result<()> {
    let mut data = load_graph_json(graph_path)?;
    let note_id = make_id(&["note", title]);
    let now = timestamp();

    // Add note node
    let nodes = data
        .get_mut("nodes")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| crate::error::GraphifyError::Other("Invalid graph.json".to_string()))?;

    let exists = nodes.iter().any(|n| {
        n.get("id").and_then(|v| v.as_str()) == Some(&note_id)
    });

    if !exists {
        nodes.push(serde_json::json!({
            "id": note_id,
            "label": title,
            "file_type": "document",
            "source_file": "ai_note",
            "source_location": format!("T{now}"),
            "confidence": "INFERRED",
            "confidence_score": 0.9,
        }));
    }

    // Link to related nodes
    let links = data
        .get_mut("links")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| crate::error::GraphifyError::Other("Invalid graph.json".to_string()))?;

    for nid in related_nodes {
        links.push(serde_json::json!({
            "source": note_id,
            "target": nid,
            "relation": "documents",
            "confidence": "INFERRED",
            "source_file": "ai_note",
            "confidence_score": 0.9,
            "weight": 0.9,
        }));
    }

    save_graph_json(graph_path, &data)?;

    // Write vault note
    if let Some(vp) = vault_path {
        write_free_note(vp, title, content, related_nodes)?;
    }

    Ok(())
}

/// Add a single edge (relationship) to the graph.
pub fn add_edge(
    graph_path: &Path,
    source: &str,
    target: &str,
    relation: &str,
    description: Option<&str>,
) -> crate::error::Result<()> {
    let mut data = load_graph_json(graph_path)?;

    let links = data
        .get_mut("links")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| crate::error::GraphifyError::Other("Invalid graph.json".to_string()))?;

    links.push(serde_json::json!({
        "source": source,
        "target": target,
        "relation": relation,
        "confidence": "INFERRED",
        "source_file": "ai_added",
        "confidence_score": 0.9,
        "weight": 0.9,
        "description": description.unwrap_or(""),
    }));

    save_graph_json(graph_path, &data)
}

// --- Internal helpers ---

fn load_graph_json(path: &Path) -> crate::error::Result<serde_json::Value> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(Into::into)
}

fn save_graph_json(path: &Path, data: &serde_json::Value) -> crate::error::Result<()> {
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| crate::error::GraphifyError::Other(format!("JSON error: {e}")))?;
    // Atomic write: temp file + rename
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path).or_else(|_| {
        std::fs::copy(&tmp, path)?;
        let _ = std::fs::remove_file(&tmp);
        Ok(())
    })
}

fn write_insight_note(
    vault_dir: &Path,
    label: &str,
    description: &str,
    node_ids: &[String],
    pattern: Option<&str>,
) -> crate::error::Result<()> {
    std::fs::create_dir_all(vault_dir)?;
    let safe_name = label.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ' '], "_");
    let path = vault_dir.join(format!("_INSIGHT_{safe_name}.md"));

    let wikilinks: Vec<String> = node_ids.iter().map(|n| format!("[[{n}]]")).collect();
    let pattern_tag = pattern
        .map(|p| format!("#pattern/{p}"))
        .unwrap_or_default();

    let md = format!(
        "---\n\
         type: insight\n\
         pattern: {pattern}\n\
         created_by: ai\n\
         tags: [graphify/insight, {pattern_tag}]\n\
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
    let safe_name = title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ' '], "_");
    let path = vault_dir.join(format!("_NOTE_{safe_name}.md"));

    let wikilinks: Vec<String> = related_nodes.iter().map(|n| format!("- [[{n}]]")).collect();

    let md = format!(
        "---\n\
         type: note\n\
         created_by: ai\n\
         tags: [graphify/note]\n\
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

fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
