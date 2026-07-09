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

/// Generalized power impedance function: `f(c) = (1 + (c / c0)^alpha)^(-beta)`.
///
/// Corresponds to the survival function of the Burr Type XII distribution.
/// See the [Burr distribution](https://en.wikipedia.org/wiki/Burr_distribution) for details.
/// When `beta = 1`, reduces to the log-logistic deterrence function.
/// Widely used in European transport planning (PTV VISUM EVA model,
/// Lohse EVA approach). Handles the zero-cost case gracefully
/// (`f(0) = 1`), unlike the plain power function.
///
/// Originates from the EVA (Erzeugung - 'generation'; Verteilung - 'distribution';
/// Aufteilung - 'mode choice') model (Lohse, 1997, TU Dresden).
/// For general context see: Ortuzar, J.D. & Willumsen, L.G. (2011).
/// "Modelling Transport", 4th ed., Wiley, Ch. 5.
/// DOI: 10.1002/9781119993308
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::{GeneralizedPowerImpedance, ImpedanceFunction};
///
/// let f = GeneralizedPowerImpedance::new(30.0, 3.0, 2.0);
///
/// // f(0) = (1 + 0)^(-2) = 1.0
/// assert_eq!(f.compute(0.0), 1.0);
///
/// // f(30) = (1 + 1^3)^(-2) = 2^(-2) = 0.25
/// assert!((f.compute(30.0) - 0.25).abs() < 1e-10);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct GeneralizedPowerImpedance {
    /// Scale parameter (characteristic cost). Must be positive.
    pub c0: f64,
    /// Shape parameter (steepness). Must be positive.
    pub alpha: f64,
    /// Decay exponent. Must be positive.
    pub beta: f64,
}

impl GeneralizedPowerImpedance {
    /// Create a new log-logistic impedance function.
    ///
    /// # Arguments
    /// * `c0` - Scale parameter (positive). Cost at which the function equals `2^(-beta)`.
    /// * `alpha` - Shape parameter (positive). Controls steepness of the transition.
    /// * `beta` - Decay exponent (positive). Controls how fast the tail drops.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::trip_distribution::GeneralizedPowerImpedance;
    ///
    /// let f = GeneralizedPowerImpedance::new(20.0, 2.5, 1.5);
    /// assert_eq!(f.c0, 20.0);
    /// assert_eq!(f.alpha, 2.5);
    /// assert_eq!(f.beta, 1.5);
    /// ```
    pub fn new(c0: f64, alpha: f64, beta: f64) -> Self {
        GeneralizedPowerImpedance { c0, alpha, beta }
    }
}

impl ImpedanceFunction for GeneralizedPowerImpedance {
    fn compute(&self, cost: f64) -> f64 {
        if cost <= 0.0 {
            return 1.0;
        }
        (1.0 + (cost / self.c0).powf(self.alpha)).powf(-self.beta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-10;
    const EPS_COARSE: f64 = 1e-6;

    #[test]
    fn exponential_zero_cost() {
        let f = ExponentialImpedance::new(2.0);
        assert_eq!(f.compute(0.0), 1.0);
    }

    #[test]
    fn exponential_known_value() {
        let f = ExponentialImpedance::new(2.0);
        // exp(-2 * 1) = exp(-2)
        let expected = (-2.0_f64).exp();
        assert!((f.compute(1.0) - expected).abs() < EPS);
    }

    #[test]
    fn exponential_large_cost_near_zero() {
        let f = ExponentialImpedance::new(1.0);
        assert!(f.compute(50.0) < 1e-20);
    }

    #[test]
    fn exponential_monotonically_decreasing() {
        let f = ExponentialImpedance::new(0.5);
        let v1 = f.compute(1.0);
        let v2 = f.compute(2.0);
        let v3 = f.compute(5.0);
        assert!(v1 > v2);
        assert!(v2 > v3);
    }

    #[test]
    fn power_unit_cost() {
        let f = PowerImpedance::new(2.0);
        // 1^(-2) = 1.0
        assert_eq!(f.compute(1.0), 1.0);
    }

    #[test]
    fn power_known_value() {
        let f = PowerImpedance::new(2.0);
        // 2^(-2) = 0.25
        assert_eq!(f.compute(2.0), 0.25);
    }

    #[test]
    fn power_zero_cost_returns_max() {
        let f = PowerImpedance::new(1.5);
        assert_eq!(f.compute(0.0), f64::MAX);
    }

    #[test]
    fn power_negative_cost_returns_max() {
        let f = PowerImpedance::new(1.5);
        assert_eq!(f.compute(-1.0), f64::MAX);
    }

    #[test]
    fn power_monotonically_decreasing() {
        let f = PowerImpedance::new(1.0);
        let v1 = f.compute(1.0);
        let v2 = f.compute(2.0);
        let v3 = f.compute(4.0);
        assert!(v1 > v2);
        assert!(v2 > v3);
    }

    #[test]
    fn combined_known_value() {
        let f = CombinedImpedance::new(1.0, 1.0);
        // f(1) = 1^(-1) * exp(-1) = exp(-1)
        let expected = (-1.0_f64).exp();
        assert!((f.compute(1.0) - expected).abs() < EPS);
    }

    #[test]
    fn combined_another_value() {
        let f = CombinedImpedance::new(1.0, 1.0);
        // f(2) = 2^(-1) * exp(-2) = 0.5 * exp(-2)
        let expected = 0.5 * (-2.0_f64).exp();
        assert!((f.compute(2.0) - expected).abs() < EPS);
    }

    #[test]
    fn combined_zero_cost_returns_max() {
        let f = CombinedImpedance::new(1.0, 1.0);
        assert_eq!(f.compute(0.0), f64::MAX);
    }

    #[test]
    fn combined_monotonically_decreasing() {
        let f = CombinedImpedance::new(0.5, 0.5);
        let v1 = f.compute(1.0);
        let v2 = f.compute(2.0);
        let v3 = f.compute(5.0);
        assert!(v1 > v2);
        assert!(v2 > v3);
    }

    #[test]
    fn log_logistic_zero_cost() {
        let f = GeneralizedPowerImpedance::new(30.0, 3.0, 2.0);
        assert_eq!(f.compute(0.0), 1.0);
    }

    #[test]
    fn log_logistic_negative_cost() {
        let f = GeneralizedPowerImpedance::new(30.0, 3.0, 2.0);
        assert_eq!(f.compute(-5.0), 1.0);
    }

    #[test]
    fn log_logistic_at_c0() {
        let f = GeneralizedPowerImpedance::new(30.0, 3.0, 2.0);
        // f(c0) = (1 + 1^3)^(-2) = 2^(-2) = 0.25
        assert!((f.compute(30.0) - 0.25).abs() < EPS);
    }

    #[test]
    fn log_logistic_known_value() {
        let f = GeneralizedPowerImpedance::new(10.0, 2.0, 1.0);
        // f(20) = (1 + (20/10)^2)^(-1) = (1 + 4)^(-1) = 0.2
        assert!((f.compute(20.0) - 0.2).abs() < EPS);
    }

    #[test]
    fn log_logistic_monotonically_decreasing() {
        let f = GeneralizedPowerImpedance::new(15.0, 2.0, 1.5);
        let v1 = f.compute(1.0);
        let v2 = f.compute(10.0);
        let v3 = f.compute(50.0);
        assert!(v1 > v2);
        assert!(v2 > v3);
    }

    #[test]
    fn log_logistic_large_cost_near_zero() {
        let f = GeneralizedPowerImpedance::new(10.0, 2.0, 2.0);
        assert!(f.compute(1000.0) < EPS_COARSE);
    }
}
