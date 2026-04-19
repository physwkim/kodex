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
    assert!(result.files.code.len() >= 3, "Should find at least 3 code files, found {}", result.files.code.len());
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
    assert!(!result.nodes.is_empty(), "Should extract nodes from Python file");

    // Should find FileReader and CsvParser classes
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| *l == "FileReader"), "Should find FileReader class, got: {:?}", labels);
    assert!(labels.iter().any(|l| *l == "CsvParser"), "Should find CsvParser class, got: {:?}", labels);

    // Should find functions
    assert!(labels.iter().any(|l| l.contains("main")), "Should find main function");
    assert!(labels.iter().any(|l| l.contains("read")), "Should find read method");

    // Should have edges
    assert!(!result.edges.is_empty(), "Should extract edges");

    // Check for contains/method edges
    let relations: Vec<&str> = result.edges.iter().map(|e| e.relation.as_str()).collect();
    assert!(relations.contains(&"contains"), "Should have 'contains' edges");

    // Check inheritance (CsvParser extends FileReader)
    let extends_edges: Vec<_> = result.edges.iter()
        .filter(|e| e.relation == "extends")
        .collect();
    assert!(!extends_edges.is_empty(), "CsvParser should extend FileReader");
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
    assert!(!result.nodes.is_empty(), "Should extract nodes from JS file");

    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| *l == "DataLoader"), "Should find DataLoader class, got: {:?}", labels);
    assert!(labels.iter().any(|l| l.contains("processData")), "Should find processData function");
    assert!(labels.iter().any(|l| l.contains("load")), "Should find load method");

    // Should have import edges
    let import_edges: Vec<_> = result.edges.iter()
        .filter(|e| e.relation.contains("import"))
        .collect();
    assert!(!import_edges.is_empty(), "Should have import edges");
}

#[test]
#[cfg(feature = "lang-go")]
fn test_extract_go() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.go");
    let result = kodex::extract::generic::extract_generic(
        &path,
        &kodex::extract::languages::go::GO_CONFIG,
    );

    assert!(result.error.is_none(), "Extract error: {:?}", result.error);
    assert!(!result.nodes.is_empty(), "Should extract nodes from Go file");

    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("NewConfig")), "Should find NewConfig func, got: {:?}", labels);

    // Should have import edges
    let import_edges: Vec<_> = result.edges.iter()
        .filter(|e| e.relation.contains("import"))
        .collect();
    assert!(!import_edges.is_empty(), "Should have import edges");
}

#[test]
#[cfg(feature = "extract")]
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
                id: "a".to_string(), label: "Alpha".to_string(),
                file_type: kodex::types::FileType::Code,
                source_file: "a.py".to_string(),
                source_location: Some("L1".to_string()),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None, norm_label: None, degree: None,
            },
            kodex::types::Node {
                id: "b".to_string(), label: "Beta".to_string(),
                file_type: kodex::types::FileType::Code,
                source_file: "b.py".to_string(),
                source_location: Some("L1".to_string()),
                confidence: Some(kodex::types::Confidence::EXTRACTED),
                confidence_score: Some(1.0),
                community: None, norm_label: None, degree: None,
            },
        ],
        edges: vec![kodex::types::Edge {
            source: "a".to_string(), target: "b".to_string(),
            relation: "imports".to_string(),
            confidence: kodex::types::Confidence::EXTRACTED,
            source_file: "a.py".to_string(),
            source_location: Some("L2".to_string()),
            confidence_score: Some(1.0), weight: 1.0,
            original_src: None, original_tgt: None,
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
        nodes: (0..10).map(|i| kodex::types::Node {
            id: format!("n{i}"), label: format!("Node{i}"),
            file_type: kodex::types::FileType::Code,
            source_file: format!("file{}.py", i % 3),
            source_location: Some(format!("L{}", i + 1)),
            confidence: Some(kodex::types::Confidence::EXTRACTED),
            confidence_score: Some(1.0),
            community: None, norm_label: None, degree: None,
        }).collect(),
        edges: vec![
            ("n0", "n1"), ("n1", "n2"), ("n0", "n2"),  // cluster 1
            ("n3", "n4"), ("n4", "n5"), ("n3", "n5"),  // cluster 2
            ("n6", "n7"), ("n7", "n8"), ("n8", "n9"),  // cluster 3
            ("n2", "n3"),  // bridge
        ].iter().map(|(s, t)| kodex::types::Edge {
            source: s.to_string(), target: t.to_string(),
            relation: "calls".to_string(),
            confidence: kodex::types::Confidence::EXTRACTED,
            source_file: "test.py".to_string(),
            source_location: None, confidence_score: Some(1.0),
            weight: 1.0, original_src: None, original_tgt: None,
        }).collect(),
        ..Default::default()
    };

    let graph = kodex::graph::build_from_extraction(&extraction);
    let communities = kodex::cluster::cluster(&graph);

    // Should detect at least 2 communities
    assert!(communities.len() >= 2, "Should detect at least 2 communities, got {}", communities.len());

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
    let labels: std::collections::HashMap<usize, String> = communities.keys()
        .map(|&c| (c, format!("Community {c}"))).collect();
    let report = kodex::report::generate(
        &graph, &communities, &cohesion, &labels,
        &gods, &surprises,
        &kodex::types::DetectionResult::default(),
        0, 0, "test", Some(&questions),
    );
    assert!(report.contains("# Graph Report"));
    assert!(report.contains("God Nodes"));
}
