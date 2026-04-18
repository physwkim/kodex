use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphifyError {
    #[error("Extraction error: {0}")]
    Extraction(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("URL error: {0}")]
    UrlValidation(String),

    #[error("Tree-sitter error: {0}")]
    TreeSitter(String),

    #[error("Graph path escapes allowed directory: {0}")]
    PathEscape(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, GraphifyError>;
