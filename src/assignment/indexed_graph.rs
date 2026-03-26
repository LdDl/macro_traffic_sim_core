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

use crate::gmns::meso::network::Network;
use crate::gmns::types::{LinkID, NodeID, ZoneID};

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
        dist[origin_idx] = 0.0;

        let mut heap: BinaryHeap<Reverse<(OrderedFloat<f64>, usize)>> = BinaryHeap::new();
        heap.push(Reverse((OrderedFloat(0.0), origin_idx)));

        while let Some(Reverse((OrderedFloat(d), u))) = heap.pop() {
            if d > dist[u] {
                continue;
            }
            let start = self.row_ptr[u];
            let end = self.row_ptr[u + 1];
            for &(v, li) in &self.edges[start..end] {
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
        (dist, pred)
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

        for &origin_zone in zone_ids {
            let origin_idx = match self.zone_node_idx(origin_zone) {
                Some(i) => i,
                None => continue,
            };

            let (_, pred) = self.dijkstra(origin_idx, link_costs);

            for &dest_zone in zone_ids {
                if origin_zone == dest_zone {
                    continue;
                }
                let demand = od_matrix.get(origin_zone, dest_zone);
                if demand <= 0.0 {
                    continue;
                }
                let dest_idx = match self.zone_node_idx(dest_zone) {
                    Some(i) => i,
                    None => continue,
                };

                // Walk predecessors, accumulate demand
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
    pub fn relative_gap(
        &self,
        volumes: &[f64],
        costs: &[f64],
        aux_volumes: &[f64],
    ) -> f64 {
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

    /// Compute skim matrix from shortest path distances.
    pub fn compute_skim(
        &self,
        link_costs: &[f64],
        zone_ids: &[ZoneID],
    ) -> crate::od::dense::DenseOdMatrix {
        let mut skim = crate::od::dense::DenseOdMatrix::new(zone_ids.to_vec());
        let n = zone_ids.len();

        for (i, &oz) in zone_ids.iter().enumerate() {
            let origin_idx = match self.zone_node_idx(oz) {
                Some(idx) => idx,
                None => continue,
            };

            let (dist, _) = self.dijkstra(origin_idx, link_costs);

            for (j, &dz) in zone_ids.iter().enumerate() {
                if i == j {
                    continue;
                }
                let dest_idx = match self.zone_node_idx(dz) {
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
}
