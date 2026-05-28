//! # Zone connectivity utilities
//!
//! Detects strongly-connected components (SCCs) among zone centroids
//! in the network graph. Used by the pipeline preflight check to catch
//! disconnected networks before Furness distribution is attempted.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::gmns::meso::network::Network;
use crate::gmns::types::ZoneID;
use crate::log_all;
use crate::verbose::EVENT_PREFLIGHT;
use crate::zone::Zone;

/// Partition zone centroids into strongly-connected components.
///
/// Returns a list of components, each a sorted list of zone IDs.
/// The list is sorted by the smallest zone ID in each component.
/// A single-element result means all zones are mutually reachable.
///
/// Two zones belong to the same component when a directed path exists
/// from each centroid to the other (strong connectivity). This is the
/// minimum requirement for Furness (IPF) to have a feasible solution.
pub fn zone_scc(network: &Network, zones: &[Zone]) -> Vec<Vec<ZoneID>> {
    let centroids: Vec<(ZoneID, i64)> = zones
        .iter()
        .filter_map(|z| network.get_zone_centroid(z.id).ok().map(|n| (z.id, n)))
        .collect();

    let n = centroids.len();
    if n <= 1 {
        return vec![centroids.iter().map(|(z, _)| *z).collect()];
    }

    let centroid_node_to_idx: HashMap<i64, usize> = centroids
        .iter()
        .enumerate()
        .map(|(i, (_, node))| (*node, i))
        .collect();

    // BFS from each centroid: collect which other centroid indices are reachable.
    let reachable: Vec<Vec<bool>> = centroids
        .iter()
        .map(|(zone_id, start)| {
            let mut visited: HashSet<i64> = HashSet::new();
            let mut queue: VecDeque<i64> = VecDeque::new();
            queue.push_back(*start);
            visited.insert(*start);
            let mut reach = vec![false; n];
            while let Some(node_id) = queue.pop_front() {
                if let Some(&idx) = centroid_node_to_idx.get(&node_id) {
                    reach[idx] = true;
                }
                if let Ok(link_ids) = network.outgoing_links(node_id) {
                    for &lid in link_ids {
                        if let Ok(link) = network.get_link(lid) {
                            let next = link.target_node_id;
                            if visited.insert(next) {
                                queue.push_back(next);
                            }
                        }
                    }
                }
            }
            let reachable_count = reach.iter().filter(|&&r| r).count();
            log_all!(
                EVENT_PREFLIGHT,
                "BFS from zone centroid",
                zone_id = zone_id,
                nodes_visited = visited.len(),
                centroids_reachable = reachable_count
            );
            reach
        })
        .collect();

    // Union-Find: merge i and j when i can reach j AND j can reach i.
    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank: Vec<usize> = vec![0; n];
    for i in 0..n {
        for j in (i + 1)..n {
            if reachable[i][j] && reachable[j][i] {
                uf_union(&mut parent, &mut rank, i, j);
            }
        }
    }

    let mut components: HashMap<usize, Vec<ZoneID>> = HashMap::new();
    for (i, (zone_id, _)) in centroids.iter().enumerate() {
        let root = uf_find(&mut parent, i);
        components.entry(root).or_default().push(*zone_id);
    }

    let mut result: Vec<Vec<ZoneID>> = components.into_values().collect();
    for comp in &mut result {
        comp.sort_unstable();
    }
    result.sort_unstable_by_key(|c| c[0]);
    result
}

fn uf_find(parent: &mut Vec<usize>, mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn uf_union(parent: &mut Vec<usize>, rank: &mut Vec<usize>, x: usize, y: usize) {
    let rx = uf_find(parent, x);
    let ry = uf_find(parent, y);
    if rx == ry {
        return;
    }
    match rank[rx].cmp(&rank[ry]) {
        std::cmp::Ordering::Less => parent[rx] = ry,
        std::cmp::Ordering::Greater => parent[ry] = rx,
        std::cmp::Ordering::Equal => {
            parent[ry] = rx;
            rank[rx] += 1;
        }
    }
}
