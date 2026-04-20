use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// FileType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Code,
    Document,
    Paper,
    Image,
    Video,
    Rationale,
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code => write!(f, "code"),
            Self::Document => write!(f, "document"),
            Self::Paper => write!(f, "paper"),
            Self::Image => write!(f, "image"),
            Self::Video => write!(f, "video"),
            Self::Rationale => write!(f, "rationale"),
        }
    }
}

impl FileType {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "code" => Some(Self::Code),
            "document" => Some(Self::Document),
            "paper" => Some(Self::Paper),
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "rationale" => Some(Self::Rationale),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Confidence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Confidence {
    EXTRACTED,
    INFERRED,
    AMBIGUOUS,
}

impl Confidence {
    pub fn default_score(&self) -> f64 {
        match self {
            Self::EXTRACTED => 1.0,
            Self::INFERRED => 0.5,
            Self::AMBIGUOUS => 0.2,
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s {
            "EXTRACTED" => Some(Self::EXTRACTED),
            "INFERRED" => Some(Self::INFERRED),
            "AMBIGUOUS" => Some(Self::AMBIGUOUS),
            _ => None,
        }
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EXTRACTED => write!(f, "EXTRACTED"),
            Self::INFERRED => write!(f, "INFERRED"),
            Self::AMBIGUOUS => write!(f, "AMBIGUOUS"),
        }
    }
}

// ---------------------------------------------------------------------------
// Node — code entity with stable UUID
// ---------------------------------------------------------------------------

fn default_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub file_type: FileType,
    pub source_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub norm_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degree: Option<usize>,
    /// Stable UUID — survives renames, moves, refactors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// Fingerprint for re-extraction matching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Human-readable logical key: project/module.py::Class.method
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_key: Option<String>,
    /// SHA256 digest of normalized function/class body (whitespace-stripped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_hash: Option<String>,
}

// ---------------------------------------------------------------------------
// Edge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: Confidence,
    pub source_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<f64>,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(default, rename = "_src", skip_serializing_if = "Option::is_none")]
    pub original_src: Option<String>,
    #[serde(default, rename = "_tgt", skip_serializing_if = "Option::is_none")]
    pub original_tgt: Option<String>,
}

// ---------------------------------------------------------------------------
// Hyperedge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hyperedge {
    #[serde(default)]
    pub id: String,
    pub label: String,
    pub nodes: Vec<String>,
    pub confidence: Confidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
}

// ---------------------------------------------------------------------------
// RawCall (unresolved cross-file call for second pass)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCall {
    pub caller_nid: String,
    pub callee: String,
    pub source_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
}

// ---------------------------------------------------------------------------
// ExtractionResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub hyperedges: Vec<Hyperedge>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_calls: Vec<RawCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// DetectionResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DetectionResult {
    pub files: DetectedFiles,
    pub total_files: usize,
    pub total_words: usize,
    pub needs_graph: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default)]
    pub skipped_sensitive: Vec<String>,
    #[serde(default)]
    pub kodexignore_patterns: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DetectedFiles {
    pub code: Vec<String>,
    pub document: Vec<String>,
    pub paper: Vec<String>,
    pub image: Vec<String>,
    pub video: Vec<String>,
}

// ---------------------------------------------------------------------------
// KnowledgeEntry — knowledge entity with its own UUID
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub uuid: String,
    pub title: String,
    pub knowledge_type: String,
    pub description: String,
    pub confidence: f64,
    pub observations: u32,
    pub tags: Vec<String>,
    /// Scope: repo / project / module / file / node
    #[serde(default)]
    pub scope: String,
    /// Status: active / tentative / obsolete / needs_review
    #[serde(default)]
    pub status: String,
    /// Origin: human / inferred / imported / agent
    #[serde(default)]
    pub source: String,
    /// Last time this was validated (unix timestamp, 0 = never)
    #[serde(default)]
    pub last_validated_at: u64,
    /// When this knowledge applies, e.g. "auth modification", "DB migration"
    #[serde(default)]
    pub applies_when: String,
    /// UUID of knowledge entry this supersedes
    #[serde(default)]
    pub supersedes: String,
    /// UUID of knowledge entry that supersedes this one
    #[serde(default)]
    pub superseded_by: String,
    /// Evidence: file/PR/commit/observation that produced this knowledge
    #[serde(default)]
    pub evidence: String,
    /// When first created (unix timestamp)
    #[serde(default)]
    pub created_at: u64,
    /// When last updated (unix timestamp)
    #[serde(default)]
    pub updated_at: u64,
}

// ---------------------------------------------------------------------------
// KnowledgeLink — knowledge ↔ node or knowledge ↔ knowledge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeLink {
    /// Source: always a knowledge UUID
    pub knowledge_uuid: String,
    /// Target: node UUID or another knowledge UUID
    pub node_uuid: String,
    /// Relationship type: related_to, depends_on, contradicts, supports, etc.
    pub relation: String,
    /// Target type: "node" (default) or "knowledge"
    #[serde(default)]
    pub target_type: String,
    /// Link confidence (0.0-1.0)
    #[serde(default)]
    pub confidence: f64,
    /// When this link was created (unix timestamp)
    #[serde(default)]
    pub created_at: u64,
    /// Snapshot of target node's body_hash at link creation time
    #[serde(default)]
    pub linked_body_hash: String,
}

impl KnowledgeLink {
    pub fn is_knowledge_link(&self) -> bool {
        self.target_type == "knowledge"
    }
}

// ---------------------------------------------------------------------------
// KodexData — complete h5 contents as a struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct KodexData {
    pub extraction: ExtractionResult,
    pub knowledge: Vec<KnowledgeEntry>,
    pub links: Vec<KnowledgeLink>,
}
