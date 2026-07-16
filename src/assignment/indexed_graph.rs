//! # Indexed Graph (CSR)
//!
//! Compact graph representation for fast shortest path and assignment.
//! All lookups are O(1) Vec indexing instead of HashMap.
//!
//! Built once from a [`Network`] before assignment starts.
//! All assignment internals (Dijkstra, AoN, cost computation,
//! volume updates) operate on dense indices and flat Vec arrays.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use ordered_float::OrderedFloat;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

use crate::gmns::meso::network::Network;
use crate::gmns::types::{LinkID, NodeID, ZoneID};

use super::od_path::OdPath;

/// Compact indexed graph in CSR (Compressed Sparse Row) format.
///
/// All node and link IDs are mapped to dense indices 0..N and 0..E.
/// Adjacency is stored as CSR: outgoing edges of node `u` are
/// `edges[row_ptr[u]..row_ptr[u+1]]`.
pub struct IndexedGraph {
    pub num_nodes: usize,
    pub num_links: usize,
    // Node mapping
    node_to_idx: HashMap<NodeID, usize>,
    // Link mapping
    link_to_idx: HashMap<LinkID, usize>,
    idx_to_link: Vec<LinkID>,
    // CSR adjacency
    row_ptr: Vec<usize>,
    // (target_node_idx, link_idx)
    edges: Vec<(usize, usize)>,
    // Link properties (indexed by link_idx)
    link_source_idx: Vec<usize>,
    pub link_ff_time: Vec<f64>,
    pub link_capacity: Vec<f64>,
    // Zone centroid mapping
    zone_to_node_idx: HashMap<ZoneID, usize>,
    zone_ids: Vec<ZoneID>,
}

impl IndexedGraph {
    /// Build an indexed graph from a Network.
    pub fn from_network(network: &Network) -> Self {
        // Build node index
        let mut node_to_idx: HashMap<NodeID, usize> = HashMap::with_capacity(network.nodes.len());
        let mut idx = 0;
        for &node_id in network.nodes.keys() {
            node_to_idx.insert(node_id, idx);
            idx += 1;
        }
        let num_nodes = idx;

        // Build link index and properties
        let num_links = network.links.len();
        let mut link_to_idx: HashMap<LinkID, usize> = HashMap::with_capacity(num_links);
        let mut idx_to_link: Vec<LinkID> = Vec::with_capacity(num_links);
        let mut link_source_idx: Vec<usize> = Vec::with_capacity(num_links);
        let mut link_ff_time: Vec<f64> = Vec::with_capacity(num_links);
        let mut link_capacity: Vec<f64> = Vec::with_capacity(num_links);

        let mut link_idx = 0;
        // Collect outgoing edge lists per node
        let mut out_edges: Vec<Vec<(usize, usize)>> = vec![Vec::new(); num_nodes];

        for (&link_id, link) in &network.links {
            link_to_idx.insert(link_id, link_idx);
            idx_to_link.push(link_id);
            link_ff_time.push(link.get_free_flow_time_hours());
            link_capacity.push(link.get_total_capacity());

            let src = node_to_idx[&link.source_node_id];
            let tgt = node_to_idx[&link.target_node_id];
            link_source_idx.push(src);
            out_edges[src].push((tgt, link_idx));

            link_idx += 1;
        }

        // Build CSR
        let mut row_ptr: Vec<usize> = Vec::with_capacity(num_nodes + 1);
        let mut edges: Vec<(usize, usize)> = Vec::with_capacity(num_links);
        let mut offset = 0;
        for node_edges in &out_edges {
            row_ptr.push(offset);
            edges.extend_from_slice(node_edges);
            offset += node_edges.len();
        }
        row_ptr.push(offset);

        // Zone centroid mapping
        let mut zone_to_node_idx: HashMap<ZoneID, usize> = HashMap::new();
        for (&zone_id, &node_id) in &network.zone_centroids {
            if let Some(&ni) = node_to_idx.get(&node_id) {
                zone_to_node_idx.insert(zone_id, ni);
            }
        }

        let mut zone_ids: Vec<ZoneID> = zone_to_node_idx.keys().copied().collect();
        zone_ids.sort();

        IndexedGraph {
            num_nodes,
            num_links,
            node_to_idx,
            link_to_idx,
            idx_to_link,
            row_ptr,
            edges,
            link_source_idx,
            link_ff_time,
            link_capacity,
            zone_to_node_idx,
            zone_ids,
        }
    }

    /// Get zone IDs.
    #[inline]
    pub fn zone_ids(&self) -> &[ZoneID] {
        &self.zone_ids
    }

    /// Get centroid node index for a zone.
    #[inline]
    pub fn zone_node_idx(&self, zone_id: ZoneID) -> Option<usize> {
        self.zone_to_node_idx.get(&zone_id).copied()
    }

    /// Get link index from link ID.
    #[inline]
    pub fn link_idx(&self, link_id: LinkID) -> Option<usize> {
        self.link_to_idx.get(&link_id).copied()
    }

    /// Get link ID from link index.
    #[inline]
    pub fn link_id(&self, idx: usize) -> LinkID {
        self.idx_to_link[idx]
    }

    /// Get source node index for a link.
    #[inline]
    pub fn link_source(&self, link_idx: usize) -> usize {
        self.link_source_idx[link_idx]
    }

    /// Run Dijkstra from a single origin node index.
    /// Returns (distances, predecessors) as flat Vecs.
    /// predecessors[v] = Some(link_idx) on the shortest path tree.
    pub fn dijkstra(
        &self,
        origin_idx: usize,
        link_costs: &[f64],
    ) -> (Vec<f64>, Vec<Option<usize>>) {
        let mut dist = vec![f64::INFINITY; self.num_nodes];
        let mut pred: Vec<Option<usize>> = vec![None; self.num_nodes];
        let mut visited = vec![false; self.num_nodes];
        self.dijkstra_into(origin_idx, link_costs, &mut dist, &mut pred, &mut visited);
        (dist, pred)
    }

    /// Run Dijkstra into pre-allocated buffers (avoids allocation per call).
    ///
    /// `dist`, `pred`, and `visited` must have length `num_nodes`.
    /// They are reset at the start, so prior contents do not matter.
    pub fn dijkstra_into(
        &self,
        origin_idx: usize,
        link_costs: &[f64],
        dist: &mut [f64],
        pred: &mut [Option<usize>],
        visited: &mut [bool],
    ) {
        dist.iter_mut().for_each(|d| *d = f64::INFINITY);
        pred.iter_mut().for_each(|p| *p = None);
        visited.iter_mut().for_each(|v| *v = false);
        dist[origin_idx] = 0.0;

        let mut heap: BinaryHeap<Reverse<(OrderedFloat<f64>, usize)>> = BinaryHeap::new();
        heap.push(Reverse((OrderedFloat(0.0), origin_idx)));

        while let Some(Reverse((OrderedFloat(d), u))) = heap.pop() {
            if visited[u] {
                continue;
            }
            visited[u] = true;
            let start = self.row_ptr[u];
            let end = self.row_ptr[u + 1];
            for &(v, li) in &self.edges[start..end] {
                if visited[v] {
                    continue;
                }
                let cost = link_costs[li];
                if !cost.is_finite() {
                    continue;
                }
                let new_dist = d + cost;
                if new_dist < dist[v] {
                    dist[v] = new_dist;
                    pred[v] = Some(li);
                    heap.push(Reverse((OrderedFloat(new_dist), v)));
                }
            }
        }
    }

    /// All-or-nothing assignment on the indexed graph.
    /// Writes volumes into the provided slice (must be num_links long).
    /// Zeros the slice first.
    pub fn all_or_nothing(
        &self,
        od_matrix: &dyn crate::od::OdMatrix,
        link_costs: &[f64],
        volumes: &mut [f64],
    ) {
        volumes.fill(0.0);
        let zone_ids = od_matrix.zone_ids();

        let zone_node_idxs: Vec<Option<usize>> =
            zone_ids.iter().map(|&z| self.zone_node_idx(z)).collect();

        let mut dist = vec![f64::INFINITY; self.num_nodes];
        let mut pred: Vec<Option<usize>> = vec![None; self.num_nodes];
        let mut visited = vec![false; self.num_nodes];

        for (oi, &origin_zone) in zone_ids.iter().enumerate() {
            let origin_idx = match zone_node_idxs[oi] {
                Some(i) => i,
                None => continue,
            };

            self.dijkstra_into(origin_idx, link_costs, &mut dist, &mut pred, &mut visited);

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

                let mut current = dest_idx;
                loop {
                    match pred[current] {
                        Some(li) => {
                            volumes[li] += demand;
                            current = self.link_source_idx[li];
                        }
                        None => break,
                    }
                }
            }
        }
    }

    /// All-or-nothing assignment that also builds per-OD shortest paths.
    ///
    /// Same as [`all_or_nothing`] but collects `OdPath` structs while
    /// walking predecessors -- no extra Dijkstra pass needed.
    /// `paths_out` is cleared and filled with one path per OD pair.
    pub fn all_or_nothing_with_paths(
        &self,
        od_matrix: &dyn crate::od::OdMatrix,
        link_costs: &[f64],
        volumes: &mut [f64],
        paths_out: &mut Vec<OdPath>,
    ) {
        volumes.fill(0.0);
        paths_out.clear();
        let zone_ids = od_matrix.zone_ids();

        let zone_node_idxs: Vec<Option<usize>> =
            zone_ids.iter().map(|&z| self.zone_node_idx(z)).collect();

        let mut dist = vec![f64::INFINITY; self.num_nodes];
        let mut pred: Vec<Option<usize>> = vec![None; self.num_nodes];
        let mut visited = vec![false; self.num_nodes];

        for (oi, &origin_zone) in zone_ids.iter().enumerate() {
            let origin_idx = match zone_node_idxs[oi] {
                Some(i) => i,
                None => continue,
            };

            self.dijkstra_into(origin_idx, link_costs, &mut dist, &mut pred, &mut visited);

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

                let mut link_indices = Vec::new();
                let mut current = dest_idx;
                loop {
                    match pred[current] {
                        Some(li) => {
                            volumes[li] += demand;
                            link_indices.push(li);
                            current = self.link_source_idx[li];
                        }
                        None => break,
                    }
                }
                link_indices.reverse();

                if link_indices.is_empty() {
                    continue;
                }

                let cost: f64 = link_indices.iter().map(|&li| link_costs[li]).sum();

                paths_out.push(OdPath {
                    origin_zone,
                    dest_zone,
                    path_index: 0,
                    flow: demand,
                    cost,
                    link_ids: link_indices.iter().map(|&li| self.idx_to_link[li]).collect(),
                    class_index: None,
                });
            }
        }
    }

    /// Parallel all-or-nothing assignment.
    /// Uses rayon fold+reduce: each worker thread reuses one volume
    /// buffer across multiple origin zones (O(num_threads * E) memory
    /// instead of O(Z * E)), then buffers are merged via pairwise
    /// summation in the reduce tree.
    #[cfg(feature = "parallel")]
    pub fn all_or_nothing_parallel(
        &self,
        od_matrix: &dyn crate::od::OdMatrix,
        link_costs: &[f64],
        volumes: &mut [f64],
    ) {
        let zone_ids = od_matrix.zone_ids();
        let zone_node_idxs: Vec<Option<usize>> =
            zone_ids.iter().map(|&z| self.zone_node_idx(z)).collect();
        let n = self.num_links;

        let merged = (0..zone_ids.len())
            .into_par_iter()
            .fold(
                || vec![0.0; n],
                |mut acc, oi| {
                    let origin_zone = zone_ids[oi];
                    let origin_idx = match zone_node_idxs[oi] {
                        Some(i) => i,
                        None => return acc,
                    };

                    let (dist, pred) = self.dijkstra(origin_idx, link_costs);
                    let _ = dist;

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

                        let mut current = dest_idx;
                        loop {
                            match pred[current] {
                                Some(li) => {
                                    acc[li] += demand;
                                    current = self.link_source_idx[li];
                                }
                                None => break,
                            }
                        }
                    }
                    acc
                },
            )
            .reduce(
                || vec![0.0; n],
                |mut a, b| {
                    for i in 0..n {
                        a[i] += b[i];
                    }
                    a
                },
            );

        volumes.copy_from_slice(&merged);
    }

    /// Compute link costs from volumes using the VDF.
    pub fn compute_costs(
        &self,
        volumes: &[f64],
        vdf: &dyn crate::assignment::VolumeDelayFunction,
        out: &mut [f64],
    ) {
        for i in 0..self.num_links {
            out[i] = vdf.travel_time(self.link_ff_time[i], volumes[i], self.link_capacity[i]);
        }
    }

    /// Compute relative gap.
    pub fn relative_gap(&self, volumes: &[f64], costs: &[f64], aux_volumes: &[f64]) -> f64 {
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        for i in 0..self.num_links {
            numerator += volumes[i] * costs[i];
            denominator += aux_volumes[i] * costs[i];
        }
        if numerator <= 0.0 {
            return 0.0;
        }
        (numerator - denominator) / numerator
    }

    /// Convert Vec-indexed volumes back to HashMap<LinkID, f64>.
    pub fn volumes_to_hashmap(&self, volumes: &[f64]) -> HashMap<LinkID, f64> {
        let mut map = HashMap::with_capacity(self.num_links);
        for i in 0..self.num_links {
            map.insert(self.idx_to_link[i], volumes[i]);
        }
        map
    }

    /// Convert Vec-indexed costs back to HashMap<LinkID, f64>.
    pub fn costs_to_hashmap(&self, costs: &[f64]) -> HashMap<LinkID, f64> {
        self.volumes_to_hashmap(costs)
    }

    /// Convert HashMap<LinkID, f64> to a flat Vec indexed by link index.
    ///
    /// Missing links default to 0.0. Used to import warm-start volumes
    /// from a previous [`AssignmentResult`](super::AssignmentResult).
    pub fn hashmap_to_vec(&self, map: &HashMap<LinkID, f64>) -> Vec<f64> {
        let mut out = vec![0.0; self.num_links];
        for (&link_id, &val) in map {
            if let Some(&idx) = self.link_to_idx.get(&link_id) {
                out[idx] = val;
            }
        }
        out
    }

    /// Compute skim matrix from shortest path distances.
    pub fn compute_skim(
        &self,
        link_costs: &[f64],
        zone_ids: &[ZoneID],
    ) -> crate::od::dense::DenseOdMatrix {
        let mut skim = crate::od::dense::DenseOdMatrix::new(zone_ids.to_vec());

        let zone_node_idxs: Vec<Option<usize>> =
            zone_ids.iter().map(|&z| self.zone_node_idx(z)).collect();

        let mut dist = vec![f64::INFINITY; self.num_nodes];
        let mut pred: Vec<Option<usize>> = vec![None; self.num_nodes];
        let mut visited = vec![false; self.num_nodes];

        for (i, _) in zone_ids.iter().enumerate() {
            let origin_idx = match zone_node_idxs[i] {
                Some(idx) => idx,
                None => continue,
            };

            self.dijkstra_into(origin_idx, link_costs, &mut dist, &mut pred, &mut visited);

            for j in 0..zone_ids.len() {
                if i == j {
                    continue;
                }
                let dest_idx = match zone_node_idxs[j] {
                    Some(idx) => idx,
                    None => continue,
                };
                let d = dist[dest_idx];
                if d.is_finite() {
                    skim.set_by_index(i, j, d);
                }
            }
        }

        skim
    }

    /// Extract one shortest path per OD pair from link costs.
    ///
    /// Runs Dijkstra from each origin zone and builds the shortest
    /// path to every destination with positive demand. Each path
    /// carries the full OD demand as flow (link-based methods do
    /// not track per-path flow distribution).
    ///
    /// Used by Frank-Wolfe and MSA when `store_paths` is true.
    pub fn extract_shortest_paths(
        &self,
        od_matrix: &dyn crate::od::OdMatrix,
        costs: &[f64],
    ) -> Vec<OdPath> {
        let zone_ids = od_matrix.zone_ids().to_vec();
        let mut result = Vec::new();

        let zone_node_idxs: Vec<Option<usize>> =
            zone_ids.iter().map(|&z| self.zone_node_idx(z)).collect();

        let mut dij_dist = vec![f64::INFINITY; self.num_nodes];
        let mut dij_pred: Vec<Option<usize>> = vec![None; self.num_nodes];
        let mut dij_visited = vec![false; self.num_nodes];

        for (oi, &origin_zone) in zone_ids.iter().enumerate() {
            let origin_idx = match zone_node_idxs[oi] {
                Some(i) => i,
                None => continue,
            };

            self.dijkstra_into(
                origin_idx,
                costs,
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

                if dij_dist[dest_idx] == f64::INFINITY {
                    continue;
                }

                let mut link_indices = Vec::new();
                let mut current = dest_idx;
                loop {
                    match dij_pred[current] {
                        Some(li) => {
                            link_indices.push(li);
                            current = self.link_source_idx[li];
                        }
                        None => break,
                    }
                }
                link_indices.reverse();

                if link_indices.is_empty() {
                    continue;
                }

                let cost: f64 = link_indices.iter().map(|&li| costs[li]).sum();

                result.push(OdPath {
                    origin_zone,
                    dest_zone,
                    path_index: 0,
                    flow: demand,
                    cost,
                    link_ids: link_indices.iter().map(|&li| self.idx_to_link[li]).collect(),
                    class_index: None,
                });
            }
        }

        result
    }

    /// Extract shortest paths for multi-class assignment.
    ///
    /// Under the Beckmann symmetry condition (`ff_time_multiplier / pcu
    /// = const`), per-class cost is `ff_time_multiplier_m * t_a(V_a)` - 
    /// a constant multiple of shared cost. Multiplying all edge
    /// weights by a positive constant does not change the shortest path
    /// tree, so all classes share the same SPT.
    ///
    /// This method exploits that property: Dijkstra runs once per
    /// origin on `shared_costs`, then the SPT is walked once per class
    /// to collect paths with class-specific demand and scaled cost.
    ///
    /// `od_matrices` and `ff_time_multipliers` must have the same
    /// length (one per class). Each path stores `class_index` matching
    /// the position in these slices.
    pub fn extract_shortest_paths_multiclass(
        &self,
        od_matrices: &[&dyn crate::od::OdMatrix],
        ff_time_multipliers: &[f64],
        shared_costs: &[f64],
    ) -> Vec<OdPath> {
        let m = od_matrices.len();
        let all_zone_ids = self.zone_ids().to_vec();
        let zone_node_idxs: Vec<Option<usize>> =
            all_zone_ids.iter().map(|&z| self.zone_node_idx(z)).collect();

        let mut result = Vec::new();
        let mut dij_dist = vec![f64::INFINITY; self.num_nodes];
        let mut dij_pred: Vec<Option<usize>> = vec![None; self.num_nodes];
        let mut dij_visited = vec![false; self.num_nodes];

        for (oi, &origin_zone) in all_zone_ids.iter().enumerate() {
            let origin_idx = match zone_node_idxs[oi] {
                Some(i) => i,
                None => continue,
            };

            self.dijkstra_into(
                origin_idx,
                shared_costs,
                &mut dij_dist,
                &mut dij_pred,
                &mut dij_visited,
            );

            for (di, &dest_zone) in all_zone_ids.iter().enumerate() {
                if oi == di {
                    continue;
                }

                let dest_idx = match zone_node_idxs[di] {
                    Some(i) => i,
                    None => continue,
                };

                if dij_dist[dest_idx] == f64::INFINITY {
                    continue;
                }

                let mut has_demand = false;
                for ci in 0..m {
                    if od_matrices[ci].get(origin_zone, dest_zone) > 0.0 {
                        has_demand = true;
                        break;
                    }
                }
                if !has_demand {
                    continue;
                }

                let mut link_indices = Vec::new();
                let mut current = dest_idx;
                loop {
                    match dij_pred[current] {
                        Some(li) => {
                            link_indices.push(li);
                            current = self.link_source_idx[li];
                        }
                        None => break,
                    }
                }
                link_indices.reverse();

                if link_indices.is_empty() {
                    continue;
                }

                let base_cost: f64 = link_indices.iter().map(|&li| shared_costs[li]).sum();
                let link_ids: Vec<LinkID> =
                    link_indices.iter().map(|&li| self.idx_to_link[li]).collect();

                for ci in 0..m {
                    let demand = od_matrices[ci].get(origin_zone, dest_zone);
                    if demand <= 0.0 {
                        continue;
                    }
                    result.push(OdPath {
                        origin_zone,
                        dest_zone,
                        path_index: 0,
                        flow: demand,
                        cost: base_cost * ff_time_multipliers[ci],
                        link_ids: link_ids.clone(),
                        class_index: Some(ci as u16),
                    });
                }
            }
        }

        result
    }

    /// Parallel skim matrix computation.
    /// Each origin zone runs Dijkstra independently, returns a row of
    /// distances. Rows are merged into the skim matrix after collection.
    #[cfg(feature = "parallel")]
    pub fn compute_skim_parallel(
        &self,
        link_costs: &[f64],
        zone_ids: &[ZoneID],
    ) -> crate::od::dense::DenseOdMatrix {
        let zone_node_idxs: Vec<Option<usize>> =
            zone_ids.iter().map(|&z| self.zone_node_idx(z)).collect();

        let rows: Vec<(usize, Vec<(usize, f64)>)> = (0..zone_ids.len())
            .into_par_iter()
            .filter_map(|i| {
                let origin_idx = zone_node_idxs[i]?;
                let (dist, _) = self.dijkstra(origin_idx, link_costs);

                let mut row = Vec::new();
                for j in 0..zone_ids.len() {
                    if i == j {
                        continue;
                    }
                    if let Some(dest_idx) = zone_node_idxs[j] {
                        let d = dist[dest_idx];
                        if d.is_finite() {
                            row.push((j, d));
                        }
                    }
                }
                Some((i, row))
            })
            .collect();

        let mut skim = crate::od::dense::DenseOdMatrix::new(zone_ids.to_vec());
        for (i, row) in rows {
            for (j, d) in row {
                skim.set_by_index(i, j, d);
            }
        }
        skim
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignment::BprFunction;
    use crate::gmns::meso::link::Link;
    use crate::gmns::meso::node::Node;
    use crate::od::OdMatrix;
    use crate::od::dense::DenseOdMatrix;

    fn two_link_network() -> IndexedGraph {
        let mut net = Network::new();
        net.add_node(Node::new(1).with_zone_id(1).with_coordinates(0.0, 0.0).build()).unwrap();
        net.add_node(Node::new(2).with_zone_id(2).with_coordinates(0.0, 1.0).build()).unwrap();
        net.add_link(
            Link::new(100, 1, 2).with_length_meters(1000.0).with_free_speed(60.0).with_capacity(1000.0).build(),
        ).unwrap();
        net.add_link(
            Link::new(101, 2, 1).with_length_meters(1000.0).with_free_speed(60.0).with_capacity(1000.0).build(),
        ).unwrap();
        IndexedGraph::from_network(&net)
    }

    fn diamond_network() -> IndexedGraph {
        let mut net = Network::new();
        net.add_node(Node::new(1).with_zone_id(1).with_coordinates(0.0, 0.0).build()).unwrap();
        net.add_node(Node::new(2).with_zone_id(2).with_coordinates(1.0, 0.0).build()).unwrap();
        net.add_node(Node::new(3).with_zone_id(3).with_coordinates(0.0, 1.0).build()).unwrap();
        net.add_node(Node::new(4).with_zone_id(4).with_coordinates(1.0, 1.0).build()).unwrap();
        net.add_link(Link::new(100, 1, 2).with_length_meters(1000.0).with_free_speed(60.0).with_capacity(1000.0).build()).unwrap();
        net.add_link(Link::new(101, 1, 3).with_length_meters(2000.0).with_free_speed(60.0).with_capacity(1000.0).build()).unwrap();
        net.add_link(Link::new(102, 2, 4).with_length_meters(1000.0).with_free_speed(60.0).with_capacity(1000.0).build()).unwrap();
        net.add_link(Link::new(103, 3, 4).with_length_meters(1000.0).with_free_speed(60.0).with_capacity(1000.0).build()).unwrap();
        IndexedGraph::from_network(&net)
    }

    fn free_flow_costs(graph: &IndexedGraph) -> Vec<f64> {
        let bpr = BprFunction::default();
        let mut costs = vec![0.0; graph.num_links];
        graph.compute_costs(&vec![0.0; graph.num_links], &bpr, &mut costs);
        costs
    }

    #[test]
    fn extract_paths_one_per_od_pair() {
        let graph = two_link_network();
        let costs = free_flow_costs(&graph);

        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 500.0);
        od.set(2, 1, 300.0);

        let paths = graph.extract_shortest_paths(&od, &costs);

        assert_eq!(paths.len(), 2);

        let p12: Vec<_> = paths.iter().filter(|p| p.origin_zone == 1 && p.dest_zone == 2).collect();
        assert_eq!(p12.len(), 1);
        assert!((p12[0].flow - 500.0).abs() < 1e-10);
        assert_eq!(p12[0].path_index, 0);
        assert_eq!(p12[0].link_ids, vec![100]);

        let p21: Vec<_> = paths.iter().filter(|p| p.origin_zone == 2 && p.dest_zone == 1).collect();
        assert_eq!(p21.len(), 1);
        assert!((p21[0].flow - 300.0).abs() < 1e-10);
        assert_eq!(p21[0].link_ids, vec![101]);
    }

    #[test]
    fn extract_paths_zero_demand_skipped() {
        let graph = two_link_network();
        let costs = free_flow_costs(&graph);

        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 500.0);

        let paths = graph.extract_shortest_paths(&od, &costs);

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].origin_zone, 1);
        assert_eq!(paths[0].dest_zone, 2);
    }

    #[test]
    fn extract_paths_multi_link() {
        let graph = diamond_network();
        let costs = free_flow_costs(&graph);

        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 100.0);

        let paths = graph.extract_shortest_paths(&od, &costs);

        assert_eq!(paths.len(), 1);
        let p = &paths[0];
        assert_eq!(p.origin_zone, 1);
        assert_eq!(p.dest_zone, 4);
        assert!((p.flow - 100.0).abs() < 1e-10);
        assert_eq!(p.link_ids, vec![100, 102]);
        assert!(p.cost > 0.0);
    }

    #[test]
    fn extract_paths_cost_equals_sum_of_link_costs() {
        let graph = diamond_network();
        let costs = free_flow_costs(&graph);

        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 100.0);

        let paths = graph.extract_shortest_paths(&od, &costs);

        let p = &paths[0];
        let expected_cost: f64 = p.link_ids.iter()
            .map(|&lid| {
                let idx = graph.link_idx(lid).unwrap();
                costs[idx]
            })
            .sum();
        assert!((p.cost - expected_cost).abs() < 1e-10);
    }

    #[test]
    fn extract_paths_picks_shorter_route() {
        let graph = diamond_network();
        let costs = free_flow_costs(&graph);

        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 100.0);

        let paths = graph.extract_shortest_paths(&od, &costs);

        let p = &paths[0];
        assert_eq!(p.link_ids, vec![100, 102]);
    }
}
