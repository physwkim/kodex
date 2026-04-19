use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

const PAPER_SIGNAL_THRESHOLD: usize = 3;

static PAPER_SIGNALS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)\barxiv\b").expect("invalid regex"),
        Regex::new(r"(?i)\bdoi\s*:").expect("invalid regex"),
        Regex::new(r"(?i)\babstract\b").expect("invalid regex"),
        Regex::new(r"(?i)\bproceedings\b").expect("invalid regex"),
        Regex::new(r"(?i)\bjournal\b").expect("invalid regex"),
        Regex::new(r"(?i)\bpreprint\b").expect("invalid regex"),
        Regex::new(r"\\cite\{").expect("invalid regex"),
        Regex::new(r"\[\d+\]").expect("invalid regex"),
        Regex::new(r"(?i)eq\.\s*\d+|equation\s+\d+").expect("invalid regex"),
        Regex::new(r"\d{4}\.\d{4,5}").expect("invalid regex"),
        Regex::new(r"(?i)\bwe propose\b").expect("invalid regex"),
        Regex::new(r"(?i)\bliterature\b").expect("invalid regex"),
    ]
});

/// Heuristic: does this file look like an academic paper?
/// For PDF files, we try to extract text; for text files, we read directly.
pub fn looks_like_paper(path: &Path) -> bool {
    // For now, only check text-based files (PDFs need a separate extractor)
    let text = match path.extension().and_then(|e| e.to_str()) {
        Some("pdf") => {
            // PDF text extraction would go here (feature-gated)
            // For now, assume PDFs could be papers
            return true;
        }
        _ => match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return false,
        },
    };

    let count = PAPER_SIGNALS
        .iter()
        .filter(|pat| pat.is_match(&text))
        .count();

    count >= PAPER_SIGNAL_THRESHOLD
}
