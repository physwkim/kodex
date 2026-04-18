use std::path::Path;

/// Check if a string looks like a URL.
pub fn is_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://") || path.starts_with("www.")
}

/// Download audio from a URL using yt-dlp (shell out).
pub fn download_audio(url: &str, output_dir: &Path) -> crate::error::Result<std::path::PathBuf> {
    std::fs::create_dir_all(output_dir)?;

    let output = std::process::Command::new("yt-dlp")
        .args([
            "-x",
            "--audio-format",
            "mp3",
            "-o",
            &format!("{}/%(title)s.%(ext)s", output_dir.display()),
            url,
        ])
        .output()
        .map_err(|e| {
            crate::error::GraphifyError::Other(format!(
                "yt-dlp not found or failed: {e}. Install with: pip install yt-dlp"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::error::GraphifyError::Other(format!(
            "yt-dlp failed: {stderr}"
        )));
    }

    // Find the downloaded file
    let entries: Vec<_> = std::fs::read_dir(output_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "mp3" || ext == "m4a" || ext == "opus")
                .unwrap_or(false)
        })
        .collect();

    entries
        .last()
        .map(|e| e.path())
        .ok_or_else(|| crate::error::GraphifyError::Other("No audio file found after download".to_string()))
}

/// Build a Whisper prompt from god node labels for domain-aware transcription.
pub fn build_whisper_prompt(god_node_labels: &[String]) -> String {
    if god_node_labels.is_empty() {
        return "Use proper punctuation and paragraph breaks.".to_string();
    }
    let terms = god_node_labels
        .iter()
        .take(10)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "This audio discusses: {terms}. Use proper punctuation and paragraph breaks."
    )
}
