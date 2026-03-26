//! # Multinomial Logit Model
//!
//! Splits a total OD matrix into per-mode OD matrices using
//! the multinomial logit choice probability.
//!
//! ## Choice probability
//!
//! For each OD pair `(i, j)` the probability of choosing mode `k` is:
//!
//! ```text
//! P(k) = exp(V_k) / sum_m( exp(V_m) )
//! ```
//!
//! The implementation uses the log-sum-exp trick (`V - V_max`) for
//! numerical stability.
//!
//! ## Examples
//!
//! ### Splitting demand with the default model
//!
//! ```
//! use std::collections::HashMap;
//! use macro_traffic_sim_core::gmns::types::AgentType;
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//! use macro_traffic_sim_core::mode_choice::{MultinomialLogit, ModeSkim};
//!
//! let zones = vec![1, 2];
//! let mut total_od = DenseOdMatrix::new(zones.clone());
//! total_od.set(1, 2, 1000.0);
//!
//! // Skim matrices: 10 min by car, 25 min by bike, 40 min on foot
//! let make_skim = |time_val: f64| -> ModeSkim {
//!     let mut time = DenseOdMatrix::new(zones.clone());
//!     time.set(1, 2, time_val);
//!     ModeSkim {
//!         time,
//!         distance: DenseOdMatrix::new(zones.clone()),
//!         cost: DenseOdMatrix::new(zones.clone()),
//!     }
//! };
//!
//! let mut skims = HashMap::new();
//! skims.insert(AgentType::Auto, make_skim(10.0));
//! skims.insert(AgentType::Bike, make_skim(25.0));
//! skims.insert(AgentType::Walk, make_skim(40.0));
//!
//! let model = MultinomialLogit::default_auto_bike_walk();
//! let result = model.split(&total_od, &skims).unwrap();
//!
//! // All demand is distributed across modes
//! let auto_demand = result[&AgentType::Auto].get(1, 2);
//! let bike_demand = result[&AgentType::Bike].get(1, 2);
//! let walk_demand = result[&AgentType::Walk].get(1, 2);
//!
//! let sum = auto_demand + bike_demand + walk_demand;
//! assert!((sum - 1000.0).abs() < 1e-6);
//!
//! // Auto should get the largest share (shortest time, highest ASC)
//! assert!(auto_demand > bike_demand);
//! assert!(bike_demand > walk_demand);
//! ```

use std::collections::HashMap;

use super::error::ModeChoiceError;
use crate::gmns::types::AgentType;
use crate::log_main;
use crate::od::OdMatrix;
use crate::od::dense::DenseOdMatrix;
use crate::verbose::EVENT_MODE_CHOICE;

use super::utility::ModeUtility;

/// Skim data for a single mode: time, distance, and cost matrices.
///
/// Each field is a zone-to-zone matrix. Values are used by
/// [`ModeUtility::compute`] to calculate the systematic utility.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
/// use macro_traffic_sim_core::mode_choice::ModeSkim;
///
/// let zones = vec![1, 2];
/// let mut time = DenseOdMatrix::new(zones.clone());
/// time.set(1, 2, 15.0);
///
/// let skim = ModeSkim {
///     time,
///     distance: DenseOdMatrix::new(zones.clone()),
///     cost: DenseOdMatrix::new(zones.clone()),
/// };
///
/// assert_eq!(skim.time.get(1, 2), 15.0);
/// ```
#[derive(Debug)]
pub struct ModeSkim {
    /// Travel time matrix (minutes).
    pub time: DenseOdMatrix,
    /// Travel distance matrix (km).
    pub distance: DenseOdMatrix,
    /// Monetary cost matrix.
    pub cost: DenseOdMatrix,
}

/// Multinomial logit mode choice model.
///
/// `P(mode_k) = exp(V_k) / sum_m( exp(V_m) )`
///
/// Splits total OD demand into per-mode OD matrices.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::mode_choice::MultinomialLogit;
/// use macro_traffic_sim_core::gmns::types::AgentType;
///
/// let model = MultinomialLogit::default_auto_bike_walk();
/// assert_eq!(model.utilities.len(), 3);
/// assert_eq!(model.utilities[0].agent_type, AgentType::Auto);
/// assert_eq!(model.utilities[1].agent_type, AgentType::Bike);
/// assert_eq!(model.utilities[2].agent_type, AgentType::Walk);
/// ```
#[derive(Debug)]
pub struct MultinomialLogit {
    /// Utility functions for each mode.
    pub utilities: Vec<ModeUtility>,
}

impl MultinomialLogit {
    /// Create a new logit model with the given mode utilities.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::gmns::types::AgentType;
    /// use macro_traffic_sim_core::mode_choice::{ModeUtility, MultinomialLogit};
    ///
    /// let utils = vec![
    ///     ModeUtility::new(AgentType::Auto)
    ///         .with_coeff_time(-0.03)
    ///         .build(),
    ///     ModeUtility::new(AgentType::Walk)
    ///         .with_asc(-2.0)
    ///         .with_coeff_time(-0.08)
    ///         .build(),
    /// ];
    ///
    /// let model = MultinomialLogit::new(utils);
    /// assert_eq!(model.utilities.len(), 2);
    /// ```
    pub fn new(utilities: Vec<ModeUtility>) -> Self {
        MultinomialLogit { utilities }
    }

    /// Create a default logit model for AUTO, BIKE, WALK.
    ///
    /// Uses typical coefficients:
    /// - AUTO: ASC=0, time=-0.03
    /// - BIKE: ASC=-1.0, time=-0.05
    /// - WALK: ASC=-2.0, time=-0.08
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::mode_choice::MultinomialLogit;
    ///
    /// let model = MultinomialLogit::default_auto_bike_walk();
    /// assert_eq!(model.utilities.len(), 3);
    /// ```
    pub fn default_auto_bike_walk() -> Self {
        MultinomialLogit {
            utilities: vec![
                ModeUtility::new(AgentType::Auto)
                    .with_asc(0.0)
                    .with_coeff_time(-0.03)
                    .build(),
                ModeUtility::new(AgentType::Bike)
                    .with_asc(-1.0)
                    .with_coeff_time(-0.05)
                    .build(),
                ModeUtility::new(AgentType::Walk)
                    .with_asc(-2.0)
                    .with_coeff_time(-0.08)
                    .build(),
            ],
        }
    }

    /// Split a total OD matrix into per-mode OD matrices.
    ///
    /// For each OD pair with positive demand, computes logit probabilities
    /// across all modes and distributes the demand proportionally.
    ///
    /// # Arguments
    /// * `total_od` - The total OD matrix to split.
    /// * `skims` - Skim data per mode (time, distance, cost).
    ///
    /// # Returns
    /// A HashMap mapping each [`AgentType`] to its share of the OD matrix.
    /// The per-mode demands sum to the total demand for each OD pair.
    pub fn split(
        &self,
        total_od: &dyn OdMatrix,
        skims: &HashMap<AgentType, ModeSkim>,
    ) -> Result<HashMap<AgentType, DenseOdMatrix>, ModeChoiceError> {
        let zone_ids = total_od.zone_ids().to_vec();
        let n = zone_ids.len();
        let num_modes = self.utilities.len();

        // Pre-allocate result matrices as a Vec (indexed access, no HashMap lookups)
        let mut result_matrices: Vec<DenseOdMatrix> = self
            .utilities
            .iter()
            .map(|_| DenseOdMatrix::new(zone_ids.clone()))
            .collect();

        // Pre-resolve skim references (avoid HashMap lookup per OD pair)
        let skim_refs: Vec<Option<&ModeSkim>> = self
            .utilities
            .iter()
            .map(|u| skims.get(&u.agent_type))
            .collect();

        // Reusable buffer for utilities per OD pair
        let mut v_buf: Vec<f64> = vec![0.0; num_modes];

        for i in 0..n {
            let oi = zone_ids[i];
            for j in 0..n {
                let dj = zone_ids[j];
                let total_demand = total_od.get(oi, dj);
                if total_demand <= 0.0 {
                    continue;
                }

                // Compute utilities and find max in one pass
                let mut v_max = f64::NEG_INFINITY;
                for (k, utility) in self.utilities.iter().enumerate() {
                    let (time, distance, cost) = if let Some(skim) = skim_refs[k] {
                        (
                            skim.time.get(oi, dj),
                            skim.distance.get(oi, dj),
                            skim.cost.get(oi, dj),
                        )
                    } else {
                        (0.0, 0.0, 0.0)
                    };
                    let v = utility.compute(time, distance, cost);
                    v_buf[k] = v;
                    if v > v_max {
                        v_max = v;
                    }
                }

                let exp_sum: f64 = v_buf[..num_modes].iter().map(|&v| (v - v_max).exp()).sum();

                if exp_sum <= 0.0 {
                    continue;
                }

                for k in 0..num_modes {
                    let prob = (v_buf[k] - v_max).exp() / exp_sum;
                    result_matrices[k].set(oi, dj, total_demand * prob);
                }
            }
        }

        // Build result HashMap from indexed Vec
        let mut result: HashMap<AgentType, DenseOdMatrix> = HashMap::with_capacity(num_modes);
        for (k, utility) in self.utilities.iter().enumerate() {
            result.insert(
                utility.agent_type,
                std::mem::replace(&mut result_matrices[k], DenseOdMatrix::new(vec![])),
            );
        }

        log_main!(
            EVENT_MODE_CHOICE,
            "Mode choice split complete",
            zones = n,
            modes = self.utilities.len()
        );

        Ok(result)
    }
}
