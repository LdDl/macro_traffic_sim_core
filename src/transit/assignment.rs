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

use hyperpaths_rs::{Graph, Link, Workspace, find_optimal_strategy};

use crate::od::OdMatrix;
use crate::transit::error::TransitError;
use crate::transit::route::TransitNetwork;

// Strict-capacity congested assignment (Cepeda-Cominetti-Florian 2006). It
// is a child module so it can reuse this module's private route-graph
// expansion and per-destination solve without exposing them in the API.
pub mod congested;

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
    /// Sum of all boardings over the whole network (every route, every
    /// destination). Equal to `route_boardings.values().sum()`.
    pub total_boardings: f64,
    /// Total assigned demand (sum of the OD entries that were loaded).
    pub total_demand: f64,
}

impl TransitAssignmentResult {
    /// Number of transfers: boardings beyond the first one of each trip.
    ///
    /// Computed as `total_boardings - total_demand`. This is exact when
    /// every assigned trip boards at least once (the usual case). Trips
    /// that reach their destination on foot without ever boarding make it
    /// an underestimate, since they add to the demand but not to the
    /// boardings; the value is clamped at zero.
    ///
    /// On the Spiess & Florian (1989) example (1 trip A -> B) it is `0.5`:
    /// half the flow rides Line 1 directly (one boarding) and half rides
    /// Line 2 and transfers at Y (two boardings).
    pub fn transfers(&self) -> f64 {
        (self.total_boardings - self.total_demand).max(0.0)
    }
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

/// Validates the assignment options. Rejects non-positive or NaN
/// `wait_factor` and negative or NaN penalties/dwell (NaN fails every
/// comparison, so we test for a valid value rather than an invalid one).
fn validate_options(options: &TransitAssignmentOptions) -> Result<(), TransitError> {
    if options.wait_factor.is_nan() || options.wait_factor <= 0.0 {
        return Err(TransitError::InvalidWaitFactor {
            wait_factor: options.wait_factor,
        });
    }
    for (name, value) in [
        ("boarding_penalty", options.boarding_penalty),
        ("alighting_penalty", options.alighting_penalty),
        ("dwell_time", options.dwell_time),
    ] {
        if value.is_nan() || value < 0.0 {
            return Err(TransitError::InvalidPenalty { name, value });
        }
    }
    Ok(())
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
        // Per-route overrides fall back to the global options.
        let dwell_time = route.dwell_time.unwrap_or(options.dwell_time);
        let boarding_penalty = route.boarding_penalty.unwrap_or(options.boarding_penalty);
        let alighting_penalty = route.alighting_penalty.unwrap_or(options.alighting_penalty);
        if dwell_time > 0.0 {
            // Two-node stop scheme: each stop on the route splits into an
            // arrival node (vehicle arrives, through riders and alighters
            // are here) and a departure node (vehicle departs, boarders
            // join here), with a dwell link between them. A through rider
            // pays the full dwell at every intermediate stop; a boarding
            // rider pays half of it (they board during the dwell), so the
            // boarding link carries `boarding_penalty + 0.5 * dwell`.
            let half_dwell = 0.5 * dwell_time;
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
                            boarding_penalty + half_dwell,
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
                            alighting_penalty,
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
                        Link::new(&arrival, &departure, &route.id, dwell_time, 0.0),
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
                            boarding_penalty,
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
                        Link::new(&node, &stop_name(stop), &route.id, alighting_penalty, 0.0),
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

impl TransitAssignmentOptions {
    /// Start from the defaults (`wait_factor = 1.0`, no penalties, no
    /// dwell). Chain `with_*` methods to override individual fields.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::TransitAssignmentOptions;
    ///
    /// let options = TransitAssignmentOptions::new()
    ///     .with_wait_factor(0.5)
    ///     .with_dwell_time(1.0);
    /// assert_eq!(options.wait_factor, 0.5);
    /// assert_eq!(options.dwell_time, 1.0);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the waiting time factor (`alpha`). See [`Self::wait_factor`].
    pub fn with_wait_factor(mut self, wait_factor: f64) -> Self {
        self.wait_factor = wait_factor;
        self
    }

    /// Set the boarding penalty. See [`Self::boarding_penalty`].
    pub fn with_boarding_penalty(mut self, boarding_penalty: f64) -> Self {
        self.boarding_penalty = boarding_penalty;
        self
    }

    /// Set the alighting penalty. See [`Self::alighting_penalty`].
    pub fn with_alighting_penalty(mut self, alighting_penalty: f64) -> Self {
        self.alighting_penalty = alighting_penalty;
        self
    }

    /// Set the dwell time. See [`Self::dwell_time`].
    pub fn with_dwell_time(mut self, dwell_time: f64) -> Self {
        self.dwell_time = dwell_time;
        self
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
    PreparedTransitNetwork::new(network, options)?.assign(od)
}

/// A transit network whose route graph is already expanded and interned, ready
/// to assign many OD matrices without rebuilding it.
///
/// [`assign_transit`] rebuilds the route graph and the interned solver graph on
/// every call. When the network and options are fixed and only the OD changes
/// between calls - the typical shape of a REST / gRPC service where the
/// timetable is loaded once and each request carries a different demand - build
/// this once and reuse it: the per-request route-graph expansion and string
/// interning disappear, leaving only the solve.
///
/// It is immutable after construction and `Sync`, so wrap it in an `Arc` and
/// share it across request threads; each [`assign`](Self::assign) call
/// allocates its own scratch, so concurrent calls do not contend. Rebuild it
/// when the network or the [`TransitAssignmentOptions`] change.
///
/// # Example
///
/// ```
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
/// use macro_traffic_sim_core::transit::{
///     PreparedTransitNetwork, TransitAssignmentOptions, TransitNetwork, TransitRoute,
/// };
///
/// let mut network = TransitNetwork::new();
/// network.add_route(TransitRoute::new("L1", vec![1, 4], vec![25.0], 6.0));
/// network.add_route(TransitRoute::new("L2", vec![1, 2, 3], vec![7.0, 6.0], 6.0));
/// network.add_route(TransitRoute::new("L3", vec![2, 3, 4], vec![4.0, 4.0], 15.0));
/// network.add_route(TransitRoute::new("L4", vec![3, 4], vec![10.0], 3.0));
///
/// // Expand and intern once...
/// let prepared =
///     PreparedTransitNetwork::new(&network, &TransitAssignmentOptions::default()).unwrap();
///
/// // ...then assign as many ODs as you like against it.
/// let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
/// od.set(1, 4, 1.0);
/// let result = prepared.assign(&od).unwrap();
/// assert!((result.od_costs[&(1, 4)] - 27.75).abs() < 1e-9);
/// ```
pub struct PreparedTransitNetwork {
    graph: RouteGraph,
    arena: Graph,
}

impl PreparedTransitNetwork {
    /// Validates the network and options, expands the route graph and interns
    /// it once. Reuse the result across many [`assign`](Self::assign) calls.
    ///
    /// # Arguments
    ///
    /// * `network` - the transit routes and walk links
    /// * `options` - assignment options baked into the route graph (wait
    ///   factor, penalties, dwell time)
    ///
    /// # Errors
    ///
    /// Returns a [`TransitError`] if the network is structurally invalid or the
    /// options are out of range.
    ///
    /// # Example
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    /// use macro_traffic_sim_core::transit::{
    ///     PreparedTransitNetwork, TransitAssignmentOptions, TransitNetwork, TransitRoute,
    /// };
    ///
    /// // One line, stops 1 -> 2: 10 minute ride, 5 minute headway.
    /// let mut network = TransitNetwork::new();
    /// network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
    ///
    /// // Expand and intern once; keep `prepared` around for every incoming OD.
    /// let prepared =
    ///     PreparedTransitNetwork::new(&network, &TransitAssignmentOptions::default()).unwrap();
    ///
    /// let mut od = DenseOdMatrix::new(vec![1, 2]);
    /// od.set(1, 2, 100.0);
    /// // wait_factor 1.0: 5 min expected wait + 10 min ride.
    /// let result = prepared.assign(&od).unwrap();
    /// assert!((result.od_costs[&(1, 2)] - 15.0).abs() < 1e-9);
    /// ```
    pub fn new(
        network: &TransitNetwork,
        options: &TransitAssignmentOptions,
    ) -> Result<Self, TransitError> {
        network.validate()?;
        validate_options(options)?;
        let graph = expand_route_graph(network, options);
        let arena = Graph::new(&graph.links, &graph.nodes);
        Ok(PreparedTransitNetwork { graph, arena })
    }

    /// Assigns one OD matrix against the prepared graph (single-threaded).
    ///
    /// Equivalent to [`assign_transit_with_options`] with the options this was
    /// built with, but without rebuilding the graph. Safe to call concurrently
    /// from many threads on a shared `&self`.
    ///
    /// # Errors
    ///
    /// Returns a [`TransitError`] when a demand zone is not a stop, or an OD
    /// pair with positive demand has no transit path.
    pub fn assign(&self, od: &dyn OdMatrix) -> Result<TransitAssignmentResult, TransitError> {
        let zone_ids = od.zone_ids().to_vec();
        let mut sink = DestSink::new(&self.arena, self.graph.links.len());
        for &destination in &zone_ids {
            sink.assign_destination(&self.graph, &self.arena, od, &zone_ids, destination)?;
        }
        let DestSink {
            volumes,
            od_costs,
            total_demand,
            ..
        } = sink;
        Ok(finalize(&self.graph, &volumes, od_costs, total_demand))
    }

    /// Parallel (opt-in) counterpart of [`assign`](Self::assign): fans the
    /// destinations out over rayon.
    ///
    /// See [`assign_transit_par_with_options`] for the concurrency caveat: in a
    /// service, prefer sharing this prepared network across single-threaded
    /// requests over calling this inside one request.
    ///
    /// # Errors
    ///
    /// Same as [`assign`](Self::assign).
    #[cfg(feature = "parallel")]
    pub fn assign_par(&self, od: &dyn OdMatrix) -> Result<TransitAssignmentResult, TransitError> {
        use rayon::prelude::*;

        let zone_ids = od.zone_ids().to_vec();
        let num_links = self.graph.links.len();

        let merged = zone_ids
            .par_iter()
            .try_fold(
                || DestSink::new(&self.arena, num_links),
                |mut sink, &destination| {
                    sink.assign_destination(&self.graph, &self.arena, od, &zone_ids, destination)?;
                    Ok(sink)
                },
            )
            .try_reduce(
                || DestSink::new(&self.arena, num_links),
                |mut a, b| {
                    a.merge(b);
                    Ok(a)
                },
            )?;

        let DestSink {
            volumes,
            od_costs,
            total_demand,
            ..
        } = merged;
        Ok(finalize(&self.graph, &volumes, od_costs, total_demand))
    }
}

/// Per-thread scratch and result accumulator for one interned route graph.
///
/// Holds the reusable Spiess-Florian workspace (so an interned graph is solved
/// for many destinations without re-interning) next to the aggregated link
/// volumes, OD costs and total demand. Used serially in
/// [`assign_transit_with_options`]; the parallel path keeps one per rayon
/// worker and combines them with [`DestSink::merge`].
struct DestSink<'g> {
    workspace: Workspace<'g>,
    demand_col: Vec<f64>,
    origins: Vec<(i64, f64)>,
    origin_ids: Vec<usize>,
    volumes: Vec<f64>,
    od_costs: HashMap<(i64, i64), f64>,
    total_demand: f64,
}

impl<'g> DestSink<'g> {
    fn new(arena: &'g Graph, num_links: usize) -> Self {
        DestSink {
            workspace: arena.new_workspace(),
            demand_col: vec![0.0; arena.num_nodes()],
            origins: Vec::new(),
            origin_ids: Vec::new(),
            volumes: vec![0.0; num_links],
            od_costs: HashMap::new(),
            total_demand: 0.0,
        }
    }

    /// Assigns one destination into this accumulator, reusing the workspace and
    /// demand column. A destination with no incoming demand is a no-op.
    fn assign_destination(
        &mut self,
        graph: &RouteGraph,
        arena: &Graph,
        od: &dyn OdMatrix,
        zone_ids: &[i64],
        destination: i64,
    ) -> Result<(), TransitError> {
        // Collect the demand column of this destination
        self.origins.clear();
        for &origin in zone_ids {
            if origin == destination {
                continue;
            }
            let demand = od.get(origin, destination);
            if demand > 0.0 {
                self.origins.push((origin, demand));
            }
        }
        if self.origins.is_empty() {
            return Ok(());
        }

        if !graph.stops.contains(&destination) {
            return Err(TransitError::UnknownStop { zone: destination });
        }
        for &(origin, _) in &self.origins {
            if !graph.stops.contains(&origin) {
                return Err(TransitError::UnknownStop { zone: origin });
            }
        }

        let dest_id = arena
            .node_index(&stop_name(destination))
            .ok_or(TransitError::UnknownStop { zone: destination })?;

        // Seed the reused demand column, remembering which entries to clear
        self.origin_ids.clear();
        for &(origin, demand) in &self.origins {
            let origin_id = arena
                .node_index(&stop_name(origin))
                .ok_or(TransitError::UnknownStop { zone: origin })?;
            self.demand_col[origin_id] = demand;
            self.origin_ids.push(origin_id);
        }

        let result = self.workspace.assign(dest_id, &self.demand_col);

        for (&(origin, demand), &origin_id) in self.origins.iter().zip(&self.origin_ids) {
            let label = result.labels[origin_id];
            if !label.is_finite() {
                return Err(TransitError::Unreachable {
                    origin,
                    destination,
                });
            }
            self.od_costs.insert((origin, destination), label);
            self.total_demand += demand;
        }

        // Arena link indices match graph.links / graph.meta order.
        for (idx, &volume) in result.link_vol.iter().enumerate() {
            self.volumes[idx] += volume;
        }

        // Reset only the touched entries so the column is reusable
        for &origin_id in &self.origin_ids {
            self.demand_col[origin_id] = 0.0;
        }

        Ok(())
    }

    /// Folds another accumulator into this one: link volumes summed element-wise,
    /// OD costs and total demand combined. Used to merge per-worker results.
    fn merge(&mut self, other: DestSink<'g>) {
        for (v, &ov) in self.volumes.iter_mut().zip(other.volumes.iter()) {
            *v += ov;
        }
        self.od_costs.extend(other.od_costs);
        self.total_demand += other.total_demand;
    }
}

/// Turns aggregated link volumes into the public assignment result (typed link
/// volumes plus per-route and total boardings).
fn finalize(
    graph: &RouteGraph,
    volumes: &[f64],
    od_costs: HashMap<(i64, i64), f64>,
    total_demand: f64,
) -> TransitAssignmentResult {
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

    let total_boardings = route_boardings.values().sum();

    TransitAssignmentResult {
        link_volumes,
        od_costs,
        route_boardings,
        total_boardings,
        total_demand,
    }
}

/// Parallel (opt-in) variant of [`assign_transit`]: assigns destinations
/// concurrently with rayon, using default options.
///
/// See [`assign_transit_par_with_options`] for when this is (and is not) the
/// right tool.
#[cfg(feature = "parallel")]
pub fn assign_transit_par(
    network: &TransitNetwork,
    od: &dyn OdMatrix,
) -> Result<TransitAssignmentResult, TransitError> {
    assign_transit_par_with_options(network, od, &TransitAssignmentOptions::default())
}

/// Parallel (opt-in) variant of [`assign_transit_with_options`].
///
/// Destinations are independent, so the assignment fans out over them with
/// rayon: the interned [`Graph`] is immutable and shared read-only, each worker
/// keeps its own reusable workspace, and the per-worker link volumes are then
/// merged. Reach for this on a single large batch assignment (many
/// destinations, a CLI or one-shot job) where the process has spare cores.
///
/// The default [`assign_transit`] / [`assign_transit_with_options`] stay
/// single-threaded on purpose.
///
/// # Services (REST / gRPC): parallelize across requests, not inside one
///
/// Do not call this from a request handler. A service is already concurrent
/// across requests; if every in-flight request also fans out over rayon's
/// global pool, the pools oversubscribe the cores and tail latency gets worse,
/// not better. In a service keep each request single-threaded (call
/// [`assign_transit`]) and let the server's executor provide parallelism across
/// requests. The larger server win is orthogonal to threading: a [`Graph`] is
/// immutable and `Sync`, so it can be built once per network and shared across
/// requests (only the OD changes), removing the per-request graph expansion and
/// interning. This function does not address that; it only splits one
/// assignment's destinations across cores.
///
/// # Determinism
///
/// Results match the serial path up to floating-point summation order: link
/// volumes and total demand are reduced across workers in a nondeterministic
/// order, so they can differ from [`assign_transit_with_options`] by rounding
/// (ULP level, far below any assignment tolerance). OD costs are computed
/// independently per destination and are identical.
///
/// # Errors
///
/// Same as [`assign_transit_with_options`]; the first destination that fails
/// aborts the run.
#[cfg(feature = "parallel")]
pub fn assign_transit_par_with_options(
    network: &TransitNetwork,
    od: &dyn OdMatrix,
    options: &TransitAssignmentOptions,
) -> Result<TransitAssignmentResult, TransitError> {
    PreparedTransitNetwork::new(network, options)?.assign_par(od)
}

/// Computes the transit level-of-service (skim): the expected travel time
/// between every ordered pair of the given zones, using default options.
///
/// See [`transit_skim_with_options`].
pub fn transit_skim(
    network: &TransitNetwork,
    zones: &[i64],
) -> Result<HashMap<(i64, i64), f64>, TransitError> {
    transit_skim_with_options(network, zones, &TransitAssignmentOptions::default())
}

/// Computes the transit level-of-service (skim) with explicit options.
///
/// For every ordered pair `(origin, destination)` of `zones` the value is
/// the optimal-strategy expected travel time (waiting + in-vehicle +
/// walking), the same quantity that [`assign_transit`] reports in
/// `od_costs`. Unlike an assignment, no demand is needed: the
/// Spiess-Florian labels give the cost from every origin to a
/// destination in a single solve and are independent of flow, so this
/// runs one solve per destination zone and reads all origin labels.
///
/// Pairs that are unreachable (no transit path, or a zone that is not a
/// stop) are omitted from the result rather than reported as infinite.
///
/// This is the transit counterpart of a road skim matrix: a
/// destination-choice or mode-choice model can consume it directly.
///
/// # Arguments
///
/// * `network` - Transit routes and walk links
/// * `zones` - Zone (stop) IDs to compute the skim over
/// * `options` - Assignment options (waiting factor, penalties, dwell)
///
/// # Errors
///
/// Returns a [`TransitError`] when the network is structurally invalid or
/// the options are out of range.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::transit::{transit_skim, TransitNetwork, TransitRoute};
///
/// // one line, 10 min ride, 5 min headway
/// let mut network = TransitNetwork::new();
/// network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
///
/// let skim = transit_skim(&network, &[1, 2]).unwrap();
/// // 1 -> 2 costs 5 wait + 10 ride; 2 -> 1 has no line, omitted
/// assert!((skim[&(1, 2)] - 15.0).abs() < 1e-9);
/// assert!(!skim.contains_key(&(2, 1)));
/// ```
pub fn transit_skim_with_options(
    network: &TransitNetwork,
    zones: &[i64],
    options: &TransitAssignmentOptions,
) -> Result<HashMap<(i64, i64), f64>, TransitError> {
    network.validate()?;
    validate_options(options)?;
    let graph = expand_route_graph(network, options);

    let mut skim: HashMap<(i64, i64), f64> = HashMap::new();
    for &destination in zones {
        // A destination that is not a stop yields no attractive links, so
        // every origin stays at infinity and no pair is recorded.
        if !graph.stops.contains(&destination) {
            continue;
        }
        let destination_name = stop_name(destination);
        let strategy = find_optimal_strategy(&graph.links, &graph.nodes, &destination_name);
        for &origin in zones {
            if origin == destination {
                continue;
            }
            let label = strategy
                .labels
                .get(&stop_name(origin))
                .copied()
                .unwrap_or(f64::INFINITY);
            if label.is_finite() {
                skim.insert((origin, destination), label);
            }
        }
    }
    Ok(skim)
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
    fn test_transit_skim_matches_od_costs() {
        // The skim must equal what assign_transit reports as od_costs for
        // the same pairs, since both are the phase-1 labels.
        let network = paper_network();
        let zones = [1, 2, 3, 4];

        let skim = transit_skim(&network, &zones).unwrap();
        // Paper labels: u_A = 27.75 (A -> B), u_X = 19.0714... (X -> B).
        assert!((skim[&(1, 4)] - 27.75).abs() <= EPS);
        assert!((skim[&(2, 4)] - 19.071428571428573).abs() <= EPS);

        // Cross-check against an assignment over the same pairs.
        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 1.0);
        od.set(2, 4, 1.0);
        let assigned = assign_transit(&network, &od).unwrap();
        assert!((skim[&(1, 4)] - assigned.od_costs[&(1, 4)]).abs() <= EPS);
        assert!((skim[&(2, 4)] - assigned.od_costs[&(2, 4)]).abs() <= EPS);
    }

    #[test]
    fn test_transit_skim_omits_unreachable() {
        // One-way line 1 -> 2: 1 -> 2 is reachable, 2 -> 1 is not.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));
        let skim = transit_skim(&network, &[1, 2]).unwrap();
        assert!(skim.contains_key(&(1, 2)));
        assert!(!skim.contains_key(&(2, 1)));
        // A zone that is not a stop produces no pairs at all.
        let skim2 = transit_skim(&network, &[1, 2, 99]).unwrap();
        assert!(!skim2.keys().any(|&(o, d)| o == 99 || d == 99));
    }

    #[test]
    fn test_transit_skim_respects_options() {
        // wait_factor 0.5 lowers the skim exactly as it lowers od_costs.
        let network = paper_network();
        let opts = TransitAssignmentOptions::new().with_wait_factor(0.5);
        let skim = transit_skim_with_options(&network, &[1, 4], &opts).unwrap();
        assert!((skim[&(1, 4)] - 25.25).abs() <= EPS);
    }

    #[test]
    fn test_options_builder() {
        let options = TransitAssignmentOptions::new()
            .with_wait_factor(0.5)
            .with_boarding_penalty(2.0)
            .with_alighting_penalty(3.0)
            .with_dwell_time(1.0);
        assert_eq!(options.wait_factor, 0.5);
        assert_eq!(options.boarding_penalty, 2.0);
        assert_eq!(options.alighting_penalty, 3.0);
        assert_eq!(options.dwell_time, 1.0);
        // Untouched fields keep their defaults.
        assert_eq!(
            TransitAssignmentOptions::new()
                .with_dwell_time(5.0)
                .wait_factor,
            1.0
        );
    }

    #[test]
    fn test_total_boardings_and_transfers() {
        // Paper network: 1 trip A -> B. Boardings: L1 0.5, L2 0.5, L4 5/12,
        // L3 1/12 => 1.5 total. Demand 1.0, so transfers = 0.5 (the Line 2
        // half transfers at Y).
        let network = paper_network();
        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 1.0);
        let result = assign_transit(&network, &od).unwrap();
        assert!((result.total_boardings - 1.5).abs() <= EPS);
        assert!((result.total_demand - 1.0).abs() <= EPS);
        assert!((result.transfers() - 0.5).abs() <= EPS);
    }

    #[test]
    fn test_transfers_zero_for_direct_trip() {
        // Single line, no transfer possible: every trip boards exactly
        // once, transfers = 0.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 50.0);
        let result = assign_transit(&network, &od).unwrap();
        assert!((result.total_boardings - 50.0).abs() <= EPS);
        assert!((result.transfers() - 0.0).abs() <= EPS);
    }

    #[test]
    fn test_per_route_dwell_override() {
        // Global dwell 0, but line L1 overrides it to 4. A through rider
        // 1 -> 3 on L1 pays the override dwell; the result gains a Dwell
        // link on L1. Same cost as the global-dwell test: 32.
        let mut network = TransitNetwork::new();
        network.add_route(
            TransitRoute::new("L1", vec![1, 2, 3], vec![10.0, 10.0], 6.0).with_dwell_time(4.0),
        );
        let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
        od.set(1, 3, 100.0);

        // Global options leave dwell at 0; only the route override applies.
        let result = assign_transit(&network, &od).unwrap();
        assert!((result.od_costs[&(1, 3)] - 32.0).abs() < EPS);
        assert!(
            result
                .link_volumes
                .iter()
                .any(|lv| lv.kind == TransitLinkKind::Dwell)
        );
    }

    #[test]
    fn test_per_route_override_falls_back_to_global() {
        // Two lines share a stop; only line B overrides the boarding
        // penalty. Line A uses the global boarding penalty (5), line B its
        // own (0). Verified through the expected travel times of two
        // separate single-line trips.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("A", vec![1, 2], vec![10.0], 6.0));
        network.add_route(
            TransitRoute::new("B", vec![3, 4], vec![10.0], 6.0).with_boarding_penalty(0.0),
        );
        let options = TransitAssignmentOptions::new().with_boarding_penalty(5.0);

        let mut od_a = DenseOdMatrix::new(vec![1, 2]);
        od_a.set(1, 2, 1.0);
        let ra = assign_transit_with_options(&network, &od_a, &options).unwrap();
        // Line A: global penalty 5 + wait 6 + ride 10 = 21.
        assert!((ra.od_costs[&(1, 2)] - 21.0).abs() < EPS);

        let mut od_b = DenseOdMatrix::new(vec![3, 4]);
        od_b.set(3, 4, 1.0);
        let rb = assign_transit_with_options(&network, &od_b, &options).unwrap();
        // Line B: override penalty 0 + wait 6 + ride 10 = 16.
        assert!((rb.od_costs[&(3, 4)] - 16.0).abs() < EPS);
    }

    #[test]
    fn test_per_route_invalid_override() {
        let mut network = TransitNetwork::new();
        network
            .add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0).with_dwell_time(-1.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 1.0);
        assert!(matches!(
            assign_transit(&network, &od),
            Err(TransitError::InvalidPenalty { .. })
        ));
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

    #[test]
    fn test_prepared_is_send_sync() {
        // A prepared network is meant to be shared across request threads via
        // an Arc, which requires Send + Sync. This guards that guarantee.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PreparedTransitNetwork>();
    }

    #[test]
    fn test_prepared_matches_oneshot_and_is_reusable() {
        let network = paper_network();
        let prepared =
            PreparedTransitNetwork::new(&network, &TransitAssignmentOptions::default()).unwrap();

        // Two different ODs assigned against the same prepared graph must each
        // match the one-shot assign_transit, and the prepared network must be
        // reusable (no state carried between calls).
        let mut od_a = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od_a.set(1, 4, 1.0);
        let mut od_b = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od_b.set(2, 4, 1.0);
        od_b.set(3, 4, 1.0);

        for od in [&od_a, &od_b] {
            let oneshot = assign_transit(&network, od).unwrap();
            let reused = prepared.assign(od).unwrap();
            assert_eq!(oneshot.od_costs, reused.od_costs);
            assert_eq!(oneshot.link_volumes.len(), reused.link_volumes.len());
            for (a, b) in oneshot.link_volumes.iter().zip(&reused.link_volumes) {
                assert_eq!((a.kind, &a.route_id), (b.kind, &b.route_id));
                assert!((a.volume - b.volume).abs() <= EPS);
            }
            assert!((oneshot.total_boardings - reused.total_boardings).abs() <= EPS);
        }
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn test_parallel_matches_serial() {
        let network = paper_network();
        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        // Several destinations and origins so the parallel fan-out and merge
        // actually exercise more than one destination.
        od.set(1, 4, 1.0);
        od.set(2, 4, 1.0);
        od.set(3, 4, 1.0);
        od.set(1, 3, 1.0);
        od.set(2, 3, 1.0);

        let serial = assign_transit(&network, &od).unwrap();
        let parallel = assign_transit_par(&network, &od).unwrap();

        // OD costs are computed independently per destination, so they are
        // identical, not just close.
        assert_eq!(serial.od_costs, parallel.od_costs);

        // Volumes and totals are reduced across workers, so they match up to
        // floating-point summation order.
        assert!((serial.total_demand - parallel.total_demand).abs() <= EPS);
        assert!((serial.total_boardings - parallel.total_boardings).abs() <= EPS);
        assert_eq!(serial.link_volumes.len(), parallel.link_volumes.len());
        for (a, b) in serial.link_volumes.iter().zip(&parallel.link_volumes) {
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.route_id, b.route_id);
            assert_eq!((a.from_stop, a.to_stop), (b.from_stop, b.to_stop));
            assert!((a.volume - b.volume).abs() <= EPS);
        }
    }
}
