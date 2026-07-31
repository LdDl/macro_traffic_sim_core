//! # Transit Assignment (Optimal Strategies)
//!
//! Frequency-based transit assignment using the Spiess & Florian (1989)
//! optimal strategies algorithm (DOI: <https://doi.org/10.1016/0191-2615(89)90034-9>),
//! provided by the [`hyperpaths-rs`](https://crates.io/crates/hyperpaths-rs) crate.
//!
//! The transit network of routes is expanded into a route graph following
//! the construction from the paper:
//!
//! - each (route, stop) pair becomes a route node;
//! - a boarding link connects the stop to the route node with the route
//!   headway (waiting) and zero cost;
//! - an alighting link connects the route node back to the stop with zero
//!   cost and no waiting;
//! - a riding link connects consecutive route nodes with the segment travel
//!   time and no waiting (passengers already on board do not wait);
//! - walk links connect stops directly with their walking time and no
//!   waiting.
//!
//! Links marked "no waiting" have infinite frequency; the solver handles
//! them exactly via the paper's modified algorithm (p. 96), so labels
//! carry no big-M artifacts.
//!
//! The algorithm runs once per destination zone with demand: phase 1 builds
//! the optimal strategy (hyperpath), phase 2 loads the OD column onto the
//! attractive links. Volumes are accumulated over all destinations.

use std::collections::{HashMap, HashSet};

use hyperpaths_rs::{Link, compute_sf};

use crate::od::OdMatrix;
use crate::transit::error::TransitError;
use crate::transit::route::TransitNetwork;

/// The role of a link in the expanded route graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitLinkKind {
    /// Stop -> route node: waiting for a vehicle (frequency = 1/headway)
    Boarding,
    /// Route node -> stop: leaving a vehicle (no cost, no waiting)
    Alighting,
    /// Route node -> route node: in-vehicle travel along one segment
    Riding,
    /// Stop -> stop: walking transfer
    Walking,
}

/// Assigned volume on one link of the expanded route graph.
#[derive(Debug, Clone)]
pub struct TransitLinkVolume {
    /// Link role in the route graph
    pub kind: TransitLinkKind,
    /// Owning route id; `None` for walk links
    pub route_id: Option<String>,
    /// Physical source stop. For boarding/alighting links this equals `to_stop`.
    pub from_stop: i64,
    /// Physical target stop
    pub to_stop: i64,
    /// Assigned passenger volume
    pub volume: f64,
}

/// Result of a transit assignment run.
#[derive(Debug, Clone)]
pub struct TransitAssignmentResult {
    /// Volumes on all links of the expanded route graph, aggregated over
    /// all destinations, in deterministic construction order
    pub link_volumes: Vec<TransitLinkVolume>,
    /// Expected travel time (waiting + in-vehicle + walking) per OD pair
    /// with positive demand
    pub od_costs: HashMap<(i64, i64), f64>,
    /// Total boardings per route, aggregated over all destinations
    pub route_boardings: HashMap<String, f64>,
}

/// Metadata of one expanded link, parallel to the hyperpath link list.
struct LinkMeta {
    kind: TransitLinkKind,
    route_id: Option<String>,
    from_stop: i64,
    to_stop: i64,
}

/// The expanded route graph in hyperpath form.
struct RouteGraph {
    links: Vec<Link>,
    meta: Vec<LinkMeta>,
    /// All node names (stops and route nodes)
    nodes: HashSet<String>,
    /// Physical stops (zone candidates)
    stops: HashSet<i64>,
    /// (from_name, to_name) -> index into links/meta
    index: HashMap<(String, String), usize>,
}

fn stop_name(stop: i64) -> String {
    stop.to_string()
}

fn route_node_name(route_id: &str, seq: usize) -> String {
    format!("{}#{}", route_id, seq)
}

fn expand_route_graph(network: &TransitNetwork) -> RouteGraph {
    let mut graph = RouteGraph {
        links: Vec::new(),
        meta: Vec::new(),
        nodes: HashSet::new(),
        stops: network.stop_ids(),
        index: HashMap::new(),
    };

    for &stop in &graph.stops {
        graph.nodes.insert(stop_name(stop));
    }

    let push = |graph: &mut RouteGraph, link: Link, meta: LinkMeta| {
        let key = (link.from_node.clone(), link.to_node.clone());
        let idx = graph.links.len();
        graph.links.push(link);
        graph.meta.push(meta);
        graph.index.insert(key, idx);
    };

    for route in &network.routes {
        let last = route.stops.len() - 1;
        for (seq, &stop) in route.stops.iter().enumerate() {
            let node = route_node_name(&route.id, seq);
            graph.nodes.insert(node.clone());

            // Boarding at every stop except the last one
            if seq < last {
                push(
                    &mut graph,
                    Link::new(&stop_name(stop), &node, &route.id, 0.0, route.headway),
                    LinkMeta {
                        kind: TransitLinkKind::Boarding,
                        route_id: Some(route.id.clone()),
                        from_stop: stop,
                        to_stop: stop,
                    },
                );
            }

            // Alighting at every stop except the first one
            if seq > 0 {
                push(
                    &mut graph,
                    Link::new(&node, &stop_name(stop), &route.id, 0.0, 0.0),
                    LinkMeta {
                        kind: TransitLinkKind::Alighting,
                        route_id: Some(route.id.clone()),
                        from_stop: stop,
                        to_stop: stop,
                    },
                );
            }

            // Riding to the next stop
            if seq < last {
                let next_node = route_node_name(&route.id, seq + 1);
                push(
                    &mut graph,
                    Link::new(&node, &next_node, &route.id, route.segment_times[seq], 0.0),
                    LinkMeta {
                        kind: TransitLinkKind::Riding,
                        route_id: Some(route.id.clone()),
                        from_stop: stop,
                        to_stop: route.stops[seq + 1],
                    },
                );
            }
        }
    }

    for walk in &network.walk_links {
        push(
            &mut graph,
            Link::new(
                &stop_name(walk.from_stop),
                &stop_name(walk.to_stop),
                "walk",
                walk.time,
                0.0,
            ),
            LinkMeta {
                kind: TransitLinkKind::Walking,
                route_id: None,
                from_stop: walk.from_stop,
                to_stop: walk.to_stop,
            },
        );
    }

    graph
}

/// Runs frequency-based transit assignment with optimal strategies
/// (Spiess & Florian, 1989).
///
/// The OD matrix is interpreted as transit trips between stops: every zone
/// ID with demand must be a stop of some route or walk link. For each
/// destination zone the optimal strategy is computed and the demand column
/// is loaded onto the attractive links; volumes are aggregated over all
/// destinations.
///
/// # Arguments
///
/// * `network` - Transit routes and walk links
/// * `od` - Transit OD matrix; zone IDs must be stop node IDs
///
/// # Errors
///
/// Returns a [`TransitError`] when the network is structurally invalid,
/// when a demand zone is not a stop, or when an OD pair with positive
/// demand has no transit path.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
/// use macro_traffic_sim_core::transit::{assign_transit, TransitNetwork, TransitRoute};
///
/// // Single line between two stops, 10 minute ride, 5 minute headway
/// let mut network = TransitNetwork::new();
/// network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
///
/// let mut od = DenseOdMatrix::new(vec![1, 2]);
/// od.set(1, 2, 100.0);
///
/// let result = assign_transit(&network, &od).unwrap();
/// // expected travel time = 5 min wait (1/frequency) + 10 min ride
/// assert!((result.od_costs[&(1, 2)] - 15.0).abs() < 1e-6);
/// assert!((result.route_boardings["L1"] - 100.0).abs() < 1e-6);
/// ```
pub fn assign_transit(
    network: &TransitNetwork,
    od: &dyn OdMatrix,
) -> Result<TransitAssignmentResult, TransitError> {
    network.validate()?;
    let graph = expand_route_graph(network);

    let mut volumes: Vec<f64> = vec![0.0; graph.links.len()];
    let mut od_costs: HashMap<(i64, i64), f64> = HashMap::new();

    let zone_ids = od.zone_ids().to_vec();
    for &destination in &zone_ids {
        // Collect the demand column of this destination
        let mut origins: Vec<(i64, f64)> = Vec::new();
        for &origin in &zone_ids {
            if origin == destination {
                continue;
            }
            let demand = od.get(origin, destination);
            if demand > 0.0 {
                origins.push((origin, demand));
            }
        }
        if origins.is_empty() {
            continue;
        }

        if !graph.stops.contains(&destination) {
            return Err(TransitError::UnknownStop { zone: destination });
        }
        for &(origin, _) in &origins {
            if !graph.stops.contains(&origin) {
                return Err(TransitError::UnknownStop { zone: origin });
            }
        }

        let destination_name = stop_name(destination);
        let mut trips: HashMap<String, HashMap<String, f64>> = HashMap::new();
        for &(origin, demand) in &origins {
            trips
                .entry(stop_name(origin))
                .or_default()
                .insert(destination_name.clone(), demand);
        }

        let result = compute_sf(&graph.links, &graph.nodes, &destination_name, &trips);

        for &(origin, _) in &origins {
            let label = result
                .strategy
                .labels
                .get(&stop_name(origin))
                .copied()
                .unwrap_or(f64::INFINITY);
            if !label.is_finite() {
                return Err(TransitError::Unreachable {
                    origin,
                    destination,
                });
            }
            od_costs.insert((origin, destination), label);
        }

        for (from_name, to_map) in &result.volumes.links {
            for (to_name, volume) in to_map {
                if *volume == 0.0 {
                    continue;
                }
                if let Some(&idx) = graph.index.get(&(from_name.clone(), to_name.clone())) {
                    volumes[idx] += volume;
                }
            }
        }
    }

    let mut route_boardings: HashMap<String, f64> = HashMap::new();
    let mut link_volumes: Vec<TransitLinkVolume> = Vec::with_capacity(graph.meta.len());
    for (idx, meta) in graph.meta.iter().enumerate() {
        if meta.kind == TransitLinkKind::Boarding {
            if let Some(route_id) = &meta.route_id {
                *route_boardings.entry(route_id.clone()).or_insert(0.0) += volumes[idx];
            }
        }
        link_volumes.push(TransitLinkVolume {
            kind: meta.kind,
            route_id: meta.route_id.clone(),
            from_stop: meta.from_stop,
            to_stop: meta.to_stop,
            volume: volumes[idx],
        });
    }

    Ok(TransitAssignmentResult {
        link_volumes,
        od_costs,
        route_boardings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::od::DenseOdMatrix;
    use crate::transit::route::TransitRoute;

    /// The example network from Spiess & Florian (1989), pages 96-97.
    /// Stops: A=1, X=2, Y=3, B=4.
    fn paper_network() -> TransitNetwork {
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 4], vec![25.0], 6.0));
        network.add_route(TransitRoute::new("L2", vec![1, 2, 3], vec![7.0, 6.0], 6.0));
        network.add_route(TransitRoute::new("L3", vec![2, 3, 4], vec![4.0, 4.0], 15.0));
        network.add_route(TransitRoute::new("L4", vec![3, 4], vec![10.0], 3.0));
        network
    }

    fn riding_volume(result: &TransitAssignmentResult, route: &str, from: i64, to: i64) -> f64 {
        result
            .link_volumes
            .iter()
            .find(|lv| {
                lv.kind == TransitLinkKind::Riding
                    && lv.route_id.as_deref() == Some(route)
                    && lv.from_stop == from
                    && lv.to_stop == to
            })
            .map(|lv| lv.volume)
            .unwrap_or(0.0)
    }

    #[test]
    fn test_paper_example() {
        let network = paper_network();
        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 1.0);

        let result = assign_transit(&network, &od).unwrap();

        const EPS: f64 = 1e-9;

        // Expected strategy cost from the paper: u_A = 27.75 minutes
        assert!(
            (result.od_costs[&(1, 4)] - 27.75).abs() <= EPS,
            "u_A = {}, want 27.75",
            result.od_costs[&(1, 4)]
        );

        // Riding volumes from the paper solution
        assert!((riding_volume(&result, "L1", 1, 4) - 0.5).abs() <= EPS);
        assert!((riding_volume(&result, "L2", 1, 2) - 0.5).abs() <= EPS);
        assert!((riding_volume(&result, "L2", 2, 3) - 0.5).abs() <= EPS);
        assert!((riding_volume(&result, "L4", 3, 4) - 5.0 / 12.0).abs() <= EPS);
        assert!((riding_volume(&result, "L3", 3, 4) - 1.0 / 12.0).abs() <= EPS);
        assert!((riding_volume(&result, "L3", 2, 3) - 0.0).abs() <= EPS);

        // Boardings: 0.5 on L1 and L2 at A, 5/12 on L4 and 1/12 on L3 at Y
        assert!((result.route_boardings["L1"] - 0.5).abs() <= EPS);
        assert!((result.route_boardings["L2"] - 0.5).abs() <= EPS);
        assert!((result.route_boardings["L4"] - 5.0 / 12.0).abs() <= EPS);
        assert!((result.route_boardings["L3"] - 1.0 / 12.0).abs() <= EPS);
    }

    #[test]
    fn test_multiple_destinations_accumulate() {
        let network = paper_network();
        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 1.0);
        od.set(2, 4, 1.0);

        let result = assign_transit(&network, &od).unwrap();

        // The 1->4 flows are unchanged; the 2->4 trip adds its own volumes
        assert!(result.od_costs[&(1, 4)] > 0.0);
        assert!(result.od_costs[&(2, 4)] > 0.0);
        // Expected time from X in the paper: u_X = 19.0714... minutes
        assert!((result.od_costs[&(2, 4)] - 19.071428571428573).abs() <= 1e-9);
    }

    #[test]
    fn test_walk_link() {
        // Two stops connected only by walking
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
        network.add_walk_link(2, 3, 4.0);

        let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
        od.set(1, 3, 100.0);

        let result = assign_transit(&network, &od).unwrap();

        // 5 wait + 10 ride + 4 walk
        assert!((result.od_costs[&(1, 3)] - 19.0).abs() < 1e-6);
        let walk = result
            .link_volumes
            .iter()
            .find(|lv| lv.kind == TransitLinkKind::Walking)
            .unwrap();
        assert!((walk.volume - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_validation_errors() {
        let mut network = TransitNetwork::new();
        assert!(matches!(
            assign_transit(&network, &DenseOdMatrix::new(vec![1])),
            Err(TransitError::EmptyNetwork)
        ));

        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0, 20.0], 5.0));
        assert!(matches!(
            assign_transit(&network, &DenseOdMatrix::new(vec![1])),
            Err(TransitError::SegmentTimesMismatch { .. })
        ));

        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 0.0));
        assert!(matches!(
            assign_transit(&network, &DenseOdMatrix::new(vec![1])),
            Err(TransitError::NonPositiveHeadway { .. })
        ));
    }

    #[test]
    fn test_unreachable() {
        // Route goes 1 -> 2, but demand asks for 2 -> 1
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));

        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(2, 1, 50.0);

        assert!(matches!(
            assign_transit(&network, &od),
            Err(TransitError::Unreachable {
                origin: 2,
                destination: 1
            })
        ));
    }

    #[test]
    fn test_unknown_stop() {
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));

        let mut od = DenseOdMatrix::new(vec![1, 99]);
        od.set(1, 99, 50.0);

        assert!(matches!(
            assign_transit(&network, &od),
            Err(TransitError::UnknownStop { zone: 99 })
        ));
    }
}
