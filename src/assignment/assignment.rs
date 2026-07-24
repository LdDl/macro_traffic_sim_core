//! # Assignment Core
//!
//! Volume-delay functions, traits, configuration, and helper functions
//! for traffic assignment algorithms.

use std::any::Any;
use std::collections::HashMap;

use super::error::AssignmentError;
use super::indexed_graph::IndexedGraph;
use super::od_path::OdPath;
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

    /// Downcast support for enum-based dispatch in diagonalization.
    ///
    /// Built-in VDFs (BPR, Conical, Akcelik) are recognized by
    /// `VdfDispatch` and inlined via match. Custom implementations
    /// fall back to vtable dispatch automatically.
    fn as_any(&self) -> &dyn Any;
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

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Conical volume-delay function (Spiess, 1990).
///
/// `t(v) = t0 * (2 + sqrt(a^2*(1 - v/c)^2 + b^2) - a*(1 - v/c) - b)`
///
/// where `b = (2a - 1) / (2(a - 1))` ensures `t(0) = t0`.
///
/// At capacity (`v = c`), travel time is always `2 * t0` regardless of `a`.
/// Requires `alpha > 1`.
///
/// Returns `f64::INFINITY` when capacity is zero or negative.
///
/// Ref: Spiess, H. (1990) "Conical Volume-Delay Functions",
///      Transportation Science, 24(2), 153-158.
///      DOI: 10.1287/trsc.24.2.153
///      https://pubsonline.informs.org/doi/10.1287/trsc.24.2.153
///      http://www.spiess.ch/emme2/conic/conic.html
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::assignment::{ConicalDelayFunction, VolumeDelayFunction};
///
/// let cdf = ConicalDelayFunction::default(); // alpha = 2
///
/// // Free-flow: 10 min, no volume
/// assert!((cdf.travel_time(10.0, 0.0, 1000.0) - 10.0).abs() < 1e-10);
///
/// // At capacity: always 2 * t0
/// assert!((cdf.travel_time(10.0, 1000.0, 1000.0) - 20.0).abs() < 1e-10);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ConicalDelayFunction {
    /// Alpha parameter (must be > 1). Controls steepness.
    pub alpha: f64,
    /// Beta parameter, derived as `(2*alpha - 1) / (2*(alpha - 1))`.
    pub beta: f64,
}

impl ConicalDelayFunction {
    /// Create a conical delay function with the given alpha.
    ///
    /// Beta is computed automatically to ensure `t(0) = t0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::assignment::ConicalDelayFunction;
    ///
    /// let cdf = ConicalDelayFunction::new(5.0);
    /// assert_eq!(cdf.alpha, 5.0);
    /// assert!((cdf.beta - 1.125).abs() < 1e-10);
    /// ```
    pub fn new(alpha: f64) -> Self {
        let beta = (2.0 * alpha - 1.0) / (2.0 * (alpha - 1.0));
        ConicalDelayFunction { alpha, beta }
    }
}

impl Default for ConicalDelayFunction {
    fn default() -> Self {
        Self::new(2.0)
    }
}

impl VolumeDelayFunction for ConicalDelayFunction {
    fn travel_time(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64 {
        if capacity <= 0.0 {
            return f64::INFINITY;
        }
        let a = self.alpha;
        let b = self.beta;
        let r = 1.0 - volume / capacity;
        free_flow_time * (2.0 + (a * a * r * r + b * b).sqrt() - a * r - b)
    }

    fn integral(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64 {
        if capacity <= 0.0 {
            return f64::INFINITY;
        }
        if volume <= 0.0 {
            return 0.0;
        }
        let a = self.alpha;
        let b = self.beta;
        let c = capacity;

        let g = |u: f64| -> f64 {
            let s = (a * a * u * u + b * b).sqrt();
            u * s / 2.0 + b * b / (2.0 * a) * (a * u + s).ln()
        };

        free_flow_time
            * ((2.0 - a - b) * volume
                + a * volume * volume / (2.0 * c)
                + c * (g(1.0) - g(1.0 - volume / c)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Akcelik volume-delay function for signalized intersections.
///
/// `t(v) = t0 + T/4 * ((v/c - 1) + sqrt((v/c - 1)^2 + 8*J*v / (c^2 * T)))`
///
/// where `T` is the analysis period (hours) and `J` is the delay parameter.
/// Accounts for queuing at signalized intersections: delay grows
/// roughly linearly above capacity, unlike BPR's polynomial growth.
///
/// When `J <= 0`, falls back to free-flow time (no signal delay).
///
/// Returns `f64::INFINITY` when capacity is zero or negative.
///
/// Ref: Akcelik, R. (1991) "Travel time functions for transport planning
///      purposes: Davidson's function, its time dependent form and an
///      alternative travel time function",
///      Australian Road Research, 21(3), 49-59.
///      https://www.researchgate.net/publication/242258239_Travel_time_functions_for_transport_planning_purposes_Davidson%27s_function_its_time-dependent_form_and_an_alternative_travel_time_function
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::assignment::{AkcelikDelayFunction, VolumeDelayFunction};
///
/// let akc = AkcelikDelayFunction::default(); // J=0.1, T=0.25h
///
/// // Free-flow: no volume
/// assert!((akc.travel_time(10.0, 0.0, 1000.0) - 10.0).abs() < 1e-10);
///
/// // Over capacity: delay increases
/// assert!(akc.travel_time(10.0, 1500.0, 1000.0) > akc.travel_time(10.0, 1000.0, 1000.0));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct AkcelikDelayFunction {
    /// Delay parameter (must be > 0).
    pub j: f64,
    /// Analysis period in hours (typically 0.25 = 15 min).
    pub t_period: f64,
}

impl AkcelikDelayFunction {
    /// Create an Akcelik delay function with custom parameters.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::assignment::AkcelikDelayFunction;
    ///
    /// let akc = AkcelikDelayFunction::new(0.5, 0.25);
    /// assert_eq!(akc.j, 0.5);
    /// assert_eq!(akc.t_period, 0.25);
    /// ```
    pub fn new(j: f64, t_period: f64) -> Self {
        AkcelikDelayFunction { j, t_period }
    }
}

impl Default for AkcelikDelayFunction {
    fn default() -> Self {
        Self::new(0.1, 0.25)
    }
}

impl VolumeDelayFunction for AkcelikDelayFunction {
    fn travel_time(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64 {
        if capacity <= 0.0 {
            return f64::INFINITY;
        }
        if self.j <= 0.0 {
            return free_flow_time;
        }
        let z = volume / capacity;
        let d = self.t_period;
        let inner = (z - 1.0).powi(2) + 8.0 * self.j * z / (capacity * d);
        free_flow_time + d / 4.0 * ((z - 1.0) + inner.sqrt())
    }

    fn integral(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64 {
        if capacity <= 0.0 {
            return f64::INFINITY;
        }
        if volume <= 0.0 {
            return 0.0;
        }
        if self.j <= 0.0 {
            return free_flow_time * volume;
        }

        let r = volume / capacity;
        let d = self.t_period;
        let k = 8.0 * self.j / (capacity * d);
        let p = k - 2.0;

        let h = |r: f64| -> f64 {
            let s = (r * r + p * r + 1.0).sqrt();
            let disc = 4.0 - p * p;
            (2.0 * r + p) / 4.0 * s + disc / 8.0 * (2.0 * s + 2.0 * r + p).ln()
        };

        capacity * (free_flow_time * r + d / 4.0 * (r * r / 2.0 - r + h(r) - h(0.0)))
    }

    fn as_any(&self) -> &dyn Any {
        self
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
///     store_paths: false,
/// };
/// assert_eq!(config.max_iterations, 500);
/// ```
#[derive(Debug, Clone)]
pub struct AssignmentConfig {
    /// Maximum number of iterations.
    pub max_iterations: usize,
    /// Convergence threshold for relative gap.
    pub convergence_gap: f64,
    /// Store per-OD paths in the result.
    /// When true, [`AssignmentResult::path_flows`] is populated.
    /// GP: multiple paths per OD pair with flow distribution (native).
    /// FW/MSA: one shortest path per OD pair from final costs (post-processing).
    /// Default: false.
    pub store_paths: bool,
}

impl Default for AssignmentConfig {
    fn default() -> Self {
        AssignmentConfig {
            max_iterations: 100,
            convergence_gap: 1e-4,
            store_paths: false,
        }
    }
}

/// Result of a traffic assignment.
///
/// Contains link-level volumes and costs, plus convergence information.
/// For multi-class assignment, `class_volumes` holds per-class link
/// volumes and `link_volumes` holds the PCU-total.
#[derive(Debug, Clone)]
pub struct AssignmentResult {
    /// Volume on each link (PCU-total for multi-class).
    pub link_volumes: HashMap<LinkID, f64>,
    /// Travel time on each link.
    pub link_costs: HashMap<LinkID, f64>,
    /// Number of iterations performed.
    pub iterations: usize,
    /// Final relative gap.
    pub relative_gap: f64,
    /// Whether the algorithm converged within the gap threshold.
    pub converged: bool,
    /// Per-class link volumes (in vehicle units, not PCU).
    /// `None` for single-class assignment.
    /// Key is the class name from [`multiclass::UserClass`].
    pub class_volumes: Option<HashMap<String, HashMap<LinkID, f64>>>,
    /// Per-OD paths with flows.
    /// `None` when `store_paths` is false.
    pub path_flows: Option<Vec<OdPath>>,
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
///
/// ## Warm start
///
/// Pass `initial_volumes` from a previous [`AssignmentResult`] to skip
/// the cold-start all-or-nothing (AON) initialization (Step 0). The algorithm
/// starts from the supplied link volumes and computes costs from them,
/// typically converging in fewer iterations when the OD matrix has
/// changed only slightly (e.g. across feedback iterations).
///
/// Ref:
/// 1) Sheffi, Y. (1985) "Urban Transportation Networks", Ch.5 s5.2 p.119-120
/// => standard Frank-Wolfe Step 0 always starts from free-flow AON
/// 2) Dial, R.B. (2006) Transportation Research Part B, 40(10), 917-936.
/// DOI: 10.1016/j.trb.2006.02.008
/// => first explicit "warm start" in static assignment context.
/// 3) Levin, M.W. and Boyles, S.D. (2015) Transportmetrica B, 3(2), 99-113.
/// DOI: 10.1080/21680566.2014.937788
/// => warm-starting DTA with STA solutions.
/// @todo: I'll consider DTA in future works
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
    /// * `initial_volumes` - Warm-start link volumes from a previous run.
    ///   When `Some`, skips the free-flow AON initialization and starts
    ///   from the given volumes. When `None`, performs the standard
    ///   cold-start (AON at free-flow costs).
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
        initial_volumes: Option<&HashMap<LinkID, f64>>,
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

    #[test]
    fn conical_zero_volume_returns_free_flow() {
        let c = ConicalDelayFunction::default();
        assert!((c.travel_time(10.0, 0.0, 1000.0) - 10.0).abs() < EPS);
    }

    #[test]
    fn conical_at_capacity_doubles_time() {
        for &alpha in &[2.0, 3.0, 5.0, 10.0] {
            let c = ConicalDelayFunction::new(alpha);
            assert!(
                (c.travel_time(10.0, 1000.0, 1000.0) - 20.0).abs() < EPS,
                "Failed for alpha={}",
                alpha
            );
        }
    }

    #[test]
    fn conical_over_capacity() {
        let c = ConicalDelayFunction::default();
        assert!(c.travel_time(10.0, 2000.0, 1000.0) > 20.0);
    }

    #[test]
    fn conical_zero_capacity_returns_infinity() {
        let c = ConicalDelayFunction::default();
        assert_eq!(c.travel_time(10.0, 100.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn conical_integral_zero_volume() {
        let c = ConicalDelayFunction::default();
        assert_eq!(c.integral(10.0, 0.0, 1000.0), 0.0);
    }

    #[test]
    fn conical_integral_zero_capacity_returns_infinity() {
        let c = ConicalDelayFunction::default();
        assert_eq!(c.integral(10.0, 100.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn conical_integral_monotonic() {
        let c = ConicalDelayFunction::default();
        let i1 = c.integral(10.0, 500.0, 1000.0);
        let i2 = c.integral(10.0, 1000.0, 1000.0);
        let i3 = c.integral(10.0, 1500.0, 1000.0);
        assert!(i1 < i2);
        assert!(i2 < i3);
    }

    #[test]
    fn conical_integral_derivative_matches_travel_time() {
        let c = ConicalDelayFunction::default();
        let h = 0.001;
        for &vol in &[100.0, 500.0, 1000.0, 1500.0] {
            let numerical = (c.integral(10.0, vol + h, 1000.0) - c.integral(10.0, vol, 1000.0)) / h;
            let analytical = c.travel_time(10.0, vol, 1000.0);
            assert!(
                (numerical - analytical).abs() < 1e-4,
                "vol={}: numerical={}, analytical={}",
                vol,
                numerical,
                analytical
            );
        }
    }

    #[test]
    fn akcelik_zero_volume_returns_free_flow() {
        let a = AkcelikDelayFunction::default();
        assert!((a.travel_time(10.0, 0.0, 1000.0) - 10.0).abs() < EPS);
    }

    #[test]
    fn akcelik_at_capacity_adds_delay() {
        let a = AkcelikDelayFunction::default();
        let t = a.travel_time(10.0, 1000.0, 1000.0);
        assert!(t > 10.0);
    }

    #[test]
    fn akcelik_over_capacity_grows() {
        let a = AkcelikDelayFunction::default();
        let t1 = a.travel_time(10.0, 1000.0, 1000.0);
        let t2 = a.travel_time(10.0, 1500.0, 1000.0);
        let t3 = a.travel_time(10.0, 2000.0, 1000.0);
        assert!(t1 < t2);
        assert!(t2 < t3);
    }

    #[test]
    fn akcelik_zero_capacity_returns_infinity() {
        let a = AkcelikDelayFunction::default();
        assert_eq!(a.travel_time(10.0, 100.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn akcelik_zero_j_returns_free_flow() {
        let a = AkcelikDelayFunction::new(0.0, 0.25);
        assert_eq!(a.travel_time(10.0, 500.0, 1000.0), 10.0);
        assert_eq!(a.travel_time(10.0, 2000.0, 1000.0), 10.0);
    }

    #[test]
    fn akcelik_integral_zero_volume() {
        let a = AkcelikDelayFunction::default();
        assert_eq!(a.integral(10.0, 0.0, 1000.0), 0.0);
    }

    #[test]
    fn akcelik_integral_zero_capacity_returns_infinity() {
        let a = AkcelikDelayFunction::default();
        assert_eq!(a.integral(10.0, 100.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn akcelik_integral_monotonic() {
        let a = AkcelikDelayFunction::default();
        let i1 = a.integral(10.0, 500.0, 1000.0);
        let i2 = a.integral(10.0, 1000.0, 1000.0);
        let i3 = a.integral(10.0, 1500.0, 1000.0);
        assert!(i1 < i2);
        assert!(i2 < i3);
    }

    #[test]
    fn akcelik_integral_derivative_matches_travel_time() {
        let a = AkcelikDelayFunction::default();
        let h = 0.001;
        for &vol in &[100.0, 500.0, 1000.0, 1500.0] {
            let numerical = (a.integral(10.0, vol + h, 1000.0) - a.integral(10.0, vol, 1000.0)) / h;
            let analytical = a.travel_time(10.0, vol, 1000.0);
            assert!(
                (numerical - analytical).abs() < 1e-4,
                "vol={}: numerical={}, analytical={}",
                vol,
                numerical,
                analytical
            );
        }
    }

    #[test]
    fn akcelik_integral_zero_j() {
        let a = AkcelikDelayFunction::new(0.0, 0.25);
        let i = a.integral(10.0, 500.0, 1000.0);
        assert!((i - 10.0 * 500.0).abs() < EPS);
    }

}
