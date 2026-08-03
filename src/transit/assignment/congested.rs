//! # Congested transit assignment (Cepeda, Cominetti and Florian, 2006)
//!
//! Strict-capacity frequency-based transit equilibrium: as the boarding
//! flow on a line approaches the residual capacity of its vehicles, the
//! line's effective frequency drops toward zero and its waiting time
//! explodes, so passengers are pushed onto other lines or onto walking.
//! Unlike the soft crowding of [`crowding`](crate::transit::crowding), the
//! capacity is a hard limit: a line cannot carry more than its vehicles
//! hold.
//!
//! This is the full-congested model of Cominetti and Correa (2001) together
//! with the solution algorithm of Cepeda, Cominetti and Florian (2006)
//! (Transportation Research Part B 40(6), 437-459, DOI:
//! <https://doi.org/10.1016/j.trb.2005.05.006>), which is the algorithm
//! behind EMME's capacitated transit assignment.
//!
//! ## The method
//!
//! The equilibrium is the flow at which every strategy carrying passengers
//! is of minimal expected time (Wardrop) under effective frequencies that
//! depend on the flow. It is computed by the method of successive averages
//! (their Section 4): each iteration freezes the effective frequencies at
//! the current flow, solves the ordinary Spiess-Florian shortest-hyperpath
//! problem (the same solver as [`assign_transit`](crate::transit::assign_transit)),
//! and averages the induced flow into the running estimate. The unchanged
//! optimal-strategies solver stays the inner kernel; congestion enters only
//! through the frozen frequencies.
//!
//! ## The gap
//!
//! Progress is measured by the computable gap function of their Theorem 3.2:
//!
//! ```text
//! G(v) = sum_d [ sum_a t_a(v) v^d_a
//!              + sum_{i!=d} max_{a in A+_i} ( v^d_a / f_a(v) )
//!              - sum_{i!=d} g^d_i tau^d_i(v) ]
//! ```
//!
//! where `t_a` is the arc travel time, `f_a(v)` the effective frequency,
//! `v^d_a` the flow on arc `a` toward destination `d`, `g^d_i` the demand
//! and `tau^d_i(v)` the shortest-hyperpath time to `d`. The first two terms
//! are the actual generalized cost of the flow, the third is the least
//! possible cost; `G(v) >= 0` always and `G(v) = 0` exactly at equilibrium.
//! The stopping rule uses the relative gap `G(v) / sum g^d_i tau^d_i`.
//!
//! ## Effective frequency
//!
//! The effective frequency used here is the strict-capacity form of the
//! paper's numerical experiments (their p. 450):
//!
//! ```text
//! f_a(v) = mu * ( 1 - ( v_a / (mu*c - v'_a + v_a) )^beta )    if v'_a < mu*c, else 0
//! ```
//!
//! with `mu` the nominal frequency, `c` the per-vehicle capacity (so
//! `mu*c = (analysis_period / headway) * capacity` is the line capacity per
//! period), `v_a` the flow boarding the arc and `v'_a` the on-board flow
//! right after the stop (`mu*c - v'_a` is the residual capacity). The
//! frequency is truncated at a floor so the effective wait never exceeds
//! `max_wait`. Only routes with a `capacity` set are congested; the rest
//! keep infinite capacity (no crowding).

use std::collections::{HashMap, HashSet};

use hyperpaths_rs::{Link, compute_sf};

use crate::od::OdMatrix;
use crate::transit::error::TransitError;
use crate::transit::route::TransitNetwork;
// Private route-graph internals of the parent `assignment` module, reachable
// here because this is a descendant module (no `pub` needed on them).
use super::{
    RouteGraph, TransitAssignmentOptions, TransitAssignmentResult, TransitLinkKind,
    TransitLinkVolume, expand_route_graph, stop_name, validate_options,
};

/// Parameters of the congested (strict-capacity) transit assignment.
#[derive(Debug, Clone, Copy)]
pub struct CongestedParams {
    /// Analysis period, in the same time unit as the route headways, used to
    /// turn a per-vehicle capacity into a line capacity per period:
    /// `mu*c = (analysis_period / headway) * capacity`. Must match the period
    /// of the OD demand.
    pub analysis_period: f64,
    /// Exponent `beta` of the effective-frequency law
    /// `f_a = mu * (1 - v_a/(mu*c - v'_a + v_a))^beta`. Default `0.2` (the
    /// value used in the Cepeda-Cominetti-Florian numerical example).
    pub beta: f64,
    /// Maximum number of outer MSA iterations. Default `100`.
    pub max_iterations: usize,
    /// Relative-gap stopping tolerance `G(v) / sum g^d_i tau^d_i`. Default
    /// `1e-4`.
    pub gap_tolerance: f64,
    /// Truncation cap on the effective waiting time (the paper caps the
    /// headway at 999 minutes). Default `999.0`, in the network's time unit.
    pub max_wait: f64,
}

impl CongestedParams {
    /// Congested parameters for the given analysis period, with the paper's
    /// defaults (`beta = 0.2`, 100 iterations, relative-gap tolerance
    /// `1e-4`, wait cap `999`).
    pub fn new(analysis_period: f64) -> Self {
        CongestedParams {
            analysis_period,
            beta: 0.2,
            max_iterations: 100,
            gap_tolerance: 1e-4,
            max_wait: 999.0,
        }
    }

    /// Set the effective-frequency exponent `beta`.
    pub fn with_beta(mut self, beta: f64) -> Self {
        self.beta = beta;
        self
    }

    /// Set the maximum number of MSA iterations and the relative-gap
    /// tolerance.
    pub fn with_limits(mut self, max_iterations: usize, gap_tolerance: f64) -> Self {
        self.max_iterations = max_iterations;
        self.gap_tolerance = gap_tolerance;
        self
    }
}

/// Result of a congested transit assignment.
#[derive(Debug, Clone)]
pub struct CongestedResult {
    /// The equilibrium assignment (volumes, od costs, boardings) at the
    /// averaged flow.
    pub assignment: TransitAssignmentResult,
    /// Relative gap `G(v) / sum g^d_i tau^d_i` of the returned flow. Small
    /// means close to equilibrium; a value that stalls well above zero means
    /// the demand cannot be carried within capacity.
    pub relative_gap: f64,
    /// Number of MSA iterations performed.
    pub iterations: usize,
    /// Relative gap after each iteration, for diagnostics.
    pub gap_history: Vec<f64>,
}

/// Per-destination arc-flow vectors `v^d_a`, each indexed like the route
/// graph's link list.
type DestFlows = HashMap<i64, Vec<f64>>;
/// Per-destination node labels `tau^d_i` (time-to-destination).
type DestLabels = HashMap<i64, HashMap<String, f64>>;

/// Effective waiting time on a boarding link under strict capacity.
///
/// Returns `base_wait / g` with `g = 1 - (v_a/(mu*c - v'_a + v_a))^beta`,
/// capped at `max_wait`; when the on-board flow reaches the line capacity
/// the residual vanishes and the wait is the cap.
fn effective_wait(
    base_wait: f64,
    boarding_flow: f64,
    onboard_flow: f64,
    line_capacity: f64,
    beta: f64,
    max_wait: f64,
) -> f64 {
    let residual_denom = line_capacity - onboard_flow + boarding_flow;
    if residual_denom <= 0.0 || onboard_flow >= line_capacity {
        return max_wait;
    }
    let ratio = boarding_flow / residual_denom;
    // Effective-frequency fraction g = 1 - (v_a / (mu*c - v'_a + v_a))^beta;
    // the exponent is on the ratio, not on the whole (1 - ratio).
    let g = 1.0 - ratio.max(0.0).powf(beta);
    if g <= 0.0 {
        return max_wait;
    }
    (base_wait / g).min(max_wait)
}

/// Solves the shortest-hyperpath problem for one destination on the given
/// links, returning the induced per-link flow (indexed like `graph.links`)
/// and the node labels (time-to-destination).
fn solve_destination(
    links: &[Link],
    nodes: &HashSet<String>,
    index: &HashMap<(String, String), usize>,
    destination: i64,
    origins: &[(i64, f64)],
) -> (Vec<f64>, HashMap<String, f64>) {
    let dest_name = stop_name(destination);
    let mut trips: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for &(origin, demand) in origins {
        trips
            .entry(stop_name(origin))
            .or_default()
            .insert(dest_name.clone(), demand);
    }
    let result = compute_sf(links, nodes, &dest_name, &trips);
    let mut flow = vec![0.0; links.len()];
    for (from_name, to_map) in &result.volumes.links {
        for (to_name, volume) in to_map {
            if let Some(&idx) = index.get(&(from_name.clone(), to_name.clone())) {
                flow[idx] += volume;
            }
        }
    }
    (flow, result.strategy.labels)
}

/// Solves every destination on `links`, returning per-destination flow
/// vectors and labels. Validates that demand zones are stops and that every
/// origin can reach its destination.
fn solve_all(
    graph: &RouteGraph,
    links: &[Link],
    zone_ids: &[i64],
    od: &dyn OdMatrix,
) -> Result<(DestFlows, DestLabels), TransitError> {
    let mut flows: DestFlows = HashMap::new();
    let mut labels: DestLabels = HashMap::new();
    for &destination in zone_ids {
        let mut origins: Vec<(i64, f64)> = Vec::new();
        for &origin in zone_ids {
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
        let (flow, node_labels) =
            solve_destination(links, &graph.nodes, &graph.index, destination, &origins);
        for &(origin, _) in &origins {
            let label = node_labels
                .get(&stop_name(origin))
                .copied()
                .unwrap_or(f64::INFINITY);
            if !label.is_finite() {
                return Err(TransitError::Unreachable {
                    origin,
                    destination,
                });
            }
        }
        flows.insert(destination, flow);
        labels.insert(destination, node_labels);
    }
    Ok((flows, labels))
}

/// Computes the Cepeda-Cominetti-Florian gap `G(v)` and the minimal-cost sum
/// `sum g^d_i tau^d_i` (the relative-gap denominator) for the flow `v_dest`
/// under the effective links `links` and labels from solving on them.
fn compute_gap(
    links: &[Link],
    v_dest: &DestFlows,
    labels: &DestLabels,
    zone_ids: &[i64],
    od: &dyn OdMatrix,
) -> (f64, f64) {
    let mut gap = 0.0;
    let mut min_cost = 0.0;
    for (&destination, flow) in v_dest {
        let node_labels = match labels.get(&destination) {
            Some(l) => l,
            None => continue,
        };
        // Term 1: in-vehicle / travel cost sum_a t_a v^d_a.
        let mut term1 = 0.0;
        // Term 2: waiting sum_i max_{a in A+_i} (v^d_a / f_a) = max flow*headway per source node.
        let mut max_by_node: HashMap<&str, f64> = HashMap::new();
        for (idx, link) in links.iter().enumerate() {
            let f = flow[idx];
            if f == 0.0 {
                continue;
            }
            term1 += link.travel_cost * f;
            let wait_weighted = f * link.headway;
            let entry = max_by_node.entry(link.from_node.as_str()).or_insert(0.0);
            if wait_weighted > *entry {
                *entry = wait_weighted;
            }
        }
        let term2: f64 = max_by_node.values().sum();
        // Term 3: demand-weighted minimal time sum_i g^d_i tau^d_i.
        let mut term3 = 0.0;
        for &origin in zone_ids {
            if origin == destination {
                continue;
            }
            let demand = od.get(origin, destination);
            if demand > 0.0 {
                let time = node_labels.get(&stop_name(origin)).copied().unwrap_or(0.0);
                term3 += demand * time;
                min_cost += demand * time;
            }
        }
        gap += term1 + term2 - term3;
    }
    (gap, min_cost)
}

/// Aggregates per-destination flows into a single per-link flow vector.
fn aggregate(v_dest: &DestFlows, n: usize) -> Vec<f64> {
    let mut agg = vec![0.0; n];
    for flow in v_dest.values() {
        for (idx, &f) in flow.iter().enumerate() {
            agg[idx] += f;
        }
    }
    agg
}

/// Runs the congested (strict-capacity) transit assignment.
///
/// Wraps the Spiess-Florian solver in the Cepeda-Cominetti-Florian (2006)
/// method of successive averages: capacity limits each line's effective
/// frequency, and the relative gap of Theorem 3.2 measures the distance to
/// equilibrium. With no capacitated routes this converges to the ordinary
/// uncongested assignment.
///
/// # Arguments
///
/// * `network` - Transit routes (with per-vehicle `capacity` on the lines to
///   be capacity-limited) and walk links
/// * `od` - Transit OD matrix (passengers per the analysis period)
/// * `options` - Base assignment options (waiting factor, penalties, dwell)
/// * `params` - Congestion law, iteration and gap-tolerance control
///
/// # Errors
///
/// Returns a [`TransitError`] when the network or options are invalid, when
/// `params.analysis_period` is not strictly positive, or when a demand pair
/// cannot be connected.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
/// use macro_traffic_sim_core::transit::{
///     assign_transit_congested, CongestedParams, TransitAssignmentOptions, TransitNetwork,
///     TransitRoute,
/// };
///
/// // Two parallel lines A -> B; the small one is capacity limited.
/// let mut network = TransitNetwork::new();
/// network.add_route(TransitRoute::new("Big", vec![1, 2], vec![10.0], 6.0).with_capacity(1000.0));
/// network.add_route(TransitRoute::new("Small", vec![1, 2], vec![10.0], 6.0).with_capacity(30.0));
///
/// let mut od = DenseOdMatrix::new(vec![1, 2]);
/// od.set(1, 2, 400.0);
///
/// let result = assign_transit_congested(
///     &network,
///     &od,
///     &TransitAssignmentOptions::default(),
///     &CongestedParams::new(60.0),
/// )
/// .unwrap();
/// // The capacity-limited line carries less than the roomy one.
/// let a = &result.assignment;
/// assert!(a.route_boardings["Small"] < a.route_boardings["Big"]);
/// ```
pub fn assign_transit_congested(
    network: &TransitNetwork,
    od: &dyn OdMatrix,
    options: &TransitAssignmentOptions,
    params: &CongestedParams,
) -> Result<CongestedResult, TransitError> {
    network.validate()?;
    validate_options(options)?;
    if params.analysis_period.is_nan() || params.analysis_period <= 0.0 {
        return Err(TransitError::InvalidAnalysisPeriod {
            value: params.analysis_period,
        });
    }

    let graph = expand_route_graph(network, options);
    let n = graph.links.len();
    let zone_ids = od.zone_ids().to_vec();

    // Per-boarding-link congestion data: (line capacity mu*c, index of the
    // on-board riding link right after the stop). Only for capacitated routes.
    let mut riding_from: HashMap<&str, usize> = HashMap::new();
    for (idx, meta) in graph.meta.iter().enumerate() {
        if meta.kind == TransitLinkKind::Riding {
            riding_from.insert(graph.links[idx].from_node.as_str(), idx);
        }
    }
    let mut route_capacity: HashMap<&str, f64> = HashMap::new();
    let mut route_headway: HashMap<&str, f64> = HashMap::new();
    for route in &network.routes {
        route_headway.insert(route.id.as_str(), route.headway);
        if let Some(capacity) = route.capacity {
            route_capacity.insert(route.id.as_str(), capacity);
        }
    }
    // (line_capacity, riding_after_idx) per boarding link that crowds.
    let mut board_info: Vec<Option<(f64, usize)>> = vec![None; n];
    for (idx, slot) in board_info.iter_mut().enumerate() {
        if graph.meta[idx].kind != TransitLinkKind::Boarding {
            continue;
        }
        let Some(route_id) = &graph.meta[idx].route_id else {
            continue;
        };
        let Some(&capacity) = route_capacity.get(route_id.as_str()) else {
            continue;
        };
        let headway = route_headway[route_id.as_str()];
        let line_capacity = (params.analysis_period / headway) * capacity;
        if let Some(&after) = riding_from.get(graph.links[idx].to_node.as_str()) {
            *slot = Some((line_capacity, after));
        }
    }
    let base_wait: Vec<f64> = graph.links.iter().map(|l| l.headway).collect();

    // Initial all-or-nothing assignment at the nominal frequencies.
    let (mut v_dest, mut labels) = solve_all(&graph, &graph.links, &zone_ids, od)?;

    let mut gap_history: Vec<f64> = Vec::new();
    let mut relative_gap = f64::INFINITY;
    let mut iterations = 0;

    for k in 1..=params.max_iterations {
        iterations = k;
        let v_agg = aggregate(&v_dest, n);

        // Effective frequencies frozen at the current flow.
        let mut links_eff = graph.links.clone();
        for (idx, info) in board_info.iter().enumerate() {
            if let Some((line_capacity, after)) = *info {
                links_eff[idx].headway = effective_wait(
                    base_wait[idx],
                    v_agg[idx],
                    v_agg[after],
                    line_capacity,
                    params.beta,
                    params.max_wait,
                );
            }
        }

        // Shortest hyperpaths at the frozen frequencies.
        let (v_hat, new_labels) = solve_all(&graph, &links_eff, &zone_ids, od)?;
        labels = new_labels;

        // Gap of the current flow under these frozen frequencies.
        let (gap, min_cost) = compute_gap(&links_eff, &v_dest, &labels, &zone_ids, od);
        relative_gap = if min_cost > 0.0 { gap / min_cost } else { 0.0 };
        gap_history.push(relative_gap);
        if relative_gap <= params.gap_tolerance {
            break;
        }

        // Method of successive averages update.
        let step = 1.0 / (k as f64 + 1.0);
        for (&destination, hat) in &v_hat {
            let current = v_dest
                .entry(destination)
                .or_insert_with(|| vec![0.0; n]);
            for idx in 0..n {
                current[idx] = (1.0 - step) * current[idx] + step * hat[idx];
            }
        }
    }

    // Build the assignment from the averaged flow.
    let v_final = aggregate(&v_dest, n);
    let mut route_boardings: HashMap<String, f64> = HashMap::new();
    let mut link_volumes: Vec<TransitLinkVolume> = Vec::with_capacity(n);
    for (idx, meta) in graph.meta.iter().enumerate() {
        if meta.kind == TransitLinkKind::Boarding
            && let Some(route_id) = &meta.route_id
        {
            *route_boardings.entry(route_id.clone()).or_insert(0.0) += v_final[idx];
        }
        link_volumes.push(TransitLinkVolume {
            kind: meta.kind,
            route_id: meta.route_id.clone(),
            from_stop: meta.from_stop,
            to_stop: meta.to_stop,
            volume: v_final[idx],
        });
    }
    let total_boardings = route_boardings.values().sum();

    let mut od_costs: HashMap<(i64, i64), f64> = HashMap::new();
    let mut total_demand = 0.0;
    for &destination in &zone_ids {
        for &origin in &zone_ids {
            if origin == destination {
                continue;
            }
            let demand = od.get(origin, destination);
            if demand <= 0.0 {
                continue;
            }
            total_demand += demand;
            if let Some(node_labels) = labels.get(&destination)
                && let Some(&time) = node_labels.get(&stop_name(origin))
            {
                od_costs.insert((origin, destination), time);
            }
        }
    }

    Ok(CongestedResult {
        assignment: TransitAssignmentResult {
            link_volumes,
            od_costs,
            route_boardings,
            total_boardings,
            total_demand,
        },
        relative_gap,
        iterations,
        gap_history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::od::DenseOdMatrix;
    use crate::transit::route::TransitRoute;

    const EPS: f64 = 1e-9;
    // Loose "the MSA converged" bound for the reported relative gap (the solver itself is run to a much tighter gap tolerance).
    const GAP_TOL: f64 = 1e-3;

    #[test]
    fn test_no_capacity_matches_uncongested() {
        // Two identical uncapacitated lines: the congested solver reduces to
        // the plain 50/50 split, and the gap is zero (the flow is already the
        // uncongested equilibrium).
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("A", vec![1, 2], vec![10.0], 6.0));
        network.add_route(TransitRoute::new("B", vec![1, 2], vec![10.0], 6.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 100.0);

        let result = assign_transit_congested(
            &network,
            &od,
            &TransitAssignmentOptions::default(),
            &CongestedParams::new(60.0),
        )
        .unwrap();
        let a = &result.assignment;
        assert!((a.route_boardings["A"] - 50.0).abs() < EPS);
        assert!((a.route_boardings["B"] - 50.0).abs() < EPS);
        assert!(result.relative_gap <= EPS);
    }

    #[test]
    fn test_ccf_small_example_high_congestion() {
        // Paper 4.1.1 second case: demand A->C raised to 350 (over the 320
        // express capacity). Paper equilibrium: express 260.5, local segments
        // 99.5, A->C time 97.36. Same network and beta = 0.2 as the first case;
        // the 0.01 dwell is folded into the segment times (24.01 / 20.01).
        //
        // Tightly converged (gap < 1e-10) this gives express 260.55 and A->C
        // time 97.42, stable across tolerances. The paper's 260.5 / 97.36 agree
        // to their reported precision: 260.55 rounds to 260.5, and the 0.06 min
        // on the time is the paper's own MSA being stopped at a looser gap plus
        // rounding, not a model difference (the pure-express time matches the
        // paper's 117.04 as well).
        let mut network = TransitNetwork::new();
        network.add_route(
            TransitRoute::new("Express", vec![1, 3], vec![24.01], 60.0 / 16.0).with_capacity(20.0),
        );
        network.add_route(
            TransitRoute::new("Local", vec![1, 2, 3], vec![20.01, 20.01], 10.0).with_capacity(20.0),
        );
        let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
        od.set(1, 2, 10.0);
        od.set(2, 3, 10.0);
        od.set(1, 3, 350.0);
        let result = assign_transit_congested(
            &network,
            &od,
            &TransitAssignmentOptions::default(),
            &CongestedParams::new(60.0).with_limits(20000, 1e-8),
        )
        .unwrap();
        let a = &result.assignment;
        let express = a.route_boardings["Express"];
        assert!(
            (express - 260.5).abs() < 0.5,
            "express boardings {} (paper 260.5)",
            express
        );
        let ac = a.od_costs[&(1, 3)];
        assert!((ac - 97.4).abs() < 0.2, "A->C time {} (paper 97.36)", ac);
        // Express stays within its 320/h capacity.
        assert!(express < 320.0, "express {} exceeds capacity", express);
        assert!(result.relative_gap <= GAP_TOL, "gap {}", result.relative_gap);
    }

    #[test]
    fn test_binding_capacity_saturates_and_gap_converges() {
        // Two parallel lines A -> B with equal nominal service. "Small" has a
        // tight capacity (line capacity (60/6)*30 = 300), "Big" is roomy.
        // With 500 passengers the small line saturates below its capacity and
        // the big line carries the rest, and the CCF gap drives to zero.
        let mut network = TransitNetwork::new();
        network
            .add_route(TransitRoute::new("Big", vec![1, 2], vec![10.0], 6.0).with_capacity(1000.0));
        network
            .add_route(TransitRoute::new("Small", vec![1, 2], vec![10.0], 6.0).with_capacity(30.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 500.0);

        let result = assign_transit_congested(
            &network,
            &od,
            &TransitAssignmentOptions::default(),
            &CongestedParams::new(60.0).with_limits(500, 1e-5),
        )
        .unwrap();
        let a = &result.assignment;
        let big = a.route_boardings["Big"];
        let small = a.route_boardings["Small"];
        assert!((big + small - 500.0).abs() < 1.0, "conservation {} {}", big, small);
        // The capacity-limited line stays below its line capacity and carries
        // less than the roomy line.
        assert!(small < 300.0, "small {} exceeds its capacity", small);
        assert!(big > small, "big {} should exceed small {}", big, small);
        // The gap function drives the MSA to (near) equilibrium.
        assert!(
            result.relative_gap <= GAP_TOL,
            "relative gap {} did not converge",
            result.relative_gap
        );
    }

    #[test]
    fn test_ccf_small_example_reproduced() {
        // Cepeda-Cominetti-Florian (2006), Section 4.1.1. Centroids A=1, B=2,
        // C=3. Express A-C (24 min, 16 bus/h, cap 320/h), Local A-B-C
        // (20+20 min, 6 bus/h, cap 120/h), 20 pax/bus, beta = 0.2. Demand 10
        // A->B, 10 B->C, 100 A->C. Paper equilibrium: express boardings 84.3,
        // A->C time 40.02. Reproduced here with the paper's own beta = 0.2,
        // the effective frequency being f_a = mu (1 - (v_a/(mu*c-v'_a+v_a))^beta),
        // with the 0.01 dwell folded into the segment times (24.01 / 20.01).
        // Tightly converged this gives express 84.26 (which rounds to the
        // paper's 84.3) and A->C time 40.02 exactly.
        let mut network = TransitNetwork::new();
        network.add_route(
            TransitRoute::new("Express", vec![1, 3], vec![24.01], 60.0 / 16.0).with_capacity(20.0),
        );
        network.add_route(
            TransitRoute::new("Local", vec![1, 2, 3], vec![20.01, 20.01], 10.0).with_capacity(20.0),
        );
        let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
        od.set(1, 2, 10.0);
        od.set(2, 3, 10.0);
        od.set(1, 3, 100.0);

        let result = assign_transit_congested(
            &network,
            &od,
            &TransitAssignmentOptions::default(),
            &CongestedParams::new(60.0).with_limits(20000, 1e-8),
        )
        .unwrap();
        let a = &result.assignment;
        let express = a.route_boardings["Express"];
        assert!(
            (express - 84.26).abs() < 0.3,
            "express boardings {} (paper 84.3)",
            express
        );
        let ac = a.od_costs[&(1, 3)];
        assert!((ac - 40.02).abs() < 0.05, "A->C time {} (paper 40.02)", ac);
        assert!(result.relative_gap <= GAP_TOL, "gap {}", result.relative_gap);
    }
}
