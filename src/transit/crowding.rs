//! # Crowded transit assignment
//!
//! Extends the optimal strategies assignment with passenger crowding: as
//! the boarding flow on a line approaches its capacity, the line's
//! effective frequency drops and its waiting time rises, so the line loses
//! share to less crowded alternatives. This is the congested extension of
//! the common-lines / optimal-strategies model.
//!
//! ## The model (De Cea and Fernandez, 1993)
//!
//! De Cea and Fernandez group the common (attractive) lines between two
//! transfer nodes into a "route section" and treat each transit stop as a
//! queueing system (p. 135): "passengers experience waiting times that
//! depend on the total capacity of the attractive set of lines considered
//! and on the total number of passengers using that set of lines [...] as
//! the number of passengers trying to use a given service approaches its
//! capacity, waiting times increase". A passenger "will board the first
//! vehicle belonging to that set that has a place available".
//!
//! Rather than a strict hard capacity (which "lead[s] to feasibility
//! problems and numerical instability", p. 137) they use, like the BPR road
//! functions, an "unbounded increasing convex volume-delay function" - so a
//! line can be overloaded when demand is too high (p. 138). The route
//! section cost is `c_s = t_s + alpha / f_s + beta * ((V_s + V_hat) / K_s)^n`
//! (their Eq. 8): in-vehicle time `t_s`, the uncongested waiting `alpha /
//! f_s` (`f_s = sum f_l` the combined nominal frequency), plus a BPR power
//! term in the volume-to-capacity ratio, with `K_s = sum x_l` the section's
//! practical capacity.
//!
//! They fold that congestion back into an "effective frequency" (their
//! Section 3.1): uncongested, "the effective frequency of a line is equal
//! to its nominal frequency"; as congestion grows "the effective
//! frequencies will be reduced" and always "`f'_l <= f_l`" (p. 139). The
//! waiting index is `w_l = alpha / f_l + phi_l(v / K)`, where `phi_l` is any
//! monotonically increasing function (Eq. 14); taking the BPR power form
//! `phi_l = beta_l * (v / K)^n` (Eq. 15, one possible choice) and the
//! effective frequency `f'_l = alpha / w_l` (Eq. 16) gives
//!
//! ```text
//! f_eff = f / (1 + (beta_l * f / alpha) * (load / K)^n)
//! ```
//!
//! which is exactly the [`crowd_network`] transform used here. `K = f * kappa`
//! is the line capacity per period (`kappa` the per-vehicle `capacity`); our
//! `CrowdingParams::alpha` bundles the constant `beta_l * f / alpha`, and our
//! `beta` is the BPR exponent `n`.
//!
//! ## Why the solver is untouched
//!
//! Cominetti and Correa (2001) put the same idea on a rigorous footing.
//! Their waiting times "obey an inverse additive law of the form
//! `1/W_s(v) = sum_{i in s} 1/W_i(v)`" (p. 250); with `W_i = 1 / f_i` that
//! is exactly the Spiess-Florian combined-frequency formula `f_s = sum f_i`,
//! now with a flow-dependent effective frequency `f_i(v)`. So crowding needs
//! no change to the hyperpaths solver: it is an outer method-of-successive-
//! averages loop that rescales each line's boarding frequency by its load
//! and re-runs the unchanged optimal-strategies assignment, which keeps
//! reproducing the paper exactly.
//!
//! ## Simplification (v1)
//!
//! The full De Cea-Fernandez cost couples route sections that share lines:
//! the competing and through flows `V_hat` of Eq. 7 make the Jacobian
//! asymmetric. Here the load of a line is taken as its own boardings only
//! (`V_hat = 0`), so each capacitated line crowds on its own volume; the
//! cross-section coupling is left for a later revision.
//!
//! - De Cea, J. and Fernandez, E. (1993) "Transit Assignment for Congested
//!   Public Transport Systems: An Equilibrium Model". Transportation
//!   Science 27(2), 133-147. DOI: <https://doi.org/10.1287/trsc.27.2.133>
//! - Cominetti, R. and Correa, J. (2001) "Common-Lines and Passenger
//!   Assignment in Congested Transit Networks". Transportation Science
//!   35(3), 250-267. DOI: <https://doi.org/10.1287/trsc.35.3.250.10154>

use std::collections::HashMap;

use crate::od::OdMatrix;
use crate::transit::assignment::{
    TransitAssignmentOptions, TransitAssignmentResult, assign_transit_with_options,
};
use crate::transit::error::TransitError;
use crate::transit::route::TransitNetwork;

/// Parameters of the crowding (congested transit) assignment.
#[derive(Debug, Clone, Copy)]
pub struct CrowdingParams {
    /// Analysis period, in the same time unit as the route headways, used to
    /// turn a per-vehicle capacity into a line capacity per period:
    /// `line_capacity = (analysis_period / headway) * capacity`. Must match
    /// the period of the OD demand (both are passengers per period).
    pub analysis_period: f64,
    /// Steepness coefficient of the effective-frequency law
    /// `f_eff = f / (1 + alpha * (load / line_capacity)^beta)`. At a load
    /// equal to the line capacity the waiting time is multiplied by
    /// `1 + alpha`. Default `1.0`.
    pub alpha: f64,
    /// Exponent of the same law - higher values keep the line uncrowded
    /// until the load is close to capacity, then bite sharply. Default
    /// `4.0` (a BPR-like shape).
    pub beta: f64,
    /// Maximum number of outer averaging iterations. Default `20`.
    pub max_iterations: usize,
    /// Convergence tolerance on the largest per-line load change between
    /// iterations. Default `1e-3`.
    pub tolerance: f64,
}

impl CrowdingParams {
    /// Crowding parameters for the given analysis period, with the default
    /// law (`alpha = 1.0`, `beta = 4.0`), 20 iterations and tolerance
    /// `1e-3`.
    pub fn new(analysis_period: f64) -> Self {
        CrowdingParams {
            analysis_period,
            alpha: 1.0,
            beta: 4.0,
            max_iterations: 20,
            tolerance: 1e-3,
        }
    }

    /// Set the effective-frequency law coefficients.
    pub fn with_law(mut self, alpha: f64, beta: f64) -> Self {
        self.alpha = alpha;
        self.beta = beta;
        self
    }

    /// Set the maximum number of outer iterations.
    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = max_iterations;
        self
    }
}

/// Multiplies the headway of every capacitated line by its crowding factor
/// `1 + alpha * (load / line_capacity)^beta`, so its effective frequency
/// `f_eff = f / factor` drops with the load. Uncapacitated lines are left
/// unchanged.
fn crowd_network(
    network: &TransitNetwork,
    loads: &HashMap<String, f64>,
    params: &CrowdingParams,
) -> TransitNetwork {
    let mut out = network.clone();
    for route in out.routes.iter_mut() {
        let Some(capacity) = route.capacity else {
            continue;
        };
        let line_capacity = (params.analysis_period / route.headway) * capacity;
        if line_capacity <= 0.0 {
            continue;
        }
        let load = loads.get(&route.id).copied().unwrap_or(0.0);
        let ratio = load / line_capacity;
        let factor = 1.0 + params.alpha * ratio.powf(params.beta);
        // Effective frequency f_eff = f / factor  <=>  effective headway =
        // headway * factor (waiting rises, line-choice share falls).
        route.headway *= factor;
    }
    out
}

/// Runs the crowded (congested) transit assignment.
///
/// Wraps [`assign_transit_with_options`] in an outer method-of-successive-
/// averages loop: each iteration scales the capacitated lines' frequencies
/// by their current load (via [`crowd_network`]), re-solves the unchanged
/// optimal strategies problem, and averages the line loads. It converges to
/// a state where each line's load is consistent with the effective
/// frequency that load implies.
///
/// With no capacitated routes this is a single plain assignment.
///
/// # Arguments
///
/// * `network` - Transit routes (with per-vehicle `capacity` set on the
///   lines that should crowd) and walk links
/// * `od` - Transit OD matrix (passengers per the crowding analysis period)
/// * `options` - Base assignment options (waiting factor, penalties, dwell)
/// * `crowding` - Crowding law and iteration control
///
/// # Errors
///
/// Returns a [`TransitError`] when the network or options are invalid, or
/// when `crowding.analysis_period` is not strictly positive.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
/// use macro_traffic_sim_core::transit::{
///     assign_transit_crowded, CrowdingParams, TransitAssignmentOptions, TransitNetwork,
///     TransitRoute,
/// };
///
/// // Two parallel lines A -> B; the small one seats few and crowds.
/// let mut network = TransitNetwork::new();
/// network.add_route(TransitRoute::new("Big", vec![1, 2], vec![10.0], 10.0).with_capacity(500.0));
/// network.add_route(TransitRoute::new("Small", vec![1, 2], vec![10.0], 10.0).with_capacity(20.0));
///
/// let mut od = DenseOdMatrix::new(vec![1, 2]);
/// od.set(1, 2, 1000.0);
///
/// let result = assign_transit_crowded(
///     &network,
///     &od,
///     &TransitAssignmentOptions::default(),
///     &CrowdingParams::new(60.0),
/// )
/// .unwrap();
/// // The small line, capped, carries less than the big one despite equal
/// // nominal frequencies.
/// assert!(result.route_boardings["Small"] < result.route_boardings["Big"]);
/// ```
pub fn assign_transit_crowded(
    network: &TransitNetwork,
    od: &dyn OdMatrix,
    options: &TransitAssignmentOptions,
    crowding: &CrowdingParams,
) -> Result<TransitAssignmentResult, TransitError> {
    if crowding.analysis_period.is_nan() || crowding.analysis_period <= 0.0 {
        return Err(TransitError::InvalidAnalysisPeriod {
            value: crowding.analysis_period,
        });
    }

    // No capacitated line -> plain assignment, no outer loop.
    let has_capacity = network.routes.iter().any(|r| r.capacity.is_some());
    let mut result = assign_transit_with_options(network, od, options)?;
    if !has_capacity {
        return Ok(result);
    }

    let mut loads = result.route_boardings.clone();
    for n in 1..=crowding.max_iterations {
        let effective = crowd_network(network, &loads, crowding);
        result = assign_transit_with_options(&effective, od, options)?;

        // Method of successive averages on the per-line loads, tracking the
        // largest change for the convergence test.
        let step = 1.0 / (n as f64 + 1.0);
        let mut max_change: f64 = 0.0;
        let route_ids: Vec<String> = network.routes.iter().map(|r| r.id.clone()).collect();
        for id in route_ids {
            let solved = result.route_boardings.get(&id).copied().unwrap_or(0.0);
            let current = loads.entry(id).or_insert(0.0);
            let updated = *current + step * (solved - *current);
            max_change = max_change.max((updated - *current).abs());
            *current = updated;
        }
        if max_change < crowding.tolerance {
            break;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::od::DenseOdMatrix;
    use crate::transit::route::TransitRoute;

    const EPS: f64 = 1e-9;

    #[test]
    fn test_no_capacity_is_plain_assignment() {
        // Two identical uncapacitated lines split 50/50, exactly as the
        // uncrowded solver would.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("A", vec![1, 2], vec![10.0], 6.0));
        network.add_route(TransitRoute::new("B", vec![1, 2], vec![10.0], 6.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 100.0);

        let result = assign_transit_crowded(
            &network,
            &od,
            &TransitAssignmentOptions::default(),
            &CrowdingParams::new(60.0),
        )
        .unwrap();
        assert!((result.route_boardings["A"] - 50.0).abs() < EPS);
        assert!((result.route_boardings["B"] - 50.0).abs() < EPS);
    }

    #[test]
    fn test_crowding_shifts_flow_off_the_small_line() {
        // Two parallel lines, equal nominal frequency, but line "Small" has
        // a tiny capacity. Without crowding they split 50/50; with crowding
        // the small line fills, its effective frequency drops, and the big
        // line takes the larger share.
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("Big", vec![1, 2], vec![10.0], 10.0).with_capacity(1000.0));
        network.add_route(TransitRoute::new("Small", vec![1, 2], vec![10.0], 10.0).with_capacity(30.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 600.0);

        let result = assign_transit_crowded(
            &network,
            &od,
            &TransitAssignmentOptions::default(),
            &CrowdingParams::new(60.0),
        )
        .unwrap();
        let big = result.route_boardings["Big"];
        let small = result.route_boardings["Small"];
        assert!((big + small - 600.0).abs() < 1e-3, "conservation: {} + {}", big, small);
        assert!(big > small, "big {} should exceed small {}", big, small);
        // The uncrowded split would be 300/300; crowding pushed flow off Small.
        assert!(small < 300.0);
    }

    #[test]
    fn test_invalid_analysis_period() {
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("A", vec![1, 2], vec![10.0], 6.0).with_capacity(100.0));
        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 1.0);
        for bad in [0.0, -1.0, f64::NAN] {
            assert!(matches!(
                assign_transit_crowded(
                    &network,
                    &od,
                    &TransitAssignmentOptions::default(),
                    &CrowdingParams::new(bad),
                ),
                Err(TransitError::InvalidAnalysisPeriod { .. })
            ));
        }
    }
}
