use std::path::PathBuf;

/// Helper: run the full pipeline on a directory and return node/edge counts.
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
fn test_hdf5_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();
    let h5_path = dir.path().join("test.h5");

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
    kodex::storage::save_hdf5(&graph, &communities, &h5_path).unwrap();
    assert!(h5_path.exists());

    let loaded = kodex::storage::load_hdf5(&h5_path).unwrap();
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
    let h5 = dir.path().join("test.h5");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_hdf5(&g, &c, &h5).unwrap();
    }

    kodex::learn::learn(
        &h5,
        kodex::learn::KnowledgeType::Pattern,
        "Test Pattern",
        "A test pattern description",
        &["node_a".to_string()],
        &["test".to_string()],
    )
    .unwrap();

    let results = kodex::learn::query_knowledge(&h5, "test", None);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Test Pattern");

    let results = kodex::learn::query_knowledge(&h5, "test", None);
    let uuid = results[0].uuid.clone();

    // Reinforce using UUID
    kodex::learn::learn_with_uuid(
        &h5,
        Some(&uuid),
        kodex::learn::KnowledgeType::Pattern,
        "Test Pattern",
        "Seen again",
        Some(&["node_b".to_string()]),
        &[],
        None,
    )
    .unwrap();

    let results = kodex::learn::query_knowledge(&h5, "test", None);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].observations, 2);
}

#[test]
fn test_knowledge_context_index() {
    let dir = tempfile::TempDir::new().unwrap();
    let h5 = dir.path().join("test.h5");
    {
        let e = kodex::types::ExtractionResult::default();
        let g = kodex::graph::build_from_extraction(&e);
        let c = kodex::cluster::cluster(&g);
        kodex::storage::save_hdf5(&g, &c, &h5).unwrap();
    }

    kodex::learn::learn(
        &h5,
        kodex::learn::KnowledgeType::Decision,
        "Use HDF5",
        "Fast storage",
        &[],
        &[],
    )
    .unwrap();
    kodex::learn::learn(
        &h5,
        kodex::learn::KnowledgeType::Convention,
        "Error Handling",
        "Use Result",
        &[],
        &[],
    )
    .unwrap();

    let ctx = kodex::learn::knowledge_context(&h5, 10);
    assert!(ctx.contains("Knowledge Index"));
    assert!(ctx.contains("Use HDF5"));
    assert!(ctx.contains("Error Handling"));
}

/// End-to-end scenario: code graph + knowledge + links + staleness + task context
#[test]
fn test_knowledge_graph_scenario() {
    let dir = tempfile::TempDir::new().unwrap();
    let h5 = dir.path().join("scenario.h5");

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
                community: None, norm_label: None, degree: None,
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
                community: None, norm_label: None, degree: None,
                uuid: Some("node-repo".into()),
                fingerprint: Some("fp-repo".into()),
                logical_key: Some("project/repo.py::UserRepo".into()),
                body_hash: Some("efgh5678".into()),
            },
        ],
        edges: vec![kodex::types::Edge {
            source: "auth_handler".into(), target: "user_repo".into(),
            relation: "calls".into(),
            confidence: kodex::types::Confidence::EXTRACTED,
            source_file: "project/auth.py".into(),
            source_location: Some("L15".into()),
            confidence_score: Some(1.0), weight: 1.0,
            original_src: None, original_tgt: None,
        }],
        ..Default::default()
    };
    let data = kodex::types::KodexData {
        extraction,
        knowledge: vec![],
        links: vec![],
    };
    kodex::storage::save(&h5, &data).unwrap();

    // 2. Agent learns knowledge and links to code nodes
    let k1 = kodex::learn::learn_with_uuid(
        &h5, None, kodex::learn::KnowledgeType::Pattern,
        "Repository Pattern", "All data access through repo classes",
        Some(&["node-repo".to_string()]), &["architecture".into()], None,
    ).unwrap();

    let k2 = kodex::learn::learn_with_uuid(
        &h5, None, kodex::learn::KnowledgeType::Decision,
        "JWT Auth", "Chose JWT for stateless auth",
        Some(&["node-auth".to_string()]), &["auth".into()], Some(&k1),
    ).unwrap();

    let k3 = kodex::learn::learn_with_uuid(
        &h5, None, kodex::learn::KnowledgeType::Convention,
        "Always validate tokens", "Every endpoint must validate",
        Some(&["node-auth".to_string()]), &[], Some(&k2),
    ).unwrap();

    // 3. Link knowledge ↔ knowledge (beyond chain)
    kodex::learn::link_knowledge_to_knowledge(&h5, &k1, &k2, "supports", true).unwrap();

    // 4. Verify thought chain
    let chain = kodex::learn::thought_chain(&h5, &k2);
    assert_eq!(chain.len(), 3, "Chain should have 3 steps");
    assert_eq!(chain[0].title, "Repository Pattern");
    assert_eq!(chain[2].title, "Always validate tokens");

    // 5. Verify related_nodes are node-only (not knowledge UUIDs)
    let all = kodex::learn::query_knowledge(&h5, "", None);
    for item in &all {
        for r in &item.related_nodes {
            assert!(r.starts_with("node-"), "related_nodes should be node UUIDs, got: {r}");
        }
    }

    // 6. Task context — auth.py is being edited
    let ctx = kodex::learn::get_task_context(&h5, "auth token", &["project/auth.py".into()], 10);
    assert!(ctx.contains("JWT Auth"), "Should surface JWT knowledge for auth file");
    assert!(ctx.contains("validate tokens"), "Should surface validation convention");

    // 7. recall_for_task — repo.py is being edited
    let results = kodex::learn::recall_for_task(
        &h5, "data access", &["project/repo.py".into()], &["node-repo".into()], 5,
    );
    assert!(!results.is_empty());
    assert_eq!(results[0].title, "Repository Pattern", "Repo pattern should rank first for repo.py");

    // 8. Knowledge graph traversal
    let graph_nodes = kodex::learn::traverse_knowledge_graph(&h5, Some(&k1), 2);
    assert!(graph_nodes.len() >= 2, "Should reach k2 from k1 within 2 hops");
    let k1_node = graph_nodes.iter().find(|n| n.uuid == k1).unwrap();
    assert!(!k1_node.links_out.is_empty(), "k1 should have outgoing knowledge links");
    assert!(!k1_node.node_links.is_empty(), "k1 should have node links");
    assert_eq!(k1_node.node_links[0].target_title, "UserRepo");

    // 9. Staleness detection — all nodes exist, nothing stale
    let stale = kodex::learn::detect_stale_knowledge(&h5).unwrap();
    assert_eq!(stale, 0, "No stale knowledge when all nodes exist");

    // 10. Simulate node deletion (re-save without auth node)
    let mut data2 = kodex::storage::load(&h5).unwrap();
    data2.extraction.nodes.retain(|n| n.uuid.as_deref() != Some("node-auth"));
    kodex::storage::save(&h5, &data2).unwrap();

    // k2 and k3 linked to node-auth should now be stale
    let stale = kodex::learn::detect_stale_knowledge(&h5).unwrap();
    assert!(stale >= 1, "Should detect stale knowledge after node deletion");

    // k1 linked to node-repo should NOT be stale
    let k1_entry = kodex::learn::query_knowledge(&h5, "Repository Pattern", None);
    assert!(!k1_entry.is_empty());
    // k1 should still be queryable and active (not needs_review)

    // 11. update_knowledge — mark k3 as validated
    kodex::learn::update_knowledge(&h5, &k3, &kodex::learn::KnowledgeUpdates {
        status: Some("active".into()),
        applies_when: Some("any endpoint modification".into()),
        validate: true,
        ..Default::default()
    }).unwrap();

    let data3 = kodex::storage::load(&h5).unwrap();
    let k3_entry = data3.knowledge.iter().find(|k| k.uuid == k3).unwrap();
    assert_eq!(k3_entry.status, "active");
    assert_eq!(k3_entry.applies_when, "any endpoint modification");
    assert!(k3_entry.last_validated_at > 0);

    // 12. Selective link removal
    kodex::learn::remove_link(&h5, &k1, &k2, Some("supports")).unwrap();
    let neighbors = kodex::learn::knowledge_neighbors(&h5, &k1);
    let support_links: Vec<_> = neighbors.iter().filter(|(_, r, _)| r == "supports").collect();
    assert!(support_links.is_empty(), "supports link should be removed");
    // leads_to chain should still exist
    let chain_after = kodex::learn::thought_chain(&h5, &k1);
    assert!(chain_after.len() >= 2, "Chain should survive supports link removal");

    // 13. Markdown rendering
    let md = kodex::learn::render_thought_chain(&chain);
    assert!(md.contains("Thought Chain"));
    assert!(md.contains("leads_to"));

    let graph_md = kodex::learn::render_knowledge_graph(&graph_nodes);
    assert!(graph_md.contains("Knowledge Graph"));
}
