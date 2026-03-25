//! # Impedance Functions
//!
//! Functions that model the effect of travel cost on trip-making.
//! Used by the gravity model in trip distribution.
//!
//! Higher impedance values indicate a more attractive (lower-cost) trip.
//! All implementations return non-negative values for positive costs.
//!
//! ## Examples
//!
//! ### Comparing impedance functions
//!
//! ```
//! use macro_traffic_sim_core::trip_distribution::{
//!     ExponentialImpedance, PowerImpedance, CombinedImpedance, ImpedanceFunction,
//! };
//!
//! let exp = ExponentialImpedance::new(2.0);
//! let pow = PowerImpedance::new(1.5);
//! let comb = CombinedImpedance::new(1.0, 1.0);
//!
//! let cost = 0.5;
//! // All produce positive values for positive cost
//! assert!(exp.compute(cost) > 0.0);
//! assert!(pow.compute(cost) > 0.0);
//! assert!(comb.compute(cost) > 0.0);
//!
//! // Higher cost => lower impedance
//! assert!(exp.compute(1.0) < exp.compute(0.5));
//! assert!(pow.compute(2.0) < pow.compute(1.0));
//! ```

/// Trait for impedance (deterrence) functions.
///
/// `f(cost)` models how travel cost reduces the likelihood of a trip.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::{ExponentialImpedance, ImpedanceFunction};
///
/// let f = ExponentialImpedance::new(1.0);
/// // f(0) = exp(0) = 1.0
/// assert!((f.compute(0.001) - 1.0).abs() < 0.01);
/// // f(large) -> 0
/// assert!(f.compute(10.0) < 0.001);
/// ```
pub trait ImpedanceFunction {
    /// Compute the impedance value for a given cost.
    ///
    /// # Arguments
    /// * `cost` - Travel cost (time in hours, distance, generalized cost, etc.).
    ///
    /// # Returns
    /// A non-negative impedance value. Higher means more attractive.
    fn compute(&self, cost: f64) -> f64;
}

/// Exponential impedance function: `f(c) = exp(-beta * c)`.
///
/// Common choice when cost is travel time.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::{ExponentialImpedance, ImpedanceFunction};
///
/// let f = ExponentialImpedance::new(2.0);
///
/// // exp(-2 * 0) = 1.0
/// assert_eq!(f.compute(0.0), 1.0);
///
/// // exp(-2 * 1) ~ 0.1353
/// assert!((f.compute(1.0) - (-2.0_f64).exp()).abs() < 1e-10);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ExponentialImpedance {
    /// Decay parameter. Must be positive.
    pub beta: f64,
}

impl ExponentialImpedance {
    /// Create a new exponential impedance function.
    ///
    /// # Arguments
    /// * `beta` - Decay parameter (positive). Larger values mean sharper drop-off.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
    ///
    /// let f = ExponentialImpedance::new(3.0);
    /// assert_eq!(f.beta, 3.0);
    /// ```
    pub fn new(beta: f64) -> Self {
        ExponentialImpedance { beta }
    }
}

impl ImpedanceFunction for ExponentialImpedance {
    fn compute(&self, cost: f64) -> f64 {
        (-self.beta * cost).exp()
    }
}

/// Power impedance function: `f(c) = c^(-alpha)`.
///
/// Common choice when cost is distance.
/// Returns `f64::MAX` for zero or negative costs.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::{PowerImpedance, ImpedanceFunction};
///
/// let f = PowerImpedance::new(2.0);
///
/// // 1^(-2) = 1.0
/// assert_eq!(f.compute(1.0), 1.0);
///
/// // 2^(-2) = 0.25
/// assert_eq!(f.compute(2.0), 0.25);
///
/// // Zero cost returns MAX
/// assert_eq!(f.compute(0.0), f64::MAX);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct PowerImpedance {
    /// Exponent parameter. Must be positive.
    pub alpha: f64,
}

impl PowerImpedance {
    /// Create a new power impedance function.
    ///
    /// # Arguments
    /// * `alpha` - Exponent (positive). Larger values mean sharper drop-off with distance.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::trip_distribution::PowerImpedance;
    ///
    /// let f = PowerImpedance::new(1.5);
    /// assert_eq!(f.alpha, 1.5);
    /// ```
    pub fn new(alpha: f64) -> Self {
        PowerImpedance { alpha }
    }
}

impl ImpedanceFunction for PowerImpedance {
    fn compute(&self, cost: f64) -> f64 {
        if cost <= 0.0 {
            return f64::MAX;
        }
        cost.powf(-self.alpha)
    }
}

/// Combined impedance function: `f(c) = c^(-alpha) * exp(-beta * c)`.
///
/// Combines both power and exponential decay for more flexible calibration.
/// Returns `f64::MAX` for zero or negative costs.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::{CombinedImpedance, ImpedanceFunction};
///
/// let f = CombinedImpedance::new(1.0, 1.0);
///
/// // f(1) = 1^(-1) * exp(-1) = exp(-1) ~ 0.3679
/// let expected = (-1.0_f64).exp();
/// assert!((f.compute(1.0) - expected).abs() < 1e-10);
///
/// // f(2) = 2^(-1) * exp(-2) = 0.5 * exp(-2) ~ 0.0677
/// let expected = 0.5 * (-2.0_f64).exp();
/// assert!((f.compute(2.0) - expected).abs() < 1e-10);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct CombinedImpedance {
    /// Power exponent.
    pub alpha: f64,
    /// Exponential decay parameter.
    pub beta: f64,
}

impl CombinedImpedance {
    /// Create a new combined impedance function.
    ///
    /// # Arguments
    /// * `alpha` - Power exponent.
    /// * `beta` - Exponential decay parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::trip_distribution::CombinedImpedance;
    ///
    /// let f = CombinedImpedance::new(1.5, 2.0);
    /// assert_eq!(f.alpha, 1.5);
    /// assert_eq!(f.beta, 2.0);
    /// ```
    pub fn new(alpha: f64, beta: f64) -> Self {
        CombinedImpedance { alpha, beta }
    }
}

impl ImpedanceFunction for CombinedImpedance {
    fn compute(&self, cost: f64) -> f64 {
        if cost <= 0.0 {
            return f64::MAX;
        }
        cost.powf(-self.alpha) * (-self.beta * cost).exp()
    }
}
