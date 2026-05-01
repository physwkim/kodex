use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Knowledge types that Claude can accumulate
// ---------------------------------------------------------------------------

/// Categories of learnable knowledge.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeType {
    Architecture,
    Pattern,
    Decision,
    Convention,
    Coupling,
    Domain,
    Preference,
    BugPattern,
    TechDebt,
    Ops,
    Api,
    Performance,
    Roadmap,
    Context,
    Lesson,
    /// Any type not in the enum — stored as-is
    #[serde(untagged)]
    Custom(String),
}

impl std::fmt::Display for KnowledgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Architecture => write!(f, "architecture"),
            Self::Pattern => write!(f, "pattern"),
            Self::Decision => write!(f, "decision"),
            Self::Convention => write!(f, "convention"),
            Self::Coupling => write!(f, "coupling"),
            Self::Domain => write!(f, "domain"),
            Self::Preference => write!(f, "preference"),
            Self::BugPattern => write!(f, "bug_pattern"),
            Self::TechDebt => write!(f, "tech_debt"),
            Self::Ops => write!(f, "ops"),
            Self::Api => write!(f, "api"),
            Self::Performance => write!(f, "performance"),
            Self::Roadmap => write!(f, "roadmap"),
            Self::Context => write!(f, "context"),
            Self::Lesson => write!(f, "lesson"),
            Self::Custom(s) => write!(f, "{s}"),
        }
    }
}

/// A piece of learned knowledge.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Knowledge {
    pub uuid: String,
    pub knowledge_type: KnowledgeType,
    pub title: String,
    pub description: String,
    /// Node IDs this knowledge relates to
    pub related_nodes: Vec<String>,
    /// How confident: 0.0–1.0 (accumulates with repeated observations)
    pub confidence: f64,
    /// How many times this was observed
    pub observations: u32,
    /// Tags for querying
    pub tags: Vec<String>,
    /// When first created (unix timestamp)
    pub created_at: u64,
    /// When last updated (unix timestamp)
    pub updated_at: u64,
}

// ---------------------------------------------------------------------------
// Knowledge store — reads/writes vault .md files
// ---------------------------------------------------------------------------

/// Store or reinforce a piece of knowledge directly in SQLite.
///
/// SQLite is the source of truth. If knowledge with the same title exists,
/// increments observations and raises confidence.
pub fn learn(
    db_path: &Path,
    knowledge_type: KnowledgeType,
    title: &str,
    description: &str,
    related_nodes: &[String],
    tags: &[String],
) -> crate::error::Result<()> {
    let nodes = if related_nodes.is_empty() {
        None
    } else {
        Some(related_nodes)
    };
    learn_with_uuid(
        db_path,
        None,
        knowledge_type,
        title,
        description,
        nodes,
        tags,
        None,
    )
    .map(|_| ())
}

/// Save a new entry that supersedes an existing one. Marks the old entry obsolete
/// and writes both supersedes/superseded_by back-references in one call.
pub fn learn_supersedes(
    db_path: &Path,
    knowledge_type: KnowledgeType,
    title: &str,
    description: &str,
    related_nodes: Option<&[String]>,
    tags: &[String],
    supersedes_uuid: &str,
) -> crate::error::Result<String> {
    let new_uuid = learn_with_uuid(
        db_path,
        None,
        knowledge_type,
        title,
        description,
        related_nodes,
        tags,
        None,
    )?;
    let updates = KnowledgeUpdates {
        superseded_by: Some(new_uuid.clone()),
        ..Default::default()
    };
    update_knowledge(db_path, supersedes_uuid, &updates)?;
    Ok(new_uuid)
}

/// Learn with explicit UUID. Returns the UUID of the created/updated entry.
/// - uuid=Some → update existing entry
/// - uuid=None → create new entry with fresh UUID
///
/// `related_nodes`:
/// - `None` → don't touch existing links
/// - `Some(&[])` → clear all links
/// - `Some(&[...])` → replace links with these nodes
///
/// `context_uuid`: if provided, auto-creates a "leads_to" chain link
/// from the context knowledge to this one (chain of thought).
#[allow(clippy::too_many_arguments)]
pub fn learn_with_uuid(
    db_path: &Path,
    knowledge_uuid: Option<&str>,
    knowledge_type: KnowledgeType,
    title: &str,
    description: &str,
    related_nodes: Option<&[String]>,
    tags: &[String],
    context_uuid: Option<&str>,
) -> crate::error::Result<String> {
    // Hard cap on description size: ~500 tokens. Forces 1-fact-per-entry —
    // multi-defect dumps (e.g. "Round 2 review B2-G1..B2-G8" in one entry)
    // make recall noisy and staleness imprecise (one fix invalidates eight).
    // Agents that hit this should split into per-fact entries linked via
    // `context_uuid` (chain-of-thought) or shared `related_nodes`. Bypass:
    // direct callers of `storage::append_knowledge` (ingest, import) — those
    // ingest pre-shaped content and aren't subject to the discipline.
    const MAX_DESCRIPTION_CHARS: usize = 2000;
    let desc_len = description.chars().count();
    if desc_len > MAX_DESCRIPTION_CHARS {
        return Err(crate::error::KodexError::Other(format!(
            "description too long ({desc_len} chars > {MAX_DESCRIPTION_CHARS} cap). \
             Split into 1-fact-per-entry; link related entries via \
             context_uuid (chain-of-thought) or shared related_nodes."
        )));
    }
    let new_uuid = crate::storage::append_knowledge_with_uuid(
        db_path,
        knowledge_uuid,
        title,
        &knowledge_type.to_string(),
        description,
        0.6,
        related_nodes,
        tags,
    )?;

    // Auto-link chain of thought: context → this
    if let Some(ctx) = context_uuid {
        if ctx != new_uuid {
            let _ = link_knowledge_to_knowledge(db_path, ctx, &new_uuid, "leads_to", false);
        }
    }

    Ok(new_uuid)
}

/// Capture lightweight provenance for an entry being saved from `cwd`.
/// Returns a string like `commit:abc1234@<basename>` when cwd is inside a git repo,
/// `cwd:<basename>` when not, or `None` if cwd is unusable.
pub fn auto_provenance(cwd: &Path) -> Option<String> {
    if !cwd.exists() {
        return None;
    }
    let basename = cwd
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let git_head = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            }
        });
    Some(match git_head {
        Some(sha) => format!("commit:{sha}@{basename}"),
        None => format!("cwd:{basename}"),
    })
}

/// Build a keyword → UUID index for fast knowledge lookup.
/// Indexes title, description, tags, and type tokens.
fn build_knowledge_index(
    knowledge: &[crate::types::KnowledgeEntry],
) -> HashMap<String, Vec<usize>> {
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, k) in knowledge.iter().enumerate() {
        for token in k
            .title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| s.len() > 2)
        {
            index.entry(token.to_string()).or_default().push(i);
        }
        for token in k
            .description
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| s.len() > 2)
        {
            index.entry(token.to_string()).or_default().push(i);
        }
        for tag in &k.tags {
            index.entry(tag.to_lowercase()).or_default().push(i);
        }
        index
            .entry(k.knowledge_type.to_lowercase())
            .or_default()
            .push(i);
    }
    index
}

/// Query knowledge by keyword, type, or tag. Uses index for fast lookup.
///
/// Multi-token queries match if ANY token appears in title/description/tags
/// (OR semantics). Results are scored and returned highest-relevance first:
/// title and tag hits weight 2, description hits weight 1.
pub fn query_knowledge(db_path: &Path, query: &str, type_filter: Option<&str>) -> Vec<Knowledge> {
    let data = match crate::storage::load_knowledge_only(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let query_lower = query.to_lowercase();

    let tokens: Vec<String> = query_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(|s| s.to_string())
        .collect();

    // Use index for fast candidate selection
    let candidates: std::collections::HashSet<usize> = if !query.is_empty() {
        let index = build_knowledge_index(&data.knowledge);
        let mut hits = std::collections::HashSet::new();
        for token in &tokens {
            if let Some(indices) = index.get(token.as_str()) {
                hits.extend(indices);
            }
        }
        // Substring fallback: handles short queries (≤2 chars filtered out
        // of tokens) and substrings inside compound identifiers.
        if hits.is_empty() {
            (0..data.knowledge.len()).collect()
        } else {
            hits
        }
    } else {
        (0..data.knowledge.len()).collect()
    };

    let links = data.links;
    let mut filtered: Vec<crate::types::KnowledgeEntry> = data
        .knowledge
        .into_iter()
        .enumerate()
        .filter(|(i, _)| candidates.contains(i))
        .map(|(_, k)| k)
        .filter(|k| {
            if let Some(tf) = type_filter {
                if k.knowledge_type != tf {
                    return false;
                }
            }
            if query.is_empty() {
                return true;
            }
            let title_l = k.title.to_lowercase();
            let desc_l = k.description.to_lowercase();
            if tokens.is_empty() {
                // Short query (≤2 chars): substring match on whole query
                title_l.contains(&query_lower)
                    || desc_l.contains(&query_lower)
                    || k.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            } else {
                // Multi-token: match if ANY token appears in title/desc/tags
                tokens.iter().any(|tok| {
                    title_l.contains(tok)
                        || desc_l.contains(tok)
                        || k.tags.iter().any(|t| t.to_lowercase().contains(tok))
                })
            }
        })
        .collect();

    // Rank by token match count (title/tag weight 2, description weight 1)
    if !tokens.is_empty() {
        filtered.sort_by_cached_key(|k| {
            let title_l = k.title.to_lowercase();
            let desc_l = k.description.to_lowercase();
            let tags_l: Vec<String> = k.tags.iter().map(|t| t.to_lowercase()).collect();
            let mut score: i32 = 0;
            for tok in &tokens {
                if title_l.contains(tok) {
                    score += 2;
                }
                if desc_l.contains(tok) {
                    score += 1;
                }
                if tags_l.iter().any(|t| t.contains(tok)) {
                    score += 2;
                }
            }
            -score // ascending sort = highest score first
        });
    }

    filtered
        .into_iter()
        .map(|k| {
            let related: Vec<String> = links
                .iter()
                .filter(|l| l.knowledge_uuid == k.uuid && !l.is_knowledge_link())
                .map(|l| l.node_uuid.clone())
                .collect();
            Knowledge {
                uuid: k.uuid,
                knowledge_type: parse_knowledge_type(&k.knowledge_type),
                title: k.title,
                description: k.description,
                confidence: k.confidence,
                observations: k.observations,
                related_nodes: related,
                tags: k.tags,
                created_at: k.created_at,
                updated_at: k.updated_at,
            }
        })
        .collect()
}

fn parse_knowledge_type(s: &str) -> KnowledgeType {
    match s {
        "architecture" => KnowledgeType::Architecture,
        "pattern" => KnowledgeType::Pattern,
        "decision" => KnowledgeType::Decision,
        "convention" => KnowledgeType::Convention,
        "coupling" => KnowledgeType::Coupling,
        "domain" => KnowledgeType::Domain,
        "preference" => KnowledgeType::Preference,
        "bug_pattern" => KnowledgeType::BugPattern,
        "tech_debt" => KnowledgeType::TechDebt,
        "ops" => KnowledgeType::Ops,
        "api" => KnowledgeType::Api,
        "performance" => KnowledgeType::Performance,
        "roadmap" => KnowledgeType::Roadmap,
        "context" => KnowledgeType::Context,
        "lesson" => KnowledgeType::Lesson,
        other => KnowledgeType::Custom(other.to_string()),
    }
}

/// Get a knowledge context summary from SQLite for Claude.
/// Compact knowledge summary for session start.
/// Shows stats + high-confidence + recent. NOT a full dump.
/// Use `recall_for_task` for task-specific retrieval.
///
/// `inline_top_k`: if > 0, append a "## Inline" section with the top-k highest-priority
/// entries (high confidence first, then recent) including full descriptions. Helps avoid a
/// follow-up `recall` round-trip on session bootstrap.
pub fn knowledge_context(db_path: &Path, max_items: usize, inline_top_k: usize) -> String {
    let data = match crate::storage::load(db_path) {
        Ok(d) => d,
        Err(_) => return "No knowledge base found. Run `kodex run` first.\n".to_string(),
    };

    let active: Vec<&crate::types::KnowledgeEntry> = data
        .knowledge
        .iter()
        .filter(|k| k.status != "obsolete")
        .collect();

    if active.is_empty() {
        return "# Knowledge: 0 items\n\nNo knowledge yet. Use `learn` to save patterns.\n"
            .to_string();
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let week_ago = now.saturating_sub(7 * 86400);
    let needs_review = active.iter().filter(|k| k.status == "needs_review").count();

    let mut ctx = format!(
        "# Knowledge: {} items ({} needs_review)\n\nUse `recall_for_task` for task-specific knowledge.\n\n",
        active.len(), needs_review,
    );

    // High confidence (>0.8) — always show
    let high_conf: Vec<&&crate::types::KnowledgeEntry> =
        active.iter().filter(|k| k.confidence > 0.8).collect();
    if !high_conf.is_empty() {
        ctx.push_str(&format!("## Established ({}, >80%)\n\n", high_conf.len()));
        for k in high_conf.iter().take(max_items / 3) {
            ctx.push_str(&format!("- **{}** ({}x)\n", k.title, k.observations));
        }
        ctx.push('\n');
    }

    // Recent (last 7 days)
    let mut recent: Vec<&&crate::types::KnowledgeEntry> =
        active.iter().filter(|k| k.updated_at > week_ago).collect();
    recent.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if !recent.is_empty() {
        ctx.push_str(&format!("## Recent ({}, last 7 days)\n\n", recent.len()));
        for k in recent.iter().take(max_items / 3) {
            let conf = (k.confidence * 100.0) as u32;
            ctx.push_str(&format!(
                "- **{}** ({conf}%, {})\n",
                k.title, k.knowledge_type
            ));
        }
        ctx.push('\n');
    }

    // Type counts only
    let mut by_type: HashMap<&str, usize> = HashMap::new();
    for k in &active {
        *by_type.entry(k.knowledge_type.as_str()).or_insert(0) += 1;
    }
    let mut types: Vec<_> = by_type.into_iter().collect();
    types.sort_by(|a, b| b.1.cmp(&a.1));
    ctx.push_str("## By type\n\n");
    for (t, count) in &types {
        ctx.push_str(&format!("- {t}: {count}\n"));
    }
    ctx.push('\n');

    // Inline top-k: full descriptions for the most useful entries
    if inline_top_k > 0 {
        let mut ranked: Vec<&&crate::types::KnowledgeEntry> = active.iter().collect();
        // Sort: high confidence first, then most recently updated
        ranked.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.updated_at.cmp(&a.updated_at))
        });
        let take = ranked.len().min(inline_top_k);
        if take > 0 {
            ctx.push_str(&format!("## Inline (top {take})\n\n"));
            for k in ranked.iter().take(take) {
                let conf = (k.confidence * 100.0) as u32;
                ctx.push_str(&format!(
                    "### {} ({conf}%, {})\n\n{}\n\n",
                    k.title, k.knowledge_type, k.description
                ));
            }
        }
    }

    ctx
}

fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ---------------------------------------------------------------------------
// Staleness detection
// ---------------------------------------------------------------------------

/// Staleness report for a single knowledge entry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StaleInfo {
    pub uuid: String,
    pub title: String,
    pub reason: String,
    /// 0.0 = fully valid, 1.0 = completely stale
    pub staleness: f64,
    pub action: String,
}

/// Check knowledge entries for staleness. Graduated assessment:
/// - All linked nodes gone → needs_review (staleness 1.0)
/// - Some linked nodes gone → partial staleness (0.3-0.7)
/// - Linked nodes exist but body_hash changed → tentative (0.2)
/// - No validation for a long time → age decay
///
/// Returns list of stale entries with details.
pub fn detect_stale_knowledge(db_path: &Path) -> crate::error::Result<usize> {
    let results = detect_stale_detailed(db_path)?;
    Ok(results.len())
}

/// Detailed staleness detection with graduated scoring.
pub fn detect_stale_detailed(db_path: &Path) -> crate::error::Result<Vec<StaleInfo>> {
    let mut data = crate::storage::load(db_path)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Build node lookup: uuid → (exists, body_hash)
    let node_info: HashMap<String, Option<String>> = data
        .extraction
        .nodes
        .iter()
        .filter_map(|n| n.uuid.as_ref().map(|u| (u.clone(), n.body_hash.clone())))
        .collect();

    let valid_node_uuids: std::collections::HashSet<&str> =
        node_info.keys().map(|s| s.as_str()).collect();

    let mut stale_entries = Vec::new();
    let mut changed = false;

    for entry in &mut data.knowledge {
        if entry.status == "obsolete" {
            continue;
        }

        // Find node-only links
        let linked: Vec<&str> = data
            .links
            .iter()
            .filter(|l| l.knowledge_uuid == entry.uuid && !l.is_knowledge_link())
            .map(|l| l.node_uuid.as_str())
            .collect();

        if linked.is_empty() {
            // Age-based decay: if no validation for 90+ days
            if entry.last_validated_at > 0 && now > entry.last_validated_at {
                let age_days = (now - entry.last_validated_at) / 86400;
                if age_days > 90 && entry.status != "needs_review" {
                    entry.status = "needs_review".to_string();
                    stale_entries.push(StaleInfo {
                        uuid: entry.uuid.clone(),
                        title: entry.title.clone(),
                        reason: format!("Not validated for {age_days} days"),
                        staleness: 0.3,
                        action: "validate or refresh".into(),
                    });
                    changed = true;
                }
            }
            continue;
        }

        let alive = linked
            .iter()
            .filter(|u| valid_node_uuids.contains(*u))
            .count();
        let total = linked.len();
        let gone = total - alive;

        if gone == total {
            // All nodes gone
            if entry.status != "needs_review" {
                entry.status = "needs_review".to_string();
                entry.confidence *= 0.9;
                stale_entries.push(StaleInfo {
                    uuid: entry.uuid.clone(),
                    title: entry.title.clone(),
                    reason: format!("All {total} linked nodes deleted"),
                    staleness: 1.0,
                    action: "review: nodes may have been refactored or removed".into(),
                });
                changed = true;
            }
        } else if gone > 0 {
            // Partial staleness
            let ratio = gone as f64 / total as f64;
            if ratio > 0.5 && entry.status == "active" {
                entry.status = "needs_review".to_string();
                entry.confidence *= 0.95;
                stale_entries.push(StaleInfo {
                    uuid: entry.uuid.clone(),
                    title: entry.title.clone(),
                    reason: format!("{gone}/{total} linked nodes deleted"),
                    staleness: ratio * 0.7,
                    action: "review: partial node loss".into(),
                });
                changed = true;
            }
        } else {
            // All nodes alive — check body_hash drift using link snapshots
            if entry.status == "active" {
                let entry_links: Vec<&crate::types::KnowledgeLink> = data
                    .links
                    .iter()
                    .filter(|l| l.knowledge_uuid == entry.uuid && !l.is_knowledge_link())
                    .collect();
                let mut drifted_count = 0;
                let mut checked_count = 0;
                for link in &entry_links {
                    if link.linked_body_hash.is_empty() {
                        continue; // no snapshot — can't detect drift
                    }
                    checked_count += 1;
                    if let Some(Some(cur)) = node_info.get(&link.node_uuid) {
                        if cur != &link.linked_body_hash {
                            drifted_count += 1;
                        }
                    }
                }
                if drifted_count > 0 {
                    let staleness =
                        0.2 + 0.3 * (drifted_count as f64 / checked_count.max(1) as f64);
                    stale_entries.push(StaleInfo {
                        uuid: entry.uuid.clone(),
                        title: entry.title.clone(),
                        reason: format!(
                            "{drifted_count}/{checked_count} linked nodes have changed body since link was created"
                        ),
                        staleness,
                        action: "validate: linked code has changed, knowledge may be outdated".into(),
                    });
                    // Auto-promote when drift is severe (>2/3 of linked bodies
                    // changed). Below this threshold the change is likely
                    // cosmetic (rename, comment edit) and stays advisory so
                    // the agent isn't flooded with false positives. Mirrors
                    // the auto-promotion already applied to other stale
                    // signals (all-nodes-gone, partial >50%, never-retrieved).
                    if staleness > 0.4 {
                        entry.status = "needs_review".to_string();
                        entry.confidence *= 0.95;
                        changed = true;
                    }
                }
            }
        }

        // Retrieval staleness: 90+ days unfetched, status still active
        if entry.status == "active" {
            let last_seen = if entry.last_fetched > 0 {
                entry.last_fetched
            } else {
                entry.created_at
            };
            if last_seen > 0 && now > last_seen {
                let idle_days = (now - last_seen) / 86400;
                if idle_days >= 90 {
                    entry.status = "needs_review".to_string();
                    let reason = if entry.fetch_count == 0 {
                        format!("Never retrieved in {idle_days} days since creation")
                    } else {
                        format!(
                            "Not retrieved in {idle_days} days (last fetch_count={})",
                            entry.fetch_count
                        )
                    };
                    stale_entries.push(StaleInfo {
                        uuid: entry.uuid.clone(),
                        title: entry.title.clone(),
                        reason,
                        staleness: 0.4,
                        action: "review: rarely surfaced — still relevant?".into(),
                    });
                    changed = true;
                }
            }
        }
    }

    if changed {
        // Clean fully dead node links (keep partial + knowledge links)
        data.links
            .retain(|l| l.is_knowledge_link() || valid_node_uuids.contains(l.node_uuid.as_str()));
        crate::storage::save_knowledge_only(db_path, &data)?;
    }

    Ok(stale_entries)
}

// ---------------------------------------------------------------------------
// Knowledge relevance scoring
// ---------------------------------------------------------------------------

/// Scoring context for relevance computation.
struct ScoringContext<'a> {
    touched_files: &'a [String],
    node_uuids: &'a std::collections::HashSet<String>,
    query_tokens: &'a [String],
    now: u64,
    /// Filenames extracted from touched_files (cached)
    touched_filenames: Vec<&'a str>,
    /// Current project name (for project affinity scoring)
    project: String,
}

impl<'a> ScoringContext<'a> {
    fn new(
        touched_files: &'a [String],
        node_uuids: &'a std::collections::HashSet<String>,
        query_tokens: &'a [String],
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let touched_filenames: Vec<&str> = touched_files
            .iter()
            .map(|f| f.rsplit('/').next().unwrap_or(f.as_str()))
            .collect();
        // Infer project from touched_files (first path component)
        let project = touched_files
            .first()
            .and_then(|f| f.split('/').next())
            .unwrap_or("")
            .to_string();
        Self {
            touched_files,
            node_uuids,
            query_tokens,
            now,
            touched_filenames,
            project,
        }
    }
}

/// Structured score breakdown for debugging/explanation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoreBreakdown {
    pub total: f64,
    pub confidence: f64,
    pub observations: f64,
    /// Log-scaled `fetch_count` — rewards entries that survived passive
    /// retrieval. Distinct from `observations` (explicit `learn` reinforcement).
    pub retrievals: f64,
    pub node_overlap: f64,
    pub file_mention: f64,
    pub scope_match: f64,
    pub applies_when: f64,
    pub keyword_match: f64,
    pub type_priority: f64,
    pub recency: f64,
    pub status_penalty: f64,
    pub reasons: Vec<String>,
}

/// Score a knowledge entry's relevance with full breakdown.
fn relevance_score_detailed(
    k: &Knowledge,
    entry: &crate::types::KnowledgeEntry,
    ctx: &ScoringContext<'_>,
) -> ScoreBreakdown {
    let mut b = ScoreBreakdown {
        total: 0.0,
        confidence: 0.0,
        observations: 0.0,
        retrievals: 0.0,
        node_overlap: 0.0,
        file_mention: 0.0,
        scope_match: 0.0,
        applies_when: 0.0,
        keyword_match: 0.0,
        type_priority: 0.0,
        recency: 0.0,
        status_penalty: 0.0,
        reasons: Vec::new(),
    };

    // 1. Confidence (0-20)
    b.confidence = k.confidence * 20.0;

    // 2. Observations (0-10, log scale): explicit `learn` reinforcement.
    b.observations = (k.observations as f64).ln().min(3.0) * 3.3;

    // 2b. Retrievals (0-10, log scale): passive recall reuse. Uses ln_1p so
    // never-fetched entries (fetch_count=0) score 0, single-fetch scores ~2.3.
    // Distinct from observations: an entry can be retrieved without anyone
    // re-running `learn` on it. Without this term, fetch_count is dead weight
    // even though storage::bump_fetch_counters maintains it on every recall.
    b.retrievals = (entry.fetch_count as f64).ln_1p().min(3.0) * 3.3;
    if entry.fetch_count > 5 {
        b.reasons
            .push(format!("retrieved {}× before", entry.fetch_count));
    }

    // 3. Node overlap (0-30)
    if !ctx.node_uuids.is_empty() {
        let linked: std::collections::HashSet<&str> =
            k.related_nodes.iter().map(|s| s.as_str()).collect();
        let overlap = ctx
            .node_uuids
            .iter()
            .filter(|u| linked.contains(u.as_str()))
            .count();
        if overlap > 0 {
            b.node_overlap = 30.0 * (overlap as f64 / ctx.node_uuids.len().max(1) as f64).min(1.0);
            b.reasons.push("linked to code in scope".into());
        }
    }

    // 4. File mention (0-20)
    for filename in &ctx.touched_filenames {
        if k.title.contains(filename)
            || k.description.contains(filename)
            || k.tags.iter().any(|t| t.contains(filename))
        {
            b.file_mention = 20.0;
            b.reasons.push("mentions touched file".into());
            break;
        }
    }

    // 5. Scope match (0-10)
    if !entry.scope.is_empty() && !ctx.touched_files.is_empty() {
        match entry.scope.as_str() {
            "file" | "node" => {
                b.scope_match = 10.0;
                b.reasons.push("file/node-scoped".into());
            }
            "module" => b.scope_match = 5.0,
            _ => {}
        }
    }

    // 6. applies_when (0-15)
    if !entry.applies_when.is_empty() && !ctx.query_tokens.is_empty() {
        let aw_lower = entry.applies_when.to_lowercase();
        let aw_matches = ctx
            .query_tokens
            .iter()
            .filter(|t| aw_lower.contains(t.as_str()))
            .count();
        if aw_matches > 0 {
            b.applies_when = 15.0 * (aw_matches as f64 / ctx.query_tokens.len() as f64);
            b.reasons.push("applies_when match".into());
        }
    }

    // 7. Keyword (0-10)
    if !ctx.query_tokens.is_empty() {
        let title_lower = k.title.to_lowercase();
        let desc_lower = k.description.to_lowercase();
        let matches = ctx
            .query_tokens
            .iter()
            .filter(|t| title_lower.contains(t.as_str()) || desc_lower.contains(t.as_str()))
            .count();
        b.keyword_match = 10.0 * (matches as f64 / ctx.query_tokens.len() as f64);
        if matches > 0 {
            b.reasons.push("matches query".into());
        }
    }

    // 8. Type priority (0-5)
    b.type_priority = match entry.knowledge_type.as_str() {
        "bug_pattern" | "convention" | "coupling" => 5.0,
        "pattern" | "decision" | "lesson" => 3.0,
        "architecture" | "domain" => 2.0,
        _ => 0.0,
    };

    // 9. Recency (-10..+10) — graduated decay from the most recent *content*
    // signal. Uses the freshest of: validation, update, creation. Excludes
    // `last_fetched` deliberately: a fetched entry already gets credit via
    // `b.retrievals`, and folding `last_fetched` into recency makes activity
    // logs perpetually fresh — every recall hit resets their age. With
    // `last_fetched` excluded, an entry's recency is anchored to when its
    // *content* was last touched (validation/edit/birth), not to whether it
    // was lately surfaced.
    let last_active = [entry.last_validated_at, entry.updated_at, entry.created_at]
        .into_iter()
        .max()
        .unwrap_or(0);
    if last_active > 0 && ctx.now >= last_active {
        let age_days = (ctx.now - last_active) / 86400;
        b.recency = if age_days < 7 {
            10.0
        } else if age_days < 30 {
            5.0
        } else if age_days < 90 {
            0.0
        } else if age_days < 180 {
            -5.0
        } else {
            -10.0
        };
        if b.recency > 0.0 {
            b.reasons.push("recently active".into());
        } else if b.recency < 0.0 {
            b.reasons
                .push(format!("stale ({age_days}d since activity)"));
        }
    }

    // 10. Project affinity (0-15)
    if !ctx.project.is_empty() {
        let project_tag = format!("project:{}", ctx.project);
        if k.tags.iter().any(|t| t == &project_tag)
            || k.description.contains(&ctx.project)
            || k.title.to_lowercase().contains(&ctx.project.to_lowercase())
        {
            b.scope_match += 15.0;
            b.reasons.push("same project".into());
        }
    }

    // Sum
    b.total = b.confidence
        + b.observations
        + b.retrievals
        + b.node_overlap
        + b.file_mention
        + b.scope_match
        + b.applies_when
        + b.keyword_match
        + b.type_priority
        + b.recency;

    // 10. Penalty
    if entry.status == "needs_review" {
        b.status_penalty = b.total * 0.5;
        b.total *= 0.5;
        b.reasons.push("needs_review penalty".into());
    }

    if b.reasons.is_empty() {
        b.reasons.push("high confidence".into());
    }

    b
}

/// A recall result with score breakdown.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecallResult {
    pub knowledge: Knowledge,
    pub score: ScoreBreakdown,
}

/// Recall knowledge ranked by relevance, with score breakdown and diversity.
pub fn recall_for_task_structured(
    db_path: &Path,
    question: &str,
    touched_files: &[String],
    node_uuids: &[String],
    max_items: usize,
    type_filter: Option<&str>,
) -> Vec<RecallResult> {
    let data = match crate::storage::load(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let type_lower = type_filter.map(|s| s.to_lowercase());

    let query_tokens: Vec<String> = question
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(String::from)
        .collect();

    let node_uuid_set: std::collections::HashSet<String> = node_uuids.iter().cloned().collect();
    let ctx = ScoringContext::new(touched_files, &node_uuid_set, &query_tokens);
    let links = &data.links;

    // Resolve node UUIDs → linked knowledge UUIDs for graph reasoning
    let seed_knowledge_uuids: Vec<String> = links
        .iter()
        .filter(|l| !l.is_knowledge_link() && node_uuids.contains(&l.node_uuid))
        .map(|l| l.knowledge_uuid.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let reasoning = crate::reasoning::propagate_confidence(
        &data.knowledge,
        &data.links,
        &seed_knowledge_uuids,
        3,
    );

    let mut scored: Vec<RecallResult> = data
        .knowledge
        .iter()
        .filter(|k| k.status != "obsolete")
        .filter(|k| match &type_lower {
            Some(t) => k.knowledge_type.to_lowercase() == *t,
            None => true,
        })
        .map(|k| {
            let related: Vec<String> = links
                .iter()
                .filter(|l| l.knowledge_uuid == k.uuid && !l.is_knowledge_link())
                .map(|l| l.node_uuid.clone())
                .collect();
            let knowledge = Knowledge {
                uuid: k.uuid.clone(),
                knowledge_type: parse_knowledge_type(&k.knowledge_type),
                title: k.title.clone(),
                description: k.description.clone(),
                confidence: k.confidence,
                observations: k.observations,
                related_nodes: related,
                tags: k.tags.clone(),
                created_at: k.created_at,
                updated_at: k.updated_at,
            };
            let mut score = relevance_score_detailed(&knowledge, k, &ctx);
            // Apply graph reasoning adjustment (±0.3 max, scaled to ±10 points)
            if let Some(&adj) = reasoning.adjustments.get(&k.uuid) {
                let reasoning_pts = adj * 33.0; // ±0.3 → ±10
                score.total += reasoning_pts;
                if reasoning_pts.abs() > 0.5 {
                    score
                        .reasons
                        .push(format!("graph reasoning: {reasoning_pts:+.1}"));
                }
            }
            RecallResult { knowledge, score }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .total
            .partial_cmp(&a.score.total)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Diversity: collapse similar entries (same type + >60% title overlap)
    let mut result = Vec::new();
    let mut seen_titles: Vec<String> = Vec::new();
    for item in scored {
        if result.len() >= max_items {
            break;
        }
        let title_lower = item.knowledge.title.to_lowercase();
        let is_dup = seen_titles.iter().any(|prev| {
            let tokens_a: Vec<&str> = prev.split_whitespace().collect();
            let tokens_b: Vec<&str> = title_lower.split_whitespace().collect();
            if tokens_a.is_empty() || tokens_b.is_empty() {
                return false;
            }
            let common = tokens_a.iter().filter(|t| tokens_b.contains(t)).count();
            let total = tokens_a.len().max(tokens_b.len());
            common as f64 / total as f64 > 0.6
        });
        if !is_dup {
            seen_titles.push(title_lower);
            result.push(item);
        }
    }

    result
}

/// Recall knowledge ranked by relevance (simple Knowledge vec).
pub fn recall_for_task(
    db_path: &Path,
    question: &str,
    touched_files: &[String],
    node_uuids: &[String],
    max_items: usize,
    type_filter: Option<&str>,
) -> Vec<Knowledge> {
    recall_for_task_structured(
        db_path,
        question,
        touched_files,
        node_uuids,
        max_items,
        type_filter,
    )
    .into_iter()
    .map(|r| r.knowledge)
    .collect()
}

/// Structured task context for machine consumption.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskContext {
    pub relevant: Vec<RecallResult>,
    pub warnings: Vec<TaskWarning>,
    pub conflicts: Vec<KnowledgeConflict>,
    pub recommendations: Vec<crate::recommend::Recommendation>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskWarning {
    pub uuid: String,
    pub title: String,
    pub status: String,
    pub confidence: u32,
    pub reason: String,
}

/// Get structured task context (JSON-friendly).
pub fn get_task_context_json(
    db_path: &Path,
    question: &str,
    touched_files: &[String],
    max_items: usize,
    task_type: &str,
) -> TaskContext {
    let data = match crate::storage::load(db_path) {
        Ok(d) => d,
        Err(_) => {
            return TaskContext {
                relevant: vec![],
                warnings: vec![],
                conflicts: vec![],
                recommendations: vec![],
            }
        }
    };

    let file_node_uuids: Vec<String> = data
        .extraction
        .nodes
        .iter()
        .filter(|n| {
            touched_files.iter().any(|f| {
                let filename = f.rsplit('/').next().unwrap_or(f);
                n.source_file.contains(filename)
            })
        })
        .filter_map(|n| n.uuid.clone())
        .collect();

    let relevant = recall_for_task_structured(
        db_path,
        question,
        touched_files,
        &file_node_uuids,
        max_items,
        None,
    );

    // Warnings
    let warned_uuids: Vec<&str> = data
        .knowledge
        .iter()
        .filter(|k| k.status == "needs_review" || k.confidence < 0.4)
        .map(|k| k.uuid.as_str())
        .collect();
    let warnings: Vec<TaskWarning> = relevant
        .iter()
        .filter(|r| warned_uuids.contains(&r.knowledge.uuid.as_str()))
        .map(|r| {
            let entry = data.knowledge.iter().find(|e| e.uuid == r.knowledge.uuid);
            let status = entry.map(|e| e.status.as_str()).unwrap_or("active");
            TaskWarning {
                uuid: r.knowledge.uuid.clone(),
                title: r.knowledge.title.clone(),
                status: status.to_string(),
                confidence: (r.knowledge.confidence * 100.0) as u32,
                reason: if status == "needs_review" {
                    "linked nodes may have changed".into()
                } else {
                    "low confidence".into()
                },
            }
        })
        .collect();

    // Conflicts
    let all_conflicts = detect_conflicts(db_path);
    let conflicts: Vec<KnowledgeConflict> = all_conflicts
        .into_iter()
        .filter(|c| {
            relevant
                .iter()
                .any(|r| r.knowledge.uuid == c.uuid_a || r.knowledge.uuid == c.uuid_b)
        })
        .collect();

    let tt = if task_type.is_empty() {
        "coding"
    } else {
        task_type
    };
    let recommendations = crate::recommend::compute_recommendations(&relevant, &conflicts, tt);

    TaskContext {
        relevant,
        warnings,
        conflicts,
        recommendations,
    }
}

/// Build a task-specific briefing for the agent (markdown string).
/// Delegates to get_task_context_json and renders to markdown.
pub fn get_task_context(
    db_path: &Path,
    question: &str,
    touched_files: &[String],
    max_items: usize,
) -> String {
    get_task_context_md(db_path, question, touched_files, max_items, "coding")
}

/// Markdown briefing with task_type support.
pub fn get_task_context_md(
    db_path: &Path,
    question: &str,
    touched_files: &[String],
    max_items: usize,
    task_type: &str,
) -> String {
    let tc = get_task_context_json(db_path, question, touched_files, max_items, task_type);

    if tc.relevant.is_empty() {
        return "No relevant knowledge found for this task.".to_string();
    }

    let mut ctx = String::new();

    // Relevant knowledge with reasons
    ctx.push_str(&format!(
        "## Relevant Knowledge ({} items)\n\n",
        tc.relevant.len()
    ));
    for r in &tc.relevant {
        let k = &r.knowledge;
        let conf = (k.confidence * 100.0) as u32;
        let status_tag = if conf < 50 { " [tentative]" } else { "" };
        let summary = k.description.lines().next().unwrap_or("");
        let summary = if summary.len() > 100 {
            let end = floor_char_boundary(summary, 100);
            format!("{}...", &summary[..end])
        } else {
            summary.to_string()
        };
        ctx.push_str(&format!(
            "- **{}** ({conf}%{status_tag}) — {summary} [{}]\n",
            k.title,
            r.score.reasons.join(", "),
        ));
    }
    ctx.push('\n');

    // Recommendations
    if !tc.recommendations.is_empty() {
        ctx.push_str("## Recommendations\n\n");
        for rec in &tc.recommendations {
            ctx.push_str(&format!(
                "- [{}] **{}** — {}\n",
                rec.category, rec.action, rec.reason
            ));
        }
        ctx.push('\n');
    }

    // Warnings
    if !tc.warnings.is_empty() {
        ctx.push_str("## Warnings\n\n");
        for w in &tc.warnings {
            ctx.push_str(&format!(
                "- **{}** ({}%, {}) — {}\n",
                w.title, w.confidence, w.status, w.reason
            ));
        }
        ctx.push('\n');
    }

    // Conflicts
    if !tc.conflicts.is_empty() {
        ctx.push_str("## Conflicts\n\n");
        for c in &tc.conflicts {
            ctx.push_str(&format!(
                "- {} vs {} — {}\n",
                c.title_a, c.title_b, c.description
            ));
        }
        ctx.push('\n');
    }

    ctx
}

// ---------------------------------------------------------------------------
// Knowledge update APIs
// ---------------------------------------------------------------------------

/// Update specific fields on an existing knowledge entry (by UUID).
pub fn update_knowledge(
    db_path: &Path,
    knowledge_uuid: &str,
    updates: &KnowledgeUpdates,
) -> crate::error::Result<()> {
    let mut data = crate::storage::load_knowledge_only(db_path)?;

    // Locate target index (release the borrow before we touch other entries).
    let target_idx = data
        .knowledge
        .iter()
        .position(|k| k.uuid == knowledge_uuid)
        .ok_or_else(|| {
            crate::error::KodexError::Other(format!("Knowledge UUID not found: {knowledge_uuid}"))
        })?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // First pass: validate the new pointer (if any) before mutating.
    let mut superseded_by_new: Option<String> = None;
    if let Some(sb) = &updates.superseded_by {
        if !sb.is_empty() {
            if !data.knowledge.iter().any(|k| k.uuid == *sb) {
                return Err(crate::error::KodexError::Other(format!(
                    "superseded_by target not found: {sb}"
                )));
            }
            superseded_by_new = Some(sb.clone());
        }
    }

    let mut changed = false;

    {
        let entry = &mut data.knowledge[target_idx];
        if let Some(status) = &updates.status {
            entry.status = status.clone();
            changed = true;
        }
        if let Some(scope) = &updates.scope {
            entry.scope = scope.clone();
            changed = true;
        }
        if let Some(applies_when) = &updates.applies_when {
            entry.applies_when = applies_when.clone();
            changed = true;
        }
        if let Some(sb) = &superseded_by_new {
            entry.superseded_by = sb.clone();
            entry.status = "obsolete".to_string();
            // Confidence decay so the obsolete entry doesn't out-rank the new one.
            entry.confidence *= 0.7;
            changed = true;
        }
        if updates.validate {
            entry.last_validated_at = now;
            changed = true;
        }
        if changed {
            entry.updated_at = now;
        }
    }

    // Back-reference: write `supersedes = old_uuid` on the new entry.
    if let Some(sb) = &superseded_by_new {
        if let Some(new_entry) = data.knowledge.iter_mut().find(|k| k.uuid == *sb) {
            // Don't clobber an existing back-reference (a chain of supersedes).
            if new_entry.supersedes.is_empty() {
                new_entry.supersedes = knowledge_uuid.to_string();
                new_entry.updated_at = now;
            }
        }
    }

    crate::storage::save_knowledge_only(db_path, &data)
}

/// Partial update fields for update_knowledge.
#[derive(Debug, Default, Clone)]
pub struct KnowledgeUpdates {
    pub status: Option<String>,
    pub scope: Option<String>,
    pub applies_when: Option<String>,
    pub superseded_by: Option<String>,
    pub validate: bool,
}

/// Validate a knowledge entry — mark as still accurate.
pub fn validate_knowledge(
    db_path: &Path,
    knowledge_uuid: &str,
    note: Option<&str>,
) -> crate::error::Result<()> {
    let mut data = crate::storage::load_knowledge_only(db_path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entry = data
        .knowledge
        .iter_mut()
        .find(|k| k.uuid == knowledge_uuid)
        .ok_or_else(|| {
            crate::error::KodexError::Other(format!("UUID not found: {knowledge_uuid}"))
        })?;

    entry.status = "active".to_string();
    entry.last_validated_at = now;
    entry.updated_at = now;
    if let Some(n) = note {
        if !n.is_empty() {
            entry.evidence = format!("{}\n[validated] {n}", entry.evidence);
        }
    }

    // Refresh link snapshots — update body_hash/logical_key to current values
    for link in &mut data.links {
        if link.knowledge_uuid == knowledge_uuid && !link.is_knowledge_link() {
            link.linked_body_hash = data
                .extraction
                .nodes
                .iter()
                .find(|n| n.uuid.as_deref() == Some(link.node_uuid.as_str()))
                .and_then(|n| n.body_hash.clone())
                .unwrap_or_default();
            link.linked_logical_key = data
                .extraction
                .nodes
                .iter()
                .find(|n| n.uuid.as_deref() == Some(link.node_uuid.as_str()))
                .and_then(|n| n.logical_key.clone())
                .unwrap_or_default();
        }
    }

    crate::storage::save_knowledge_only(db_path, &data)
}

/// Mark knowledge as obsolete with a reason.
pub fn mark_obsolete(
    db_path: &Path,
    knowledge_uuid: &str,
    reason: &str,
) -> crate::error::Result<()> {
    update_knowledge(
        db_path,
        knowledge_uuid,
        &KnowledgeUpdates {
            status: Some("obsolete".into()),
            ..Default::default()
        },
    )?;
    // Append reason to evidence
    if !reason.is_empty() {
        let mut data = crate::storage::load_knowledge_only(db_path)?;
        if let Some(entry) = data.knowledge.iter_mut().find(|k| k.uuid == knowledge_uuid) {
            entry.evidence = format!("{}\n[obsolete] {reason}", entry.evidence);
            entry.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
        crate::storage::save_knowledge_only(db_path, &data)?;
    }
    Ok(())
}

/// Link knowledge to specific nodes (additive — doesn't remove existing links).
pub fn link_knowledge_to_nodes(
    db_path: &Path,
    knowledge_uuid: &str,
    node_uuids: &[String],
    relation: &str,
) -> crate::error::Result<()> {
    let mut data = crate::storage::load_knowledge_only(db_path)?;

    // Verify knowledge exists
    if !data.knowledge.iter().any(|k| k.uuid == knowledge_uuid) {
        return Err(crate::error::KodexError::Other(format!(
            "Knowledge UUID not found: {knowledge_uuid}"
        )));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for node_uuid in node_uuids {
        // Don't add duplicate links
        let exists = data.links.iter().any(|l| {
            l.knowledge_uuid == knowledge_uuid
                && l.node_uuid == *node_uuid
                && l.relation == relation
        });
        if !exists {
            let linked_bh = data.node_body_hash(node_uuid);
            let linked_lk = data.node_logical_key(node_uuid);
            data.links.push(crate::types::KnowledgeLink {
                knowledge_uuid: knowledge_uuid.to_string(),
                node_uuid: node_uuid.clone(),
                relation: relation.to_string(),
                target_type: String::new(),
                confidence: 1.0,
                created_at: now,
                linked_body_hash: linked_bh,
                linked_logical_key: linked_lk,
                source: "agent".to_string(),
                ..Default::default()
            });
        }
    }

    crate::storage::save_knowledge_only(db_path, &data)
}

/// Clear all links for a given knowledge entry.
pub fn clear_knowledge_links(db_path: &Path, knowledge_uuid: &str) -> crate::error::Result<usize> {
    let mut data = crate::storage::load(db_path)?;
    let before = data.links.len();
    data.links.retain(|l| l.knowledge_uuid != knowledge_uuid);
    let removed = before - data.links.len();
    if removed > 0 {
        crate::storage::save_knowledge_only(db_path, &data)?;
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Knowledge deduplication
// ---------------------------------------------------------------------------

/// A pair of similar knowledge entries that may be duplicates.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DuplicateCandidate {
    pub uuid_a: String,
    pub title_a: String,
    pub uuid_b: String,
    pub title_b: String,
    pub similarity: f64,
    pub reason: String,
}

/// Find knowledge entries that are likely duplicates.
/// Uses title similarity + description overlap + same type.
pub fn find_duplicates(db_path: &Path, threshold: f64) -> Vec<DuplicateCandidate> {
    let data = match crate::storage::load_knowledge_only(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let threshold = if threshold <= 0.0 { 0.6 } else { threshold };
    let mut candidates = Vec::new();

    let entries: Vec<&crate::types::KnowledgeEntry> = data
        .knowledge
        .iter()
        .filter(|k| k.status != "obsolete")
        .collect();

    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            let a = entries[i];
            let b = entries[j];

            let sim = knowledge_similarity(a, b);
            if sim >= threshold {
                let reason = if a.title.to_lowercase() == b.title.to_lowercase() {
                    "identical title".into()
                } else if a.knowledge_type == b.knowledge_type {
                    format!("similar content ({:.0}%), same type", sim * 100.0)
                } else {
                    format!("similar content ({:.0}%)", sim * 100.0)
                };
                candidates.push(DuplicateCandidate {
                    uuid_a: a.uuid.clone(),
                    title_a: a.title.clone(),
                    uuid_b: b.uuid.clone(),
                    title_b: b.title.clone(),
                    similarity: sim,
                    reason,
                });
            }
        }
    }

    candidates.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

/// Merge candidate from the perspective of one specific entry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MergeCandidate {
    pub uuid: String,
    pub title: String,
    pub similarity: f64,
    pub reason: String,
}

/// Find entries similar to `target_uuid`. Used by `learn` to surface merge candidates.
/// Skips obsolete entries and the target itself.
pub fn find_similar_to_uuid(
    db_path: &Path,
    target_uuid: &str,
    threshold: f64,
) -> Vec<MergeCandidate> {
    let data = match crate::storage::load_knowledge_only(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let threshold = if threshold <= 0.0 { 0.6 } else { threshold };

    let target = match data.knowledge.iter().find(|k| k.uuid == target_uuid) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut out: Vec<MergeCandidate> = data
        .knowledge
        .iter()
        .filter(|k| k.uuid != target_uuid && k.status != "obsolete")
        .filter_map(|k| {
            let sim = knowledge_similarity(target, k);
            if sim < threshold {
                return None;
            }
            let reason = if target.title.to_lowercase() == k.title.to_lowercase() {
                "identical title".into()
            } else if target.knowledge_type == k.knowledge_type {
                format!("similar content ({:.0}%), same type", sim * 100.0)
            } else {
                format!("similar content ({:.0}%)", sim * 100.0)
            };
            Some(MergeCandidate {
                uuid: k.uuid.clone(),
                title: k.title.clone(),
                similarity: sim,
                reason,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Compute similarity between two knowledge entries (0.0-1.0).
fn knowledge_similarity(a: &crate::types::KnowledgeEntry, b: &crate::types::KnowledgeEntry) -> f64 {
    let mut score = 0.0;
    let mut max_score = 0.0;

    // Title similarity (token overlap)
    max_score += 40.0;
    let a_tokens: Vec<String> = a
        .title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 1)
        .map(String::from)
        .collect();
    let b_tokens: Vec<String> = b
        .title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 1)
        .map(String::from)
        .collect();
    if !a_tokens.is_empty() && !b_tokens.is_empty() {
        let common = a_tokens.iter().filter(|t| b_tokens.contains(t)).count();
        let total = a_tokens.len().max(b_tokens.len());
        score += 40.0 * (common as f64 / total as f64);
    }

    // Same type
    max_score += 20.0;
    if a.knowledge_type == b.knowledge_type {
        score += 20.0;
    }

    // Description token overlap (first 200 chars)
    max_score += 30.0;
    let a_desc: Vec<String> = a
        .description
        .to_lowercase()
        .chars()
        .take(200)
        .collect::<String>()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(String::from)
        .collect();
    let b_desc: Vec<String> = b
        .description
        .to_lowercase()
        .chars()
        .take(200)
        .collect::<String>()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(String::from)
        .collect();
    if !a_desc.is_empty() && !b_desc.is_empty() {
        let common = a_desc.iter().filter(|t| b_desc.contains(t)).count();
        let total = a_desc.len().max(b_desc.len());
        score += 30.0 * (common as f64 / total as f64);
    }

    // Tag overlap
    max_score += 10.0;
    if !a.tags.is_empty() && !b.tags.is_empty() {
        let common = a.tags.iter().filter(|t| b.tags.contains(t)).count();
        let total = a.tags.len().max(b.tags.len());
        score += 10.0 * (common as f64 / total as f64);
    }

    if max_score == 0.0 {
        return 0.0;
    }
    score / max_score
}

/// Merge two knowledge entries: keep the higher-confidence one, absorb the other.
/// The absorbed entry is marked obsolete and superseded.
pub fn merge_knowledge(
    db_path: &Path,
    uuid_keep: &str,
    uuid_absorb: &str,
) -> crate::error::Result<()> {
    let mut data = crate::storage::load_knowledge_only(db_path)?;

    let keep_idx = data
        .knowledge
        .iter()
        .position(|k| k.uuid == uuid_keep)
        .ok_or_else(|| crate::error::KodexError::Other(format!("UUID not found: {uuid_keep}")))?;
    let absorb_idx = data
        .knowledge
        .iter()
        .position(|k| k.uuid == uuid_absorb)
        .ok_or_else(|| crate::error::KodexError::Other(format!("UUID not found: {uuid_absorb}")))?;

    // Absorb metadata from absorbed into keeper
    let absorb = data.knowledge[absorb_idx].clone();

    let keep = &mut data.knowledge[keep_idx];
    keep.observations += absorb.observations;
    keep.confidence = 1.0 - (1.0 - keep.confidence) * 0.8_f64.powi(absorb.observations as i32);
    for tag in &absorb.tags {
        if !keep.tags.contains(tag) {
            keep.tags.push(tag.clone());
        }
    }
    // Description: append if different
    if !absorb.description.is_empty() && keep.description != absorb.description {
        keep.description = format!("{}\n---\n{}", keep.description, absorb.description);
    }
    // Evidence: merge both
    if !absorb.evidence.is_empty() {
        if keep.evidence.is_empty() {
            keep.evidence = absorb.evidence.clone();
        } else {
            keep.evidence = format!("{}\n{}", keep.evidence, absorb.evidence);
        }
    }
    // applies_when: keep the more specific one (non-empty wins, both → merge)
    if keep.applies_when.is_empty() && !absorb.applies_when.is_empty() {
        keep.applies_when = absorb.applies_when.clone();
    } else if !absorb.applies_when.is_empty() && keep.applies_when != absorb.applies_when {
        keep.applies_when = format!("{}, {}", keep.applies_when, absorb.applies_when);
    }
    // scope: keep the narrower scope (file > module > project > repo)
    let scope_rank = |s: &str| match s {
        "node" => 5,
        "file" => 4,
        "module" => 3,
        "project" => 2,
        "repo" => 1,
        _ => 0,
    };
    if scope_rank(&absorb.scope) > scope_rank(&keep.scope) {
        keep.scope = absorb.scope.clone();
    }
    keep.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Transfer ALL outgoing links from absorbed to keeper (node + knowledge)
    let keep_uuid = uuid_keep.to_string();
    let absorb_uuid = uuid_absorb.to_string();
    for link in &mut data.links {
        if link.knowledge_uuid == absorb_uuid {
            // Skip self-referential links to keep (will add supersedes separately)
            if link.node_uuid == keep_uuid {
                continue;
            }
            link.knowledge_uuid = keep_uuid.clone();
        }
        // Also rewrite incoming knowledge links that point TO absorbed
        if link.is_knowledge_link() && link.node_uuid == absorb_uuid {
            // If keeper is the source, this becomes self-referential → will be cleaned below
            link.node_uuid = keep_uuid.clone();
        }
    }
    // Remove self-referential knowledge links and deduplicate
    data.links.retain(|l| {
        // Remove knowledge links that now point to themselves
        if l.is_knowledge_link() && l.knowledge_uuid == l.node_uuid {
            return false;
        }
        true
    });
    let mut seen = std::collections::HashSet::new();
    data.links.retain(|l| {
        let key = (
            l.knowledge_uuid.clone(),
            l.node_uuid.clone(),
            l.relation.clone(),
        );
        seen.insert(key)
    });

    // Mark absorbed as obsolete
    data.knowledge[absorb_idx].status = "obsolete".to_string();
    data.knowledge[absorb_idx].superseded_by = uuid_keep.to_string();

    // Add supersedes link
    let already_linked = data.links.iter().any(|l| {
        l.knowledge_uuid == uuid_keep && l.node_uuid == uuid_absorb && l.relation == "supersedes"
    });
    if !already_linked {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        data.links.push(crate::types::KnowledgeLink {
            knowledge_uuid: uuid_keep.to_string(),
            node_uuid: uuid_absorb.to_string(),
            relation: "supersedes".to_string(),
            target_type: "knowledge".to_string(),
            confidence: 1.0,
            created_at: now,
            source: "agent".to_string(),
            reason: "duplicate merge".to_string(),
            ..Default::default()
        });
    }

    crate::storage::save_knowledge_only(db_path, &data)
}

/// Remove a specific link by knowledge_uuid + target_uuid + relation.
pub fn remove_link(
    db_path: &Path,
    knowledge_uuid: &str,
    target_uuid: &str,
    relation: Option<&str>,
) -> crate::error::Result<bool> {
    let mut data = crate::storage::load_knowledge_only(db_path)?;
    let before = data.links.len();
    data.links.retain(|l| {
        !(l.knowledge_uuid == knowledge_uuid
            && l.node_uuid == target_uuid
            && relation.is_none_or(|r| l.relation == r))
    });
    let removed = before != data.links.len();
    if removed {
        crate::storage::save_knowledge_only(db_path, &data)?;
    }
    Ok(removed)
}

/// Link two knowledge entries together (knowledge ↔ knowledge).
pub fn link_knowledge_to_knowledge(
    db_path: &Path,
    source_uuid: &str,
    target_uuid: &str,
    relation: &str,
    bidirectional: bool,
) -> crate::error::Result<()> {
    let mut data = crate::storage::load_knowledge_only(db_path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Verify both exist
    let source_exists = data.knowledge.iter().any(|k| k.uuid == source_uuid);
    let target_exists = data.knowledge.iter().any(|k| k.uuid == target_uuid);
    if !source_exists {
        return Err(crate::error::KodexError::Other(format!(
            "Source knowledge not found: {source_uuid}"
        )));
    }
    if !target_exists {
        return Err(crate::error::KodexError::Other(format!(
            "Target knowledge not found: {target_uuid}"
        )));
    }

    // Add forward link
    let exists = data.links.iter().any(|l| {
        l.knowledge_uuid == source_uuid
            && l.node_uuid == target_uuid
            && l.relation == relation
            && l.is_knowledge_link()
    });
    if !exists {
        data.links.push(crate::types::KnowledgeLink {
            knowledge_uuid: source_uuid.to_string(),
            node_uuid: target_uuid.to_string(),
            relation: relation.to_string(),
            target_type: "knowledge".to_string(),
            confidence: 1.0,
            created_at: now,
            source: "agent".to_string(),
            ..Default::default()
        });
    }

    // Add reverse link if bidirectional
    if bidirectional {
        let reverse_rel = match relation {
            "supersedes" => "superseded_by",
            "superseded_by" => "supersedes",
            "depends_on" => "depended_by",
            "supports" => "supported_by",
            "contradicts" => "contradicts",
            other => other,
        };
        let rev_exists = data.links.iter().any(|l| {
            l.knowledge_uuid == target_uuid
                && l.node_uuid == source_uuid
                && l.relation == reverse_rel
                && l.is_knowledge_link()
        });
        if !rev_exists {
            data.links.push(crate::types::KnowledgeLink {
                knowledge_uuid: target_uuid.to_string(),
                node_uuid: source_uuid.to_string(),
                relation: reverse_rel.to_string(),
                target_type: "knowledge".to_string(),
                confidence: 1.0,
                created_at: now,
                source: "agent".to_string(),
                ..Default::default()
            });
        }
    }

    crate::storage::save_knowledge_only(db_path, &data)
}

/// Get all knowledge entries connected to a given knowledge UUID.
pub fn knowledge_neighbors(db_path: &Path, knowledge_uuid: &str) -> Vec<(String, String, String)> {
    let data = match crate::storage::load_knowledge_only(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    data.links
        .iter()
        .filter(|l| {
            l.is_knowledge_link()
                && (l.knowledge_uuid == knowledge_uuid || l.node_uuid == knowledge_uuid)
        })
        .map(|l| {
            let other = if l.knowledge_uuid == knowledge_uuid {
                l.node_uuid.clone()
            } else {
                l.knowledge_uuid.clone()
            };
            let direction = if l.knowledge_uuid == knowledge_uuid {
                "outgoing"
            } else {
                "incoming"
            };
            (other, l.relation.clone(), direction.to_string())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Chain of thought
// ---------------------------------------------------------------------------

/// A step in a thought chain.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThoughtStep {
    pub uuid: String,
    pub title: String,
    pub knowledge_type: String,
    pub description: String,
    pub confidence: f64,
    pub relation_to_next: Option<String>,
}

/// Trace the chain of thought starting from a knowledge UUID.
/// Follows `leads_to` links forward (and `because`/`resolved_by`/etc. as alternatives).
/// Also walks backward to find the chain root.
pub fn thought_chain(db_path: &Path, knowledge_uuid: &str) -> Vec<ThoughtStep> {
    let data = match crate::storage::load_knowledge_only(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let chain_relations = ["leads_to", "because", "resolved_by", "therefore", "implies"];

    // Build forward/backward adjacency for chain relations
    let mut forward: HashMap<String, (String, String)> = HashMap::new(); // uuid → (next, relation)
    let mut backward: HashMap<String, String> = HashMap::new(); // uuid → prev

    for link in &data.links {
        if link.is_knowledge_link() && chain_relations.contains(&link.relation.as_str()) {
            forward.insert(
                link.knowledge_uuid.clone(),
                (link.node_uuid.clone(), link.relation.clone()),
            );
            backward
                .entry(link.node_uuid.clone())
                .or_insert_with(|| link.knowledge_uuid.clone());
        }
    }

    // Walk backward to find chain root
    let mut root = knowledge_uuid.to_string();
    let mut visited = std::collections::HashSet::new();
    visited.insert(root.clone());
    while let Some(prev) = backward.get(&root) {
        if !visited.insert(prev.clone()) {
            break; // cycle
        }
        root = prev.clone();
    }

    // Walk forward from root
    let mut chain = Vec::new();
    let mut current = root;
    visited.clear();

    loop {
        if !visited.insert(current.clone()) {
            break; // cycle
        }

        let entry = data.knowledge.iter().find(|k| k.uuid == current);
        let (next, rel) = forward.get(&current).cloned().unzip();

        chain.push(ThoughtStep {
            uuid: current.clone(),
            title: entry.map(|e| e.title.clone()).unwrap_or_default(),
            knowledge_type: entry.map(|e| e.knowledge_type.clone()).unwrap_or_default(),
            description: entry.map(|e| e.description.clone()).unwrap_or_default(),
            confidence: entry.map(|e| e.confidence).unwrap_or(0.0),
            relation_to_next: rel,
        });

        match next {
            Some(n) => current = n,
            None => break,
        }
    }

    chain
}

/// Render a thought chain as readable markdown.
pub fn render_thought_chain(steps: &[ThoughtStep]) -> String {
    if steps.is_empty() {
        return "No thought chain found.".to_string();
    }

    let mut out = format!("## Thought Chain ({} steps)\n\n", steps.len());

    for (i, step) in steps.iter().enumerate() {
        let conf = (step.confidence * 100.0) as u32;
        let summary = step.description.lines().next().unwrap_or("");
        out.push_str(&format!(
            "{}. **{}** ({}, {conf}%)\n   {summary}\n",
            i + 1,
            step.title,
            step.knowledge_type,
        ));

        if let Some(rel) = &step.relation_to_next {
            out.push_str(&format!("   ↓ _{rel}_\n"));
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Knowledge graph traversal
// ---------------------------------------------------------------------------

/// A node in the knowledge graph (enriched with metadata for display).
#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeGraphNode {
    pub uuid: String,
    pub title: String,
    pub knowledge_type: String,
    pub confidence: f64,
    pub status: String,
    pub links_out: Vec<KnowledgeGraphEdge>,
    pub links_in: Vec<KnowledgeGraphEdge>,
    pub node_links: Vec<KnowledgeGraphEdge>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeGraphEdge {
    pub target_uuid: String,
    pub target_title: String,
    pub relation: String,
}

/// BFS traversal of the knowledge graph from a starting UUID.
/// Returns all reachable knowledge within `max_depth` hops.
/// If `start_uuid` is None, returns the entire knowledge graph.
pub fn traverse_knowledge_graph(
    db_path: &Path,
    start_uuid: Option<&str>,
    max_depth: usize,
) -> Vec<KnowledgeGraphNode> {
    let data = match crate::storage::load(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    // Build title lookup
    let uuid_to_title: HashMap<String, String> = data
        .knowledge
        .iter()
        .map(|k| (k.uuid.clone(), k.title.clone()))
        .collect();

    // Build adjacency: knowledge↔knowledge links
    let mut outgoing: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut incoming: HashMap<String, Vec<(String, String)>> = HashMap::new();
    // Node links (knowledge→node)
    let mut node_links: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for link in &data.links {
        if link.is_knowledge_link() {
            outgoing
                .entry(link.knowledge_uuid.clone())
                .or_default()
                .push((link.node_uuid.clone(), link.relation.clone()));
            incoming
                .entry(link.node_uuid.clone())
                .or_default()
                .push((link.knowledge_uuid.clone(), link.relation.clone()));
        } else {
            node_links
                .entry(link.knowledge_uuid.clone())
                .or_default()
                .push((link.node_uuid.clone(), link.relation.clone()));
        }
    }

    // Determine which UUIDs to include
    let included: std::collections::HashSet<String> = if let Some(start) = start_uuid {
        // BFS from start
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((start.to_string(), 0usize));
        visited.insert(start.to_string());

        while let Some((uuid, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            // Follow outgoing
            if let Some(edges) = outgoing.get(&uuid) {
                for (target, _) in edges {
                    if visited.insert(target.clone()) {
                        queue.push_back((target.clone(), depth + 1));
                    }
                }
            }
            // Follow incoming
            if let Some(edges) = incoming.get(&uuid) {
                for (source, _) in edges {
                    if visited.insert(source.clone()) {
                        queue.push_back((source.clone(), depth + 1));
                    }
                }
            }
        }
        visited
    } else {
        // All knowledge
        data.knowledge.iter().map(|k| k.uuid.clone()).collect()
    };

    // Build result
    data.knowledge
        .iter()
        .filter(|k| included.contains(&k.uuid))
        .map(|k| {
            let out = outgoing.get(&k.uuid).cloned().unwrap_or_default();
            let inc = incoming.get(&k.uuid).cloned().unwrap_or_default();
            let nl = node_links.get(&k.uuid).cloned().unwrap_or_default();

            KnowledgeGraphNode {
                uuid: k.uuid.clone(),
                title: k.title.clone(),
                knowledge_type: k.knowledge_type.clone(),
                confidence: k.confidence,
                status: k.status.clone(),
                links_out: out
                    .iter()
                    .map(|(target, rel)| KnowledgeGraphEdge {
                        target_uuid: target.clone(),
                        target_title: uuid_to_title.get(target).cloned().unwrap_or_default(),
                        relation: rel.clone(),
                    })
                    .collect(),
                links_in: inc
                    .iter()
                    .map(|(source, rel)| KnowledgeGraphEdge {
                        target_uuid: source.clone(),
                        target_title: uuid_to_title.get(source).cloned().unwrap_or_default(),
                        relation: rel.clone(),
                    })
                    .collect(),
                node_links: nl
                    .iter()
                    .map(|(node_uuid, rel)| {
                        // Try to resolve node label
                        let label = data
                            .extraction
                            .nodes
                            .iter()
                            .find(|n| n.uuid.as_deref() == Some(node_uuid.as_str()))
                            .map(|n| n.label.clone())
                            .unwrap_or_else(|| node_uuid.clone());
                        KnowledgeGraphEdge {
                            target_uuid: node_uuid.clone(),
                            target_title: label,
                            relation: rel.clone(),
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Render knowledge graph as markdown for agent consumption.
pub fn render_knowledge_graph(nodes: &[KnowledgeGraphNode]) -> String {
    if nodes.is_empty() {
        return "No knowledge in graph.".to_string();
    }

    let mut out = format!("## Knowledge Graph ({} entries)\n\n", nodes.len());

    for node in nodes {
        let conf = (node.confidence * 100.0) as u32;
        out.push_str(&format!(
            "### {} ({}, {conf}%)\n",
            node.title, node.knowledge_type
        ));

        if !node.links_out.is_empty() {
            for edge in &node.links_out {
                out.push_str(&format!(
                    "  → {} **{}**\n",
                    edge.relation, edge.target_title
                ));
            }
        }
        if !node.links_in.is_empty() {
            for edge in &node.links_in {
                out.push_str(&format!(
                    "  ← {} **{}**\n",
                    edge.relation, edge.target_title
                ));
            }
        }
        if !node.node_links.is_empty() {
            let labels: Vec<&str> = node
                .node_links
                .iter()
                .map(|e| e.target_title.as_str())
                .collect();
            out.push_str(&format!("  code: {}\n", labels.join(", ")));
        }
        out.push('\n');
    }

    out
}

// ---------------------------------------------------------------------------
// Conflict detection
// ---------------------------------------------------------------------------

/// A conflict between two knowledge entries.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeConflict {
    pub uuid_a: String,
    pub title_a: String,
    pub uuid_b: String,
    pub title_b: String,
    pub conflict_type: String,
    pub description: String,
}

/// Find conflicting knowledge entries:
/// - Same scope with opposing decisions
/// - Superseded but not marked obsolete
/// - High-confidence entries with contradicts links
pub fn detect_conflicts(db_path: &Path) -> Vec<KnowledgeConflict> {
    let data = match crate::storage::load_knowledge_only(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut conflicts = Vec::new();

    // 1. Superseded but not obsolete
    for entry in &data.knowledge {
        if !entry.superseded_by.is_empty() && entry.status != "obsolete" {
            if let Some(successor) = data
                .knowledge
                .iter()
                .find(|k| k.uuid == entry.superseded_by)
            {
                conflicts.push(KnowledgeConflict {
                    uuid_a: entry.uuid.clone(),
                    title_a: entry.title.clone(),
                    uuid_b: successor.uuid.clone(),
                    title_b: successor.title.clone(),
                    conflict_type: "superseded_not_obsolete".into(),
                    description: format!(
                        "'{}' superseded by '{}' but still active",
                        entry.title, successor.title
                    ),
                });
            }
        }
    }

    // 2. Contradicts links between active entries
    for link in &data.links {
        if link.is_knowledge_link() && link.relation == "contradicts" {
            let a = data
                .knowledge
                .iter()
                .find(|k| k.uuid == link.knowledge_uuid);
            let b = data.knowledge.iter().find(|k| k.uuid == link.node_uuid);
            if let (Some(a), Some(b)) = (a, b) {
                if a.status == "active" && b.status == "active" {
                    conflicts.push(KnowledgeConflict {
                        uuid_a: a.uuid.clone(),
                        title_a: a.title.clone(),
                        uuid_b: b.uuid.clone(),
                        title_b: b.title.clone(),
                        conflict_type: "contradiction".into(),
                        description: format!(
                            "'{}' contradicts '{}', both active",
                            a.title, b.title
                        ),
                    });
                }
            }
        }
    }

    // 3. Same type + same scope with different conclusions (decision/pattern conflicts)
    let active: Vec<&crate::types::KnowledgeEntry> = data
        .knowledge
        .iter()
        .filter(|k| {
            k.status == "active"
                && !k.scope.is_empty()
                && (k.knowledge_type == "decision" || k.knowledge_type == "pattern")
        })
        .collect();
    for i in 0..active.len() {
        for j in (i + 1)..active.len() {
            let a = active[i];
            let b = active[j];
            if a.knowledge_type == b.knowledge_type
                && a.scope == b.scope
                && knowledge_similarity(a, b) > 0.4
                && knowledge_similarity(a, b) < 0.8
            {
                // Similar but not duplicate — potential conflict
                conflicts.push(KnowledgeConflict {
                    uuid_a: a.uuid.clone(),
                    title_a: a.title.clone(),
                    uuid_b: b.uuid.clone(),
                    title_b: b.title.clone(),
                    conflict_type: "scope_overlap".into(),
                    description: format!(
                        "Same scope '{}', same type '{}' — may conflict",
                        a.scope, a.knowledge_type
                    ),
                });
            }
        }
    }

    conflicts
}

// ---------------------------------------------------------------------------
// Observability
// ---------------------------------------------------------------------------

/// Health metrics for the knowledge base.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeHealth {
    pub total_knowledge: usize,
    pub active: usize,
    pub tentative: usize,
    pub needs_review: usize,
    pub obsolete: usize,
    pub total_links: usize,
    pub node_links: usize,
    pub knowledge_links: usize,
    pub orphan_node_links: usize,
    pub orphan_knowledge_links: usize,
    pub duplicate_candidates: usize,
    pub conflicts: usize,
    pub avg_confidence: f64,
    pub avg_observations: f64,
    pub total_nodes: usize,
    pub validation_overdue: usize,
    pub recently_changed_7d: usize,
    pub recently_changed_30d: usize,
}

/// Compute health metrics for the knowledge base.
pub fn knowledge_health(db_path: &Path) -> KnowledgeHealth {
    let data = match crate::storage::load(db_path) {
        Ok(d) => d,
        Err(_) => {
            return KnowledgeHealth {
                total_knowledge: 0,
                active: 0,
                tentative: 0,
                needs_review: 0,
                obsolete: 0,
                total_links: 0,
                node_links: 0,
                knowledge_links: 0,
                orphan_node_links: 0,
                orphan_knowledge_links: 0,
                duplicate_candidates: 0,
                conflicts: 0,
                avg_confidence: 0.0,
                avg_observations: 0.0,
                total_nodes: 0,
                validation_overdue: 0,
                recently_changed_7d: 0,
                recently_changed_30d: 0,
            };
        }
    };

    let valid_node_uuids: std::collections::HashSet<&str> = data
        .extraction
        .nodes
        .iter()
        .filter_map(|n| n.uuid.as_deref())
        .collect();
    let valid_knowledge_uuids: std::collections::HashSet<&str> =
        data.knowledge.iter().map(|k| k.uuid.as_str()).collect();

    let mut active = 0;
    let mut tentative = 0;
    let mut needs_review = 0;
    let mut obsolete = 0;
    let mut total_conf = 0.0;
    let mut total_obs = 0u64;

    for k in &data.knowledge {
        match k.status.as_str() {
            "tentative" => tentative += 1,
            "needs_review" => needs_review += 1,
            "obsolete" => obsolete += 1,
            _ => active += 1,
        }
        total_conf += k.confidence;
        total_obs += k.observations as u64;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let validation_overdue = data
        .knowledge
        .iter()
        .filter(|k| {
            k.status == "active"
                && k.last_validated_at > 0
                && now > k.last_validated_at
                && (now - k.last_validated_at) / 86400 > 90
        })
        .count();
    let recently_changed_7d = data
        .knowledge
        .iter()
        .filter(|k| k.updated_at > 0 && now > k.updated_at && (now - k.updated_at) / 86400 < 7)
        .count();
    let recently_changed_30d = data
        .knowledge
        .iter()
        .filter(|k| k.updated_at > 0 && now > k.updated_at && (now - k.updated_at) / 86400 < 30)
        .count();

    let n = data.knowledge.len().max(1);
    let node_links = data.links.iter().filter(|l| !l.is_knowledge_link()).count();
    let knowledge_links = data.links.iter().filter(|l| l.is_knowledge_link()).count();

    let orphan_node_links = data
        .links
        .iter()
        .filter(|l| !l.is_knowledge_link() && !valid_node_uuids.contains(l.node_uuid.as_str()))
        .count();
    let orphan_knowledge_links = data
        .links
        .iter()
        .filter(|l| {
            l.is_knowledge_link()
                && (!valid_knowledge_uuids.contains(l.knowledge_uuid.as_str())
                    || !valid_knowledge_uuids.contains(l.node_uuid.as_str()))
        })
        .count();

    let duplicates = find_duplicates(db_path, 0.6).len();
    let conflicts_count = detect_conflicts(db_path).len();

    KnowledgeHealth {
        total_knowledge: data.knowledge.len(),
        active,
        tentative,
        needs_review,
        obsolete,
        total_links: data.links.len(),
        node_links,
        knowledge_links,
        orphan_node_links,
        orphan_knowledge_links,
        duplicate_candidates: duplicates,
        conflicts: conflicts_count,
        avg_confidence: total_conf / n as f64,
        avg_observations: total_obs as f64 / n as f64,
        total_nodes: data.extraction.nodes.len(),
        validation_overdue,
        recently_changed_7d,
        recently_changed_30d,
    }
}

// ---------------------------------------------------------------------------
// Review queue
// ---------------------------------------------------------------------------

/// Get the pending review queue, sorted by priority descending.
pub fn get_review_queue(db_path: &Path) -> Vec<crate::types::ReviewQueueItem> {
    let data = match crate::storage::load_knowledge_only(db_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut queue: Vec<_> = data
        .review_queue
        .into_iter()
        .filter(|q| !q.completed)
        .collect();
    queue.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.created_at.cmp(&b.created_at))
    });
    queue
}

/// Enqueue a knowledge entry for review.
/// Enqueue a knowledge entry for review. Returns true if actually enqueued.
pub fn enqueue_review(
    db_path: &Path,
    knowledge_uuid: &str,
    reason: &str,
    priority: u8,
) -> crate::error::Result<bool> {
    let mut data = crate::storage::load(db_path)?;
    // Don't duplicate
    if data
        .review_queue
        .iter()
        .any(|q| q.knowledge_uuid == knowledge_uuid && !q.completed)
    {
        return Ok(false);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    data.review_queue.push(crate::types::ReviewQueueItem {
        knowledge_uuid: knowledge_uuid.to_string(),
        reason: reason.to_string(),
        created_at: now,
        priority,
        completed: false,
    });
    crate::storage::save_knowledge_only(db_path, &data)?;
    Ok(true)
}

/// Complete a review queue item (mark as done).
pub fn complete_review(db_path: &Path, knowledge_uuid: &str) -> crate::error::Result<()> {
    let mut data = crate::storage::load(db_path)?;
    for item in &mut data.review_queue {
        if item.knowledge_uuid == knowledge_uuid && !item.completed {
            item.completed = true;
        }
    }
    crate::storage::save_knowledge_only(db_path, &data)
}

/// Auto-enqueue stale/conflict/duplicate items for review.
pub fn refresh_review_queue(db_path: &Path) -> crate::error::Result<usize> {
    let stale = detect_stale_detailed(db_path)?;
    let conflicts = detect_conflicts(db_path);
    let duplicates = find_duplicates(db_path, 0.6);

    let mut count = 0;
    for s in &stale {
        if enqueue_review(db_path, &s.uuid, &format!("stale: {}", s.reason), 7)? {
            count += 1;
        }
    }
    for c in &conflicts {
        if enqueue_review(
            db_path,
            &c.uuid_a,
            &format!("conflict: {}", c.description),
            8,
        )? {
            count += 1;
        }
    }
    for d in &duplicates {
        if enqueue_review(db_path, &d.uuid_a, &format!("duplicate: {}", d.reason), 5)? {
            count += 1;
        }
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// Diff-aware recall
// ---------------------------------------------------------------------------

/// Recall knowledge relevant to a git diff.
pub fn recall_for_diff(
    db_path: &Path,
    diff_text: &str,
    max_items: usize,
) -> (crate::diff::DiffAnalysis, Vec<RecallResult>) {
    let analysis = match crate::diff::analyze_diff(diff_text, db_path) {
        Ok(a) => a,
        Err(_) => {
            return (
                crate::diff::DiffAnalysis {
                    hunks_count: 0,
                    changed_files: vec![],
                    changed_node_uuids: vec![],
                    affected_knowledge_uuids: vec![],
                },
                vec![],
            )
        }
    };

    let mut results = recall_for_task_structured(
        db_path,
        "",
        &analysis.changed_files,
        &analysis.changed_node_uuids,
        max_items * 2, // fetch extra for re-ranking
        None,
    );

    // Boost knowledge directly affected by the diff
    let affected: std::collections::HashSet<&str> = analysis
        .affected_knowledge_uuids
        .iter()
        .map(|s| s.as_str())
        .collect();
    for item in &mut results {
        if affected.contains(item.knowledge.uuid.as_str()) {
            item.score.total += 20.0;
            item.score.reasons.push("directly affected by diff".into());
        }
    }

    // Re-sort after boost and trim to max_items
    results.sort_by(|a, b| {
        b.score
            .total
            .partial_cmp(&a.score.total)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(max_items);

    (analysis, results)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_db(dir: &std::path::Path) -> std::path::PathBuf {
        let db_path = dir.join("test.db");
        // Create a minimal SQLite with empty graph
        let extraction = crate::types::ExtractionResult::default();
        let graph = crate::graph::build_from_extraction(&extraction);
        let communities = crate::cluster::cluster(&graph);
        crate::storage::save_db(&graph, &communities, &db_path).unwrap();
        db_path
    }

    #[test]
    fn test_learn_and_load() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        learn(
            &db,
            KnowledgeType::Pattern,
            "Repository Pattern",
            "All data access goes through Repository classes",
            &["user_repo".to_string()],
            &["architecture".to_string()],
        )
        .unwrap();

        let items = query_knowledge(&db, "", None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Repository Pattern");
        assert_eq!(items[0].observations, 1);
        let conf1 = items[0].confidence;
        let uuid = items[0].uuid.clone();

        // Reinforce using UUID
        learn_with_uuid(
            &db,
            Some(&uuid),
            KnowledgeType::Pattern,
            "Repository Pattern",
            "Confirmed: ProductRepo also follows this",
            Some(&["product_repo".to_string()]),
            &[],
            None,
        )
        .unwrap();

        let items = query_knowledge(&db, "", None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].observations, 2);
        assert!(items[0].confidence > conf1, "Confidence should increase");
    }

    #[test]
    fn test_find_similar_to_uuid() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        // Save two near-identical entries and one unrelated
        let uuid_a = learn_with_uuid(
            &db,
            None,
            KnowledgeType::BugPattern,
            "PVA BEACON header is 4 bytes after GUID, not 5",
            "pvxs server emits beacon metadata as flags:u8 + seq:u8 + change:u16",
            None,
            &[],
            None,
        )
        .unwrap();
        let uuid_b = learn_with_uuid(
            &db,
            None,
            KnowledgeType::BugPattern,
            "PVA BEACON header layout — 4 bytes after GUID",
            "pvxs server beacon metadata: flags+seq+change totals 4 bytes after GUID",
            None,
            &[],
            None,
        )
        .unwrap();
        let _uuid_c = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Convention,
            "Error Handling",
            "Use AppError",
            None,
            &[],
            None,
        )
        .unwrap();

        let cands = find_similar_to_uuid(&db, &uuid_a, 0.6);
        assert_eq!(cands.len(), 1, "should find exactly one near-duplicate");
        assert_eq!(cands[0].uuid, uuid_b);
        assert!(cands[0].similarity >= 0.6);

        // Higher threshold suppresses
        let cands = find_similar_to_uuid(&db, &uuid_a, 0.99);
        assert!(cands.is_empty());

        // Unknown uuid returns empty
        assert!(find_similar_to_uuid(&db, "nonexistent", 0.6).is_empty());
    }

    #[test]
    fn test_query_knowledge() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        learn(
            &db,
            KnowledgeType::Pattern,
            "Singleton",
            "Global state",
            &[],
            &["design".to_string()],
        )
        .unwrap();
        learn(
            &db,
            KnowledgeType::Convention,
            "Error Handling",
            "Use AppError",
            &[],
            &["rust".to_string()],
        )
        .unwrap();
        learn(
            &db,
            KnowledgeType::Decision,
            "JWT Auth",
            "Chose JWT",
            &[],
            &["auth".to_string()],
        )
        .unwrap();

        let all = query_knowledge(&db, "", None);
        assert_eq!(all.len(), 3);

        let patterns = query_knowledge(&db, "", Some("pattern"));
        assert_eq!(patterns.len(), 1);

        let auth = query_knowledge(&db, "auth", None);
        assert_eq!(auth.len(), 1);
    }

    #[test]
    fn test_recency_penalizes_old_entries() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        // Two entries with sufficiently distinct titles to survive diversity collapse.
        let fresh_uuid = learn_with_uuid(
            &db,
            None,
            KnowledgeType::BugPattern,
            "fresh recent bug regarding tokens",
            "body",
            None,
            &[],
            None,
        )
        .unwrap();
        let stale_uuid = learn_with_uuid(
            &db,
            None,
            KnowledgeType::BugPattern,
            "ancient deprecated migration topic",
            "body",
            None,
            &[],
            None,
        )
        .unwrap();

        // Backdate stale entry to 200 days ago via direct SQL.
        let conn = rusqlite::Connection::open(&db).unwrap();
        let old = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(200 * 86400) as i64;
        conn.execute(
            "UPDATE knowledge SET created_at = ?1, updated_at = ?1, last_validated_at = 0, last_fetched = 0 WHERE uuid = ?2",
            rusqlite::params![old, stale_uuid],
        )
        .unwrap();
        crate::storage::cache_remove(&db);

        let results = recall_for_task_structured(&db, "", &[], &[], 10, None);
        let fresh_score = results
            .iter()
            .find(|r| r.knowledge.uuid == fresh_uuid)
            .unwrap()
            .score
            .total;
        let stale_score = results
            .iter()
            .find(|r| r.knowledge.uuid == stale_uuid)
            .unwrap()
            .score
            .total;
        assert!(
            fresh_score - stale_score >= 15.0,
            "fresh entry should outrank 200-day-old entry by ≥15 points: fresh={fresh_score}, stale={stale_score}"
        );
    }

    #[test]
    fn test_learn_supersedes_propagates_back_reference() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        let old_uuid = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Convention,
            "Use HDF5 backend",
            "All knowledge stored in HDF5 files",
            None,
            &[],
            None,
        )
        .unwrap();

        // Save a new entry that supersedes the old one
        let new_uuid = learn_supersedes(
            &db,
            KnowledgeType::Convention,
            "Use SQLite backend",
            "All knowledge stored in SQLite (replaces HDF5)",
            None,
            &[],
            &old_uuid,
        )
        .unwrap();

        let data = crate::storage::load_knowledge_only(&db).unwrap();
        let old = data.knowledge.iter().find(|k| k.uuid == old_uuid).unwrap();
        let new = data.knowledge.iter().find(|k| k.uuid == new_uuid).unwrap();

        assert_eq!(old.status, "obsolete");
        assert_eq!(old.superseded_by, new_uuid);
        assert!(
            old.confidence < 0.6,
            "obsolete entry confidence should decay: {}",
            old.confidence
        );
        assert_eq!(new.supersedes, old_uuid);
    }

    #[test]
    fn test_update_knowledge_rejects_unknown_superseded_by() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());
        let uuid =
            learn_with_uuid(&db, None, KnowledgeType::Pattern, "x", "y", None, &[], None).unwrap();
        let updates = KnowledgeUpdates {
            superseded_by: Some("does-not-exist".to_string()),
            ..Default::default()
        };
        let res = update_knowledge(&db, &uuid, &updates);
        assert!(res.is_err(), "unknown target should error");
    }

    #[test]
    fn test_auto_link_knowledge_cluster() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        // Save several entries on the same topic (epics-pva-rs cluster)
        let a = learn_with_uuid(
            &db,
            None,
            KnowledgeType::BugPattern,
            "epics-pva-rs handshake bug",
            "client failed pvxs handshake when bitset decode misaligned",
            None,
            &["pva".to_string(), "epics".to_string()],
            None,
        )
        .unwrap();
        let b = learn_with_uuid(
            &db,
            None,
            KnowledgeType::BugPattern,
            "epics-pva-rs bitset decode bug",
            "bitset decode misaligned during pvxs handshake",
            None,
            &["pva".to_string(), "epics".to_string()],
            None,
        )
        .unwrap();
        // Unrelated entry should NOT auto-link
        let _c = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Convention,
            "Use AppError",
            "for error handling in rust modules",
            None,
            &["rust".to_string()],
            None,
        )
        .unwrap();

        let data = crate::storage::load_knowledge_only(&db).unwrap();
        let kk_links: Vec<&crate::types::KnowledgeLink> = data
            .links
            .iter()
            .filter(|l| l.is_knowledge_link())
            .collect();
        assert!(
            !kk_links.is_empty(),
            "expected at least one auto knowledge↔knowledge link"
        );
        // The auto-link is created when `b` is saved → b → a edge
        assert!(
            kk_links
                .iter()
                .any(|l| l.knowledge_uuid == b && l.node_uuid == a),
            "expected b→a auto-link, got: {:?}",
            kk_links
                .iter()
                .map(|l| (l.knowledge_uuid.clone(), l.node_uuid.clone()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_auto_provenance_non_git() {
        let dir = TempDir::new().unwrap();
        let prov = auto_provenance(dir.path());
        assert!(prov.is_some(), "should produce a provenance string");
        let s = prov.unwrap();
        // Outside a git repo → cwd:basename
        assert!(s.starts_with("cwd:"), "expected cwd: prefix, got {s}");
    }

    #[test]
    fn test_set_evidence_if_empty_only_fills_blank() {
        use crate::storage::set_evidence_if_empty;
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        let uuid = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Pattern,
            "p1",
            "body",
            None,
            &[],
            None,
        )
        .unwrap();

        // First call writes
        set_evidence_if_empty(&db, &uuid, "commit:abc123@kodex").unwrap();
        let data = crate::storage::load_knowledge_only(&db).unwrap();
        let entry = data.knowledge.iter().find(|k| k.uuid == uuid).unwrap();
        assert_eq!(entry.evidence, "commit:abc123@kodex");

        // Second call must not overwrite
        set_evidence_if_empty(&db, &uuid, "different").unwrap();
        let data = crate::storage::load_knowledge_only(&db).unwrap();
        let entry = data.knowledge.iter().find(|k| k.uuid == uuid).unwrap();
        assert_eq!(entry.evidence, "commit:abc123@kodex");
    }

    #[test]
    fn test_bump_fetch_counters() {
        use crate::storage::bump_fetch_counters;
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        let uuid = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Pattern,
            "fetched entry",
            "body",
            None,
            &[],
            None,
        )
        .unwrap();

        let before = query_knowledge(&db, "", None);
        let entry = before.iter().find(|k| k.uuid == uuid).unwrap();
        let conf_before = entry.confidence;

        bump_fetch_counters(&db, std::slice::from_ref(&uuid)).unwrap();
        bump_fetch_counters(&db, std::slice::from_ref(&uuid)).unwrap();

        // Re-load via storage to see fetch_count + last_fetched
        let data = crate::storage::load_knowledge_only(&db).unwrap();
        let entry = data.knowledge.iter().find(|k| k.uuid == uuid).unwrap();
        assert_eq!(entry.fetch_count, 2);
        assert!(entry.last_fetched > 0);
        assert!(
            entry.confidence > conf_before,
            "confidence should grow: {} > {}",
            entry.confidence,
            conf_before
        );
        // Cap at 0.95: bump 50 more times and confirm no overshoot
        for _ in 0..50 {
            bump_fetch_counters(&db, std::slice::from_ref(&uuid)).unwrap();
        }
        let data = crate::storage::load_knowledge_only(&db).unwrap();
        let entry = data.knowledge.iter().find(|k| k.uuid == uuid).unwrap();
        assert!(entry.confidence <= 0.95 + 1e-9);
    }

    #[test]
    fn test_recall_for_task_type_filter() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        learn(
            &db,
            KnowledgeType::BugPattern,
            "auth bug",
            "session token issue",
            &[],
            &[],
        )
        .unwrap();
        learn(
            &db,
            KnowledgeType::Convention,
            "auth convention",
            "use AppError for auth failures",
            &[],
            &[],
        )
        .unwrap();
        learn(
            &db,
            KnowledgeType::Decision,
            "auth decision",
            "chose JWT",
            &[],
            &[],
        )
        .unwrap();

        // No filter → all three
        let all = recall_for_task(&db, "auth", &[], &[], 10, None);
        assert_eq!(all.len(), 3);

        // Filter to bug_pattern only
        let bugs = recall_for_task(&db, "auth", &[], &[], 10, Some("bug_pattern"));
        assert_eq!(bugs.len(), 1);
        assert_eq!(bugs[0].title, "auth bug");

        // Filter case-insensitive
        let convs = recall_for_task(&db, "auth", &[], &[], 10, Some("CONVENTION"));
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].title, "auth convention");

        // Unknown type → empty
        let none = recall_for_task(&db, "auth", &[], &[], 10, Some("nonexistent"));
        assert!(none.is_empty());
    }

    #[test]
    fn test_query_knowledge_multi_token() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        learn(
            &db,
            KnowledgeType::Architecture,
            "PVA wire format encoding",
            "How epics-pva-rs encodes the protocol buffer for pvxs interop.",
            &[],
            &[],
        )
        .unwrap();
        learn(
            &db,
            KnowledgeType::Pattern,
            "Connection retry logic",
            "Exponential backoff for reconnect after socket close.",
            &[],
            &[],
        )
        .unwrap();
        learn(
            &db,
            KnowledgeType::Decision,
            "Use SQLite over HDF5",
            "Switched storage backend for portability.",
            &[],
            &[],
        )
        .unwrap();

        // Multi-word natural-language query — must find the PVA item via title tokens
        let r = query_knowledge(&db, "pva wire format epics-pva-rs pvxs", None);
        assert!(
            r.iter().any(|k| k.title.contains("PVA wire format")),
            "multi-token query should find PVA item, got {:?}",
            r.iter().map(|k| &k.title).collect::<Vec<_>>()
        );

        // Description-only token must hit (was missing from the index)
        let r = query_knowledge(&db, "backoff", None);
        assert_eq!(r.len(), 1);
        assert!(r[0].title.contains("Connection retry"));

        // Ranking: title hit should outrank description hit when both present
        let r = query_knowledge(&db, "pva backoff", None);
        assert!(r.len() >= 2);
        assert!(
            r[0].title.contains("PVA"),
            "title-token hit should rank first, got {:?}",
            r.iter().map(|k| &k.title).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_knowledge_context() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        learn(
            &db,
            KnowledgeType::Pattern,
            "Observer",
            "Event-driven",
            &[],
            &[],
        )
        .unwrap();
        learn(
            &db,
            KnowledgeType::Preference,
            "Functional Style",
            "User prefers FP",
            &[],
            &[],
        )
        .unwrap();

        let ctx = knowledge_context(&db, 10, 0);
        assert!(ctx.contains("Observer"));
        assert!(ctx.contains("Functional Style"));
        assert!(ctx.contains("Knowledge:"));
        // Without inline_top_k, no Inline section
        assert!(!ctx.contains("## Inline"));
    }

    #[test]
    fn test_knowledge_context_inline_top_k() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        learn(
            &db,
            KnowledgeType::Pattern,
            "First Pattern",
            "Detailed body of the first pattern. Multiple lines of context.",
            &[],
            &[],
        )
        .unwrap();
        learn(
            &db,
            KnowledgeType::Convention,
            "Second Convention",
            "Body of the second entry.",
            &[],
            &[],
        )
        .unwrap();

        let ctx = knowledge_context(&db, 10, 2);
        assert!(ctx.contains("## Inline"));
        // First-line description must show in inline
        assert!(ctx.contains("Detailed body of the first pattern"));
        assert!(ctx.contains("Body of the second entry"));
    }

    #[test]
    fn test_knowledge_links_exclude_knowledge_targets() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        // Create two knowledge entries
        let k1 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Pattern,
            "Pattern A",
            "desc",
            Some(&["node-1".to_string()]),
            &[],
            None,
        )
        .unwrap();
        let k2 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Pattern,
            "Pattern B",
            "desc",
            Some(&["node-2".to_string()]),
            &[],
            None,
        )
        .unwrap();

        // Link knowledge ↔ knowledge
        link_knowledge_to_knowledge(&db, &k1, &k2, "supports", true).unwrap();

        // query_knowledge should only return node UUIDs in related_nodes
        let items = query_knowledge(&db, "", None);
        for item in &items {
            // related_nodes should never contain a knowledge UUID
            assert!(
                !item.related_nodes.contains(&k1),
                "k1 UUID leaked into related_nodes"
            );
            assert!(
                !item.related_nodes.contains(&k2),
                "k2 UUID leaked into related_nodes"
            );
        }
        let a = items.iter().find(|k| k.title == "Pattern A").unwrap();
        assert_eq!(a.related_nodes, vec!["node-1"]);
        let b = items.iter().find(|k| k.title == "Pattern B").unwrap();
        assert_eq!(b.related_nodes, vec!["node-2"]);
    }

    #[test]
    fn test_stale_detection_ignores_knowledge_links() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        // K1 has only knowledge↔knowledge links (no node links)
        let k1 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Pattern,
            "Pure Knowledge",
            "no nodes",
            None,
            &[],
            None,
        )
        .unwrap();
        let k2 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Decision,
            "Another",
            "also no nodes",
            None,
            &[],
            None,
        )
        .unwrap();
        link_knowledge_to_knowledge(&db, &k1, &k2, "depends_on", false).unwrap();

        // Stale detection should NOT mark these as needs_review
        let stale = detect_stale_knowledge(&db).unwrap();
        assert_eq!(
            stale, 0,
            "knowledge-only entries should not be marked stale"
        );

        let items = query_knowledge(&db, "Pure Knowledge", None);
        assert_eq!(items[0].uuid, k1);
        // status should still be active (not needs_review)
    }

    #[test]
    fn test_thought_chain_formation() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        let k1 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Pattern,
            "Step 1",
            "first",
            None,
            &[],
            None,
        )
        .unwrap();
        let k2 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Decision,
            "Step 2",
            "second",
            None,
            &[],
            Some(&k1),
        )
        .unwrap();
        let k3 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Convention,
            "Step 3",
            "third",
            None,
            &[],
            Some(&k2),
        )
        .unwrap();

        // Chain from any node should give all 3 steps in order
        let chain = thought_chain(&db, &k2);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].uuid, k1);
        assert_eq!(chain[1].uuid, k2);
        assert_eq!(chain[2].uuid, k3);
        assert_eq!(chain[0].relation_to_next.as_deref(), Some("leads_to"));
        assert_eq!(chain[1].relation_to_next.as_deref(), Some("leads_to"));
        assert!(chain[2].relation_to_next.is_none());
    }

    #[test]
    fn test_load_knowledge_entries_node_only() {
        let dir = TempDir::new().unwrap();
        let db = make_test_db(dir.path());

        let k1 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Pattern,
            "Mixed",
            "has both link types",
            Some(&["node-x".to_string()]),
            &[],
            None,
        )
        .unwrap();
        let k2 = learn_with_uuid(
            &db,
            None,
            KnowledgeType::Decision,
            "Other",
            "target",
            None,
            &[],
            None,
        )
        .unwrap();
        link_knowledge_to_knowledge(&db, &k1, &k2, "supports", false).unwrap();

        let entries = crate::storage::load_knowledge_entries(&db).unwrap();
        let mixed = entries.iter().find(|e| e.0 == "Mixed").unwrap();
        // related field (index 5) should only contain "node-x", not k2's UUID
        assert_eq!(mixed.5, "node-x");
        assert!(!mixed.5.contains(&k2));
    }
}
