//! # Regression-Based Trip Generation
//!
//! Linear regression model for computing trip productions and attractions
//! based on zone socioeconomic attributes.
//!
//! ## Formula
//!
//! ```text
//! value = intercept
//!       + pop_coeff * population
//!       + emp_coeff * employment
//!       + hh_coeff * households
//!       + income_coeff * avg_income
//! ```
//!
//! Negative results are clamped to zero.
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::zone::Zone;
//! use macro_traffic_sim_core::trip_generation::{
//!     RegressionGenerator, RegressionCoefficients, TripGenerator,
//! };
//!
//! let production_coeffs = RegressionCoefficients {
//!     intercept: 10.0,
//!     pop_coeff: 0.3,
//!     emp_coeff: 0.0,
//!     hh_coeff: 0.0,
//!     income_coeff: 0.0,
//! };
//! let attraction_coeffs = RegressionCoefficients {
//!     intercept: 0.0,
//!     pop_coeff: 0.0,
//!     emp_coeff: 1.2,
//!     hh_coeff: 0.0,
//!     income_coeff: 0.0,
//! };
//!
//! let trip_gen =RegressionGenerator::with_coefficients(production_coeffs, attraction_coeffs);
//!
//! let zones = vec![
//!     Zone::new(1).with_population(1000.0).with_employment(500.0).build(),
//! ];
//! let (prods, attrs) = trip_gen.generate(&zones).unwrap();
//!
//! // production = 10 + 0.3 * 1000 = 310
//! assert!((prods[0] - 310.0).abs() < 1e-10);
//! // attraction = 0 + 1.2 * 500 = 600
//! assert!((attrs[0] - 600.0).abs() < 1e-10);
//! ```

use super::TripGenerator;
use super::error::TripGenerationError;
use crate::zone::Zone;

/// Coefficients for the linear regression equation.
///
/// ```text
/// value = intercept + pop_coeff * population + emp_coeff * employment
///       + hh_coeff * households + income_coeff * avg_income
/// ```
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_generation::RegressionCoefficients;
///
/// // Default: intercept=0, pop_coeff=0.5, rest zero
/// let c = RegressionCoefficients::default();
/// assert_eq!(c.intercept, 0.0);
/// assert_eq!(c.pop_coeff, 0.5);
/// assert_eq!(c.emp_coeff, 0.0);
/// ```
///
/// ```
/// use macro_traffic_sim_core::trip_generation::RegressionCoefficients;
///
/// let c = RegressionCoefficients {
///     intercept: 5.0,
///     pop_coeff: 0.4,
///     emp_coeff: 0.6,
///     hh_coeff: 0.0,
///     income_coeff: 0.001,
/// };
/// assert_eq!(c.emp_coeff, 0.6);
/// ```
#[derive(Debug, Clone)]
pub struct RegressionCoefficients {
    /// Constant term.
    pub intercept: f64,
    /// Population coefficient.
    pub pop_coeff: f64,
    /// Employment coefficient.
    pub emp_coeff: f64,
    /// Households coefficient.
    pub hh_coeff: f64,
    /// Average income coefficient.
    pub income_coeff: f64,
}

impl Default for RegressionCoefficients {
    fn default() -> Self {
        RegressionCoefficients {
            intercept: 0.0,
            pop_coeff: 0.5,
            emp_coeff: 0.0,
            hh_coeff: 0.0,
            income_coeff: 0.0,
        }
    }
}

/// Trip generator using linear regression.
///
/// Computes productions and attractions as linear combinations
/// of zone socioeconomic attributes. Separate coefficient sets
/// are used for productions and attractions.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::zone::Zone;
/// use macro_traffic_sim_core::trip_generation::{RegressionGenerator, TripGenerator};
///
/// let trip_gen =RegressionGenerator::new();
/// let zones = vec![
///     Zone::new(1).with_population(10000.0).with_employment(4000.0).build(),
///     Zone::new(2).with_population(5000.0).with_employment(8000.0).build(),
/// ];
///
/// let (prods, attrs) = trip_gen.generate(&zones).unwrap();
///
/// // Zone 1: more population -> more production
/// // Zone 2: more employment -> more attraction
/// assert!(prods[0] > prods[1]);
/// assert!(attrs[1] > attrs[0]);
/// ```
#[derive(Debug, Clone)]
pub struct RegressionGenerator {
    /// Coefficients for the production equation.
    pub production_coeffs: RegressionCoefficients,
    /// Coefficients for the attraction equation.
    pub attraction_coeffs: RegressionCoefficients,
}

impl RegressionGenerator {
    /// Create a new regression generator with default coefficients.
    ///
    /// Default production: `0.5 * population + 0.1 * employment`
    /// Default attraction: `0.1 * population + 0.8 * employment`
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::trip_generation::RegressionGenerator;
    ///
    /// let trip_gen =RegressionGenerator::new();
    /// assert_eq!(trip_gen.production_coeffs.pop_coeff, 0.5);
    /// assert_eq!(trip_gen.attraction_coeffs.emp_coeff, 0.8);
    /// ```
    pub fn new() -> Self {
        RegressionGenerator {
            production_coeffs: RegressionCoefficients {
                intercept: 0.0,
                pop_coeff: 0.5,
                emp_coeff: 0.1,
                hh_coeff: 0.0,
                income_coeff: 0.0,
            },
            attraction_coeffs: RegressionCoefficients {
                intercept: 0.0,
                pop_coeff: 0.1,
                emp_coeff: 0.8,
                hh_coeff: 0.0,
                income_coeff: 0.0,
            },
        }
    }

    /// Create a generator with custom coefficients.
    ///
    /// # Arguments
    /// * `production` - Coefficients for computing productions.
    /// * `attraction` - Coefficients for computing attractions.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::trip_generation::{
    ///     RegressionGenerator, RegressionCoefficients,
    /// };
    ///
    /// let trip_gen =RegressionGenerator::with_coefficients(
    ///     RegressionCoefficients { intercept: 0.0, pop_coeff: 1.0, emp_coeff: 0.0, hh_coeff: 0.0, income_coeff: 0.0 },
    ///     RegressionCoefficients { intercept: 0.0, pop_coeff: 0.0, emp_coeff: 1.0, hh_coeff: 0.0, income_coeff: 0.0 },
    /// );
    ///
    /// assert_eq!(trip_gen.production_coeffs.pop_coeff, 1.0);
    /// assert_eq!(trip_gen.attraction_coeffs.emp_coeff, 1.0);
    /// ```
    pub fn with_coefficients(
        production: RegressionCoefficients,
        attraction: RegressionCoefficients,
    ) -> Self {
        RegressionGenerator {
            production_coeffs: production,
            attraction_coeffs: attraction,
        }
    }

    fn compute_value(zone: &Zone, coeffs: &RegressionCoefficients) -> f64 {
        let value = coeffs.intercept
            + coeffs.pop_coeff * zone.population
            + coeffs.emp_coeff * zone.employment
            + coeffs.hh_coeff * zone.households
            + coeffs.income_coeff * zone.avg_income;
        value.max(0.0)
    }
}

impl Default for RegressionGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl TripGenerator for RegressionGenerator {
    fn generate(&self, zones: &[Zone]) -> Result<(Vec<f64>, Vec<f64>), TripGenerationError> {
        if zones.is_empty() {
            return Err(TripGenerationError::NoZones);
        }

        let productions: Vec<f64> = zones
            .iter()
            .map(|z| Self::compute_value(z, &self.production_coeffs))
            .collect();

        let attractions: Vec<f64> = zones
            .iter()
            .map(|z| Self::compute_value(z, &self.attraction_coeffs))
            .collect();

        Ok((productions, attractions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-10;

    #[test]
    fn default_coefficients_single_zone() {
        let trip_gen = RegressionGenerator::new();
        let zones = vec![
            Zone::new(1)
                .with_population(10000.0)
                .with_employment(5000.0)
                .build(),
        ];

        let (prods, attrs) = trip_gen.generate(&zones).unwrap();

        // P = 0.5 * 10000 + 0.1 * 5000 = 5500
        assert!((prods[0] - 5500.0).abs() < EPS);
        // A = 0.1 * 10000 + 0.8 * 5000 = 5000
        assert!((attrs[0] - 5000.0).abs() < EPS);
    }

    #[test]
    fn multiple_zones() {
        let trip_gen = RegressionGenerator::new();
        let zones = vec![
            Zone::new(1)
                .with_population(6000.0)
                .with_employment(500.0)
                .build(),
            Zone::new(2)
                .with_population(2000.0)
                .with_employment(3000.0)
                .build(),
        ];

        let (prods, attrs) = trip_gen.generate(&zones).unwrap();

        // Zone 1: P = 0.5*6000 + 0.1*500 = 3050
        assert!((prods[0] - 3050.0).abs() < EPS);
        // Zone 2: P = 0.5*2000 + 0.1*3000 = 1300
        assert!((prods[1] - 1300.0).abs() < EPS);
        // Zone 1: A = 0.1*6000 + 0.8*500 = 1000
        assert!((attrs[0] - 1000.0).abs() < EPS);
        // Zone 2: A = 0.1*2000 + 0.8*3000 = 2600
        assert!((attrs[1] - 2600.0).abs() < EPS);
    }

    #[test]
    fn custom_coefficients() {
        let trip_gen = RegressionGenerator::with_coefficients(
            RegressionCoefficients {
                intercept: 10.0,
                pop_coeff: 0.3,
                emp_coeff: 0.0,
                hh_coeff: 0.0,
                income_coeff: 0.0,
            },
            RegressionCoefficients {
                intercept: 0.0,
                pop_coeff: 0.0,
                emp_coeff: 1.2,
                hh_coeff: 0.0,
                income_coeff: 0.0,
            },
        );

        let zones = vec![
            Zone::new(1)
                .with_population(1000.0)
                .with_employment(500.0)
                .build(),
        ];
        let (prods, attrs) = trip_gen.generate(&zones).unwrap();

        // P = 10 + 0.3 * 1000 = 310
        assert!((prods[0] - 310.0).abs() < EPS);
        // A = 0 + 1.2 * 500 = 600
        assert!((attrs[0] - 600.0).abs() < EPS);
    }

    #[test]
    fn negative_result_clamped_to_zero() {
        let trip_gen = RegressionGenerator::with_coefficients(
            RegressionCoefficients {
                intercept: -1000.0,
                pop_coeff: 0.1,
                emp_coeff: 0.0,
                hh_coeff: 0.0,
                income_coeff: 0.0,
            },
            RegressionCoefficients::default(),
        );

        let zones = vec![
            Zone::new(1).with_population(100.0).build(),
        ];
        let (prods, _) = trip_gen.generate(&zones).unwrap();

        // -1000 + 0.1*100 = -990 -> clamped to 0
        assert_eq!(prods[0], 0.0);
    }

    #[test]
    fn empty_zones_returns_error() {
        let trip_gen = RegressionGenerator::new();
        let result = trip_gen.generate(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn households_and_income_coefficients() {
        let trip_gen = RegressionGenerator::with_coefficients(
            RegressionCoefficients {
                intercept: 0.0,
                pop_coeff: 0.0,
                emp_coeff: 0.0,
                hh_coeff: 2.0,
                income_coeff: 0.001,
            },
            RegressionCoefficients::default(),
        );

        let zones = vec![
            Zone::new(1)
                .with_households(500.0)
                .with_avg_income(50000.0)
                .build(),
        ];
        let (prods, _) = trip_gen.generate(&zones).unwrap();

        // P = 2.0*500 + 0.001*50000 = 1000 + 50 = 1050
        assert!((prods[0] - 1050.0).abs() < EPS);
    }
}
