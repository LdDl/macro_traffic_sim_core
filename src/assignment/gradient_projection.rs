//! # Gradient Projection Algorithm
//!
//! Path-based traffic assignment using gradient projection
//! to find User Equilibrium. Maintains explicit path sets
//! per OD pair and shifts flow along the gradient direction.
//!
//! ## Algorithm
//!
//! 1. Initialize: find the shortest path for each OD pair at
//!    free-flow costs and load all demand onto it.
//! 2. At each iteration:
//!    a. Compute link costs from current volumes via the VDF.
//!    b. Update cost of every stored path.
//!    c. For each OD pair, find the current shortest path.
//!       If it is new, add it to the path set.
//!    d. Shift flow from expensive paths toward the shortest path.
//!       The shift amount is proportional to `step * cost_diff * flow`,
//!       where `step = step_scale / sqrt(iteration)`.
//!    e. Remove paths with negligible flow (< 1e-10).
//!    f. Recompute link volumes from all path flows.
//!    g. Check the relative gap -- if below threshold, stop.
//!
//! ## Path management
//!
//! Each OD pair maintains a set of active paths. New shortest paths
//! are added to the set as they are discovered. Paths whose flow
//! drops below `1e-10` are removed (except the current shortest path,
//! which is always retained). This keeps the path set compact while
//! still capturing route diversity.
//!
//! ## Trade-offs
//!
//! - Stores explicit paths per OD pair, enabling path-level analysis.
//! - More memory than link-based methods (Frank-Wolfe, MSA).
//! - Each iteration requires one shortest path computation per origin.
//! - The `step_scale` parameter controls aggressiveness of flow shifting.
//!   Smaller values are more conservative but may converge slower.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::error::AssignmentError;
use super::indexed_graph::IndexedGraph;
use crate::gmns::meso::network::Network;
use crate::gmns::types::{LinkID, ZoneID};
use crate::log_additional;
use crate::log_main;
use crate::od::OdMatrix;
use crate::verbose::{EVENT_ASSIGNMENT, EVENT_ASSIGNMENT_ITERATION, EVENT_CONVERGENCE};

use super::{AssignmentConfig, AssignmentMethod, AssignmentResult, OdPath, VolumeDelayFunction};

/// A path stored as link indices for O(1) cost lookup.
#[derive(Debug, Clone)]
struct Path {
    /// Ordered sequence of link indices (into IndexedGraph).
    link_indices: Vec<usize>,
    /// Hash fingerprint of the link indices (for fast equality check).
    fingerprint: u64,
    /// Flow (demand) currently assigned to this path.
    flow: f64,
    /// Current travel cost (sum of link costs along the path).
    cost: f64,
}

fn path_fingerprint(link_indices: &[usize]) -> u64 {
    let mut hasher = DefaultHasher::new();
    link_indices.hash(&mut hasher);
    hasher.finish()
}

/// Compute path cost from link index array and flat cost Vec.
#[inline]
fn path_cost_indexed(link_indices: &[usize], costs: &[f64]) -> f64 {
    link_indices.iter().map(|&li| costs[li]).sum()
}

/// Build path as link indices by walking predecessors from dest to origin.
fn build_path_indexed(pred: &[Option<usize>], graph: &IndexedGraph, dest_idx: usize) -> Vec<usize> {
    let mut path = Vec::new();
    let mut current = dest_idx;
    loop {
        match pred[current] {
            Some(li) => {
                path.push(li);
                current = graph.link_source(li);
            }
            None => break,
        }
    }
    path.reverse();
    path
}

/// Gradient Projection traffic assignment.
///
/// Path-based method that maintains explicit path sets per OD pair.
/// At each iteration, shifts flow from expensive paths to the shortest
/// path proportional to the cost difference (gradient).
///
/// Unlike Frank-Wolfe and MSA, this method stores paths explicitly,
/// so after convergence you can inspect which routes carry flow.
///
/// # Usage
///
/// ```text
/// // Default step_scale = 0.1
/// let gp = GradientProjection::new();
///
/// // Custom step scale for more aggressive shifting
/// let gp = GradientProjection::with_step_scale(0.2);
///
/// let result = gp.assign(&network, &od_matrix, &bpr, &config)?;
/// ```
#[derive(Debug)]
pub struct GradientProjection {
    /// Step size scaling parameter.
    ///
    /// At iteration `n`, the effective step is `step_scale / sqrt(n)`.
    /// Larger values shift more flow per iteration but may overshoot.
    /// Default: `0.1`.
    pub step_scale: f64,
}

impl GradientProjection {
    /// Create a new gradient projection instance with default `step_scale = 0.1`.
    pub fn new() -> Self {
        GradientProjection { step_scale: 0.1 }
    }

    /// Create a gradient projection instance with a custom step scale.
    ///
    /// # Arguments
    /// * `step_scale` - Scaling factor for flow shifting. The effective step
    ///   at iteration `n` is `step_scale / sqrt(n)`.
    pub fn with_step_scale(step_scale: f64) -> Self {
        GradientProjection { step_scale }
    }
}

impl Default for GradientProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl AssignmentMethod for GradientProjection {
    fn assign(
        &self,
        _network: &Network,
        graph: &IndexedGraph,
        od_matrix: &dyn OdMatrix,
        vdf: &dyn VolumeDelayFunction,
        config: &AssignmentConfig,
        _initial_volumes: Option<&HashMap<LinkID, f64>>,
    ) -> Result<AssignmentResult, AssignmentError> {
        log_main!(EVENT_ASSIGNMENT, "Starting Gradient Projection assignment",);

        let zone_ids = od_matrix.zone_ids().to_vec();
        let n = graph.num_links;

        let zone_node_idxs: Vec<Option<usize>> =
            zone_ids.iter().map(|&z| graph.zone_node_idx(z)).collect();

        let mut costs = vec![0.0; n];
        let mut volumes = vec![0.0; n];

        // Pre-allocate Dijkstra buffers (reused across all calls)
        let mut dij_dist = vec![f64::INFINITY; graph.num_nodes];
        let mut dij_pred: Vec<Option<usize>> = vec![None; graph.num_nodes];
        let mut dij_visited = vec![false; graph.num_nodes];

        // Initialize with free-flow costs
        graph.compute_costs(&volumes, vdf, &mut costs);

        // path_sets keyed by (origin_zone, dest_zone)
        let mut path_sets: HashMap<(ZoneID, ZoneID), Vec<Path>> = HashMap::new();

        // Initial shortest path loading
        for (oi, &origin_zone) in zone_ids.iter().enumerate() {
            let origin_idx = match zone_node_idxs[oi] {
                Some(i) => i,
                None => continue,
            };

            graph.dijkstra_into(
                origin_idx,
                &costs,
                &mut dij_dist,
                &mut dij_pred,
                &mut dij_visited,
            );

            for (di, &dest_zone) in zone_ids.iter().enumerate() {
                if oi == di {
                    continue;
                }

                let demand = od_matrix.get(origin_zone, dest_zone);
                if demand <= 0.0 {
                    continue;
                }

                let dest_idx = match zone_node_idxs[di] {
                    Some(i) => i,
                    None => continue,
                };

                let link_indices = build_path_indexed(&dij_pred, graph, dest_idx);
                if link_indices.is_empty() {
                    continue;
                }

                let cost = path_cost_indexed(&link_indices, &costs);
                let fp = path_fingerprint(&link_indices);

                for &li in &link_indices {
                    volumes[li] += demand;
                }

                path_sets.insert(
                    (origin_zone, dest_zone),
                    vec![Path {
                        link_indices,
                        fingerprint: fp,
                        flow: demand,
                        cost,
                    }],
                );
            }
        }

        let mut converged = false;
        let mut relative_gap = f64::MAX;
        let mut iteration = 0;

        for iter in 0..config.max_iterations {
            iteration = iter + 1;

            // Update costs from current volumes
            graph.compute_costs(&volumes, vdf, &mut costs);

            // Update all path costs
            for paths in path_sets.values_mut() {
                for path in paths.iter_mut() {
                    path.cost = path_cost_indexed(&path.link_indices, &costs);
                }
            }

            // For each OD pair: find shortest path, add to set if new, shift flow
            let mut total_cost = 0.0;
            let mut total_shortest_cost = 0.0;

            for (oi, &origin_zone) in zone_ids.iter().enumerate() {
                let origin_idx = match zone_node_idxs[oi] {
                    Some(i) => i,
                    None => continue,
                };

                graph.dijkstra_into(
                    origin_idx,
                    &costs,
                    &mut dij_dist,
                    &mut dij_pred,
                    &mut dij_visited,
                );

                for (di, &dest_zone) in zone_ids.iter().enumerate() {
                    if oi == di {
                        continue;
                    }

                    let paths = match path_sets.get_mut(&(origin_zone, dest_zone)) {
                        Some(p) => p,
                        None => continue,
                    };

                    let dest_idx = match zone_node_idxs[di] {
                        Some(i) => i,
                        None => continue,
                    };

                    let sp_links = build_path_indexed(&dij_pred, graph, dest_idx);
                    if sp_links.is_empty() {
                        continue;
                    }
                    let sp_cost = path_cost_indexed(&sp_links, &costs);
                    let sp_fp = path_fingerprint(&sp_links);

                    // Add shortest path to set if not already present
                    let sp_idx = match paths.iter().position(|p| p.fingerprint == sp_fp) {
                        Some(idx) => idx,
                        None => {
                            paths.push(Path {
                                link_indices: sp_links,
                                fingerprint: sp_fp,
                                flow: 0.0,
                                cost: sp_cost,
                            });
                            paths.len() - 1
                        }
                    };

                    let mut total_demand = 0.0;
                    let mut weighted_cost = 0.0;
                    for path in paths.iter() {
                        total_demand += path.flow;
                        weighted_cost += path.flow * path.cost;
                    }
                    total_cost += weighted_cost;
                    total_shortest_cost += total_demand * sp_cost;

                    // Gradient projection: shift flow, track deficit inline
                    let step = self.step_scale / (iteration as f64).sqrt();
                    let mut deficit = 0.0;

                    for (idx, path) in paths.iter_mut().enumerate() {
                        if idx == sp_idx {
                            continue;
                        }
                        let cost_diff = path.cost - sp_cost;
                        if cost_diff > 0.0 && path.flow > 0.0 {
                            let shift = (step * cost_diff * path.flow).min(path.flow);
                            path.flow -= shift;
                            deficit += shift;
                        }
                    }

                    if deficit > 0.0 {
                        paths[sp_idx].flow += deficit;
                    }

                    // Remove paths with negligible flow (keep shortest path)
                    paths.retain(|p| p.flow > 1e-10 || p.fingerprint == sp_fp);
                }
            }

            // Rebuild volumes from all path flows
            volumes.fill(0.0);
            for paths in path_sets.values() {
                for path in paths {
                    for &li in &path.link_indices {
                        volumes[li] += path.flow;
                    }
                }
            }

            if total_cost > 0.0 {
                relative_gap = (total_cost - total_shortest_cost) / total_cost;
            } else {
                relative_gap = 0.0;
            }

            log_additional!(
                EVENT_ASSIGNMENT_ITERATION,
                "Gradient Projection iteration",
                iteration = iteration,
                gap = format!("{:.8}", relative_gap)
            );

            if relative_gap.abs() < config.convergence_gap {
                converged = true;
                break;
            }
        }

        // Final cost computation
        graph.compute_costs(&volumes, vdf, &mut costs);

        log_main!(
            EVENT_CONVERGENCE,
            "Gradient Projection complete",
            iterations = iteration,
            gap = format!("{:.8}", relative_gap),
            converged = converged
        );

        let path_flows = if config.store_paths {
            let mut result = Vec::new();
            for (&(origin, dest), paths) in &path_sets {
                for (idx, path) in paths.iter().enumerate() {
                    if path.flow <= 0.0 {
                        continue;
                    }
                    result.push(OdPath {
                        origin_zone: origin,
                        dest_zone: dest,
                        path_index: idx as u32,
                        flow: path.flow,
                        cost: path.cost,
                        link_ids: path
                            .link_indices
                            .iter()
                            .map(|&li| graph.link_id(li))
                            .collect(),
                    });
                }
            }
            Some(result)
        } else {
            None
        };

        Ok(AssignmentResult {
            link_volumes: graph.volumes_to_hashmap(&volumes),
            link_costs: graph.costs_to_hashmap(&costs),
            iterations: iteration,
            relative_gap,
            converged,
            class_volumes: None,
            path_flows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignment::{AssignmentConfig, BprFunction};
    use crate::gmns::meso::link::Link;
    use crate::gmns::meso::network::Network;
    use crate::gmns::meso::node::Node;
    use crate::od::dense::DenseOdMatrix;

    const EPS: f64 = 1e-10;

    fn two_link_network() -> (Network, IndexedGraph) {
        let mut net = Network::new();
        net.add_node(
            Node::new(1)
                .with_zone_id(1)
                .with_coordinates(0.0, 0.0)
                .build(),
        )
        .unwrap();
        net.add_node(
            Node::new(2)
                .with_zone_id(2)
                .with_coordinates(0.0, 1.0)
                .build(),
        )
        .unwrap();
        net.add_link(
            Link::new(100, 1, 2)
                .with_length_meters(1000.0)
                .with_free_speed(60.0)
                .with_capacity(1000.0)
                .build(),
        )
        .unwrap();
        net.add_link(
            Link::new(101, 2, 1)
                .with_length_meters(1000.0)
                .with_free_speed(60.0)
                .with_capacity(1000.0)
                .build(),
        )
        .unwrap();
        let graph = IndexedGraph::from_network(&net);
        (net, graph)
    }

    #[test]
    fn store_paths_returns_od_paths() {
        let (net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 50,
            convergence_gap: 1e-4,
            store_paths: true,
        };
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 500.0);
        od.set(2, 1, 300.0);

        let gp = GradientProjection::new();
        let result = gp
            .assign(&net, &graph, &od, &bpr, &config, None)
            .unwrap();

        let paths = result.path_flows.expect("path_flows should be Some");
        assert!(!paths.is_empty());

        let paths_1_2: Vec<_> = paths
            .iter()
            .filter(|p| p.origin_zone == 1 && p.dest_zone == 2)
            .collect();
        assert!(!paths_1_2.is_empty());

        let total_flow_1_2: f64 = paths_1_2.iter().map(|p| p.flow).sum();
        assert!((total_flow_1_2 - 500.0).abs() < EPS);

        for p in &paths {
            assert!(p.flow > 0.0);
            assert!(p.cost > 0.0);
            assert!(!p.link_ids.is_empty());
            assert!(p.link_ids.iter().all(|&id| id == 100 || id == 101));
        }
    }

    #[test]
    fn store_paths_false_returns_none() {
        let (net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 50,
            convergence_gap: 1e-4,
            store_paths: false,
        };
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 500.0);

        let gp = GradientProjection::new();
        let result = gp
            .assign(&net, &graph, &od, &bpr, &config, None)
            .unwrap();

        assert!(result.path_flows.is_none());
    }
}
