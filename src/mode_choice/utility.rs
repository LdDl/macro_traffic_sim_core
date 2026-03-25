//! # Mode Utility
//!
//! Utility function definition for the multinomial logit model.
//!
//! Each mode has a linear-in-parameters utility of the form:
//!
//! ```text
//! V = ASC + b_time * time + b_distance * distance + b_cost * cost
//! ```
//!
//! where:
//! - `ASC` - alternative-specific constant (captures unobserved preference)
//! - `b_time`, `b_distance`, `b_cost` - coefficients (typically negative)
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::gmns::types::AgentType;
//! use macro_traffic_sim_core::mode_choice::ModeUtility;
//!
//! let walk = ModeUtility::new(AgentType::Walk)
//!     .with_asc(-2.0)
//!     .with_coeff_time(-0.08)
//!     .build();
//!
//! // 30-minute walk, no distance/cost component
//! let v = walk.compute(30.0, 0.0, 0.0);
//! assert!((v - (-4.4)).abs() < 1e-10);
//! ```

use crate::gmns::types::AgentType;

/// Utility function coefficients for a single mode.
///
/// `V = ASC + b_time * time + b_distance * distance + b_cost * cost`
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::gmns::types::AgentType;
/// use macro_traffic_sim_core::mode_choice::ModeUtility;
///
/// let u = ModeUtility::new(AgentType::Auto)
///     .with_asc(0.0)
///     .with_coeff_time(-0.03)
///     .build();
///
/// assert_eq!(u.agent_type, AgentType::Auto);
/// assert_eq!(u.asc, 0.0);
/// assert_eq!(u.coeff_time, -0.03);
/// ```
#[derive(Debug, Clone)]
pub struct ModeUtility {
    /// Agent type this utility applies to.
    pub agent_type: AgentType,
    /// Alternative-specific constant (ASC).
    pub asc: f64,
    /// Coefficient for travel time (typically negative).
    pub coeff_time: f64,
    /// Coefficient for travel distance (typically negative).
    pub coeff_distance: f64,
    /// Coefficient for monetary cost (typically negative).
    pub coeff_cost: f64,
}

impl ModeUtility {
    /// Create a new mode utility builder.
    ///
    /// Default coefficients: `coeff_time = -0.03`, all others `0.0`.
    ///
    /// # Arguments
    /// * `agent_type` - The mode this utility applies to.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::gmns::types::AgentType;
    /// use macro_traffic_sim_core::mode_choice::ModeUtility;
    ///
    /// let bike = ModeUtility::new(AgentType::Bike)
    ///     .with_asc(-1.0)
    ///     .with_coeff_time(-0.05)
    ///     .with_coeff_distance(-0.02)
    ///     .build();
    ///
    /// assert_eq!(bike.agent_type, AgentType::Bike);
    /// assert_eq!(bike.asc, -1.0);
    /// assert_eq!(bike.coeff_distance, -0.02);
    /// ```
    pub fn new(agent_type: AgentType) -> ModeUtilityBuilder {
        ModeUtilityBuilder {
            instance: ModeUtility {
                agent_type,
                asc: 0.0,
                coeff_time: -0.03,
                coeff_distance: 0.0,
                coeff_cost: 0.0,
            },
        }
    }

    /// Compute the utility value for given travel attributes.
    ///
    /// # Arguments
    /// * `time` - Travel time (minutes).
    /// * `distance` - Travel distance (km).
    /// * `cost` - Monetary cost.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::gmns::types::AgentType;
    /// use macro_traffic_sim_core::mode_choice::ModeUtility;
    ///
    /// let u = ModeUtility::new(AgentType::Auto)
    ///     .with_asc(0.5)
    ///     .with_coeff_time(-0.04)
    ///     .with_coeff_cost(-0.02)
    ///     .build();
    ///
    /// // V = 0.5 + (-0.04)*10 + 0*5 + (-0.02)*3 = 0.5 - 0.4 - 0.06 = 0.04
    /// let v = u.compute(10.0, 5.0, 3.0);
    /// assert!((v - 0.04).abs() < 1e-10);
    /// ```
    ///
    /// ```
    /// use macro_traffic_sim_core::gmns::types::AgentType;
    /// use macro_traffic_sim_core::mode_choice::ModeUtility;
    ///
    /// // With all-zero attributes the utility equals the ASC
    /// let u = ModeUtility::new(AgentType::Walk)
    ///     .with_asc(-2.0)
    ///     .build();
    ///
    /// assert_eq!(u.compute(0.0, 0.0, 0.0), -2.0);
    /// ```
    pub fn compute(&self, time: f64, distance: f64, cost: f64) -> f64 {
        self.asc + self.coeff_time * time + self.coeff_distance * distance + self.coeff_cost * cost
    }
}

/// Builder for [`ModeUtility`].
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::gmns::types::AgentType;
/// use macro_traffic_sim_core::mode_choice::ModeUtility;
///
/// let u = ModeUtility::new(AgentType::Auto)
///     .with_asc(0.0)
///     .with_coeff_time(-0.03)
///     .with_coeff_distance(-0.01)
///     .with_coeff_cost(-0.005)
///     .build();
///
/// assert_eq!(u.coeff_cost, -0.005);
/// ```
pub struct ModeUtilityBuilder {
    instance: ModeUtility,
}

impl ModeUtilityBuilder {
    /// Set the alternative-specific constant.
    pub fn with_asc(mut self, asc: f64) -> Self {
        self.instance.asc = asc;
        self
    }

    /// Set the travel time coefficient.
    pub fn with_coeff_time(mut self, coeff: f64) -> Self {
        self.instance.coeff_time = coeff;
        self
    }

    /// Set the travel distance coefficient.
    pub fn with_coeff_distance(mut self, coeff: f64) -> Self {
        self.instance.coeff_distance = coeff;
        self
    }

    /// Set the monetary cost coefficient.
    pub fn with_coeff_cost(mut self, coeff: f64) -> Self {
        self.instance.coeff_cost = coeff;
        self
    }

    /// Construct the final [`ModeUtility`].
    pub fn build(self) -> ModeUtility {
        self.instance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-10;

    #[test]
    fn compute_zero_attributes_returns_asc() {
        let u = ModeUtility::new(AgentType::Walk)
            .with_asc(-2.0)
            .build();
        assert_eq!(u.compute(0.0, 0.0, 0.0), -2.0);
    }

    #[test]
    fn compute_time_only() {
        let u = ModeUtility::new(AgentType::Auto)
            .with_asc(0.0)
            .with_coeff_time(-0.03)
            .build();
        // V = 0 + (-0.03)*10 = -0.3
        assert!((u.compute(10.0, 0.0, 0.0) - (-0.3)).abs() < EPS);
    }

    #[test]
    fn compute_all_coefficients() {
        let u = ModeUtility::new(AgentType::Auto)
            .with_asc(0.5)
            .with_coeff_time(-0.04)
            .with_coeff_distance(-0.01)
            .with_coeff_cost(-0.02)
            .build();
        // V = 0.5 + (-0.04)*10 + (-0.01)*5 + (-0.02)*3
        // = 0.5 - 0.4 - 0.05 - 0.06 = -0.01
        let v = u.compute(10.0, 5.0, 3.0);
        assert!((v - (-0.01)).abs() < EPS);
    }

    #[test]
    fn builder_defaults() {
        let u = ModeUtility::new(AgentType::Bike).build();
        assert_eq!(u.agent_type, AgentType::Bike);
        assert_eq!(u.asc, 0.0);
        assert_eq!(u.coeff_time, -0.03);
        assert_eq!(u.coeff_distance, 0.0);
        assert_eq!(u.coeff_cost, 0.0);
    }
}
