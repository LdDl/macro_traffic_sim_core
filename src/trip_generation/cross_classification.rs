//! # Cross-Classification Trip Generation
//!
//! Lookup-table based trip generation using household size
//! and income categories.
//!
//! ## How it works
//!
//! 1. For each zone, compute the average household size
//!    (`population / households`) and classify it into a bin
//!    using `hh_size_thresholds`.
//! 2. Classify the zone's average income into a bin using
//!    `income_thresholds`.
//! 3. Look up the per-household trip rate from `production_rates`
//!    using the `(hh_size_cat, income_cat)` key.
//! 4. Production = rate * households.
//! 5. Attraction = `attraction_rate_per_employee` * employment.
//!
//! ## Examples
//!
//! ```
//! use std::collections::HashMap;
//! use macro_traffic_sim_core::zone::Zone;
//! use macro_traffic_sim_core::trip_generation::{
//!     CrossClassificationGenerator, CategoryKey, TripGenerator,
//! };
//!
//! // 2 hh-size bins (threshold at 2.5), 2 income bins (threshold at 50000)
//! let mut rates = HashMap::new();
//! rates.insert(CategoryKey { hh_size_cat: 0, income_cat: 0 }, 4.0);
//! rates.insert(CategoryKey { hh_size_cat: 0, income_cat: 1 }, 5.0);
//! rates.insert(CategoryKey { hh_size_cat: 1, income_cat: 0 }, 6.0);
//! rates.insert(CategoryKey { hh_size_cat: 1, income_cat: 1 }, 8.0);
//!
//! let trip_gen =CrossClassificationGenerator::new(
//!     rates,
//!     1.5,
//!     vec![2.5],
//!     vec![50000.0],
//! );
//!
//! let zones = vec![
//!     // avg hh size = 10000/4000 = 2.5 -> cat 1, income 60000 -> cat 1
//!     // rate = 8.0, production = 8.0 * 4000 = 32000
//!     Zone::new(1)
//!         .with_population(10000.0)
//!         .with_employment(3000.0)
//!         .with_households(4000.0)
//!         .with_avg_income(60000.0)
//!         .build(),
//! ];
//!
//! let (prods, attrs) = trip_gen.generate(&zones).unwrap();
//! assert!((prods[0] - 32000.0).abs() < 1e-10);
//! // attraction = 1.5 * 3000 = 4500
//! assert!((attrs[0] - 4500.0).abs() < 1e-10);
//! ```

use std::collections::HashMap;

use super::TripGenerator;
use super::error::TripGenerationError;
use crate::zone::Zone;

/// Category key for cross-classification lookup.
///
/// Represents a bin combination of household size and income level.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::trip_generation::CategoryKey;
///
/// let key = CategoryKey { hh_size_cat: 0, income_cat: 2 };
/// assert_eq!(key.hh_size_cat, 0);
/// assert_eq!(key.income_cat, 2);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CategoryKey {
    /// Household size category (0-based index).
    pub hh_size_cat: usize,
    /// Income category (0-based index).
    pub income_cat: usize,
}

/// Cross-classification trip generator.
///
/// Uses a lookup table keyed by `(household_size_category, income_category)`
/// to determine production rates per household. Attractions are computed
/// as a flat rate per employee.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use macro_traffic_sim_core::trip_generation::{
///     CrossClassificationGenerator, CategoryKey,
/// };
///
/// let mut rates = HashMap::new();
/// rates.insert(CategoryKey { hh_size_cat: 0, income_cat: 0 }, 5.0);
///
/// let trip_gen =CrossClassificationGenerator::new(
///     rates,
///     2.0,
///     vec![3.0],
///     vec![40000.0],
/// );
///
/// assert_eq!(trip_gen.attraction_rate_per_employee, 2.0);
/// assert_eq!(trip_gen.hh_size_thresholds, vec![3.0]);
/// ```
#[derive(Debug, Clone)]
pub struct CrossClassificationGenerator {
    /// Production trip rates per household by category.
    pub production_rates: HashMap<CategoryKey, f64>,
    /// Attraction trip rate per employee (simple scalar).
    pub attraction_rate_per_employee: f64,
    /// Household size category thresholds (e.g. `[1.5, 2.5, 3.5]` for 4 bins).
    pub hh_size_thresholds: Vec<f64>,
    /// Income category thresholds (e.g. `[30000, 60000, 90000]` for 4 bins).
    pub income_thresholds: Vec<f64>,
}

impl CrossClassificationGenerator {
    /// Create a new cross-classification generator.
    ///
    /// # Arguments
    /// * `production_rates` - Lookup table of trip rates per household.
    /// * `attraction_rate_per_employee` - Trips attracted per employee.
    /// * `hh_size_thresholds` - Breakpoints for household size bins.
    /// * `income_thresholds` - Breakpoints for income bins.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use macro_traffic_sim_core::trip_generation::{
    ///     CrossClassificationGenerator, CategoryKey,
    /// };
    ///
    /// let mut rates = HashMap::new();
    /// rates.insert(CategoryKey { hh_size_cat: 0, income_cat: 0 }, 3.5);
    /// rates.insert(CategoryKey { hh_size_cat: 1, income_cat: 0 }, 5.5);
    ///
    /// let trip_gen =CrossClassificationGenerator::new(
    ///     rates.clone(),
    ///     1.0,
    ///     vec![2.0],
    ///     Vec::new(),
    /// );
    ///
    /// // With no income thresholds, all zones fall into income_cat 0
    /// assert_eq!(trip_gen.income_thresholds.len(), 0);
    /// ```
    pub fn new(
        production_rates: HashMap<CategoryKey, f64>,
        attraction_rate_per_employee: f64,
        hh_size_thresholds: Vec<f64>,
        income_thresholds: Vec<f64>,
    ) -> Self {
        CrossClassificationGenerator {
            production_rates,
            attraction_rate_per_employee,
            hh_size_thresholds,
            income_thresholds,
        }
    }

    /// Classify a value into a category bin based on thresholds.
    ///
    /// Returns the index of the first threshold that exceeds `value`,
    /// or `thresholds.len()` if the value is above all thresholds.
    fn classify(value: f64, thresholds: &[f64]) -> usize {
        for (i, &threshold) in thresholds.iter().enumerate() {
            if value < threshold {
                return i;
            }
        }
        thresholds.len()
    }
}

impl TripGenerator for CrossClassificationGenerator {
    fn generate(&self, zones: &[Zone]) -> Result<(Vec<f64>, Vec<f64>), TripGenerationError> {
        if zones.is_empty() {
            return Err(TripGenerationError::NoZones);
        }

        let mut productions = Vec::with_capacity(zones.len());
        let mut attractions = Vec::with_capacity(zones.len());

        for zone in zones {
            let avg_hh_size = if zone.households > 0.0 {
                zone.population / zone.households
            } else {
                1.0
            };

            let hh_size_cat = Self::classify(avg_hh_size, &self.hh_size_thresholds);
            let income_cat = Self::classify(zone.avg_income, &self.income_thresholds);

            let key = CategoryKey {
                hh_size_cat,
                income_cat,
            };

            let rate = self.production_rates.get(&key).copied().unwrap_or(0.0);
            let production = rate * zone.households;
            productions.push(production.max(0.0));

            let attraction = self.attraction_rate_per_employee * zone.employment;
            attractions.push(attraction.max(0.0));
        }

        Ok((productions, attractions))
    }
}
