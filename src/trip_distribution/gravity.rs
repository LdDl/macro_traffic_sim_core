//! # Gravity Model
//!
//! Trip distribution using the gravity model with doubly-constrained
//! Furness balancing.
//!
//! ## Formula
//!
//! The initial (singly-constrained) estimate:
//!
//! ```text
//! T_ij = O_i * D_j * f(c_ij) / sum_k( D_k * f(c_ik) )
//! ```
//!
//! This is then passed to Furness balancing to produce a
//! doubly-constrained result where row sums = productions and
//! column sums = attractions.
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//! use macro_traffic_sim_core::trip_distribution::{
//!     GravityModel, ExponentialImpedance,
//! };
//!
//! let zones = vec![1, 2, 3];
//! let productions = vec![100.0, 150.0, 100.0];
//! let attractions = vec![120.0, 110.0, 120.0];
//!
//! let mut cost = DenseOdMatrix::new(zones.clone());
//! cost.set(1, 2, 0.5);
//! cost.set(1, 3, 1.0);
//! cost.set(2, 1, 0.5);
//! cost.set(2, 3, 0.7);
//! cost.set(3, 1, 1.0);
//! cost.set(3, 2, 0.7);
//!
//! let impedance = ExponentialImpedance::new(2.0);
//! let gravity = GravityModel::new();
//!
//! let od = gravity.distribute(
//!     &productions, &attractions, &cost, &impedance, &zones,
//! ).unwrap();
//!
//! // Total trips preserved
//! assert!((od.total() - 350.0).abs() < 1.0);
//! ```

use super::error::TripDistributionError;
use crate::gmns::types::ZoneID;
use crate::log_main;
use crate::od::OdMatrix;
use crate::od::dense::DenseOdMatrix;
use crate::verbose::EVENT_GRAVITY_MODEL;

use super::furness::{FurnessConfig, furness_balance};
use super::impedance::ImpedanceFunction;

/// Gravity model for trip distribution.
///
/// Computes `T_ij = O_i * D_j * f(c_ij) / sum_k( D_k * f(c_ik) )`
/// then applies Furness balancing for a doubly-constrained result.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::{GravityModel, FurnessConfig};
///
/// // Default Furness config
/// let g = GravityModel::new();
/// assert_eq!(g.furness_config.max_iterations, 100);
///
/// // Custom Furness config
/// let g = GravityModel::with_furness_config(FurnessConfig {
///     max_iterations: 200,
///     tolerance: 1e-6,
/// });
/// assert_eq!(g.furness_config.max_iterations, 200);
/// ```
#[derive(Debug, Clone)]
pub struct GravityModel {
    /// Furness balancing configuration.
    pub furness_config: FurnessConfig,
}

impl GravityModel {
    /// Create a new gravity model with default Furness config.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::trip_distribution::GravityModel;
    ///
    /// let g = GravityModel::new();
    /// assert_eq!(g.furness_config.tolerance, 1e-4);
    /// ```
    pub fn new() -> Self {
        GravityModel {
            furness_config: FurnessConfig::default(),
        }
    }

    /// Create a gravity model with custom Furness config.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::trip_distribution::{GravityModel, FurnessConfig};
    ///
    /// let g = GravityModel::with_furness_config(FurnessConfig {
    ///     max_iterations: 50,
    ///     tolerance: 1e-3,
    /// });
    /// assert_eq!(g.furness_config.max_iterations, 50);
    /// ```
    pub fn with_furness_config(config: FurnessConfig) -> Self {
        GravityModel {
            furness_config: config,
        }
    }

    /// Distribute trips using the gravity model.
    ///
    /// # Arguments
    /// * `productions` - Trip productions per zone (indexed same as `zone_ids`).
    /// * `attractions` - Trip attractions per zone (indexed same as `zone_ids`).
    /// * `cost_matrix` - Travel cost matrix (zone-to-zone).
    /// * `impedance` - Impedance function to apply to costs.
    /// * `zone_ids` - Ordered list of zone IDs.
    ///
    /// # Returns
    /// A doubly-constrained OD matrix.
    ///
    /// # Errors
    /// - [`TripDistributionError::DimensionMismatch`] if `productions` or
    ///   `attractions` length differs from `zone_ids`.
    /// - [`TripDistributionError::FurnessNotConverged`] if Furness balancing
    ///   does not converge.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    /// use macro_traffic_sim_core::trip_distribution::{
    ///     GravityModel, PowerImpedance,
    /// };
    ///
    /// let zones = vec![10, 20, 30];
    /// let prods = vec![50.0, 80.0, 70.0];
    /// let attrs = vec![60.0, 70.0, 70.0];
    ///
    /// let mut cost = DenseOdMatrix::new(zones.clone());
    /// cost.set(10, 20, 5.0);
    /// cost.set(10, 30, 10.0);
    /// cost.set(20, 10, 5.0);
    /// cost.set(20, 30, 8.0);
    /// cost.set(30, 10, 10.0);
    /// cost.set(30, 20, 8.0);
    ///
    /// let od = GravityModel::new().distribute(
    ///     &prods, &attrs, &cost, &PowerImpedance::new(1.0), &zones,
    /// ).unwrap();
    ///
    /// // Row sums approximate productions
    /// assert!((od.row_sum(10) - 50.0).abs() < 1.0);
    /// assert!((od.row_sum(20) - 80.0).abs() < 1.0);
    /// ```
    pub fn distribute(
        &self,
        productions: &[f64],
        attractions: &[f64],
        cost_matrix: &dyn OdMatrix,
        impedance: &dyn ImpedanceFunction,
        zone_ids: &[ZoneID],
    ) -> Result<DenseOdMatrix, TripDistributionError> {
        let n = zone_ids.len();
        if productions.len() != n || attractions.len() != n {
            return Err(TripDistributionError::DimensionMismatch(
                "productions/attractions length mismatch with zone_ids".to_string(),
            ));
        }

        // Pre-compute impedance values for all zone pairs (one pass).
        // Avoids calling impedance.compute() twice per (i,j) pair
        // and eliminates repeated zone-index HashMap lookups.
        let mut imp = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..n {
                let cost = cost_matrix.get(zone_ids[i], zone_ids[j]);
                if cost > 0.0 && cost.is_finite() {
                    imp[i * n + j] = impedance.compute(cost);
                }
            }
        }

        let mut data = vec![0.0; n * n];
        for i in 0..n {
            if productions[i] <= 0.0 {
                continue;
            }
            let row = i * n;
            let mut denom = 0.0;
            for k in 0..n {
                if attractions[k] > 0.0 {
                    denom += attractions[k] * imp[row + k];
                }
            }

            if denom <= 0.0 {
                continue;
            }

            for j in 0..n {
                if attractions[j] > 0.0 && imp[row + j] > 0.0 {
                    data[row + j] = productions[i] * attractions[j] * imp[row + j] / denom;
                }
            }
        }

        let iterations =
            furness_balance(&mut data, n, productions, attractions, &self.furness_config)?;

        log_main!(
            EVENT_GRAVITY_MODEL,
            "Gravity model distribution complete",
            zones = n,
            furness_iterations = iterations
        );

        Ok(DenseOdMatrix::from_data(zone_ids.to_vec(), data))
    }
}

impl Default for GravityModel {
    fn default() -> Self {
        Self::new()
    }
}
