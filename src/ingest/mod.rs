use std::path::{Path, PathBuf};
#[cfg(feature = "fetch")]
use crate::security::sanitize_label;

/// URL type classification.
pub fn detect_url_type(url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.contains("twitter.com") || lower.contains("x.com") {
        "tweet"
    } else if lower.contains("arxiv.org") {
        "arxiv"
    } else if lower.contains("github.com") {
        "github"
    } else if lower.contains("youtube.com") || lower.contains("youtu.be") {
        "youtube"
    } else if lower.ends_with(".pdf") {
        "pdf"
    } else if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
    {
        "image"
    } else {
        "webpage"
    }
}

/// Fetch a URL and save it as annotated markdown.
#[cfg(feature = "fetch")]
pub fn ingest(
    url: &str,
    target_dir: &Path,
    author: Option<&str>,
    contributor: Option<&str>,
) -> crate::error::Result<PathBuf> {
    crate::security::validate_url(url)?;
    std::fs::create_dir_all(target_dir)?;

    let url_type = detect_url_type(url);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Fetch content
    let client = reqwest::blocking::Client::builder()
        .user_agent("engram/1.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| crate::error::EngramError::Other(format!("HTTP client error: {e}")))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| crate::error::EngramError::Other(format!("Fetch failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(crate::error::EngramError::Other(format!(
            "HTTP {status} for {url}"
        )));
    }

    let body = response
        .text()
        .map_err(|e| crate::error::EngramError::Other(format!("Read body failed: {e}")))?;

    // Build safe filename
    let safe: String = url
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(50)
        .collect();
    let filename = format!("{url_type}_{safe}_{now}.md");
    let out_path = target_dir.join(&filename);

    // Strip HTML for web pages (basic)
    let content_body = if url_type == "webpage" || url_type == "github" {
        // Simple HTML tag stripping
        let re = regex::Regex::new(r"<[^>]+>").unwrap();
        let stripped = re.replace_all(&body, "");
        let title = extract_html_title(&body).unwrap_or_else(|| url.to_string());
        format!("# {}\n\n{}", sanitize_label(&title), stripped.trim())
    } else {
        body
    };

    // Write markdown with YAML frontmatter
    let mut md = String::new();
    md.push_str("---\n");
    md.push_str(&format!("source_url: \"{url}\"\n"));
    md.push_str(&format!("type: {url_type}\n"));
    if let Some(a) = author {
        md.push_str(&format!("author: \"{a}\"\n"));
    }
    if let Some(c) = contributor {
        md.push_str(&format!("contributor: \"{c}\"\n"));
    }
    md.push_str(&format!("captured_at: {now}\n"));
    md.push_str("---\n\n");
    md.push_str(&content_body);

    std::fs::write(&out_path, md)?;
    Ok(out_path)
}

#[cfg(feature = "fetch")]
fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title>")? + 7;
    let end = lower[start..].find("</title>")? + start;
    Some(html[start..end].trim().to_string())
}

/// Save a Q&A result as markdown for the feedback loop.
pub fn save_query_result(
    question: &str,
    answer: &str,
    memory_dir: &Path,
    query_type: &str,
    source_nodes: Option<&[String]>,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(memory_dir)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = format!("{query_type}_{now}.md");
    let path = memory_dir.join(&filename);

    let mut content = String::new();
    content.push_str("---\n");
    content.push_str(&format!("type: {query_type}\n"));
    content.push_str(&format!("timestamp: {now}\n"));
    if let Some(nodes) = source_nodes {
        content.push_str(&format!("source_nodes: [{}]\n", nodes.join(", ")));
    }
    content.push_str("---\n\n");
    content.push_str(&format!("## Question\n\n{question}\n\n"));
    content.push_str(&format!("## Answer\n\n{answer}\n"));

    std::fs::write(&path, content)?;
    Ok(path)
}
