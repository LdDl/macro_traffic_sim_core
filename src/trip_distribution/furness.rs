//! # Furness Method (Iterative Proportional Fitting)
//!
//! Doubly-constrained balancing algorithm that adjusts a seed matrix
//! so that row sums match productions and column sums match attractions.
//!
//! ## Algorithm
//!
//! Each iteration performs two steps:
//! 1. Scale each row so that row sums match target productions.
//! 2. Scale each column so that column sums match target attractions.
//!
//! Convergence is checked as the maximum relative error across all
//! row and column sums. The process repeats until the error falls
//! below the configured tolerance or the iteration limit is reached.
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::trip_distribution::{FurnessConfig, furness_balance};
//!
//! // 2x2 seed matrix (row-major)
//! let mut matrix = vec![30.0, 70.0, 40.0, 60.0];
//!
//! let productions = [120.0, 80.0];
//! let attractions = [90.0, 110.0];
//!
//! let config = FurnessConfig::default();
//! let iterations = furness_balance(
//!     &mut matrix,
//!     2,
//!     &productions,
//!     &attractions,
//!     &config,
//! ).unwrap();
//!
//! // Row sums match productions
//! let row0 = matrix[0] + matrix[1];
//! let row1 = matrix[2] + matrix[3];
//! assert!((row0 - 120.0).abs() < 0.1);
//! assert!((row1 - 80.0).abs() < 0.1);
//!
//! // Column sums match attractions
//! let col0 = matrix[0] + matrix[2];
//! let col1 = matrix[1] + matrix[3];
//! assert!((col0 - 90.0).abs() < 0.1);
//! assert!((col1 - 110.0).abs() < 0.1);
//! ```

use super::error::TripDistributionError;
use crate::log_all;
use crate::verbose::EVENT_FURNESS_ITERATION;

/// Furness / IPF balancing parameters.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::FurnessConfig;
///
/// // Default: 100 iterations, tolerance 1e-4
/// let config = FurnessConfig::default();
/// assert_eq!(config.max_iterations, 100);
/// assert_eq!(config.tolerance, 1e-4);
/// ```
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::FurnessConfig;
///
/// let config = FurnessConfig {
///     max_iterations: 200,
///     tolerance: 1e-6,
/// };
/// assert_eq!(config.max_iterations, 200);
/// ```
#[derive(Debug, Clone)]
pub struct FurnessConfig {
    /// Maximum number of balancing iterations.
    pub max_iterations: usize,
    /// Convergence tolerance (max relative error in row/col sums).
    pub tolerance: f64,
}

impl Default for FurnessConfig {
    fn default() -> Self {
        FurnessConfig {
            max_iterations: 100,
            tolerance: 1e-4,
        }
    }
}

/// Apply Furness (IPF) balancing to a seed matrix in-place.
///
/// Adjusts the matrix so that:
/// - Row sums match `target_productions`
/// - Column sums match `target_attractions`
///
/// # Arguments
/// * `matrix` - Flat row-major matrix of size `n * n`. Modified in-place.
/// * `n` - Number of zones.
/// * `target_productions` - Target row sums (one per zone).
/// * `target_attractions` - Target column sums (one per zone).
/// * `config` - Balancing parameters.
///
/// # Returns
/// The number of iterations performed.
///
/// # Errors
/// Returns [`TripDistributionError::FurnessNotConverged`] if the maximum
/// relative error does not drop below `config.tolerance` within
/// `config.max_iterations`.
///
/// # Panics
/// Panics if `matrix.len() != n * n`, `target_productions.len() != n`,
/// or `target_attractions.len() != n`.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_distribution::{FurnessConfig, furness_balance};
///
/// let mut matrix = vec![10.0, 20.0, 30.0, 40.0];
/// let config = FurnessConfig { max_iterations: 50, tolerance: 1e-6 };
///
/// let iters = furness_balance(
///     &mut matrix, 2,
///     &[50.0, 50.0],
///     &[40.0, 60.0],
///     &config,
/// ).unwrap();
///
/// assert!(iters <= 50);
/// ```
pub fn furness_balance(
    matrix: &mut [f64],
    n: usize,
    target_productions: &[f64],
    target_attractions: &[f64],
    config: &FurnessConfig,
) -> Result<usize, TripDistributionError> {
    assert_eq!(matrix.len(), n * n);
    assert_eq!(target_productions.len(), n);
    assert_eq!(target_attractions.len(), n);

    for iteration in 0..config.max_iterations {
        for i in 0..n {
            let row_sum: f64 = (0..n).map(|j| matrix[i * n + j]).sum();
            if row_sum > 0.0 && target_productions[i] > 0.0 {
                let factor = target_productions[i] / row_sum;
                for j in 0..n {
                    matrix[i * n + j] *= factor;
                }
            }
        }

        for j in 0..n {
            let col_sum: f64 = (0..n).map(|i| matrix[i * n + j]).sum();
            if col_sum > 0.0 && target_attractions[j] > 0.0 {
                let factor = target_attractions[j] / col_sum;
                for i in 0..n {
                    matrix[i * n + j] *= factor;
                }
            }
        }

        let mut max_error = 0.0_f64;

        for i in 0..n {
            let row_sum: f64 = (0..n).map(|j| matrix[i * n + j]).sum();
            if target_productions[i] > 0.0 {
                let err = ((row_sum - target_productions[i]) / target_productions[i]).abs();
                max_error = max_error.max(err);
            }
        }

        for j in 0..n {
            let col_sum: f64 = (0..n).map(|i| matrix[i * n + j]).sum();
            if target_attractions[j] > 0.0 {
                let err = ((col_sum - target_attractions[j]) / target_attractions[j]).abs();
                max_error = max_error.max(err);
            }
        }

        log_all!(
            EVENT_FURNESS_ITERATION,
            "Furness iteration",
            iteration = iteration,
            max_error = format!("{:.8}", max_error)
        );

        if max_error < config.tolerance {
            return Ok(iteration + 1);
        }
    }

    Err(TripDistributionError::FurnessNotConverged {
        max_iterations: config.max_iterations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furness_2x2_balanced() {
        let mut matrix = vec![30.0, 70.0, 40.0, 60.0];
        let productions = [120.0, 80.0];
        let attractions = [90.0, 110.0];
        let config = FurnessConfig {
            max_iterations: 100,
            tolerance: 1e-6,
        };

        let iters = furness_balance(&mut matrix, 2, &productions, &attractions, &config).unwrap();
        assert!(iters <= 100);

        let row0 = matrix[0] + matrix[1];
        let row1 = matrix[2] + matrix[3];
        assert!((row0 - 120.0).abs() < 0.01);
        assert!((row1 - 80.0).abs() < 0.01);

        let col0 = matrix[0] + matrix[2];
        let col1 = matrix[1] + matrix[3];
        assert!((col0 - 90.0).abs() < 0.01);
        assert!((col1 - 110.0).abs() < 0.01);
    }

    #[test]
    fn furness_3x3_balanced() {
        let mut matrix = vec![
            10.0, 20.0, 30.0,
            15.0, 25.0, 35.0,
            20.0, 10.0, 40.0,
        ];
        let productions = [100.0, 150.0, 50.0];
        let attractions = [80.0, 120.0, 100.0];
        let config = FurnessConfig {
            max_iterations: 200,
            tolerance: 1e-6,
        };

        let iters = furness_balance(&mut matrix, 3, &productions, &attractions, &config).unwrap();
        assert!(iters <= 200);

        for i in 0..3 {
            let row_sum: f64 = (0..3).map(|j| matrix[i * 3 + j]).sum();
            assert!(
                (row_sum - productions[i]).abs() < 0.01,
                "row {i}: got {row_sum}, expected {}",
                productions[i]
            );
        }
        for j in 0..3 {
            let col_sum: f64 = (0..3).map(|i| matrix[i * 3 + j]).sum();
            assert!(
                (col_sum - attractions[j]).abs() < 0.01,
                "col {j}: got {col_sum}, expected {}",
                attractions[j]
            );
        }
    }

    #[test]
    fn furness_preserves_total() {
        let mut matrix = vec![10.0, 20.0, 30.0, 40.0];
        let productions = [50.0, 50.0];
        let attractions = [40.0, 60.0];
        let config = FurnessConfig::default();

        furness_balance(&mut matrix, 2, &productions, &attractions, &config).unwrap();

        let total: f64 = matrix.iter().sum();
        assert!((total - 100.0).abs() < 0.01);
    }

    #[test]
    fn furness_not_converged_returns_error() {
        let mut matrix = vec![10.0, 20.0, 30.0, 40.0];
        let productions = [50.0, 50.0];
        let attractions = [40.0, 60.0];
        let config = FurnessConfig {
            max_iterations: 0,
            tolerance: 1e-10,
        };

        let result = furness_balance(&mut matrix, 2, &productions, &attractions, &config);
        assert!(result.is_err());
    }

    #[test]
    fn furness_already_balanced() {
        // Matrix that already satisfies constraints
        let mut matrix = vec![30.0, 70.0, 60.0, 40.0];
        let productions = [100.0, 100.0];
        let attractions = [90.0, 110.0];
        let config = FurnessConfig {
            max_iterations: 100,
            tolerance: 1e-4,
        };

        let iters = furness_balance(&mut matrix, 2, &productions, &attractions, &config).unwrap();
        // Should converge very quickly
        assert!(iters <= 10);
    }
}
