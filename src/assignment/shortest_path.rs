//! # Shortest Path Algorithms
//!
//! Dijkstra's algorithm and all-or-nothing assignment utility.
//!
//! ## Components
//!
//! - [`dijkstra_one_to_all`] - Single-source shortest path using Dijkstra's
//!   algorithm with a binary heap. Produces a [`ShortestPathTree`] containing
//!   distances and predecessor links for every reachable node.
//!
//! - [`build_path`] - Reconstructs the link sequence from a predecessor map
//!   by backtracking from the destination to the origin.
//!
//! - [`all_or_nothing`] - Loads all OD demand onto the shortest paths.
//!   For each origin, runs Dijkstra once, then traces the path for every
//!   destination and adds the demand to those links. This is the core
//!   subroutine used by Frank-Wolfe, MSA, and Gradient Projection at
//!   each iteration.
//!
//! - [`compute_skim_matrix`] - Builds a zone-to-zone travel cost matrix
//!   (skim) from shortest path distances. Used to feed trip distribution
//!   and mode choice with updated costs after assignment.
//!
//! ## Dijkstra implementation
//!
//! Uses `BinaryHeap<Reverse<(OrderedFloat<f64>, NodeID)>>` for a
//! min-heap ordered by distance. `OrderedFloat` from the `ordered_float`
//! crate provides `Ord` for `f64`. Links with non-finite costs are
//! skipped. Complexity: `O((V + E) log V)` where `V` is the number of
//! nodes and `E` is the number of links.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::HashMap;

use ordered_float::OrderedFloat;

use super::error::AssignmentError;
use crate::gmns::meso::network::Network;
use crate::gmns::types::{LinkID, NodeID, ZoneID};
use crate::od::OdMatrix;

/// Result of a single-source shortest path computation.
///
/// Produced by [`dijkstra_one_to_all`]. Contains the shortest distance
/// from the origin to every reachable node, and the predecessor link
/// on the shortest path tree (used by [`build_path`] to reconstruct
/// the full route).
pub struct ShortestPathTree {
    /// Shortest distance from the origin to each reachable node.
    /// Nodes not reachable from the origin are absent from the map.
    pub distances: HashMap<NodeID, f64>,
    /// Predecessor link for each node on the shortest path tree.
    /// The origin node maps to `None`. Unreachable nodes are absent.
    pub predecessors: HashMap<NodeID, Option<LinkID>>,
}

/// Run Dijkstra's algorithm from a single origin node.
///
/// # Arguments
/// * `network` - The macroscopic network.
/// * `origin` - Source node ID.
/// * `link_costs` - Travel cost (time) for each link.
///
/// # Returns
/// A `ShortestPathTree` with distances and predecessors.
pub fn dijkstra_one_to_all(
    network: &Network,
    origin: NodeID,
    link_costs: &HashMap<LinkID, f64>,
) -> ShortestPathTree {
    let mut distances: HashMap<NodeID, f64> = HashMap::new();
    let mut predecessors: HashMap<NodeID, Option<LinkID>> = HashMap::new();

    distances.insert(origin, 0.0);
    predecessors.insert(origin, None);

    let mut heap: BinaryHeap<Reverse<(OrderedFloat<f64>, NodeID)>> = BinaryHeap::new();
    heap.push(Reverse((OrderedFloat(0.0), origin)));

    while let Some(Reverse((OrderedFloat(dist), node_id))) = heap.pop() {
        if let Some(&best) = distances.get(&node_id) {
            if dist > best {
                continue;
            }
        }

        // Explore outgoing links
        let outgoing = match network.outgoing_links(node_id) {
            Ok(links) => links.to_vec(),
            Err(_) => continue,
        };

        for &link_id in &outgoing {
            let link_cost = link_costs.get(&link_id).copied().unwrap_or(f64::INFINITY);
            if !link_cost.is_finite() {
                continue;
            }

            let target = match network.link_target(link_id) {
                Ok(t) => t,
                Err(_) => continue,
            };

            let new_dist = dist + link_cost;
            let is_shorter = distances
                .get(&target)
                .map_or(true, |&current| new_dist < current);

            if is_shorter {
                distances.insert(target, new_dist);
                predecessors.insert(target, Some(link_id));
                heap.push(Reverse((OrderedFloat(new_dist), target)));
            }
        }
    }

    ShortestPathTree {
        distances,
        predecessors,
    }
}

/// Reconstruct a path from the shortest path tree.
///
/// # Arguments
/// * `predecessors` - Predecessor link map from Dijkstra.
/// * `network` - The macroscopic network (to get link source nodes).
/// * `destination` - Target node ID.
///
/// # Returns
/// A vector of link IDs forming the shortest path, or empty if unreachable.
pub fn build_path(
    predecessors: &HashMap<NodeID, Option<LinkID>>,
    network: &Network,
    destination: NodeID,
) -> Vec<LinkID> {
    let mut path = Vec::new();
    let mut current = destination;

    loop {
        match predecessors.get(&current) {
            Some(Some(link_id)) => {
                path.push(*link_id);
                match network.link_source(*link_id) {
                    Ok(source) => current = source,
                    Err(_) => break,
                }
            }
            _ => break,
        }
    }

    path.reverse();
    path
}

/// Perform all-or-nothing assignment.
///
/// For each OD pair, finds the shortest path and loads all demand onto it.
///
/// # Arguments
/// * `network` - The macroscopic network.
/// * `od_matrix` - Origin-destination demand matrix.
/// * `link_costs` - Current link travel costs.
///
/// # Returns
/// Link volumes from all-or-nothing loading.
pub fn all_or_nothing(
    network: &Network,
    od_matrix: &dyn OdMatrix,
    link_costs: &HashMap<LinkID, f64>,
) -> Result<HashMap<LinkID, f64>, AssignmentError> {
    let mut volumes: HashMap<LinkID, f64> = HashMap::new();

    for &link_id in network.links.keys() {
        volumes.insert(link_id, 0.0);
    }

    let zone_ids = od_matrix.zone_ids().to_vec();

    for &origin_zone in &zone_ids {
        let origin_node = match network.get_zone_centroid(origin_zone) {
            Ok(n) => n,
            Err(_) => continue,
        };

        // Run Dijkstra from this origin
        let spt = dijkstra_one_to_all(network, origin_node, link_costs);

        for &dest_zone in &zone_ids {
            if origin_zone == dest_zone {
                continue;
            }

            let demand = od_matrix.get(origin_zone, dest_zone);
            if demand <= 0.0 {
                continue;
            }

            let dest_node = match network.get_zone_centroid(dest_zone) {
                Ok(n) => n,
                Err(_) => continue,
            };

            // Build path and load demand
            let path = build_path(&spt.predecessors, network, dest_node);
            for &link_id in &path {
                *volumes.entry(link_id).or_insert(0.0) += demand;
            }
        }
    }

    Ok(volumes)
}

/// Compute a zone-to-zone cost (skim) matrix from shortest path trees.
///
/// For each origin zone, runs Dijkstra's algorithm and records the
/// shortest distance to every other zone's centroid. The result is a
/// [`DenseOdMatrix`](crate::od::dense::DenseOdMatrix) that can be
/// used as input to trip distribution (impedance) or mode choice
/// (travel time skims).
///
/// Zones whose centroid is unreachable from a given origin will have
/// a cost of `0.0` (the default for an unset OD pair).
///
/// # Arguments
/// * `network` - The macroscopic network.
/// * `link_costs` - Current link travel costs.
/// * `zone_ids` - Ordered list of zone IDs.
///
/// # Returns
/// A cost matrix as a `DenseOdMatrix`.
pub fn compute_skim_matrix(
    network: &Network,
    link_costs: &HashMap<LinkID, f64>,
    zone_ids: &[ZoneID],
) -> crate::od::dense::DenseOdMatrix {
    let mut skim = crate::od::dense::DenseOdMatrix::new(zone_ids.to_vec());

    for &origin_zone in zone_ids {
        let origin_node = match network.get_zone_centroid(origin_zone) {
            Ok(n) => n,
            Err(_) => continue,
        };

        let spt = dijkstra_one_to_all(network, origin_node, link_costs);

        for &dest_zone in zone_ids {
            if origin_zone == dest_zone {
                continue;
            }

            let dest_node = match network.get_zone_centroid(dest_zone) {
                Ok(n) => n,
                Err(_) => continue,
            };

            if let Some(&dist) = spt.distances.get(&dest_node) {
                skim.set(origin_zone, dest_zone, dist);
            }
        }
    }

    skim
}
