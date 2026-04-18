use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use crate::graph::GraphifyGraph;
use super::node_community_map;

/// Export graph to GraphML XML format (compatible with Gephi, yEd).
pub fn to_graphml(
    graph: &GraphifyGraph,
    communities: &HashMap<usize, Vec<String>>,
    output_path: &Path,
) -> std::io::Result<()> {
    let node_comm = node_community_map(communities);
    let mut xml = String::new();

    writeln!(xml, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(
        xml,
        r#"<graphml xmlns="http://graphml.graphstorm.org/graphml">"#
    )
    .unwrap();

    // Key declarations
    writeln!(xml, r#"  <key id="label" for="node" attr.name="label" attr.type="string"/>"#).unwrap();
    writeln!(xml, r#"  <key id="file_type" for="node" attr.name="file_type" attr.type="string"/>"#).unwrap();
    writeln!(xml, r#"  <key id="source_file" for="node" attr.name="source_file" attr.type="string"/>"#).unwrap();
    writeln!(xml, r#"  <key id="community" for="node" attr.name="community" attr.type="int"/>"#).unwrap();
    writeln!(xml, r#"  <key id="relation" for="edge" attr.name="relation" attr.type="string"/>"#).unwrap();
    writeln!(xml, r#"  <key id="confidence" for="edge" attr.name="confidence" attr.type="string"/>"#).unwrap();
    writeln!(xml, r#"  <graph id="G" edgedefault="directed">"#).unwrap();

    // Nodes
    for id in graph.node_ids() {
        if let Some(node) = graph.get_node(id) {
            let cid = node_comm.get(id).copied().unwrap_or(0);
            writeln!(xml, r#"    <node id="{}">"#, xml_escape(id)).unwrap();
            writeln!(xml, r#"      <data key="label">{}</data>"#, xml_escape(&node.label)).unwrap();
            writeln!(xml, r#"      <data key="file_type">{}</data>"#, node.file_type).unwrap();
            writeln!(xml, r#"      <data key="source_file">{}</data>"#, xml_escape(&node.source_file)).unwrap();
            writeln!(xml, r#"      <data key="community">{cid}</data>"#).unwrap();
            writeln!(xml, "    </node>").unwrap();
        }
    }

    // Edges
    for (i, (src, tgt, edge)) in graph.edges().enumerate() {
        writeln!(
            xml,
            r#"    <edge id="e{i}" source="{}" target="{}">"#,
            xml_escape(src),
            xml_escape(tgt)
        )
        .unwrap();
        writeln!(xml, r#"      <data key="relation">{}</data>"#, xml_escape(&edge.relation)).unwrap();
        writeln!(xml, r#"      <data key="confidence">{}</data>"#, edge.confidence).unwrap();
        writeln!(xml, "    </edge>").unwrap();
    }

    writeln!(xml, "  </graph>").unwrap();
    writeln!(xml, "</graphml>").unwrap();

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output_path, xml)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
