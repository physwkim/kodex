use std::path::PathBuf;

/// Helper: run the full pipeline on a directory and return node/edge counts.
#[cfg(feature = "lang-python")]
fn run_pipeline(dir: &std::path::Path) -> (usize, usize) {
    let detection = kodex::detect::detect(dir, false);
    let code_paths: Vec<PathBuf> = detection.files.code.iter().map(PathBuf::from).collect();

    #[cfg(feature = "extract")]
    {
        let extraction = kodex::extract::extract(&code_paths, Some(dir));
        let graph = kodex::graph::build_from_extraction(&extraction);
        (graph.node_count(), graph.edge_count())
    }
    #[cfg(not(feature = "extract"))]
    {
        (0, 0)
    }
}

#[test]
fn test_detect_fixtures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let result = kodex::detect::detect(&dir, false);
    assert!(
        result.files.code.len() >= 3,
        "Should find at least 3 code files, found {}",
        result.files.code.len()
    );
    assert!(result.total_files >= 3);
}

#[test]
#[cfg(feature = "lang-python")]
fn test_extract_python() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.py");
    let result = kodex::extract::generic::extract_generic(
        &path,
        &kodex::extract::languages::python::PYTHON_CONFIG,
    );

    assert!(result.error.is_none(), "Extract error: {:?}", result.error);
    assert!(
        !result.nodes.is_empty(),
        "Should extract nodes from Python file"
    );

    // Should find FileReader and CsvParser classes
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(
        labels.contains(&"FileReader"),
        "Should find FileReader class, got: {:?}",
        labels
    );
    assert!(
        labels.contains(&"CsvParser"),
        "Should find CsvParser class, got: {:?}",
        labels
    );

    // Should find functions
    assert!(
        labels.iter().any(|l| l.contains("main")),
        "Should find main function"
    );
    assert!(
        labels.iter().any(|l| l.contains("read")),
        "Should find read method"
    );

    // Should have edges
    assert!(!result.edges.is_empty(), "Should extract edges");

    // Check for contains/method edges
    let relations: Vec<&str> = result.edges.iter().map(|e| e.relation.as_str()).collect();
    assert!(
        relations.contains(&"contains"),
        "Should have 'contains' edges"
    );

    // Check inheritance (CsvParser extends FileReader)
    let extends_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "extends")
        .collect();
    assert!(
        !extends_edges.is_empty(),
        "CsvParser should extend FileReader"
    );
}

#[test]
#[cfg(feature = "lang-javascript")]
fn test_extract_javascript() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.js");
    let result = kodex::extract::generic::extract_generic(
        &path,
        &kodex::extract::languages::javascript::JS_CONFIG,
    );

    assert!(result.error.is_none(), "Extract error: {:?}", result.error);
    assert!(
        !result.nodes.is_empty(),
        "Should extract nodes from JS file"
    );

    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(
        labels.contains(&"DataLoader"),
        "Should find DataLoader class, got: {:?}",
        labels
    );
    assert!(
        labels.iter().any(|l| l.contains("processData")),
        "Should find processData function"
    );
    assert!(
        labels.iter().any(|l| l.contains("load")),
        "Should find load method"
    );

    // Should have import edges
    let import_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation.contains("import"))
        .collect();
    assert!(!import_edges.is_empty(), "Should have import edges");
}

#[test]
#[cfg(feature = "lang-go")]
fn test_extract_go() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.go");
    let result =
        kodex::extract::generic::extract_generic(&path, &kodex::extract::languages::go::GO_CONFIG);

    assert!(result.error.is_none(), "Extract error: {:?}", result.error);
    assert!(
        !result.nodes.is_empty(),
        "Should extract nodes from Go file"
    );

    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(
        labels.iter().any(|l| l.contains("NewConfig")),
        "Should find NewConfig func, got: {:?}",
        labels
    );

    // Should have import edges
    let import_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation.contains("import"))
        .collect();
    assert!(!import_edges.is_empty(), "Should have import edges");
}

#[test]
#[cfg(feature = "lang-rust")]
fn test_extract_rust_trait_impl_attaches_methods_to_type() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.rs");
    let result = kodex::extract::generic::extract_generic(
        &path,
        &kodex::extract::languages::rust_lang::RUST_CONFIG,
    );

    assert!(result.error.is_none(), "Extract error: {:?}", result.error);
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();

    // The struct itself.
    assert!(
        labels.contains(&"LocalChannel"),
        "expected LocalChannel struct: {labels:?}"
    );

    // Methods from `impl LocalChannel` AND `impl ChannelSource for LocalChannel`
    // both have to land under a node labelled `LocalChannel`. We look at
    // contains-edges sourced from any node whose label is "LocalChannel".
    let local_channel_ids: Vec<String> = result
        .nodes
        .iter()
        .filter(|n| n.label == "LocalChannel")
        .map(|n| n.id.clone())
        .collect();

    let method_labels: Vec<&str> = result
        .edges
        .iter()
        .filter(|e| {
            (e.relation == "contains" || e.relation == "method")
                && local_channel_ids.contains(&e.source)
        })
        .filter_map(|e| {
            result
                .nodes
                .iter()
                .find(|n| n.id == e.target)
                .map(|n| n.label.as_str())
        })
        .collect();

    for expected in &["new()", "name()", "open()", "close()"] {
        assert!(
            method_labels.iter().any(|l| l == expected),
            "expected `{expected}` under LocalChannel, got: {method_labels:?}"
        );
    }
}

#[test]
#[cfg(feature = "lang-python")]
fn test_full_pipeline_on_fixtures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let (nodes, edges) = run_pipeline(&dir);
    assert!(nodes > 0, "Pipeline should produce nodes");
    assert!(edges > 0, "Pipeline should produce edges");
}

#[test]
fn test_graph_export_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();
    let json_path = dir.path().join("graph.json");

    // Build a test graph
    let extraction = kodex::types::ExtractionResult {
        nodes: vec![
            kodex::types::Node {
                id: "a".to_string(),
                label: "Alpha".to_string(),
                file_type: kodex::types::FileType::Code,
                source_file: "a.py".to_string(),
                source_location: Some("L1".to_string()),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None,
                norm_label: None,
                degree: None,
                uuid: None,
                fingerprint: None,
                logical_key: None,
                body_hash: None,
            },
            kodex::types::Node {
                id: "b".to_string(),
                label: "Beta".to_string(),
                file_type: kodex::types::FileType::Code,
                source_file: "b.py".to_string(),
                source_location: Some("L1".to_string()),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None,
                norm_label: None,
                degree: None,
                uuid: None,
                fingerprint: None,
                logical_key: None,
                body_hash: None,
            },
        ],
        edges: vec![kodex::types::Edge {
            source: "a".to_string(),
            target: "b".to_string(),
            relation: "imports".to_string(),
            confidence: kodex::types::Confidence::EXTRACTED,
            source_file: "a.py".to_string(),
            source_location: Some("L2".to_string()),
            confidence_score: Some(1.0),
            weight: 1.0,
            original_src: None,
            original_tgt: None,
        }],
        ..Default::default()
    };

    let graph = kodex::graph::build_from_extraction(&extraction);
    let communities = kodex::cluster::cluster(&graph);

    // Export JSON
    kodex::export::to_json(&graph, &communities, &json_path).unwrap();
    assert!(json_path.exists());

    // Load back
    let loaded = kodex::serve::load_graph(&json_path).unwrap();
    assert_eq!(loaded.node_count(), 2);
    assert_eq!(loaded.edge_count(), 1);

    // Export HTML
    let html_path = dir.path().join("graph.html");
    kodex::export::to_html(&graph, &communities, &html_path, None).unwrap();
    assert!(html_path.exists());
    let html = std::fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("vis-network") || html.contains("vis.js"));

    // Export GraphML
    let graphml_path = dir.path().join("graph.graphml");
    kodex::export::to_graphml(&graph, &communities, &graphml_path).unwrap();
    assert!(graphml_path.exists());
    let graphml = std::fs::read_to_string(&graphml_path).unwrap();
    assert!(graphml.contains("<graphml"));

    // Export Cypher
    let cypher_path = dir.path().join("import.cypher");
    kodex::export::to_cypher(&graph, &cypher_path).unwrap();
    assert!(cypher_path.exists());
    let cypher = std::fs::read_to_string(&cypher_path).unwrap();
    assert!(cypher.contains("MERGE"));
}

#[test]
fn test_cluster_and_analyze() {
    let extraction = kodex::types::ExtractionResult {
        nodes: (0..10)
            .map(|i| kodex::types::Node {
                id: format!("n{i}"),
                label: format!("Node{i}"),
                file_type: kodex::types::FileType::Code,
                source_file: format!("file{}.py", i % 3),
                source_location: Some(format!("L{}", i + 1)),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None,
                norm_label: None,
                degree: None,
                uuid: None,
                fingerprint: None,
                logical_key: None,
                body_hash: None,
            })
            .collect(),
        edges: [
            ("n0", "n1"),
            ("n1", "n2"),
            ("n0", "n2"), // cluster 1
            ("n3", "n4"),
            ("n4", "n5"),
            ("n3", "n5"), // cluster 2
            ("n6", "n7"),
            ("n7", "n8"),
            ("n8", "n9"), // cluster 3
            ("n2", "n3"), // bridge
        ]
        .iter()
        .map(|(s, t)| kodex::types::Edge {
            source: s.to_string(),
            target: t.to_string(),
            relation: "calls".to_string(),
            confidence: kodex::types::Confidence::EXTRACTED,
            source_file: "test.py".to_string(),
            source_location: None,
            confidence_score: Some(1.0),
            weight: 1.0,
            original_src: None,
            original_tgt: None,
        })
        .collect(),
        ..Default::default()
    };

    let graph = kodex::graph::build_from_extraction(&extraction);
    let communities = kodex::cluster::cluster(&graph);

    // Should detect at least 2 communities
    assert!(
        communities.len() >= 2,
        "Should detect at least 2 communities, got {}",
        communities.len()
    );

    // God nodes
    let gods = kodex::analyze::god_nodes(&graph, 5);
    assert!(!gods.is_empty(), "Should find god nodes");

    // Surprising connections
    let surprises = kodex::analyze::surprising_connections(&graph, Some(&communities), 5);
    // Bridge edge n2→n3 should be surprising (cross-community)

    // Questions
    let questions = kodex::analyze::suggest_questions(&graph, Some(&communities), 5);

    // Report
    let cohesion = kodex::cluster::score_all(&graph, &communities);
    let labels: std::collections::HashMap<usize, String> = communities
        .keys()
        .map(|&c| (c, format!("Community {c}")))
        .collect();
    let report = kodex::report::generate(
        &graph,
        &communities,
        &cohesion,
        &labels,
        &gods,
        &surprises,
        &kodex::types::DetectionResult::default(),
        0,
        0,
        "test",
        Some(&questions),
    );
    assert!(report.contains("# Graph Report"));
    assert!(report.contains("God Nodes"));
}

#[test]
fn test_sqlite_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");

    let extraction = kodex::types::ExtractionResult {
        nodes: vec![
            kodex::types::Node {
                id: "x".to_string(),
                label: "X".to_string(),
                file_type: kodex::types::FileType::Code,
                source_file: "x.py".to_string(),
                source_location: Some("L1".to_string()),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None,
                norm_label: None,
                degree: None,
                uuid: None,
                fingerprint: None,
                logical_key: None,
                body_hash: None,
            },
            kodex::types::Node {
                id: "y".to_string(),
                label: "Y".to_string(),
                file_type: kodex::types::FileType::Code,
                source_file: "y.py".to_string(),
                source_location: None,
                confidence: Some(kodex::types::Confidence::INFERRED),
                confidence_score: Some(0.5),
                community: None,
                norm_label: None,
                degree: None,
                uuid: None,
                fingerprint: None,
                logical_key: None,
                body_hash: None,
            },
        ],
        edges: vec![kodex::types::Edge {
            source: "x".to_string(),
            target: "y".to_string(),
            relation: "calls".to_string(),
            confidence: kodex::types::Confidence::EXTRACTED,
            source_file: "x.py".to_string(),
            source_location: None,
            confidence_score: Some(1.0),
            weight: 1.0,
            original_src: None,
            original_tgt: None,
        }],
        ..Default::default()
    };

    let graph = kodex::graph::build_from_extraction(&extraction);
    let communities = kodex::cluster::cluster(&graph);
    kodex::storage::save_db(&graph, &communities, &db_path).unwrap();
    assert!(db_path.exists());

    let loaded = kodex::storage::load_db(&db_path).unwrap();
    assert_eq!(loaded.node_count(), 2);
    assert_eq!(loaded.edge_count(), 1);
    assert_eq!(loaded.get_node("x").unwrap().label, "X");
}

#[test]
fn test_vault_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();

    std::fs::write(
        dir.path().join("Foo.md"),
        "---\nid: foo\nfile_type: code\nsource_file: foo.py\n---\n# Foo\n\n## Connections\n- [[Bar]] - calls [EXTRACTED]\n",
    ).unwrap();
    std::fs::write(
        dir.path().join("Bar.md"),
        "---\nid: bar\nfile_type: code\nsource_file: bar.py\n---\n# Bar\n",
    )
    .unwrap();

    let graph = kodex::vault::load_graph_from_vault(dir.path()).unwrap();
    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.edge_count(), 1);
    assert!(graph.get_node("foo").is_some());
    assert!(graph.get_node("bar").is_some());
}

#[test]
fn test_knowledge_learn_and_recall() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_db(&g, &c, &db).unwrap();
    }

    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::Pattern,
        "Test Pattern",
        "A test pattern description",
        &["node_a".to_string()],
        &["test".to_string()],
    )
    .unwrap();

    let results = kodex::learn::query_knowledge(&db, "test", None);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Test Pattern");

    let results = kodex::learn::query_knowledge(&db, "test", None);
    let uuid = results[0].uuid.clone();

    // Reinforce using UUID
    kodex::learn::learn_with_uuid(
        &db,
        Some(&uuid),
        kodex::learn::KnowledgeType::Pattern,
        "Test Pattern",
        "Seen again",
        Some(&["node_b".to_string()]),
        &[],
        None,
    )
    .unwrap();

    let results = kodex::learn::query_knowledge(&db, "test", None);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].observations, 2);
}

#[test]
fn test_forget_knowledge_and_logic() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_db(&g, &c, &db).unwrap();
    }

    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::Pattern,
        "Foo Pattern",
        "desc",
        &[],
        &[],
    )
    .unwrap();
    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::Decision,
        "Bar Decision",
        "desc",
        &[],
        &[],
    )
    .unwrap();
    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::Pattern,
        "Baz Pattern",
        "desc",
        &[],
        &[],
    )
    .unwrap();

    // forget with title + type should AND: only "Foo Pattern" matches both
    let removed =
        kodex::storage::forget_knowledge(&db, Some("Foo"), Some("pattern"), None, None).unwrap();
    assert_eq!(
        removed, 1,
        "AND logic: only Foo Pattern matches title=Foo AND type=pattern"
    );

    let remaining = kodex::learn::query_knowledge(&db, "", None);
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|k| k.title == "Bar Decision"));
    assert!(remaining.iter().any(|k| k.title == "Baz Pattern"));

    // forget with type only should match all remaining patterns
    let removed = kodex::storage::forget_knowledge(&db, None, Some("pattern"), None, None).unwrap();
    assert_eq!(removed, 1, "Baz Pattern should be removed");

    let remaining = kodex::learn::query_knowledge(&db, "", None);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].title, "Bar Decision");

    // forget with no criteria should remove nothing
    let removed = kodex::storage::forget_knowledge(&db, None, None, None, None).unwrap();
    assert_eq!(removed, 0, "No criteria = no deletion");
}

#[test]
fn test_knowledge_context_index() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_db(&g, &c, &db).unwrap();
    }

    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::Decision,
        "Use SQLite",
        "Fast storage",
        &[],
        &[],
    )
    .unwrap();
    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::Convention,
        "Error Handling",
        "Use Result",
        &[],
        &[],
    )
    .unwrap();

    let ctx = kodex::learn::knowledge_context(&db, 10, 0);
    assert!(ctx.contains("Knowledge:"));
    assert!(ctx.contains("Use SQLite"));
    assert!(ctx.contains("Error Handling"));
}

/// End-to-end scenario: code graph + knowledge + links + staleness + task context
#[test]
fn test_knowledge_graph_scenario() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("scenario.db");

    // 1. Create graph with real nodes (simulating kodex run)
    let extraction = kodex::types::ExtractionResult {
        nodes: vec![
            kodex::types::Node {
                id: "auth_handler".into(),
                label: "AuthHandler".into(),
                file_type: kodex::types::FileType::Code,
                source_file: "project/auth.py".into(),
                source_location: Some("L10".into()),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None,
                norm_label: None,
                degree: None,
                uuid: Some("node-auth".into()),
                fingerprint: Some("fp-auth".into()),
                logical_key: Some("project/auth.py::AuthHandler".into()),
                body_hash: Some("abcd1234".into()),
            },
            kodex::types::Node {
                id: "user_repo".into(),
                label: "UserRepo".into(),
                file_type: kodex::types::FileType::Code,
                source_file: "project/repo.py".into(),
                source_location: Some("L5".into()),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None,
                norm_label: None,
                degree: None,
                uuid: Some("node-repo".into()),
                fingerprint: Some("fp-repo".into()),
                logical_key: Some("project/repo.py::UserRepo".into()),
                body_hash: Some("efgh5678".into()),
            },
        ],
        edges: vec![kodex::types::Edge {
            source: "auth_handler".into(),
            target: "user_repo".into(),
            relation: "calls".into(),
            confidence: kodex::types::Confidence::EXTRACTED,
            source_file: "project/auth.py".into(),
            source_location: Some("L15".into()),
            confidence_score: Some(1.0),
            weight: 1.0,
            original_src: None,
            original_tgt: None,
        }],
        ..Default::default()
    };
    let data = kodex::types::KodexData {
        extraction,
        knowledge: vec![],
        links: vec![],
        review_queue: vec![],
    };
    kodex::storage::save(&db, &data).unwrap();

    // 2. Agent learns knowledge and links to code nodes
    let k1 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "Repository Pattern",
        "All data access through repo classes",
        Some(&["node-repo".to_string()]),
        &["architecture".into()],
        None,
    )
    .unwrap();

    let k2 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Decision,
        "JWT Auth",
        "Chose JWT for stateless auth",
        Some(&["node-auth".to_string()]),
        &["auth".into()],
        Some(&k1),
    )
    .unwrap();

    let k3 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Convention,
        "Always validate tokens",
        "Every endpoint must validate",
        Some(&["node-auth".to_string()]),
        &[],
        Some(&k2),
    )
    .unwrap();

    // 3. Link knowledge ↔ knowledge (beyond chain)
    kodex::learn::link_knowledge_to_knowledge(&db, &k1, &k2, "supports", true).unwrap();

    // 4. Verify thought chain
    let chain = kodex::learn::thought_chain(&db, &k2);
    assert_eq!(chain.len(), 3, "Chain should have 3 steps");
    assert_eq!(chain[0].title, "Repository Pattern");
    assert_eq!(chain[2].title, "Always validate tokens");

    // 5. Verify related_nodes are node-only (not knowledge UUIDs)
    let all = kodex::learn::query_knowledge(&db, "", None);
    for item in &all {
        for r in &item.related_nodes {
            assert!(
                r.starts_with("node-"),
                "related_nodes should be node UUIDs, got: {r}"
            );
        }
    }

    // 6. Task context — auth.py is being edited
    let ctx = kodex::learn::get_task_context(&db, "auth token", &["project/auth.py".into()], 10);
    assert!(
        ctx.contains("JWT Auth"),
        "Should surface JWT knowledge for auth file"
    );
    assert!(
        ctx.contains("validate tokens"),
        "Should surface validation convention"
    );

    // 7. recall_for_task — repo.py is being edited
    let results = kodex::learn::recall_for_task(
        &db,
        "data access",
        &["project/repo.py".into()],
        &["node-repo".into()],
        5,
        None,
    );
    assert!(!results.is_empty());
    assert_eq!(
        results[0].title, "Repository Pattern",
        "Repo pattern should rank first for repo.py"
    );

    // 8. Knowledge graph traversal
    let graph_nodes = kodex::learn::traverse_knowledge_graph(&db, Some(&k1), 2);
    assert!(
        graph_nodes.len() >= 2,
        "Should reach k2 from k1 within 2 hops"
    );
    let k1_node = graph_nodes.iter().find(|n| n.uuid == k1).unwrap();
    assert!(
        !k1_node.links_out.is_empty(),
        "k1 should have outgoing knowledge links"
    );
    assert!(!k1_node.node_links.is_empty(), "k1 should have node links");
    assert_eq!(k1_node.node_links[0].target_title, "UserRepo");

    // 9. Staleness detection — all nodes exist, nothing stale
    let stale = kodex::learn::detect_stale_knowledge(&db).unwrap();
    assert_eq!(stale, 0, "No stale knowledge when all nodes exist");

    // 10. Simulate node deletion (re-save without auth node)
    let mut data2 = kodex::storage::load(&db).unwrap();
    data2
        .extraction
        .nodes
        .retain(|n| n.uuid.as_deref() != Some("node-auth"));
    kodex::storage::save(&db, &data2).unwrap();

    // k2 and k3 linked to node-auth should now be stale
    let stale = kodex::learn::detect_stale_knowledge(&db).unwrap();
    assert!(
        stale >= 1,
        "Should detect stale knowledge after node deletion"
    );

    // k1 linked to node-repo should NOT be stale
    let k1_entry = kodex::learn::query_knowledge(&db, "Repository Pattern", None);
    assert!(!k1_entry.is_empty());
    // k1 should still be queryable and active (not needs_review)

    // 11. update_knowledge — mark k3 as validated
    kodex::learn::update_knowledge(
        &db,
        &k3,
        &kodex::learn::KnowledgeUpdates {
            status: Some("active".into()),
            applies_when: Some("any endpoint modification".into()),
            validate: true,
            ..Default::default()
        },
    )
    .unwrap();

    let data3 = kodex::storage::load(&db).unwrap();
    let k3_entry = data3.knowledge.iter().find(|k| k.uuid == k3).unwrap();
    assert_eq!(k3_entry.status, "active");
    assert_eq!(k3_entry.applies_when, "any endpoint modification");
    assert!(k3_entry.last_validated_at > 0);

    // 12. Selective link removal
    kodex::learn::remove_link(&db, &k1, &k2, Some("supports")).unwrap();
    let neighbors = kodex::learn::knowledge_neighbors(&db, &k1);
    let support_links: Vec<_> = neighbors
        .iter()
        .filter(|(_, r, _)| r == "supports")
        .collect();
    assert!(support_links.is_empty(), "supports link should be removed");
    // leads_to chain should still exist
    let chain_after = kodex::learn::thought_chain(&db, &k1);
    assert!(
        chain_after.len() >= 2,
        "Chain should survive supports link removal"
    );

    // 13. Markdown rendering
    let md = kodex::learn::render_thought_chain(&chain);
    assert!(md.contains("Thought Chain"));
    assert!(md.contains("leads_to"));

    let graph_md = kodex::learn::render_knowledge_graph(&graph_nodes);
    assert!(graph_md.contains("Knowledge Graph"));
}

/// Test: node rename preserves UUID + knowledge link integrity
#[test]
fn test_rename_preserves_knowledge_links() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("rename.db");

    // Session 1: create graph with authenticate()
    let mut extraction = kodex::types::ExtractionResult::default();
    extraction.nodes.push(kodex::types::Node {
        id: "auth_authenticate".into(),
        label: "authenticate()".into(),
        file_type: kodex::types::FileType::Code,
        source_file: "project/auth.py".into(),
        source_location: Some("L42".into()),
        confidence: Some(kodex::types::Confidence::EXTRACTED),
        confidence_score: Some(1.0),
        community: None,
        norm_label: None,
        degree: None,
        uuid: None,
        fingerprint: None,
        logical_key: None,
        body_hash: Some("abcd1234abcd1234".into()),
    });
    kodex::storage::merge_project(&db, "project", &extraction).unwrap();

    // Get the assigned UUID
    let data1 = kodex::storage::load(&db).unwrap();
    let node_uuid = data1.extraction.nodes[0].uuid.clone().unwrap();

    // Learn knowledge linked to this node
    kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "JWT Auth Pattern",
        "Token-based auth",
        Some(std::slice::from_ref(&node_uuid)),
        &[],
        None,
    )
    .unwrap();

    // Session 2: rename authenticate() → verify_token(), same body_hash
    let mut extraction2 = kodex::types::ExtractionResult::default();
    extraction2.nodes.push(kodex::types::Node {
        id: "auth_verify_token".into(),
        label: "verify_token()".into(),
        file_type: kodex::types::FileType::Code,
        source_file: "project/auth.py".into(),
        source_location: Some("L42".into()),
        confidence: Some(kodex::types::Confidence::EXTRACTED),
        confidence_score: Some(1.0),
        community: None,
        norm_label: None,
        degree: None,
        uuid: None,
        fingerprint: None,
        logical_key: None,
        body_hash: Some("abcd1234abcd1234".into()), // same body
    });
    kodex::storage::merge_project(&db, "project", &extraction2).unwrap();

    // Verify UUID was preserved through rename
    let data2 = kodex::storage::load(&db).unwrap();
    let new_node_uuid = data2
        .extraction
        .nodes
        .iter()
        .find(|n| n.label == "verify_token()")
        .unwrap()
        .uuid
        .clone()
        .unwrap();
    assert_eq!(new_node_uuid, node_uuid, "Rename should preserve UUID");

    // Knowledge link should still work
    let knowledge = kodex::learn::query_knowledge(&db, "JWT", None);
    assert_eq!(knowledge.len(), 1);
    assert!(
        knowledge[0].related_nodes.contains(&node_uuid),
        "Knowledge should still link to the renamed node"
    );

    // No staleness — node exists with same UUID
    let stale = kodex::learn::detect_stale_knowledge(&db).unwrap();
    assert_eq!(stale, 0, "Renamed node should not trigger staleness");
}

/// Test: SQLite save/load preserves all fields including defaults
#[test]
fn test_sqlite_preserves_defaults() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("defaults.db");

    let data = kodex::types::KodexData {
        extraction: kodex::types::ExtractionResult {
            nodes: vec![kodex::types::Node {
                id: "a".into(),
                label: "Alpha".into(),
                file_type: kodex::types::FileType::Code,
                source_file: "a.py".into(),
                source_location: Some("L1".into()),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None,
                norm_label: None,
                degree: None,
                uuid: Some("node-a".into()),
                fingerprint: Some("fp-a".into()),
                logical_key: Some("a.py::Alpha".into()),
                body_hash: None,
            }],
            ..Default::default()
        },
        knowledge: vec![kodex::types::KnowledgeEntry {
            uuid: "k-1".into(),
            title: "Test".into(),
            knowledge_type: "pattern".into(),
            description: "A pattern".into(),
            confidence: 0.7,
            observations: 3,
            tags: vec!["test".into()],
            ..Default::default()
        }],
        links: vec![kodex::types::KnowledgeLink {
            knowledge_uuid: "k-1".into(),
            node_uuid: "node-a".into(),
            relation: "related_to".into(),
            target_type: String::new(),
            ..Default::default()
        }],
        review_queue: vec![],
    };
    kodex::storage::save(&db, &data).unwrap();
    kodex::storage::cache_remove(&db);

    let loaded = kodex::storage::load(&db).unwrap();
    assert_eq!(loaded.extraction.nodes.len(), 1);
    assert_eq!(loaded.extraction.nodes[0].uuid.as_deref(), Some("node-a"));
    assert_eq!(loaded.knowledge.len(), 1);
    assert_eq!(loaded.knowledge[0].uuid, "k-1");
    assert_eq!(loaded.knowledge[0].confidence, 0.7);
    assert_eq!(loaded.knowledge[0].observations, 3);
    assert_eq!(loaded.knowledge[0].status, "active");
    assert_eq!(loaded.links.len(), 1);
    assert_eq!(loaded.links[0].knowledge_uuid, "k-1");
}

/// Test: duplicate detection + merge preserves links and evidence
#[test]
fn test_duplicate_merge_preserves_links() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("dedup.db");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_db(&g, &c, &db).unwrap();
    }

    // Create two similar knowledge entries
    let k1 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "Repository Pattern",
        "All data access through repos",
        Some(&["node-a".to_string()]),
        &["arch".into()],
        None,
    )
    .unwrap();
    let k2 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "Repository Pattern Design",
        "Data access via repository classes",
        Some(&["node-b".to_string()]),
        &["arch".into(), "design".into()],
        None,
    )
    .unwrap();

    // Should detect as duplicate
    let dupes = kodex::learn::find_duplicates(&db, 0.4);
    assert!(!dupes.is_empty(), "Should detect similar entries");

    // Merge k2 into k1
    kodex::learn::merge_knowledge(&db, &k1, &k2).unwrap();

    // Verify k1 absorbed k2's data
    let data = kodex::storage::load(&db).unwrap();
    let kept = data.knowledge.iter().find(|k| k.uuid == k1).unwrap();
    assert!(kept.observations >= 2, "Should absorb observations");
    assert!(
        kept.tags.contains(&"design".to_string()),
        "Should absorb tags"
    );

    // k2 should be obsolete
    let absorbed = data.knowledge.iter().find(|k| k.uuid == k2).unwrap();
    assert_eq!(absorbed.status, "obsolete");
    assert_eq!(absorbed.superseded_by, k1);

    // k1 should now have links to both node-a and node-b
    let k1_links: Vec<_> = data
        .links
        .iter()
        .filter(|l| l.knowledge_uuid == k1 && !l.is_knowledge_link())
        .map(|l| l.node_uuid.as_str())
        .collect();
    assert!(k1_links.contains(&"node-a"), "Should keep original link");
    assert!(k1_links.contains(&"node-b"), "Should absorb merged link");

    // Supersedes link should exist
    let supersedes = data
        .links
        .iter()
        .any(|l| l.knowledge_uuid == k1 && l.node_uuid == k2 && l.relation == "supersedes");
    assert!(supersedes, "Should have supersedes link");
}

/// Test: multi-project merge + recall correctness
#[test]
fn test_multi_project_recall() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("multi.db");

    // Project A
    let mut ext_a = kodex::types::ExtractionResult::default();
    ext_a.nodes.push(kodex::types::Node {
        id: "a_handler".into(),
        label: "AuthHandler".into(),
        file_type: kodex::types::FileType::Code,
        source_file: "project-a/auth.py".into(),
        source_location: Some("L10".into()),
        confidence: Some(kodex::types::Confidence::EXTRACTED),
        confidence_score: Some(1.0),
        community: None,
        norm_label: None,
        degree: None,
        uuid: None,
        fingerprint: None,
        logical_key: None,
        body_hash: Some("aaaa".into()),
    });
    kodex::storage::merge_project(&db, "project-a", &ext_a).unwrap();

    let data_a = kodex::storage::load(&db).unwrap();
    let uuid_a = data_a.extraction.nodes[0].uuid.clone().unwrap();

    // Learn knowledge for project A
    kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "Auth Pattern A",
        "JWT auth in project A",
        Some(std::slice::from_ref(&uuid_a)),
        &[],
        None,
    )
    .unwrap();

    // Project B
    let mut ext_b = kodex::types::ExtractionResult::default();
    ext_b.nodes.push(kodex::types::Node {
        id: "b_handler".into(),
        label: "PaymentHandler".into(),
        file_type: kodex::types::FileType::Code,
        source_file: "project-b/payment.py".into(),
        source_location: Some("L5".into()),
        confidence: Some(kodex::types::Confidence::EXTRACTED),
        confidence_score: Some(1.0),
        community: None,
        norm_label: None,
        degree: None,
        uuid: None,
        fingerprint: None,
        logical_key: None,
        body_hash: Some("bbbb".into()),
    });
    kodex::storage::merge_project(&db, "project-b", &ext_b).unwrap();

    let data_b = kodex::storage::load(&db).unwrap();
    let uuid_b = data_b
        .extraction
        .nodes
        .iter()
        .find(|n| n.label == "PaymentHandler")
        .unwrap()
        .uuid
        .clone()
        .unwrap();

    kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Convention,
        "Payment Validation",
        "Always validate amounts",
        Some(std::slice::from_ref(&uuid_b)),
        &[],
        None,
    )
    .unwrap();

    // Both projects coexist
    let data = kodex::storage::load(&db).unwrap();
    assert_eq!(data.extraction.nodes.len(), 2);
    assert_eq!(data.knowledge.len(), 2);

    // Recall for auth.py should prioritize Auth Pattern
    let results = kodex::learn::recall_for_task(
        &db,
        "auth",
        &["project-a/auth.py".into()],
        &[uuid_a],
        5,
        None,
    );
    assert!(!results.is_empty());
    assert_eq!(
        results[0].title, "Auth Pattern A",
        "Auth knowledge should rank first for auth file"
    );

    // Recall for payment.py should prioritize Payment Validation
    let results = kodex::learn::recall_for_task(
        &db,
        "payment",
        &["project-b/payment.py".into()],
        &[uuid_b],
        5,
        None,
    );
    assert!(!results.is_empty());
    assert_eq!(
        results[0].title, "Payment Validation",
        "Payment knowledge should rank first for payment file"
    );
}

/// Test: merge preserves knowledge↔knowledge links
#[test]
fn test_merge_preserves_knowledge_links() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("merge_kk.db");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_db(&g, &c, &db).unwrap();
    }

    // K1 depends_on K2. K3 supports K2. Then merge K2 into K1.
    let k1 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "Auth Pattern",
        "desc",
        None,
        &[],
        None,
    )
    .unwrap();
    let k2 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Decision,
        "JWT Decision",
        "desc",
        None,
        &[],
        None,
    )
    .unwrap();
    let k3 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Convention,
        "Token Convention",
        "desc",
        None,
        &[],
        None,
    )
    .unwrap();

    // K1 depends_on K2
    kodex::learn::link_knowledge_to_knowledge(&db, &k1, &k2, "depends_on", false).unwrap();
    // K3 supports K2 (incoming link to K2)
    kodex::learn::link_knowledge_to_knowledge(&db, &k3, &k2, "supports", false).unwrap();

    // Merge K2 into K1
    kodex::learn::merge_knowledge(&db, &k1, &k2).unwrap();

    let data = kodex::storage::load(&db).unwrap();

    // K3's "supports" link should now point to K1 (not K2)
    let k3_supports: Vec<_> = data
        .links
        .iter()
        .filter(|l| l.knowledge_uuid == k3 && l.relation == "supports" && l.is_knowledge_link())
        .collect();
    assert_eq!(k3_supports.len(), 1);
    assert_eq!(
        k3_supports[0].node_uuid, k1,
        "Incoming knowledge link should be rewritten to keeper UUID"
    );

    // K1's outgoing "depends_on" to K2 should be gone (self-referential after merge)
    // or rewritten — it was K1→K2, and K2 is now absorbed into K1
    let k1_deps: Vec<_> = data
        .links
        .iter()
        .filter(|l| l.knowledge_uuid == k1 && l.relation == "depends_on" && l.is_knowledge_link())
        .collect();
    // The old K1→K2 link got rewritten: K1→K1 is self-referential and was skipped
    assert!(
        k1_deps.is_empty() || k1_deps.iter().all(|l| l.node_uuid != k2),
        "Self-referential link should not exist after merge"
    );
}

/// Test: update_knowledge sets updated_at
#[test]
fn test_update_knowledge_timestamps() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("ts.db");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_db(&g, &c, &db).unwrap();
    }

    let k = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "Test",
        "desc",
        None,
        &[],
        None,
    )
    .unwrap();

    let data1 = kodex::storage::load(&db).unwrap();
    let entry1 = data1.knowledge.iter().find(|e| e.uuid == k).unwrap();
    assert!(entry1.created_at > 0, "created_at should be set");
    let created = entry1.created_at;

    // Update
    kodex::learn::update_knowledge(
        &db,
        &k,
        &kodex::learn::KnowledgeUpdates {
            scope: Some("module".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let data2 = kodex::storage::load(&db).unwrap();
    let entry2 = data2.knowledge.iter().find(|e| e.uuid == k).unwrap();
    assert_eq!(entry2.created_at, created, "created_at should not change");
    assert!(
        entry2.updated_at >= created,
        "updated_at should be set after update"
    );
    assert_eq!(entry2.scope, "module");
}

/// Gen3 test: graph reasoning changes actual recall ranking
#[test]
fn test_reasoning_affects_ranking() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("reason.db");

    // Create a graph with one node
    let mut ext = kodex::types::ExtractionResult::default();
    ext.nodes.push(kodex::types::Node {
        id: "auth".into(),
        label: "AuthHandler".into(),
        file_type: kodex::types::FileType::Code,
        source_file: "project/auth.py".into(),
        source_location: Some("L10".into()),
        confidence: Some(kodex::types::Confidence::EXTRACTED),
        confidence_score: Some(1.0),
        community: None,
        norm_label: None,
        degree: None,
        uuid: Some("node-auth".into()),
        fingerprint: None,
        logical_key: None,
        body_hash: Some("aaaa".into()),
    });
    let data = kodex::types::KodexData {
        extraction: ext,
        knowledge: vec![],
        links: vec![],
        review_queue: vec![],
    };
    kodex::storage::save(&db, &data).unwrap();

    // K1: directly linked to node-auth (high base relevance)
    let k1 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "JWT Auth",
        "Token-based auth",
        Some(std::slice::from_ref(&"node-auth".to_string())),
        &[],
        None,
    )
    .unwrap();

    // K2: NOT linked to any node (low base relevance)
    let k2 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Decision,
        "Session Config",
        "Session settings",
        None,
        &[],
        None,
    )
    .unwrap();

    // K1 supports K2 — reasoning should boost K2
    kodex::learn::link_knowledge_to_knowledge(&db, &k1, &k2, "supports", false).unwrap();

    // Recall with node-auth context — K1 scores high, K2 gets reasoning boost
    let results = kodex::learn::recall_for_task_structured(
        &db,
        "",
        &["project/auth.py".into()],
        &["node-auth".into()],
        10,
        None,
    );

    assert!(results.len() >= 2, "Should return both entries");

    // K2 should have a reasoning boost in its score
    let k2_result = results.iter().find(|r| r.knowledge.uuid == k2).unwrap();
    assert!(
        k2_result
            .score
            .reasons
            .iter()
            .any(|r| r.contains("graph reasoning")),
        "K2 should have graph reasoning in its reasons: {:?}",
        k2_result.score.reasons
    );
}

/// Gen3 test: task_type changes recommendations
#[test]
fn test_task_type_recommendations() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("recs.db");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_db(&g, &c, &db).unwrap();
    }

    // Create a bug_pattern knowledge
    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::BugPattern,
        "Off-by-one in pagination",
        "Page count wrong by 1",
        &[],
        &[],
    )
    .unwrap();

    // Create a convention knowledge
    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::Convention,
        "Always validate input",
        "All endpoints must validate",
        &[],
        &[],
    )
    .unwrap();

    // Create a tech_debt knowledge
    kodex::learn::learn(
        &db,
        kodex::learn::KnowledgeType::TechDebt,
        "Legacy auth module",
        "Needs rewrite",
        &[],
        &[],
    )
    .unwrap();

    // bugfix context: should recommend test for bug_pattern
    let bugfix_ctx = kodex::learn::get_task_context_json(&db, "pagination", &[], 10, "bugfix");
    let has_test_rec = bugfix_ctx
        .recommendations
        .iter()
        .any(|r| r.category == "test");
    assert!(
        has_test_rec,
        "bugfix should produce test recommendations: {:?}",
        bugfix_ctx
            .recommendations
            .iter()
            .map(|r| &r.category)
            .collect::<Vec<_>>()
    );

    // refactor context: should recommend opportunity for tech_debt
    let refactor_ctx = kodex::learn::get_task_context_json(&db, "auth", &[], 10, "refactor");
    let has_opportunity = refactor_ctx
        .recommendations
        .iter()
        .any(|r| r.category == "opportunity");
    assert!(
        has_opportunity,
        "refactor should produce opportunity recommendations: {:?}",
        refactor_ctx
            .recommendations
            .iter()
            .map(|r| &r.category)
            .collect::<Vec<_>>()
    );

    // coding context: should NOT produce test or opportunity recs for these types
    let coding_ctx = kodex::learn::get_task_context_json(&db, "pagination", &[], 10, "coding");
    let has_test_in_coding = coding_ctx
        .recommendations
        .iter()
        .any(|r| r.category == "test");
    assert!(
        !has_test_in_coding,
        "coding should not produce test recommendations from bug_pattern"
    );
}

/// Gen3 test: recall_for_diff boosts affected knowledge
#[test]
fn test_recall_for_diff_boost() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("diff_recall.db");

    // Create graph with two nodes in different files
    let mut ext = kodex::types::ExtractionResult::default();
    ext.nodes.push(kodex::types::Node {
        id: "auth_fn".into(),
        label: "authenticate()".into(),
        file_type: kodex::types::FileType::Code,
        source_file: "project/auth.py".into(),
        source_location: Some("L10".into()),
        confidence: Some(kodex::types::Confidence::EXTRACTED),
        confidence_score: Some(1.0),
        community: None,
        norm_label: None,
        degree: None,
        uuid: Some("node-auth".into()),
        fingerprint: None,
        logical_key: None,
        body_hash: Some("aaaa".into()),
    });
    ext.nodes.push(kodex::types::Node {
        id: "payment_fn".into(),
        label: "process_payment()".into(),
        file_type: kodex::types::FileType::Code,
        source_file: "project/payment.py".into(),
        source_location: Some("L20".into()),
        confidence: Some(kodex::types::Confidence::EXTRACTED),
        confidence_score: Some(1.0),
        community: None,
        norm_label: None,
        degree: None,
        uuid: Some("node-pay".into()),
        fingerprint: None,
        logical_key: None,
        body_hash: Some("bbbb".into()),
    });
    let data = kodex::types::KodexData {
        extraction: ext,
        knowledge: vec![],
        links: vec![],
        review_queue: vec![],
    };
    kodex::storage::save(&db, &data).unwrap();

    // K1: linked to auth node
    let k1 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Pattern,
        "JWT Auth Pattern",
        "Token auth",
        Some(std::slice::from_ref(&"node-auth".to_string())),
        &[],
        None,
    )
    .unwrap();

    // K2: linked to payment node
    let _k2 = kodex::learn::learn_with_uuid(
        &db,
        None,
        kodex::learn::KnowledgeType::Convention,
        "Payment Validation",
        "Always validate amounts",
        Some(std::slice::from_ref(&"node-pay".to_string())),
        &[],
        None,
    )
    .unwrap();

    // Diff that changes auth.py only
    let diff = r#"diff --git a/project/auth.py b/project/auth.py
--- a/project/auth.py
+++ b/project/auth.py
@@ -10,3 +10,5 @@ def authenticate():
+    validate_token()
+    check_expiry()
"#;

    let (analysis, results) = kodex::learn::recall_for_diff(&db, diff, 10);

    // Analysis should find auth.py changed
    assert!(
        analysis.changed_files.iter().any(|f| f.contains("auth")),
        "Should detect auth.py change"
    );

    // K1 should be affected (linked to node-auth which is in auth.py)
    assert!(
        analysis.affected_knowledge_uuids.contains(&k1),
        "K1 should be in affected list"
    );

    // K1 should rank first (affected by diff + linked to changed node)
    assert!(!results.is_empty());
    assert_eq!(
        results[0].knowledge.uuid, k1,
        "JWT Auth should rank first for auth.py diff"
    );

    // K1 should have "directly affected by diff" in reasons
    assert!(
        results[0]
            .score
            .reasons
            .iter()
            .any(|r| r.contains("affected by diff")),
        "Should have diff boost reason: {:?}",
        results[0].score.reasons
    );
}
