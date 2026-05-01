pub mod call_graph;
pub mod chunker;
pub mod config;
pub mod generic;
pub mod languages;

#[cfg(feature = "extract")]
use std::collections::HashMap;
#[cfg(feature = "extract")]
use std::path::{Path, PathBuf};

#[cfg(feature = "extract")]
use crate::types::ExtractionResult;

#[cfg(feature = "extract")]
use crate::cache;

/// Extension-to-language dispatch table.
#[cfg(feature = "extract")]
fn dispatch_table() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert(".py", "python");
    m.insert(".js", "javascript");
    m.insert(".jsx", "javascript");
    m.insert(".mjs", "javascript");
    m.insert(".ts", "javascript");
    m.insert(".tsx", "javascript");
    m.insert(".go", "go");
    m.insert(".rs", "rust");
    m.insert(".java", "java");
    m.insert(".c", "c");
    m.insert(".h", "c");
    m.insert(".cpp", "cpp");
    m.insert(".cc", "cpp");
    m.insert(".cxx", "cpp");
    m.insert(".hpp", "cpp");
    m.insert(".rb", "ruby");
    m.insert(".cs", "csharp");
    m.insert(".kt", "kotlin");
    m.insert(".kts", "kotlin");
    m.insert(".scala", "scala");
    m.insert(".php", "php");
    m.insert(".swift", "swift");
    m.insert(".lua", "lua");
    m.insert(".toc", "lua");
    m.insert(".vue", "javascript");
    m.insert(".svelte", "javascript");
    m
}

/// Extract AST nodes and edges from a list of code files.
///
/// Two-pass process:
/// 1. Per-file structural extraction (classes, functions, imports)
/// 2. Cross-file import resolution and call resolution
#[cfg(feature = "extract")]
pub fn extract(paths: &[PathBuf], cache_root: Option<&Path>) -> ExtractionResult {
    let table = dispatch_table();

    let root = cache_root
        .map(|p| p.to_path_buf())
        .or_else(|| infer_common_root(paths))
        .unwrap_or_else(|| PathBuf::from("."));

    // Build work items: (path, lang) pairs
    let work: Vec<(&PathBuf, &str)> = paths
        .iter()
        .filter_map(|path| {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{}", e.to_lowercase()))
                .unwrap_or_default();
            table.get(ext.as_str()).map(|&lang| (path, lang))
        })
        .collect();

    // Extract files (parallel with rayon if enabled, sequential otherwise)
    #[cfg(feature = "parallel")]
    let per_file: Vec<ExtractionResult> = {
        use rayon::prelude::*;
        work.par_iter()
            .map(|(path, lang)| extract_or_cache(path, lang, &root))
            .collect()
    };

    #[cfg(not(feature = "parallel"))]
    let per_file: Vec<ExtractionResult> = work
        .iter()
        .map(|(path, lang)| extract_or_cache(path, lang, &root))
        .collect();

    // Aggregate
    let mut all_nodes = Vec::new();
    let mut all_edges = Vec::new();
    let mut all_raw_calls = Vec::new();

    for result in &per_file {
        all_nodes.extend(result.nodes.clone());
        all_edges.extend(result.edges.clone());
        all_raw_calls.extend(result.raw_calls.clone());
    }

    // Cross-file call resolution.
    //
    // Receiver-aware disambiguation: when multiple nodes share a label
    // (e.g. `Database.query` and `HttpClient.query`), use the RawCall's
    // `receiver` / `receiver_is_self` to pick the right target. When we
    // can't disambiguate, the call is dropped rather than mis-routed —
    // a wrong "calls" edge silently misleads navigation more than a
    // missing one.
    let mut label_to_nids: HashMap<String, Vec<String>> = HashMap::new();
    for node in &all_nodes {
        let label = node.label.trim_end_matches("()").to_lowercase();
        if !label.is_empty() {
            label_to_nids
                .entry(label)
                .or_default()
                .push(node.id.clone());
        }
    }

    // Build {method_nid → containing_class_label_lower} from "method" edges.
    // Class label is the source of a `method` edge; method nid is the target.
    let nid_to_label_lower: HashMap<String, String> = all_nodes
        .iter()
        .map(|n| (n.id.clone(), n.label.trim_end_matches("()").to_lowercase()))
        .collect();
    let mut method_to_class_label: HashMap<String, String> = HashMap::new();
    for edge in &all_edges {
        if edge.relation == "method" {
            if let Some(class_label) = nid_to_label_lower.get(&edge.source) {
                method_to_class_label.insert(edge.target.clone(), class_label.clone());
            }
        }
    }

    // Build inheritance: class_label_lower → Set<base_class_label_lower>.
    // Cross-file inheritance: extract_inheritance writes the target nid using
    // the caller's stem (e.g. `sub_base` when Sub is in sub.py), so a Base
    // class in base.py won't match. Fall back to `original_tgt`, which the
    // extractor populates with the literal base name.
    let mut class_inheritance: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for edge in &all_edges {
        if edge.relation == "extends" {
            let class_label = match nid_to_label_lower.get(&edge.source) {
                Some(c) => c.clone(),
                None => continue,
            };
            let base_label = nid_to_label_lower
                .get(&edge.target)
                .cloned()
                .or_else(|| edge.original_tgt.as_ref().map(|s| s.to_lowercase()));
            if let Some(b) = base_label {
                class_inheritance.entry(class_label).or_default().insert(b);
            }
        }
    }

    for rc in &all_raw_calls {
        let callee_lower = rc.callee.to_lowercase();
        let candidates = match label_to_nids.get(&callee_lower) {
            Some(c) if !c.is_empty() => c,
            _ => continue,
        };

        let target_nid: Option<&String> = if candidates.len() == 1 {
            Some(&candidates[0])
        } else if rc.receiver_is_self {
            // self.method() → method whose containing class matches caller's
            // class, walking inheritance if a direct match misses.
            method_to_class_label
                .get(&rc.caller_nid)
                .and_then(|caller_class| {
                    find_in_class_chain(
                        candidates,
                        caller_class,
                        &method_to_class_label,
                        &class_inheritance,
                        true, // include caller's own class
                    )
                })
        } else if rc.receiver.as_deref().map(is_super_ref).unwrap_or(false) {
            // super.method() → walk inheritance chain from caller's class,
            // skipping the caller's own class.
            method_to_class_label
                .get(&rc.caller_nid)
                .and_then(|caller_class| {
                    find_in_class_chain(
                        candidates,
                        caller_class,
                        &method_to_class_label,
                        &class_inheritance,
                        false, // skip caller's own class
                    )
                })
        } else if let Some(recv) = rc.receiver.as_deref() {
            // Type::method() or Type.method() — match receiver to class label,
            // walking the receiver's inheritance chain if a direct match misses
            // (e.g., `Sub::inherited_method()` resolves to `Base::method`).
            let recv_lower = recv.to_lowercase();
            find_in_class_chain(
                candidates,
                &recv_lower,
                &method_to_class_label,
                &class_inheritance,
                true,
            )
        } else {
            // Bare call with multiple candidates and no receiver hint:
            // ambiguous, skip rather than guess.
            None
        };

        if let Some(tgt) = target_nid {
            if *tgt != rc.caller_nid {
                all_edges.push(crate::types::Edge {
                    source: rc.caller_nid.clone(),
                    target: tgt.clone(),
                    relation: "calls".to_string(),
                    confidence: crate::types::Confidence::INFERRED,
                    source_file: rc.source_file.clone(),
                    source_location: rc.source_location.clone(),
                    confidence_score: Some(0.8),
                    weight: 0.8,
                    original_src: None,
                    original_tgt: None,
                });
            }
        }
    }

    // Cross-file Python import resolution
    #[cfg(feature = "lang-python")]
    {
        let py_paths: Vec<&PathBuf> = paths
            .iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
            .collect();
        if !py_paths.is_empty() {
            let cross_edges = resolve_cross_file_imports(&per_file, &py_paths, &all_nodes);
            all_edges.extend(cross_edges);
        }
    }

    // Python rationale extraction
    #[cfg(feature = "lang-python")]
    {
        for path in paths {
            if path.extension().and_then(|e| e.to_str()) == Some("py") {
                let (rat_nodes, rat_edges) = extract_python_rationale(path, &all_nodes);
                all_nodes.extend(rat_nodes);
                all_edges.extend(rat_edges);
            }
        }
    }

    ExtractionResult {
        nodes: all_nodes,
        edges: all_edges,
        hyperedges: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        raw_calls: Vec::new(),
        error: None,
    }
}

/// `super` keyword in Java/Python/JS/Scala/Kotlin — refers to the parent
/// class. Distinct from `self`/`this`/`Self` because it intentionally skips
/// the caller's own class. Accepts `super` and `super()` (Python's
/// `super().method()` form, where the receiver expression is the call
/// `super()`).
#[cfg(feature = "extract")]
pub(crate) fn is_super_ref(s: &str) -> bool {
    let t = s.trim().trim_end_matches("()").trim();
    t == "super"
}

/// BFS over the inheritance chain rooted at `start_class`, returning the
/// first candidate whose containing class label appears in the chain.
///
/// `include_root` is true for `self.method()` (caller's own class is a valid
/// match) and false for `super.method()` (skip own class, only ancestors).
#[cfg(feature = "extract")]
fn find_in_class_chain<'a>(
    candidates: &'a [String],
    start_class: &str,
    method_to_class_label: &HashMap<String, String>,
    class_inheritance: &HashMap<String, std::collections::HashSet<String>>,
    include_root: bool,
) -> Option<&'a String> {
    use std::collections::{HashSet, VecDeque};

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    if include_root {
        queue.push_back(start_class.to_string());
    } else if let Some(parents) = class_inheritance.get(start_class) {
        for p in parents {
            queue.push_back(p.clone());
        }
    }

    while let Some(class_label) = queue.pop_front() {
        if !visited.insert(class_label.clone()) {
            continue;
        }
        if let Some(found) = candidates
            .iter()
            .find(|nid| method_to_class_label.get(*nid) == Some(&class_label))
        {
            return Some(found);
        }
        if let Some(parents) = class_inheritance.get(&class_label) {
            for p in parents {
                queue.push_back(p.clone());
            }
        }
    }
    None
}

/// Two-pass cross-file import resolution for Python.
///
/// Turns file-level `from .models import X` into class-level INFERRED edges:
/// `DigestAuth --uses--> Response`
#[cfg(feature = "lang-python")]
fn resolve_cross_file_imports(
    per_file: &[ExtractionResult],
    py_paths: &[&PathBuf],
    all_nodes: &[crate::types::Node],
) -> Vec<crate::types::Edge> {
    use tree_sitter::Parser;

    let mut parser = Parser::new();
    let language = (languages::python::PYTHON_CONFIG.ts_language)();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }

    // Pass 1: build stem → {ClassName: node_id}
    let mut stem_to_entities: HashMap<String, HashMap<String, String>> = HashMap::new();
    for node in all_nodes {
        if node.source_file.is_empty() {
            continue;
        }
        let stem = Path::new(&node.source_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let label = &node.label;
        // Only index real classes/functions (not file nodes, not method stubs)
        if !label.is_empty()
            && !label.ends_with("()")
            && !label.ends_with(".py")
            && !label.starts_with('_')
        {
            stem_to_entities
                .entry(stem)
                .or_default()
                .insert(label.clone(), node.id.clone());
        }
    }

    // Pass 2: for each file, find `from .X import A, B` and resolve
    let mut new_edges = Vec::new();

    for (file_idx, &path) in py_paths.iter().enumerate() {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let str_path = path.to_string_lossy().to_string();
        let file_nid = crate::id::make_id(&[stem]);

        // Find local classes defined in this file
        let local_classes: Vec<String> = if file_idx < per_file.len() {
            per_file[file_idx]
                .nodes
                .iter()
                .filter(|n| {
                    n.source_file == str_path
                        && !n.label.ends_with("()")
                        && !n.label.ends_with(".py")
                        && n.id != file_nid
                })
                .map(|n| n.id.clone())
                .collect()
        } else {
            Vec::new()
        };

        if local_classes.is_empty() {
            continue;
        }

        // Parse imports
        let source = match std::fs::read(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let tree = match parser.parse(&source, None) {
            Some(t) => t,
            None => continue,
        };

        walk_python_imports(
            &tree.root_node(),
            &source,
            &str_path,
            &local_classes,
            &stem_to_entities,
            &mut new_edges,
        );
    }

    new_edges
}

#[cfg(feature = "lang-python")]
fn walk_python_imports<'tree>(
    root: &tree_sitter::Node<'tree>,
    source: &[u8],
    str_path: &str,
    local_classes: &[String],
    stem_to_entities: &HashMap<String, HashMap<String, String>>,
    new_edges: &mut Vec<crate::types::Edge>,
) {
    // Iterative DFS — same stack-safety motivation as the other walkers.
    // A Python file with deeply nested blocks (machine-generated, etc.)
    // would overflow the recursive form before the import scan completed.
    let mut stack: Vec<tree_sitter::Node<'tree>> = vec![*root];

    while let Some(node) = stack.pop() {
        if node.kind() != "import_from_statement" {
            // Default: descend.
            let cursor = &mut node.walk();
            let kids: Vec<tree_sitter::Node<'tree>> = node.children(cursor).collect();
            for child in kids.into_iter().rev() {
                stack.push(child);
            }
            continue;
        }

        // import_from_statement: try to resolve target_stem.
        let mut target_stem: Option<String> = None;
        let cursor = &mut node.walk();
        for child in node.children(cursor) {
            if child.kind() == "relative_import" {
                let inner = &mut child.walk();
                for sub in child.children(inner) {
                    if sub.kind() == "dotted_name" {
                        let raw = std::str::from_utf8(&source[sub.start_byte()..sub.end_byte()])
                            .unwrap_or("");
                        target_stem = raw.rsplit('.').next().map(|s| s.to_string());
                        break;
                    }
                }
                break;
            }
            if child.kind() == "dotted_name" && target_stem.is_none() {
                let raw = std::str::from_utf8(&source[child.start_byte()..child.end_byte()])
                    .unwrap_or("");
                target_stem = raw.rsplit('.').next().map(|s| s.to_string());
            }
        }

        let resolved_stem = match target_stem {
            Some(s) if stem_to_entities.contains_key(&s) => s,
            _ => {
                // Stem not recognized — descend into children to keep
                // searching for nested import statements (preserves the
                // recursive fallback).
                let cursor = &mut node.walk();
                let kids: Vec<tree_sitter::Node<'tree>> = node.children(cursor).collect();
                for child in kids.into_iter().rev() {
                    stack.push(child);
                }
                continue;
            }
        };

        // Collect imported names after 'import' keyword.
        let mut imported_names = Vec::new();
        let mut past_import = false;
        let cursor = &mut node.walk();
        for child in node.children(cursor) {
            if child.kind() == "import" {
                past_import = true;
                continue;
            }
            if !past_import {
                continue;
            }
            if child.kind() == "dotted_name" {
                let name = std::str::from_utf8(&source[child.start_byte()..child.end_byte()])
                    .unwrap_or("")
                    .to_string();
                imported_names.push(name);
            } else if child.kind() == "aliased_import" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name =
                        std::str::from_utf8(&source[name_node.start_byte()..name_node.end_byte()])
                            .unwrap_or("")
                            .to_string();
                    imported_names.push(name);
                }
            }
        }

        let line = node.start_position().row + 1;
        if let Some(entities) = stem_to_entities.get(&resolved_stem) {
            for name in &imported_names {
                if let Some(tgt_nid) = entities.get(name) {
                    for src_nid in local_classes {
                        new_edges.push(crate::types::Edge {
                            source: src_nid.clone(),
                            target: tgt_nid.clone(),
                            relation: "uses".to_string(),
                            confidence: crate::types::Confidence::INFERRED,
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            confidence_score: Some(0.8),
                            weight: 0.8,
                            original_src: None,
                            original_tgt: None,
                        });
                    }
                }
            }
        }
        // Resolved import: don't descend further — same as the original
        // recursive form which returned here.
    }
}

/// Extract docstrings and rationale comments from Python source.
#[cfg(feature = "lang-python")]
fn extract_python_rationale(
    path: &Path,
    existing_nodes: &[crate::types::Node],
) -> (Vec<crate::types::Node>, Vec<crate::types::Edge>) {
    use std::collections::HashSet;
    use tree_sitter::Parser;

    const RATIONALE_PREFIXES: &[&str] = &[
        "# NOTE:",
        "# IMPORTANT:",
        "# HACK:",
        "# WHY:",
        "# RATIONALE:",
        "# TODO:",
        "# FIXME:",
    ];

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let mut parser = Parser::new();
    let language = (languages::python::PYTHON_CONFIG.ts_language)();
    if parser.set_language(&language).is_err() {
        return (nodes, edges);
    }

    let source = match std::fs::read(path) {
        Ok(s) => s,
        Err(_) => return (nodes, edges),
    };
    let tree = match parser.parse(&source, None) {
        Some(t) => t,
        None => return (nodes, edges),
    };

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let str_path = path.to_string_lossy().to_string();
    let file_nid = crate::id::make_id(&[stem]);
    let mut seen_ids: HashSet<String> = existing_nodes.iter().map(|n| n.id.clone()).collect();

    // Module-level docstring
    if let Some((text, line)) = get_docstring(&tree.root_node(), &source) {
        add_rationale_node(
            &text,
            line,
            &file_nid,
            stem,
            &str_path,
            &mut nodes,
            &mut edges,
            &mut seen_ids,
        );
    }

    // Class and function docstrings
    walk_docstrings(
        &tree.root_node(),
        &source,
        stem,
        &file_nid,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &str_path,
    );

    // Rationale comments
    let source_text = String::from_utf8_lossy(&source);
    for (lineno, line_text) in source_text.lines().enumerate() {
        let stripped = line_text.trim();
        if RATIONALE_PREFIXES.iter().any(|p| stripped.starts_with(p)) {
            add_rationale_node(
                stripped,
                lineno + 1,
                &file_nid,
                stem,
                &str_path,
                &mut nodes,
                &mut edges,
                &mut seen_ids,
            );
        }
    }

    (nodes, edges)
}

#[cfg(feature = "lang-python")]
fn get_docstring(body_node: &tree_sitter::Node, source: &[u8]) -> Option<(String, usize)> {
    // Only check first statement in body
    let cursor = &mut body_node.walk();
    let child = body_node.children(cursor).next()?;
    if child.kind() == "expression_statement" {
        let inner = &mut child.walk();
        for sub in child.children(inner) {
            if sub.kind() == "string" || sub.kind() == "concatenated_string" {
                let text = std::str::from_utf8(&source[sub.start_byte()..sub.end_byte()])
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim();
                if text.len() > 20 {
                    return Some((text.to_string(), child.start_position().row + 1));
                }
            }
        }
    }
    None
}

#[cfg(feature = "lang-python")]
#[allow(clippy::too_many_arguments)]
fn walk_docstrings<'tree>(
    root: &tree_sitter::Node<'tree>,
    source: &[u8],
    stem: &str,
    _parent_nid: &str,
    nodes: &mut Vec<crate::types::Node>,
    edges: &mut Vec<crate::types::Edge>,
    seen_ids: &mut std::collections::HashSet<String>,
    str_path: &str,
) {
    // Iterative DFS — original recursion threaded `parent_nid` but never
    // read it, so the iterative version drops it (kept in the signature for
    // call-site stability via leading underscore). Class bodies recurse;
    // function bodies don't (matches original control flow).
    let mut stack: Vec<tree_sitter::Node<'tree>> = vec![*root];

    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if kind == "class_definition" {
            let name_node = node.child_by_field_name("name");
            let body = node.child_by_field_name("body");
            if let (Some(nn), Some(b)) = (name_node, body) {
                let class_name =
                    std::str::from_utf8(&source[nn.start_byte()..nn.end_byte()]).unwrap_or("");
                let nid = crate::id::make_id(&[stem, class_name]);
                if let Some((text, line)) = get_docstring(&b, source) {
                    let label: String = text
                        .chars()
                        .take(80)
                        .collect::<String>()
                        .replace(['\n', '\r'], " ");
                    let rid = crate::id::make_id(&[stem, "rationale", &line.to_string()]);
                    if seen_ids.insert(rid.clone()) {
                        nodes.push(crate::types::Node {
                            id: rid.clone(),
                            label,
                            file_type: crate::types::FileType::Rationale,
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            confidence: Some(crate::types::Confidence::EXTRACTED),
                            confidence_score: Some(1.0),
                            community: None,
                            norm_label: None,
                            degree: None,
                            uuid: None,
                            fingerprint: None,
                            logical_key: None,
                            body_hash: None,
                        });
                    }
                    edges.push(crate::types::Edge {
                        source: rid,
                        target: nid.clone(),
                        relation: "rationale_for".to_string(),
                        confidence: crate::types::Confidence::EXTRACTED,
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        confidence_score: Some(1.0),
                        weight: 1.0,
                        original_src: None,
                        original_tgt: None,
                    });
                }
                // Recurse into class body to find nested classes/functions.
                let cursor = &mut b.walk();
                let kids: Vec<tree_sitter::Node<'tree>> = b.children(cursor).collect();
                for child in kids.into_iter().rev() {
                    stack.push(child);
                }
            }
            continue;
        }
        if kind == "function_definition" {
            let name_node = node.child_by_field_name("name");
            let body = node.child_by_field_name("body");
            if let (Some(nn), Some(b)) = (name_node, body) {
                let func_name =
                    std::str::from_utf8(&source[nn.start_byte()..nn.end_byte()]).unwrap_or("");
                let nid = crate::id::make_id(&[stem, func_name]);
                if let Some((text, line)) = get_docstring(&b, source) {
                    let label: String = text
                        .chars()
                        .take(80)
                        .collect::<String>()
                        .replace(['\n', '\r'], " ");
                    let rid = crate::id::make_id(&[stem, "rationale", &line.to_string()]);
                    if seen_ids.insert(rid.clone()) {
                        nodes.push(crate::types::Node {
                            id: rid.clone(),
                            label,
                            file_type: crate::types::FileType::Rationale,
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            confidence: Some(crate::types::Confidence::EXTRACTED),
                            confidence_score: Some(1.0),
                            community: None,
                            norm_label: None,
                            degree: None,
                            uuid: None,
                            fingerprint: None,
                            logical_key: None,
                            body_hash: None,
                        });
                    }
                    edges.push(crate::types::Edge {
                        source: rid,
                        target: nid,
                        relation: "rationale_for".to_string(),
                        confidence: crate::types::Confidence::EXTRACTED,
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        confidence_score: Some(1.0),
                        weight: 1.0,
                        original_src: None,
                        original_tgt: None,
                    });
                }
            }
            // Function bodies aren't descended (matches original).
            continue;
        }
        // Default: descend.
        let cursor = &mut node.walk();
        let kids: Vec<tree_sitter::Node<'tree>> = node.children(cursor).collect();
        for child in kids.into_iter().rev() {
            stack.push(child);
        }
    }
}

/// Try cache, then extract, then save to cache.
#[cfg(feature = "extract")]
fn extract_or_cache(path: &Path, lang: &str, root: &Path) -> ExtractionResult {
    if let Some(cached) = cache::load_cached(path, root) {
        if let Ok(result) = serde_json::from_value::<ExtractionResult>(cached) {
            return result;
        }
    }

    let result = extract_file(path, lang);

    if result.error.is_none() {
        if let Ok(val) = serde_json::to_value(&result) {
            let _ = cache::save_cached(path, &val, root);
        }
    }

    result
}

/// Extract a single file using the appropriate language handler.
#[cfg(feature = "extract")]
fn extract_file(#[allow(unused)] path: &Path, lang: &str) -> ExtractionResult {
    match lang {
        #[cfg(feature = "lang-python")]
        "python" => generic::extract_generic(path, &languages::python::PYTHON_CONFIG),
        #[cfg(feature = "lang-javascript")]
        "javascript" => generic::extract_generic(path, &languages::javascript::JS_CONFIG),
        #[cfg(feature = "lang-go")]
        "go" => generic::extract_generic(path, &languages::go::GO_CONFIG),
        #[cfg(feature = "lang-rust")]
        "rust" => generic::extract_generic(path, &languages::rust_lang::RUST_CONFIG),
        #[cfg(feature = "lang-java")]
        "java" => generic::extract_generic(path, &languages::java::JAVA_CONFIG),
        #[cfg(feature = "lang-c")]
        "c" => generic::extract_generic(path, &languages::c::C_CONFIG),
        #[cfg(feature = "lang-cpp")]
        "cpp" => generic::extract_generic(path, &languages::cpp::CPP_CONFIG),
        #[cfg(feature = "lang-ruby")]
        "ruby" => generic::extract_generic(path, &languages::ruby::RUBY_CONFIG),
        #[cfg(feature = "lang-csharp")]
        "csharp" => generic::extract_generic(path, &languages::csharp::CSHARP_CONFIG),
        // kotlin disabled: ABI mismatch
        #[cfg(feature = "lang-scala")]
        "scala" => generic::extract_generic(path, &languages::scala::SCALA_CONFIG),
        #[cfg(feature = "lang-php")]
        "php" => generic::extract_generic(path, &languages::php::PHP_CONFIG),
        #[cfg(feature = "lang-swift")]
        "swift" => generic::extract_generic(path, &languages::swift::SWIFT_CONFIG),
        #[cfg(feature = "lang-lua")]
        "lua" => generic::extract_generic(path, &languages::lua::LUA_CONFIG),
        _ => ExtractionResult {
            error: Some(format!("Unsupported language: {lang}")),
            ..Default::default()
        },
    }
}

/// Infer common root directory from a list of paths.
#[cfg(feature = "extract")]
fn infer_common_root(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    let first = paths[0].parent()?;
    let mut root = first.to_path_buf();
    for p in &paths[1..] {
        while !p.starts_with(&root) {
            if !root.pop() {
                return Some(PathBuf::from("."));
            }
        }
    }
    Some(root)
}

/// Add a rationale node and its edge to the graph.
#[cfg(feature = "lang-python")]
#[allow(clippy::too_many_arguments)]
fn add_rationale_node(
    text: &str,
    line: usize,
    parent_nid: &str,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<crate::types::Node>,
    edges: &mut Vec<crate::types::Edge>,
    seen_ids: &mut std::collections::HashSet<String>,
) {
    let label: String = text
        .chars()
        .take(80)
        .collect::<String>()
        .replace(['\n', '\r'], " ");
    let rid = crate::id::make_id(&[stem, "rationale", &line.to_string()]);
    if seen_ids.insert(rid.clone()) {
        nodes.push(crate::types::Node {
            id: rid.clone(),
            label,
            file_type: crate::types::FileType::Rationale,
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            confidence: Some(crate::types::Confidence::EXTRACTED),
            confidence_score: Some(1.0),
            community: None,
            norm_label: None,
            degree: None,
            uuid: None,
            fingerprint: None,
            logical_key: None,
            body_hash: None,
        });
    }
    edges.push(crate::types::Edge {
        source: rid,
        target: parent_nid.to_string(),
        relation: "rationale_for".to_string(),
        confidence: crate::types::Confidence::EXTRACTED,
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        confidence_score: Some(1.0),
        weight: 1.0,
        original_src: None,
        original_tgt: None,
    });
}
