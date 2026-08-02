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
    /// Arrival node -> departure node: in-vehicle dwell at a stop.
    /// Only present when the two-node stop scheme is active
    /// (`dwell_time > 0`).
    Dwell,
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

/// Arrival node name in the two-node stop scheme (vehicle arrives here).
fn arrival_node_name(route_id: &str, seq: usize) -> String {
    format!("{}#{}a", route_id, seq)
}

/// Departure node name in the two-node stop scheme (vehicle departs here).
fn departure_node_name(route_id: &str, seq: usize) -> String {
    format!("{}#{}d", route_id, seq)
}

fn expand_route_graph(network: &TransitNetwork, options: &TransitAssignmentOptions) -> RouteGraph {
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

    let wait_of = |headway: f64| options.wait_factor * headway;
    for route in &network.routes {
        let last = route.stops.len() - 1;
        if options.dwell_time > 0.0 {
            // Two-node stop scheme: each stop on the route splits into an
            // arrival node (vehicle arrives, through riders and alighters
            // are here) and a departure node (vehicle departs, boarders
            // join here), with a dwell link between them. A through rider
            // pays the full dwell at every intermediate stop; a boarding
            // rider pays half of it (they board during the dwell), so the
            // boarding link carries `boarding_penalty + 0.5 * dwell`.
            let half_dwell = 0.5 * options.dwell_time;
            for (seq, &stop) in route.stops.iter().enumerate() {
                let arrival = arrival_node_name(&route.id, seq);
                let departure = departure_node_name(&route.id, seq);
                if seq > 0 {
                    graph.nodes.insert(arrival.clone());
                }
                if seq < last {
                    graph.nodes.insert(departure.clone());
                }

                // Boarding: stop -> departure node
                if seq < last {
                    push(
                        &mut graph,
                        Link::new(
                            &stop_name(stop),
                            &departure,
                            &route.id,
                            options.boarding_penalty + half_dwell,
                            wait_of(route.headway),
                        ),
                        LinkMeta {
                            kind: TransitLinkKind::Boarding,
                            route_id: Some(route.id.clone()),
                            from_stop: stop,
                            to_stop: stop,
                        },
                    );
                }

                // Alighting: arrival node -> stop
                if seq > 0 {
                    push(
                        &mut graph,
                        Link::new(
                            &arrival,
                            &stop_name(stop),
                            &route.id,
                            options.alighting_penalty,
                            0.0,
                        ),
                        LinkMeta {
                            kind: TransitLinkKind::Alighting,
                            route_id: Some(route.id.clone()),
                            from_stop: stop,
                            to_stop: stop,
                        },
                    );
                }

                // Dwell: arrival node -> departure node (through riders)
                if seq > 0 && seq < last {
                    push(
                        &mut graph,
                        Link::new(&arrival, &departure, &route.id, options.dwell_time, 0.0),
                        LinkMeta {
                            kind: TransitLinkKind::Dwell,
                            route_id: Some(route.id.clone()),
                            from_stop: stop,
                            to_stop: stop,
                        },
                    );
                }

                // Riding: departure node -> next arrival node
                if seq < last {
                    let next_arrival = arrival_node_name(&route.id, seq + 1);
                    push(
                        &mut graph,
                        Link::new(
                            &departure,
                            &next_arrival,
                            &route.id,
                            route.segment_times[seq],
                            0.0,
                        ),
                        LinkMeta {
                            kind: TransitLinkKind::Riding,
                            route_id: Some(route.id.clone()),
                            from_stop: stop,
                            to_stop: route.stops[seq + 1],
                        },
                    );
                }
            }
        } else {
            // One-node stop scheme (dwell = 0): the plain Spiess-Florian
            // construction, one route node per stop.
            for (seq, &stop) in route.stops.iter().enumerate() {
                let node = route_node_name(&route.id, seq);
                graph.nodes.insert(node.clone());

                // Boarding at every stop except the last one. The waiting
                // time factor scales the effective headway: expected wait =
                // wait_factor * headway. Since the solver derives freq =
                // 1/headway and expected wait = 1/freq (alpha = 1 in the
                // Spiess-Florian label), feeding an effective headway of
                // wait_factor * headway is exactly the paper's WLOG (without
                // loss of generality) frequency scaling (p. 91): all boarding
                // links at a stop are scaled by the same factor, so the
                // line-choice proportions f_a/f_i are unchanged and only the
                // waiting term moves. The boarding penalty is a plain cost on
                // the same link (charged once per boarding, hence per transfer).
                if seq < last {
                    push(
                        &mut graph,
                        Link::new(
                            &stop_name(stop),
                            &node,
                            &route.id,
                            options.boarding_penalty,
                            wait_of(route.headway),
                        ),
                        LinkMeta {
                            kind: TransitLinkKind::Boarding,
                            route_id: Some(route.id.clone()),
                            from_stop: stop,
                            to_stop: stop,
                        },
                    );
                }

                // Alighting at every stop except the first one. Still a
                // no-wait link (headway 0); the alighting penalty is a plain
                // cost on it.
                if seq > 0 {
                    push(
                        &mut graph,
                        Link::new(
                            &node,
                            &stop_name(stop),
                            &route.id,
                            options.alighting_penalty,
                            0.0,
                        ),
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

/// Options controlling the transit assignment.
#[derive(Debug, Clone, Copy)]
pub struct TransitAssignmentOptions {
    /// Waiting time factor (the `alpha` of Spiess & Florian, p. 91):
    /// expected wait at a stop = `wait_factor * headway` for a single
    /// line, `wait_factor / combined_frequency` for a set of attractive
    /// lines.
    ///
    /// - `1.0` (default): exponentially distributed vehicle arrivals with
    ///   a uniform passenger arrival rate; the value used in the paper's
    ///   own worked example.
    /// - `0.5`: constant vehicle interarrival times, i.e. the passenger
    ///   waits on average half the headway - a common practical choice.
    ///
    /// Must be strictly positive.
    pub wait_factor: f64,

    /// Penalty added to every boarding, in the same time units as the
    /// segment travel times (a perceived cost of boarding a vehicle). It
    /// is charged once per boarding, so it also acts as a transfer
    /// penalty: any strategy that boards a second line pays it again,
    /// which discourages unnecessary transfers.
    ///
    /// Default `0.0`. Must be non-negative.
    pub boarding_penalty: f64,

    /// Penalty added to every alighting, in the same time units as the
    /// segment travel times (a perceived cost of leaving a vehicle,
    /// including the final one at the destination).
    ///
    /// Default `0.0`. Must be non-negative.
    pub alighting_penalty: f64,

    /// In-vehicle dwell time spent standing at a stop, in the same time
    /// units as the segment travel times. A through passenger (staying on
    /// board) pays the full dwell at every intermediate stop; a boarding
    /// passenger pays half of it on average (they board during the dwell).
    ///
    /// A positive dwell switches the route expansion to a two-node stop
    /// scheme (separate arrival and departure nodes with a dwell link
    /// between them). With the default `0.0` the one-node scheme is used,
    /// identical to the plain Spiess-Florian construction.
    ///
    /// Default `0.0`. Must be non-negative.
    pub dwell_time: f64,
}

impl Default for TransitAssignmentOptions {
    fn default() -> Self {
        Self {
            wait_factor: 1.0,
            boarding_penalty: 0.0,
            alighting_penalty: 0.0,
            dwell_time: 0.0,
        }
    }
}

/// Runs frequency-based transit assignment with optimal strategies
/// (Spiess & Florian, 1989), using default options
/// ([`TransitAssignmentOptions::default`], `wait_factor = 1.0`).
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
    assign_transit_with_options(network, od, &TransitAssignmentOptions::default())
}

/// Runs frequency-based transit assignment with explicit options.
///
/// Identical to [`assign_transit`] but lets the caller set the waiting
/// time factor (see [`TransitAssignmentOptions`]).
///
/// # Arguments
///
/// * `network` - Transit routes and walk links
/// * `od` - Transit OD matrix; zone IDs must be stop node IDs
/// * `options` - Assignment options (waiting time factor)
///
/// # Errors
///
/// Returns a [`TransitError`] when the network is structurally invalid,
/// when `options.wait_factor` is not strictly positive, when a demand zone
/// is not a stop, or when an OD pair with positive demand has no transit
/// path.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
/// use macro_traffic_sim_core::transit::{
///     assign_transit_with_options, TransitAssignmentOptions, TransitNetwork, TransitRoute,
/// };
///
/// // Single line between two stops, 10 minute ride, 5 minute headway
/// let mut network = TransitNetwork::new();
/// network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
///
/// let mut od = DenseOdMatrix::new(vec![1, 2]);
/// od.set(1, 2, 100.0);
///
/// // wait_factor 0.5 halves the waiting time: 2.5 min wait + 10 min ride
/// let options = TransitAssignmentOptions { wait_factor: 0.5, ..Default::default() };
/// let result = assign_transit_with_options(&network, &od, &options).unwrap();
/// assert!((result.od_costs[&(1, 2)] - 12.5).abs() < 1e-6);
/// ```
pub fn assign_transit_with_options(
    network: &TransitNetwork,
    od: &dyn OdMatrix,
    options: &TransitAssignmentOptions,
) -> Result<TransitAssignmentResult, TransitError> {
    network.validate()?;
    // Reject non-positive and NaN factors (NaN fails every comparison,
    // so test for a valid value rather than an invalid one).
    if options.wait_factor.is_nan() || options.wait_factor <= 0.0 {
        return Err(TransitError::InvalidWaitFactor {
            wait_factor: options.wait_factor,
        });
    }
    if options.boarding_penalty.is_nan() || options.boarding_penalty < 0.0 {
        return Err(TransitError::InvalidPenalty {
            name: "boarding_penalty",
            value: options.boarding_penalty,
        });
    }
    if options.alighting_penalty.is_nan() || options.alighting_penalty < 0.0 {
        return Err(TransitError::InvalidPenalty {
            name: "alighting_penalty",
            value: options.alighting_penalty,
        });
    }
    if options.dwell_time.is_nan() || options.dwell_time < 0.0 {
        return Err(TransitError::InvalidPenalty {
            name: "dwell_time",
            value: options.dwell_time,
        });
    }
    let graph = expand_route_graph(network, options);

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

    /// Tolerance for exact-value comparisons in the assignment tests.
    const EPS: f64 = 1e-9;

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
        assert!((result.od_costs[&(2, 4)] - 19.071428571428573).abs() <= EPS);
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
        assert!((result.od_costs[&(1, 3)] - 19.0).abs() < EPS);
        let walk = result
            .link_volumes
            .iter()
            .find(|lv| lv.kind == TransitLinkKind::Walking)
            .unwrap();
        assert!((walk.volume - 100.0).abs() < EPS);
    }

    #[test]
    fn test_wait_factor_scales_only_waiting() {
        // Single line: 10 min ride, 6 min headway. Default wait_factor 1.0
        // gives wait = headway = 6, so cost = 16. wait_factor 0.5 halves
        // only the waiting term: wait = 3, ride unchanged, cost = 13.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 100.0);

        let default = assign_transit(&network, &od).unwrap();
        assert!((default.od_costs[&(1, 2)] - 16.0).abs() < EPS);

        let half = assign_transit_with_options(
            &network,
            &od,
            &TransitAssignmentOptions {
                wait_factor: 0.5,
                ..Default::default()
            },
        )
        .unwrap();
        assert!((half.od_costs[&(1, 2)] - 13.0).abs() < EPS);
        // The line still carries the whole demand: waiting scaling does
        // not change the loading of a single-line strategy.
        assert!((half.route_boardings["L1"] - 100.0).abs() < EPS);
    }

    #[test]
    fn test_wait_factor_preserves_line_split() {
        // Two competing lines at stop 1 to destination 2: same segment
        // time, headways 6 and 12 (frequencies 1/6 and 1/12). The split
        // is f_a/f_i = 2/3 vs 1/3 regardless of the waiting factor, since
        // both boarding links are scaled by the same factor.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("Fast", vec![1, 2], vec![10.0], 6.0));
        network.add_route(TransitRoute::new("Slow", vec![1, 2], vec![10.0], 12.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 90.0);

        for wait_factor in [1.0, 0.5, 2.0] {
            let result = assign_transit_with_options(
                &network,
                &od,
                &TransitAssignmentOptions {
                    wait_factor,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(
                (result.route_boardings["Fast"] - 60.0).abs() < EPS,
                "wait_factor {}: Fast = {}",
                wait_factor,
                result.route_boardings["Fast"]
            );
            assert!((result.route_boardings["Slow"] - 30.0).abs() < EPS);
        }
    }

    #[test]
    fn test_invalid_wait_factor() {
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 1.0);

        for bad in [0.0, -1.0, f64::NAN] {
            assert!(matches!(
                assign_transit_with_options(
                    &network,
                    &od,
                    &TransitAssignmentOptions {
                        wait_factor: bad,
                        ..Default::default()
                    }
                ),
                Err(TransitError::InvalidWaitFactor { .. })
            ));
        }
    }

    #[test]
    fn test_boarding_and_alighting_penalties() {
        // Single line: 6 min wait (default alpha) + 10 min ride. A
        // boarding penalty and an alighting penalty are plain costs added
        // once each on the traveled path (board once, alight once at the
        // destination).
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 100.0);

        let boarding = assign_transit_with_options(
            &network,
            &od,
            &TransitAssignmentOptions {
                boarding_penalty: 2.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!((boarding.od_costs[&(1, 2)] - 18.0).abs() < EPS);

        let alighting = assign_transit_with_options(
            &network,
            &od,
            &TransitAssignmentOptions {
                alighting_penalty: 3.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!((alighting.od_costs[&(1, 2)] - 19.0).abs() < EPS);

        let both = assign_transit_with_options(
            &network,
            &od,
            &TransitAssignmentOptions {
                boarding_penalty: 2.0,
                alighting_penalty: 3.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!((both.od_costs[&(1, 2)] - 21.0).abs() < EPS);
        // Penalties do not change a single-line loading.
        assert!((both.route_boardings["L1"] - 100.0).abs() < EPS);
    }

    #[test]
    fn test_boarding_penalty_discourages_transfer() {
        // Direct line D (1 -> 3, 20 min) competes with a two-leg trip via
        // a transfer: line A (1 -> 2, 8 min) then line B (2 -> 3, 8 min).
        // All headways are 6. Without a penalty the optimal strategy at
        // node 1 is {D, A}: sharing the wait for either vehicle makes the
        // transfer worthwhile (expected 24 min, versus 26 riding D alone),
        // so some passengers take A and transfer to B. A boarding penalty
        // is charged on every boarding, so the transfer path pays it twice
        // and, once large enough, {D, A} (39 min) becomes worse than D
        // alone (36 min): the whole strategy collapses onto the direct
        // line and B carries nothing.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("D", vec![1, 3], vec![20.0], 6.0));
        network.add_route(TransitRoute::new("A", vec![1, 2], vec![8.0], 6.0));
        network.add_route(TransitRoute::new("B", vec![2, 3], vec![8.0], 6.0));
        let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
        od.set(1, 3, 100.0);

        let no_penalty =
            assign_transit_with_options(&network, &od, &TransitAssignmentOptions::default())
                .unwrap();
        // The transfer path is used: line B carries flow on 2 -> 3.
        assert!(no_penalty.route_boardings.get("B").copied().unwrap_or(0.0) > 0.0);

        let penalized = assign_transit_with_options(
            &network,
            &od,
            &TransitAssignmentOptions {
                boarding_penalty: 10.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!((penalized.route_boardings["D"] - 100.0).abs() < EPS);
        assert!(penalized.route_boardings.get("B").copied().unwrap_or(0.0) < EPS);
    }

    #[test]
    fn test_dwell_two_node_scheme() {
        // Line 1 -> 2 -> 3, segments 10 + 10, headway 6, default alpha.
        // A through rider from 1 to 3 pays: 0.5*dwell on boarding (they
        // board during the dwell at stop 1) + 6 wait + 10 ride + dwell at
        // the intermediate stop 2 + 10 ride + 0 alighting at destination.
        // With dwell = 4: 2 (half dwell) + 6 + 10 + 4 + 10 = 32.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new(
            "L1",
            vec![1, 2, 3],
            vec![10.0, 10.0],
            6.0,
        ));
        let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
        od.set(1, 3, 100.0);

        let dwell = assign_transit_with_options(
            &network,
            &od,
            &TransitAssignmentOptions {
                dwell_time: 4.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            (dwell.od_costs[&(1, 3)] - 32.0).abs() < EPS,
            "through cost = {}, want 32",
            dwell.od_costs[&(1, 3)]
        );

        // A boarder-then-alight-at-2 trip (1 -> 2) pays only the half
        // dwell at boarding, not the full intermediate dwell:
        // 0.5*4 + 6 + 10 = 18.
        od.set(1, 3, 0.0);
        od.set(1, 2, 100.0);
        let short = assign_transit_with_options(
            &network,
            &od,
            &TransitAssignmentOptions {
                dwell_time: 4.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            (short.od_costs[&(1, 2)] - 18.0).abs() < EPS,
            "short cost = {}, want 18",
            short.od_costs[&(1, 2)]
        );

        // The dwell link appears in the typed volumes and carries the
        // through flow at the intermediate stop.
        let dwell_link = dwell
            .link_volumes
            .iter()
            .find(|lv| lv.kind == TransitLinkKind::Dwell)
            .expect("dwell link present");
        assert_eq!(dwell_link.from_stop, 2);
        assert!((dwell_link.volume - 100.0).abs() < EPS);
    }

    #[test]
    fn test_dwell_zero_matches_one_node() {
        // dwell = 0 must reproduce the plain one-node scheme exactly:
        // no dwell links, and the paper's 27.75 min result unchanged.
        let network = paper_network();
        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 1.0);

        let result = assign_transit_with_options(
            &network,
            &od,
            &TransitAssignmentOptions {
                dwell_time: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!((result.od_costs[&(1, 4)] - 27.75).abs() <= EPS);
        assert!(
            !result
                .link_volumes
                .iter()
                .any(|lv| lv.kind == TransitLinkKind::Dwell)
        );
    }

    #[test]
    fn test_invalid_penalty() {
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 1.0);

        for bad in [-1.0, f64::NAN] {
            assert!(matches!(
                assign_transit_with_options(
                    &network,
                    &od,
                    &TransitAssignmentOptions {
                        boarding_penalty: bad,
                        ..Default::default()
                    }
                ),
                Err(TransitError::InvalidPenalty { .. })
            ));
            assert!(matches!(
                assign_transit_with_options(
                    &network,
                    &od,
                    &TransitAssignmentOptions {
                        alighting_penalty: bad,
                        ..Default::default()
                    }
                ),
                Err(TransitError::InvalidPenalty { .. })
            ));
            assert!(matches!(
                assign_transit_with_options(
                    &network,
                    &od,
                    &TransitAssignmentOptions {
                        dwell_time: bad,
                        ..Default::default()
                    }
                ),
                Err(TransitError::InvalidPenalty { .. })
            ));
        }
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
