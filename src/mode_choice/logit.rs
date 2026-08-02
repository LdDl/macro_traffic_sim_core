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
//! use std::rc::Rc;
//! use macro_traffic_sim_core::gmns::types::AgentType;
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//! use macro_traffic_sim_core::mode_choice::{MultinomialLogit, ModeSkim};
//!
//! let zones = vec![1, 2];
//! let mut total_od = DenseOdMatrix::new(zones.clone());
//! total_od.set(1, 2, 1000.0);
//!
//! // Shared zero matrices for distance and cost
//! let zero_dist = Rc::new(DenseOdMatrix::new(zones.clone()));
//! let zero_cost = Rc::new(DenseOdMatrix::new(zones.clone()));
//!
//! // Skim matrices: 10 min by car, 25 min by bike, 40 min on foot
//! let make_skim = |time_val: f64| -> ModeSkim {
//!     let mut time = DenseOdMatrix::new(zones.clone());
//!     time.set(1, 2, time_val);
//!     ModeSkim {
//!         time,
//!         distance: Rc::clone(&zero_dist),
//!         cost: Rc::clone(&zero_cost),
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
use std::rc::Rc;

use super::error::ModeChoiceError;
use crate::gmns::types::AgentType;
use crate::log_main;
use crate::od::OdMatrix;
use crate::od::dense::DenseOdMatrix;
use crate::verbose::EVENT_MODE_CHOICE;

use super::utility::ModeUtility;

/// Typical (illustrative, not calibrated) mode-choice coefficients used by
/// the `default_*` constructors. Real applications must calibrate their own
/// against observed mode shares; these are placeholders for examples and
/// tests, ordered so that AUTO is the most attractive base mode.
///
/// The constants are part of the public API and can be referenced directly:
///
/// ```
/// use macro_traffic_sim_core::mode_choice::default_coefficients::TRANSIT_ASC;
/// assert_eq!(TRANSIT_ASC, -0.5);
/// ```
pub mod default_coefficients {
    /// AUTO alternative-specific constant.
    pub const AUTO_ASC: f64 = 0.0;
    /// AUTO in-vehicle time coefficient (per minute).
    pub const AUTO_COEFF_TIME: f64 = -0.03;
    /// BIKE alternative-specific constant.
    pub const BIKE_ASC: f64 = -1.0;
    /// BIKE time coefficient (per minute).
    pub const BIKE_COEFF_TIME: f64 = -0.05;
    /// WALK alternative-specific constant.
    pub const WALK_ASC: f64 = -2.0;
    /// WALK time coefficient (per minute).
    pub const WALK_COEFF_TIME: f64 = -0.08;
    /// TRANSIT alternative-specific constant.
    pub const TRANSIT_ASC: f64 = -0.5;
    /// TRANSIT time coefficient (per minute).
    pub const TRANSIT_COEFF_TIME: f64 = -0.04;
}

/// Skim data for a single mode: time, distance, and cost matrices.
///
/// Each field is a zone-to-zone matrix. Values are used by
/// [`ModeUtility::compute`] to calculate the systematic utility.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
/// use macro_traffic_sim_core::mode_choice::ModeSkim;
///
/// let zones = vec![1, 2];
/// let mut time = DenseOdMatrix::new(zones.clone());
/// time.set(1, 2, 15.0);
///
/// let skim = ModeSkim {
///     time,
///     distance: Rc::new(DenseOdMatrix::new(zones.clone())),
///     cost: Rc::new(DenseOdMatrix::new(zones.clone())),
/// };
///
/// assert_eq!(skim.time.get(1, 2), 15.0);
/// ```
#[derive(Debug)]
pub struct ModeSkim {
    /// Travel time matrix (minutes). Unique per mode.
    pub time: DenseOdMatrix,
    /// Travel distance matrix (km). Shared across modes via Rc.
    pub distance: Rc<DenseOdMatrix>,
    /// Monetary cost matrix. Shared across modes via Rc.
    pub cost: Rc<DenseOdMatrix>,
}

impl ModeSkim {
    /// Builds a mode skim whose time matrix comes from a sparse cost map,
    /// such as the output of
    /// [`transit_skim`](crate::transit::transit_skim).
    ///
    /// Each `(origin, destination)` present in `time_map` gets that time;
    /// every other off-diagonal pair is set to `f64::INFINITY`, meaning the
    /// mode is unavailable for that pair (its logit utility goes to minus
    /// infinity, so it receives zero share). `distance` and `cost` are
    /// taken as given (a transit fare matrix, or the shared road matrices).
    ///
    /// # Arguments
    ///
    /// * `zone_ids` - Zone IDs defining the matrix dimensions
    /// * `time_map` - `(origin, destination) -> travel time` (sparse)
    /// * `distance` - Distance matrix for the mode
    /// * `cost` - Monetary cost matrix for the mode
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use std::rc::Rc;
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    /// use macro_traffic_sim_core::mode_choice::ModeSkim;
    ///
    /// let zones = vec![1, 2];
    /// let time_map = HashMap::from([((1, 2), 15.0)]);
    /// let skim = ModeSkim::from_time_map(
    ///     &zones,
    ///     &time_map,
    ///     Rc::new(DenseOdMatrix::new(zones.clone())),
    ///     Rc::new(DenseOdMatrix::new(zones.clone())),
    /// );
    /// assert_eq!(skim.time.get(1, 2), 15.0);
    /// // the unspecified 2 -> 1 pair is unavailable
    /// assert!(skim.time.get(2, 1).is_infinite());
    /// ```
    pub fn from_time_map(
        zone_ids: &[crate::gmns::types::ZoneID],
        time_map: &std::collections::HashMap<(i64, i64), f64>,
        distance: Rc<DenseOdMatrix>,
        cost: Rc<DenseOdMatrix>,
    ) -> Self {
        let mut time = DenseOdMatrix::new(zone_ids.to_vec());
        for &o in zone_ids {
            for &d in zone_ids {
                if o != d {
                    time.set(o, d, f64::INFINITY);
                }
            }
        }
        for (&(o, d), &t) in time_map {
            time.set(o, d, t);
        }
        ModeSkim {
            time,
            distance,
            cost,
        }
    }
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
    /// Uses the typical (illustrative) coefficients from
    /// [`default_coefficients`]:
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
        use default_coefficients::*;
        MultinomialLogit {
            utilities: vec![
                ModeUtility::new(AgentType::Auto)
                    .with_asc(AUTO_ASC)
                    .with_coeff_time(AUTO_COEFF_TIME)
                    .build(),
                ModeUtility::new(AgentType::Bike)
                    .with_asc(BIKE_ASC)
                    .with_coeff_time(BIKE_COEFF_TIME)
                    .build(),
                ModeUtility::new(AgentType::Walk)
                    .with_asc(WALK_ASC)
                    .with_coeff_time(WALK_COEFF_TIME)
                    .build(),
            ],
        }
    }

    /// Create a default logit model for AUTO, BIKE, WALK, TRANSIT.
    ///
    /// Uses the typical (illustrative) coefficients from
    /// [`default_coefficients`]:
    /// - AUTO: ASC=0, time=-0.03
    /// - BIKE: ASC=-1.0, time=-0.05
    /// - WALK: ASC=-2.0, time=-0.08
    /// - TRANSIT: ASC=-0.5, time=-0.04
    ///
    /// Pair it with a transit skim built from
    /// [`transit_skim`](crate::transit::transit_skim) via
    /// [`ModeSkim::from_time_map`].
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::mode_choice::MultinomialLogit;
    /// use macro_traffic_sim_core::gmns::types::AgentType;
    ///
    /// let model = MultinomialLogit::default_auto_bike_walk_transit();
    /// assert_eq!(model.utilities.len(), 4);
    /// assert_eq!(model.utilities[3].agent_type, AgentType::Transit);
    /// ```
    pub fn default_auto_bike_walk_transit() -> Self {
        use default_coefficients::*;
        let mut model = Self::default_auto_bike_walk();
        model.utilities.push(
            ModeUtility::new(AgentType::Transit)
                .with_asc(TRANSIT_ASC)
                .with_coeff_time(TRANSIT_COEFF_TIME)
                .build(),
        );
        model
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

        // Pre-resolve and validate skim references
        let mut skim_refs: Vec<&ModeSkim> = Vec::with_capacity(num_modes);
        for utility in &self.utilities {
            match skims.get(&utility.agent_type) {
                Some(skim) => skim_refs.push(skim),
                None => {
                    return Err(ModeChoiceError::MissingSkim(format!(
                        "{:?}",
                        utility.agent_type
                    )));
                }
            }
        }

        // Reusable buffer for utilities per OD pair
        let mut v_buf: Vec<f64> = vec![0.0; num_modes];

        for i in 0..n {
            for j in 0..n {
                let total_demand = total_od.get_by_index(i, j);
                if total_demand <= 0.0 {
                    continue;
                }

                // Compute utilities and find max in one pass
                let mut v_max = f64::NEG_INFINITY;
                for (k, utility) in self.utilities.iter().enumerate() {
                    let skim = skim_refs[k];
                    let time = skim.time.get_by_index(i, j);
                    let distance = skim.distance.get_by_index(i, j);
                    let cost = skim.cost.get_by_index(i, j);
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
                    result_matrices[k].set_by_index(i, j, total_demand * prob);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::od::OdMatrix;
    use crate::transit::{TransitNetwork, TransitRoute, transit_skim};

    const EPS: f64 = 1e-9;

    fn flat_skim(zones: &[i64], time_val: f64) -> ModeSkim {
        let mut time = DenseOdMatrix::new(zones.to_vec());
        for &o in zones {
            for &d in zones {
                if o != d {
                    time.set(o, d, time_val);
                }
            }
        }
        ModeSkim {
            time,
            distance: Rc::new(DenseOdMatrix::new(zones.to_vec())),
            cost: Rc::new(DenseOdMatrix::new(zones.to_vec())),
        }
    }

    #[test]
    fn test_from_time_map_fills_missing_with_infinity() {
        let zones = vec![1, 2, 3];
        let map = HashMap::from([((1, 2), 15.0), ((1, 3), 22.0)]);
        let skim = ModeSkim::from_time_map(
            &zones,
            &map,
            Rc::new(DenseOdMatrix::new(zones.clone())),
            Rc::new(DenseOdMatrix::new(zones.clone())),
        );
        assert!((skim.time.get(1, 2) - 15.0).abs() < EPS);
        assert!((skim.time.get(1, 3) - 22.0).abs() < EPS);
        // absent pair -> unavailable
        assert!(skim.time.get(2, 1).is_infinite());
        assert!(skim.time.get(3, 2).is_infinite());
    }

    #[test]
    fn test_split_gives_transit_zero_share_when_unavailable() {
        let zones = vec![1, 2];
        // Transit reachable only 1 -> 2.
        let transit_map = HashMap::from([((1, 2), 20.0)]);
        let skims = HashMap::from([
            (AgentType::Auto, flat_skim(&zones, 20.0)),
            (AgentType::Bike, flat_skim(&zones, 30.0)),
            (AgentType::Walk, flat_skim(&zones, 60.0)),
            (
                AgentType::Transit,
                ModeSkim::from_time_map(
                    &zones,
                    &transit_map,
                    Rc::new(DenseOdMatrix::new(zones.clone())),
                    Rc::new(DenseOdMatrix::new(zones.clone())),
                ),
            ),
        ]);

        let mut total = DenseOdMatrix::new(zones.clone());
        total.set(1, 2, 100.0);
        total.set(2, 1, 100.0);

        let model = MultinomialLogit::default_auto_bike_walk_transit();
        let split = model.split(&total, &skims).unwrap();

        // 1 -> 2: transit is available, gets a positive share.
        assert!(split[&AgentType::Transit].get(1, 2) > 0.0);
        // 2 -> 1: transit unavailable, exactly zero; the other three carry all 100.
        assert!((split[&AgentType::Transit].get(2, 1) - 0.0).abs() < EPS);
        let carried_21: f64 = [AgentType::Auto, AgentType::Bike, AgentType::Walk]
            .iter()
            .map(|m| split[m].get(2, 1))
            .sum();
        assert!((carried_21 - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_end_to_end_transit_skim_into_mode_choice() {
        // A real transit skim feeds the transit alternative.
        let zones = vec![1, 2];
        let mut network = TransitNetwork::new();
        network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));
        let transit_map = transit_skim(&network, &zones).unwrap();
        // 1 -> 2 = 6 wait + 10 ride = 16; 2 -> 1 absent.
        assert!((transit_map[&(1, 2)] - 16.0).abs() < EPS);

        let skims = HashMap::from([
            (AgentType::Auto, flat_skim(&zones, 25.0)),
            (AgentType::Bike, flat_skim(&zones, 40.0)),
            (AgentType::Walk, flat_skim(&zones, 80.0)),
            (
                AgentType::Transit,
                ModeSkim::from_time_map(
                    &zones,
                    &transit_map,
                    Rc::new(DenseOdMatrix::new(zones.clone())),
                    Rc::new(DenseOdMatrix::new(zones.clone())),
                ),
            ),
        ]);

        let mut total = DenseOdMatrix::new(zones.clone());
        total.set(1, 2, 100.0);
        let split = model_split(&skims, &total);

        // Transit is available on 1 -> 2, so it takes a positive share, and
        // all four modes together carry the whole 100 trips.
        let t = split[&AgentType::Transit].get(1, 2);
        assert!(t > 0.0, "transit share = {}", t);
        let total_split: f64 = [
            AgentType::Auto,
            AgentType::Bike,
            AgentType::Walk,
            AgentType::Transit,
        ]
        .iter()
        .map(|m| split[m].get(1, 2))
        .sum();
        assert!((total_split - 100.0).abs() < 1e-6);
    }

    fn model_split(
        skims: &HashMap<AgentType, ModeSkim>,
        total: &DenseOdMatrix,
    ) -> HashMap<AgentType, DenseOdMatrix> {
        MultinomialLogit::default_auto_bike_walk_transit()
            .split(total, skims)
            .unwrap()
    }
}
