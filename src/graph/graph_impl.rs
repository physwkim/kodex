use std::collections::HashMap;

use petgraph::graph::{DiGraph, NodeIndex};

use crate::types::{Edge, Hyperedge, Node};

/// Wrapper around petgraph providing O(1) node lookup by ID string.
pub struct KodexGraph {
    pub inner: DiGraph<Node, Edge>,
    pub node_index: HashMap<String, NodeIndex>,
    pub hyperedges: Vec<Hyperedge>,
}

impl KodexGraph {
    pub fn new() -> Self {
        Self {
            inner: DiGraph::new(),
            node_index: HashMap::new(),
            hyperedges: Vec::new(),
        }
    }

    /// Add a node, returning its index. If the ID already exists, update the node data.
    pub fn add_node(&mut self, node: Node) -> NodeIndex {
        if let Some(&idx) = self.node_index.get(&node.id) {
            // Update existing node (last write wins, matching networkx behavior)
            self.inner[idx] = node;
            idx
        } else {
            let id = node.id.clone();
            let idx = self.inner.add_node(node);
            self.node_index.insert(id, idx);
            idx
        }
    }

    /// Add an edge between two nodes identified by their string IDs.
    pub fn add_edge(&mut self, edge: Edge) -> bool {
        let src_idx = self.node_index.get(&edge.source);
        let tgt_idx = self.node_index.get(&edge.target);
        if let (Some(&s), Some(&t)) = (src_idx, tgt_idx) {
            self.inner.add_edge(s, t, edge);
            true
        } else {
            false // Skip edges to unknown nodes
        }
    }

    /// Get a node by its string ID.
    pub fn get_node(&self, id: &str) -> Option<&Node> {
        self.node_index.get(id).map(|&idx| &self.inner[idx])
    }

    /// Total number of nodes.
    pub fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Total number of edges.
    pub fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }

    /// Get degree (total edges, both directions) for a node by ID.
    pub fn degree(&self, id: &str) -> usize {
        self.node_index
            .get(id)
            .map(|&idx| {
                self.inner
                    .edges_directed(idx, petgraph::Direction::Outgoing)
                    .count()
                    + self
                        .inner
                        .edges_directed(idx, petgraph::Direction::Incoming)
                        .count()
            })
            .unwrap_or(0)
    }

    /// Get neighbor node IDs for a given node ID.
    pub fn neighbors(&self, id: &str) -> Vec<String> {
        self.node_index
            .get(id)
            .map(|&idx| {
                self.inner
                    .neighbors_undirected(idx)
                    .map(|n| self.inner[n].id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Iterate over all node IDs.
    pub fn node_ids(&self) -> impl Iterator<Item = &String> {
        self.node_index.keys()
    }

    /// Iterate over all edges as (source_id, target_id, &Edge).
    pub fn edges(&self) -> impl Iterator<Item = (&str, &str, &Edge)> {
        self.inner.edge_indices().filter_map(|e| {
            let (s, t) = self.inner.edge_endpoints(e)?;
            let edge = &self.inner[e];
            Some((self.inner[s].id.as_str(), self.inner[t].id.as_str(), edge))
        })
    }
}

impl Default for KodexGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Confidence, FileType};

    fn test_node(id: &str) -> Node {
        Node {
            id: id.to_string(),
            label: id.to_string(),
            file_type: FileType::Code,
            source_file: "test.py".to_string(),
            source_location: None,
            confidence: Some(Confidence::EXTRACTED),
            confidence_score: Some(1.0),
            community: None,
            norm_label: None,
            degree: None,
            uuid: None,
            fingerprint: None,
            logical_key: None,
        }
    }

    fn test_edge(src: &str, tgt: &str) -> Edge {
        Edge {
            source: src.to_string(),
            target: tgt.to_string(),
            relation: "contains".to_string(),
            confidence: Confidence::EXTRACTED,
            source_file: "test.py".to_string(),
            source_location: None,
            confidence_score: Some(1.0),
            weight: 1.0,
            original_src: None,
            original_tgt: None,
        }
    }

    #[test]
    fn test_add_and_lookup() {
        let mut g = KodexGraph::new();
        g.add_node(test_node("a"));
        g.add_node(test_node("b"));
        g.add_edge(test_edge("a", "b"));

        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert!(g.get_node("a").is_some());
        assert!(g.get_node("c").is_none());
    }

    #[test]
    fn test_degree() {
        let mut g = KodexGraph::new();
        g.add_node(test_node("a"));
        g.add_node(test_node("b"));
        g.add_node(test_node("c"));
        g.add_edge(test_edge("a", "b"));
        g.add_edge(test_edge("a", "c"));

        assert_eq!(g.degree("a"), 2);
        assert_eq!(g.degree("b"), 1);
    }

    #[test]
    fn test_node_overwrite() {
        let mut g = KodexGraph::new();
        g.add_node(test_node("a"));
        let mut updated = test_node("a");
        updated.label = "Updated".to_string();
        g.add_node(updated);

        assert_eq!(g.node_count(), 1);
        assert_eq!(g.get_node("a").unwrap().label, "Updated");
    }
}
