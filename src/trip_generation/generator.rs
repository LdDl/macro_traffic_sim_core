//! # Trip Generator Trait
//!
//! Common interface for trip generation methods.

use super::error::TripGenerationError;
use crate::zone::Zone;

/// Trait for trip generation methods.
///
/// Implementations produce production and attraction vectors
/// from zone socioeconomic data.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::zone::Zone;
/// use macro_traffic_sim_core::trip_generation::{RegressionGenerator, TripGenerator};
///
/// let zones = vec![
///     Zone::new(1).with_population(1000.0).with_employment(500.0).build(),
/// ];
///
/// let trip_gen = RegressionGenerator::new();
/// let (prods, attrs) = trip_gen.generate(&zones).unwrap();
///
/// // One result per zone
/// assert_eq!(prods.len(), 1);
/// assert_eq!(attrs.len(), 1);
/// ```
pub trait TripGenerator {
    /// Generate trip productions and attractions.
    ///
    /// # Arguments
    /// * `zones` - Slice of zones with socioeconomic attributes.
    ///
    /// # Returns
    /// A tuple of (productions, attractions) vectors, one entry per zone.
    ///
    /// # Errors
    /// Returns [`TripGenerationError::NoZones`] if `zones` is empty.
    fn generate(&self, zones: &[Zone]) -> Result<(Vec<f64>, Vec<f64>), TripGenerationError>;
}
