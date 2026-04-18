use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Code,
    Document,
    Paper,
    Image,
    Video,
    Office,
}

pub static CODE_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        ".py", ".ts", ".js", ".jsx", ".tsx", ".mjs", ".ejs",
        ".go", ".rs", ".java", ".cpp", ".cc", ".cxx", ".c", ".h", ".hpp",
        ".rb", ".swift", ".kt", ".kts", ".cs", ".scala", ".php",
        ".lua", ".toc", ".zig", ".ps1", ".ex", ".exs",
        ".m", ".mm", ".jl", ".vue", ".svelte", ".dart", ".v", ".sv",
    ]
    .into_iter()
    .collect()
});

pub static DOC_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [".md", ".mdx", ".txt", ".rst", ".html"]
        .into_iter()
        .collect()
});

pub static PAPER_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [".pdf"].into_iter().collect()
});

pub static IMAGE_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"]
        .into_iter()
        .collect()
});

pub static VIDEO_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        ".mp4", ".mov", ".webm", ".mkv", ".avi", ".m4v",
        ".mp3", ".wav", ".m4a", ".ogg",
    ]
    .into_iter()
    .collect()
});

pub static OFFICE_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [".docx", ".xlsx"].into_iter().collect()
});

/// Classify a file by its extension.
pub fn classify_file(path: &Path) -> Option<FileCategory> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    let dotted = format!(".{ext}");
    let ext_str = dotted.as_str();

    if CODE_EXTENSIONS.contains(ext_str) {
        return Some(FileCategory::Code);
    }
    if DOC_EXTENSIONS.contains(ext_str) {
        return Some(FileCategory::Document);
    }
    if PAPER_EXTENSIONS.contains(ext_str) {
        // PDF: check if it looks like a paper or a generic document
        if super::looks_like_paper(path) {
            return Some(FileCategory::Paper);
        }
        return Some(FileCategory::Document);
    }
    if IMAGE_EXTENSIONS.contains(ext_str) {
        return Some(FileCategory::Image);
    }
    if VIDEO_EXTENSIONS.contains(ext_str) {
        return Some(FileCategory::Video);
    }
    if OFFICE_EXTENSIONS.contains(ext_str) {
        return Some(FileCategory::Office);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_classify_code() {
        assert_eq!(classify_file(&PathBuf::from("main.py")), Some(FileCategory::Code));
        assert_eq!(classify_file(&PathBuf::from("app.tsx")), Some(FileCategory::Code));
        assert_eq!(classify_file(&PathBuf::from("lib.rs")), Some(FileCategory::Code));
    }

    #[test]
    fn test_classify_document() {
        assert_eq!(classify_file(&PathBuf::from("README.md")), Some(FileCategory::Document));
        assert_eq!(classify_file(&PathBuf::from("notes.txt")), Some(FileCategory::Document));
    }

    #[test]
    fn test_classify_image() {
        assert_eq!(classify_file(&PathBuf::from("logo.png")), Some(FileCategory::Image));
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(classify_file(&PathBuf::from("data.bin")), None);
    }
}
