//! # Assignment Core
//!
//! Volume-delay functions, traits, configuration, and helper functions
//! for traffic assignment algorithms.

use std::collections::HashMap;

use super::error::AssignmentError;
use super::indexed_graph::IndexedGraph;
use crate::gmns::meso::network::Network;
use crate::gmns::types::LinkID;
use crate::od::OdMatrix;

const EPS_BETA: f64 = 1e-12;

/// Trait for volume-delay functions.
///
/// Models the relationship between link volume and travel time.
/// Implementations must also provide the integral (needed for the
/// Beckmann objective in Frank-Wolfe).
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::assignment::{BprFunction, VolumeDelayFunction};
///
/// let bpr = BprFunction::new(0.15, 4.0);
///
/// // Zero volume -> free-flow time
/// assert_eq!(bpr.travel_time(5.0, 0.0, 500.0), 5.0);
///
/// // Integral at zero volume is zero
/// assert_eq!(bpr.integral(5.0, 0.0, 500.0), 0.0);
/// ```
pub trait VolumeDelayFunction {
    /// Compute travel time given free-flow time, volume, and capacity.
    fn travel_time(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64;

    /// Compute the integral of the VDF from 0 to volume.
    /// Needed for the Beckmann objective function in Frank-Wolfe.
    fn integral(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64;
}

/// BPR (Bureau of Public Roads) volume-delay function.
///
/// `t(x) = t0 * (1 + alpha * (x / c) ^ beta)`
///
/// Returns `f64::INFINITY` when capacity is zero or negative.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::assignment::{BprFunction, VolumeDelayFunction};
///
/// // Default: alpha=0.15, beta=4.0
/// let bpr = BprFunction::default();
/// assert_eq!(bpr.alpha, 0.15);
/// assert_eq!(bpr.beta, 4.0);
///
/// // Custom parameters
/// let bpr = BprFunction::new(0.20, 5.0);
/// assert_eq!(bpr.alpha, 0.20);
/// ```
///
/// ```
/// use macro_traffic_sim_core::assignment::{BprFunction, VolumeDelayFunction};
///
/// let bpr = BprFunction::default();
///
/// // Volume/capacity = 0.5 -> mild delay
/// let t = bpr.travel_time(10.0, 500.0, 1000.0);
/// // t = 10 * (1 + 0.15 * 0.5^4) = 10 * 1.009375 = 10.09375
/// assert!((t - 10.09375).abs() < 1e-10);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BprFunction {
    /// Alpha parameter.
    pub alpha: f64,
    /// Beta parameter.
    pub beta: f64,
    // Cached integer beta for fast exponentiation.
    // Some(n) when beta is close to a positive integer, None otherwise.
    beta_int: Option<u32>,
}

impl BprFunction {
    /// Create a BPR function with custom parameters.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::assignment::BprFunction;
    ///
    /// let bpr = BprFunction::new(0.25, 5.0);
    /// assert_eq!(bpr.alpha, 0.25);
    /// assert_eq!(bpr.beta, 5.0);
    /// ```
    pub fn new(alpha: f64, beta: f64) -> Self {
        BprFunction {
            alpha,
            beta,
            beta_int: Self::detect_integer_beta(beta),
        }
    }

    // If beta is within EPS_BETA of a positive integer, return that integer.
    fn detect_integer_beta(beta: f64) -> Option<u32> {
        if beta < 0.0 || !beta.is_finite() {
            return None;
        }
        let rounded = beta.round();
        if (beta - rounded).abs() < EPS_BETA && rounded >= 1.0 && rounded <= 20.0 {
            Some(rounded as u32)
        } else {
            None
        }
    }
}

impl Default for BprFunction {
    fn default() -> Self {
        BprFunction::new(0.15, 4.0)
    }
}

impl BprFunction {
    // Raise ratio to power beta using integer exponentiation when possible.
    // For integer beta values (e.g. standard beta=4), uses powi() which
    // I believe should be faster than powf().
    #[inline]
    fn ratio_pow(&self, ratio: f64) -> f64 {
        match self.beta_int {
            Some(n) => ratio.powi(n as i32),
            None => ratio.powf(self.beta),
        }
    }

    // Raise ratio to power (beta + 1).
    #[inline]
    fn ratio_pow_plus1(&self, ratio: f64) -> f64 {
        match self.beta_int {
            Some(n) => ratio.powi(n as i32 + 1),
            None => ratio.powf(self.beta + 1.0),
        }
    }
}

impl VolumeDelayFunction for BprFunction {
    fn travel_time(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64 {
        if capacity <= 0.0 {
            return f64::INFINITY;
        }
        free_flow_time * (1.0 + self.alpha * self.ratio_pow(volume / capacity))
    }

    fn integral(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64 {
        if capacity <= 0.0 {
            return f64::INFINITY;
        }
        if volume <= 0.0 {
            return 0.0;
        }
        let ratio = volume / capacity;
        free_flow_time
            * (volume + self.alpha * capacity * self.ratio_pow_plus1(ratio) / (self.beta + 1.0))
    }
}

/// Configuration for traffic assignment algorithms.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::assignment::AssignmentConfig;
///
/// // Default: 100 iterations, gap 1e-4
/// let config = AssignmentConfig::default();
/// assert_eq!(config.max_iterations, 100);
/// assert_eq!(config.convergence_gap, 1e-4);
/// ```
///
/// ```
/// use macro_traffic_sim_core::assignment::AssignmentConfig;
///
/// let config = AssignmentConfig {
///     max_iterations: 500,
///     convergence_gap: 1e-6,
/// };
/// assert_eq!(config.max_iterations, 500);
/// ```
#[derive(Debug, Clone)]
pub struct AssignmentConfig {
    /// Maximum number of iterations.
    pub max_iterations: usize,
    /// Convergence threshold for relative gap.
    pub convergence_gap: f64,
}

impl Default for AssignmentConfig {
    fn default() -> Self {
        AssignmentConfig {
            max_iterations: 100,
            convergence_gap: 1e-4,
        }
    }
}

/// Result of a traffic assignment.
///
/// Contains link-level volumes and costs, plus convergence information.
#[derive(Debug, Clone)]
pub struct AssignmentResult {
    /// Volume on each link.
    pub link_volumes: HashMap<LinkID, f64>,
    /// Travel time on each link.
    pub link_costs: HashMap<LinkID, f64>,
    /// Number of iterations performed.
    pub iterations: usize,
    /// Final relative gap.
    pub relative_gap: f64,
    /// Whether the algorithm converged within the gap threshold.
    pub converged: bool,
}

/// Trait for traffic assignment methods.
///
/// All implementations find User Equilibrium (Wardrop's first principle)
/// by iteratively adjusting link volumes until no traveller can
/// unilaterally improve their travel time.
///
/// Three methods are available:
/// - [`FrankWolfe`](super::FrankWolfe) - convex combinations with bisection line search
/// - [`Msa`](super::Msa) - fixed step size 1/n
/// - [`GradientProjection`](super::GradientProjection) - path-based with gradient flow shifting
pub trait AssignmentMethod {
    /// Perform traffic assignment.
    ///
    /// # Arguments
    /// * `network` - The macroscopic network.
    /// * `graph` - Pre-built indexed graph (CSR). Built once and reused
    ///   across feedback iterations to avoid redundant construction.
    /// * `od_matrix` - Origin-destination demand matrix.
    /// * `vdf` - Volume-delay function.
    /// * `config` - Algorithm configuration.
    ///
    /// # Returns
    /// An [`AssignmentResult`] with link volumes, costs, and convergence info.
    fn assign(
        &self,
        network: &Network,
        graph: &IndexedGraph,
        od_matrix: &dyn OdMatrix,
        vdf: &dyn VolumeDelayFunction,
        config: &AssignmentConfig,
    ) -> Result<AssignmentResult, AssignmentError>;
}

/// Compute link costs from volumes using the VDF.
///
/// Iterates over all links in the network and applies the volume-delay
/// function to compute the travel time for each link.
pub fn compute_link_costs(
    network: &Network,
    volumes: &HashMap<LinkID, f64>,
    vdf: &dyn VolumeDelayFunction,
) -> HashMap<LinkID, f64> {
    let mut costs = HashMap::with_capacity(network.links.len());
    compute_link_costs_into(network, volumes, vdf, &mut costs);
    costs
}

/// Compute link costs into a pre-allocated HashMap, avoiding reallocation.
///
/// Same as [`compute_link_costs`] but reuses the provided map.
/// Call this in hot loops to avoid allocating a new HashMap per iteration.
pub fn compute_link_costs_into(
    network: &Network,
    volumes: &HashMap<LinkID, f64>,
    vdf: &dyn VolumeDelayFunction,
    out: &mut HashMap<LinkID, f64>,
) {
    for (&link_id, link) in &network.links {
        let volume = volumes.get(&link_id).copied().unwrap_or(0.0);
        let ff_time = link.get_free_flow_time_hours();
        let capacity = link.get_total_capacity();
        let cost = vdf.travel_time(ff_time, volume, capacity);
        *out.entry(link_id).or_insert(0.0) = cost;
    }
}

/// Compute the Beckmann objective function value.
///
/// ```text
/// Z = sum_a integral_0^{x_a} t_a(w) dw
/// ```
///
/// Used by Frank-Wolfe for line search.
pub fn beckmann_objective(
    network: &Network,
    volumes: &HashMap<LinkID, f64>,
    vdf: &dyn VolumeDelayFunction,
) -> f64 {
    let mut objective = 0.0;
    for (&link_id, link) in &network.links {
        let volume = volumes.get(&link_id).copied().unwrap_or(0.0);
        let ff_time = link.get_free_flow_time_hours();
        let capacity = link.get_total_capacity();
        objective += vdf.integral(ff_time, volume, capacity);
    }
    objective
}

/// Compute the relative gap for convergence checking.
///
/// ```text
/// gap = (sum(x_a * t_a) - sum(y_a * t_a)) / sum(x_a * t_a)
/// ```
///
/// where `x_a` are current volumes and `y_a` are all-or-nothing
/// (auxiliary) volumes at current costs. A gap of zero means
/// User Equilibrium has been reached.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use macro_traffic_sim_core::assignment::compute_relative_gap;
///
/// let volumes = HashMap::from([(1, 100.0), (2, 50.0)]);
/// let costs = HashMap::from([(1, 2.0), (2, 4.0)]);
///
/// // Auxiliary with lower total weighted cost
/// let aux = HashMap::from([(1, 120.0), (2, 30.0)]);
///
/// let gap = compute_relative_gap(&volumes, &costs, &aux);
/// // numerator = 100*2 + 50*4 = 400
/// // denominator = 120*2 + 30*4 = 360
/// // gap = (400 - 360) / 400 = 0.1
/// assert!((gap - 0.1).abs() < 1e-10);
/// ```
pub fn compute_relative_gap(
    volumes: &HashMap<LinkID, f64>,
    costs: &HashMap<LinkID, f64>,
    aux_volumes: &HashMap<LinkID, f64>,
) -> f64 {
    let numerator: f64 = volumes
        .iter()
        .map(|(&link_id, &vol)| {
            let cost = costs.get(&link_id).copied().unwrap_or(0.0);
            vol * cost
        })
        .sum();

    let denominator: f64 = aux_volumes
        .iter()
        .map(|(&link_id, &vol)| {
            let cost = costs.get(&link_id).copied().unwrap_or(0.0);
            vol * cost
        })
        .sum();

    if numerator <= 0.0 {
        return 0.0;
    }

    (numerator - denominator) / numerator
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-10;

    #[test]
    fn bpr_zero_volume_returns_free_flow() {
        let bpr = BprFunction::default();
        assert_eq!(bpr.travel_time(10.0, 0.0, 1000.0), 10.0);
    }

    #[test]
    fn bpr_at_capacity() {
        let bpr = BprFunction::default();
        // t = 10 * (1 + 0.15 * 1^4) = 11.5
        let t = bpr.travel_time(10.0, 1000.0, 1000.0);
        assert!((t - 11.5).abs() < EPS);
    }

    #[test]
    fn bpr_half_capacity() {
        let bpr = BprFunction::default();
        // t = 10 * (1 + 0.15 * 0.5^4) = 10 * 1.009375 = 10.09375
        let t = bpr.travel_time(10.0, 500.0, 1000.0);
        assert!((t - 10.09375).abs() < EPS);
    }

    #[test]
    fn bpr_double_capacity() {
        let bpr = BprFunction::default();
        // t = 10 * (1 + 0.15 * 2^4) = 10 * (1 + 0.15 * 16) = 10 * 3.4 = 34.0
        let t = bpr.travel_time(10.0, 2000.0, 1000.0);
        assert!((t - 34.0).abs() < EPS);
    }

    #[test]
    fn bpr_zero_capacity_returns_infinity() {
        let bpr = BprFunction::default();
        assert_eq!(bpr.travel_time(10.0, 100.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn bpr_negative_capacity_returns_infinity() {
        let bpr = BprFunction::default();
        assert_eq!(bpr.travel_time(10.0, 100.0, -500.0), f64::INFINITY);
    }

    #[test]
    fn bpr_custom_params() {
        let bpr = BprFunction::new(0.5, 2.0);
        // t = 10 * (1 + 0.5 * (500/1000)^2) = 10 * (1 + 0.5 * 0.25) = 10 * 1.125 = 11.25
        let t = bpr.travel_time(10.0, 500.0, 1000.0);
        assert!((t - 11.25).abs() < EPS);
    }

    #[test]
    fn bpr_integral_zero_volume() {
        let bpr = BprFunction::default();
        assert_eq!(bpr.integral(10.0, 0.0, 1000.0), 0.0);
    }

    #[test]
    fn bpr_integral_at_capacity() {
        let bpr = BprFunction::default();
        // integral = t0 * (x + alpha * c * (x/c)^(beta+1) / (beta+1))
        // = 10 * (1000 + 0.15 * 1000 * 1^5 / 5) = 10 * (1000 + 30) = 10300
        let integral = bpr.integral(10.0, 1000.0, 1000.0);
        assert!((integral - 10300.0).abs() < 1e-6);
    }

    #[test]
    fn bpr_integral_zero_capacity_returns_infinity() {
        let bpr = BprFunction::default();
        assert_eq!(bpr.integral(10.0, 100.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn bpr_integral_monotonic() {
        let bpr = BprFunction::default();
        let i1 = bpr.integral(10.0, 500.0, 1000.0);
        let i2 = bpr.integral(10.0, 1000.0, 1000.0);
        let i3 = bpr.integral(10.0, 1500.0, 1000.0);
        assert!(i1 < i2);
        assert!(i2 < i3);
    }

    #[test]
    fn relative_gap_equal_volumes_is_zero() {
        let volumes = HashMap::from([(1, 100.0), (2, 200.0)]);
        let costs = HashMap::from([(1, 5.0), (2, 10.0)]);
        let gap = compute_relative_gap(&volumes, &costs, &volumes);
        assert_eq!(gap, 0.0);
    }

    #[test]
    fn relative_gap_known_value() {
        let volumes = HashMap::from([(1, 100.0), (2, 50.0)]);
        let costs = HashMap::from([(1, 2.0), (2, 4.0)]);
        let aux = HashMap::from([(1, 120.0), (2, 30.0)]);
        // numerator = 100*2 + 50*4 = 400
        // denominator = 120*2 + 30*4 = 360
        // gap = (400 - 360) / 400 = 0.1
        let gap = compute_relative_gap(&volumes, &costs, &aux);
        assert!((gap - 0.1).abs() < EPS);
    }

    #[test]
    fn relative_gap_zero_volumes_returns_zero() {
        let volumes: HashMap<LinkID, f64> = HashMap::new();
        let costs = HashMap::from([(1, 5.0)]);
        let aux = HashMap::from([(1, 100.0)]);
        let gap = compute_relative_gap(&volumes, &costs, &aux);
        assert_eq!(gap, 0.0);
    }
}
