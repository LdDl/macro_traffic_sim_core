//! # Trip Generation Module (Step 1)
//!
//! Computes trip productions and attractions for each transport zone
//! based on socioeconomic data (population, employment, households, income).
//!
//! ## Methods
//!
//! | Method | Approach | Best for |
//! |---|---|---|
//! | [`RegressionGenerator`] | Linear combination of zone attributes | Simple models, quick calibration |
//! | [`CrossClassificationGenerator`] | Lookup table by category bins | Survey-based rates, heterogeneous zones |
//!
//! Both implement the [`TripGenerator`] trait, so any downstream code
//! that accepts `&dyn TripGenerator` works with either method.
//!
//! ## Components
//!
//! - [`TripGenerator`] - Trait for trip generation methods
//! - [`RegressionGenerator`] - Linear regression approach
//! - [`RegressionCoefficients`] - Coefficient set for the regression equation
//! - [`CrossClassificationGenerator`] - Lookup table approach
//! - [`CategoryKey`] - Key for cross-classification lookup
//! - [`error`] - Trip generation error types
//!
//! ## Examples
//!
//! ### Regression-based generation
//!
//! ```
//! use macro_traffic_sim_core::zone::Zone;
//! use macro_traffic_sim_core::trip_generation::{RegressionGenerator, TripGenerator};
//!
//! let zones = vec![
//!     Zone::new(1).with_population(10000.0).with_employment(5000.0).build(),
//!     Zone::new(2).with_population(20000.0).with_employment(8000.0).build(),
//! ];
//!
//! let trip_gen = RegressionGenerator::new();
//! let (productions, attractions) = trip_gen.generate(&zones).unwrap();
//!
//! assert_eq!(productions.len(), 2);
//! assert!(productions[0] > 0.0);
//! assert!(productions[1] > productions[0]);
//! ```
//!
//! ### Using either method via trait object
//!
//! ```
//! use macro_traffic_sim_core::zone::Zone;
//! use macro_traffic_sim_core::trip_generation::{RegressionGenerator, TripGenerator};
//!
//! fn total_productions(trip_gen: &dyn TripGenerator, zones: &[Zone]) -> f64 {
//!     let (prods, _) = trip_gen.generate(zones).unwrap();
//!     prods.iter().sum()
//! }
//!
//! let zones = vec![
//!     Zone::new(1).with_population(5000.0).with_employment(2000.0).build(),
//! ];
//!
//! let reg = RegressionGenerator::new();
//! assert!(total_productions(&reg, &zones) > 0.0);
//! ```

pub mod cross_classification;
pub mod error;
mod generator;
pub mod regression;

pub use self::generator::*;
pub use self::{cross_classification::*, regression::*};
