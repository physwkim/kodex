use std::collections::HashMap;

use crate::graph::EngramGraph;

/// Louvain community detection with modularity optimization.
///
/// Greedily moves nodes to the neighboring community that yields the
/// greatest modularity gain, iterating until no improvement is found.
pub fn louvain_communities(graph: &EngramGraph) -> HashMap<String, usize> {
    let node_ids: Vec<String> = graph.node_ids().cloned().collect();
    if node_ids.is_empty() {
        return HashMap::new();
    }

    // Initialize: each node in its own community
    let mut community: HashMap<String, usize> = node_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect();

    let m = graph.edge_count() as f64;
    if m == 0.0 {
        return community;
    }
    let m2 = 2.0 * m;

    // Precompute degrees
    let degrees: HashMap<&str, f64> = node_ids
        .iter()
        .map(|id| (id.as_str(), graph.degree(id) as f64))
        .collect();

    // Precompute adjacency
    let mut adjacency: HashMap<&str, Vec<(&str, f64)>> = HashMap::new();
    for (src, tgt, edge) in graph.edges() {
        adjacency.entry(src).or_default().push((tgt, edge.weight));
        adjacency.entry(tgt).or_default().push((src, edge.weight));
    }

    // Maintain sigma_tot incrementally: sum of degrees per community
    let mut sigma_tot: HashMap<usize, f64> = HashMap::new();
    for (id, &comm) in &community {
        let d = degrees.get(id.as_str()).copied().unwrap_or(0.0);
        *sigma_tot.entry(comm).or_default() += d;
    }

    let mut improved = true;
    let mut iterations = 0;
    let max_iterations = 50;

    while improved && iterations < max_iterations {
        improved = false;
        iterations += 1;

        for node_id in &node_ids {
            let current_comm = community[node_id.as_str()];
            let ki = degrees.get(node_id.as_str()).copied().unwrap_or(0.0);

            let neighbors = match adjacency.get(node_id.as_str()) {
                Some(n) if ki > 0.0 => n,
                _ => continue,
            };

            // Sum of weights to each neighboring community
            let mut comm_weights: HashMap<usize, f64> = HashMap::new();
            for &(neighbor, weight) in neighbors {
                if let Some(&comm) = community.get(neighbor) {
                    *comm_weights.entry(comm).or_default() += weight;
                }
            }

            // Find community with maximum modularity gain
            let ki_in_current = comm_weights.get(&current_comm).copied().unwrap_or(0.0);
            let sc = sigma_tot.get(&current_comm).copied().unwrap_or(0.0);
            let remove_cost = ki_in_current / m2 - (sc * ki) / (m2 * m2);

            let mut best_comm = current_comm;
            let mut best_gain = 0.0;

            for (&cand_comm, &ki_in_cand) in &comm_weights {
                if cand_comm == current_comm {
                    continue;
                }
                let s_cand = sigma_tot.get(&cand_comm).copied().unwrap_or(0.0);
                let insert_gain = ki_in_cand / m2 - (s_cand * ki) / (m2 * m2);
                let delta_q = insert_gain - remove_cost;

                if delta_q > best_gain {
                    best_gain = delta_q;
                    best_comm = cand_comm;
                }
            }

            if best_comm != current_comm {
                // Update sigma_tot incrementally
                *sigma_tot.entry(current_comm).or_default() -= ki;
                *sigma_tot.entry(best_comm).or_default() += ki;
                community.insert(node_id.clone(), best_comm);
                improved = true;
            }
        }
    }

    // Compact community IDs
    let mut unique_comms: Vec<usize> = community.values().copied().collect();
    unique_comms.sort();
    unique_comms.dedup();

    let remap: HashMap<usize, usize> = unique_comms
        .into_iter()
        .enumerate()
        .map(|(new, old)| (old, new))
        .collect();

    community
        .into_iter()
        .map(|(id, comm)| (id, remap[&comm]))
        .collect()
}
