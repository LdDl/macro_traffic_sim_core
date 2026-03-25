//! # Mode Choice Module (Step 3)
//!
//! Splits total trip demand into per-mode OD matrices
//! using the multinomial logit model.
//!
//! Modes: AUTO, BIKE, WALK (no public transit).
//!
//! ## Overview
//!
//! The mode choice step takes the total OD matrix from trip distribution
//! (Step 2) and splits each OD pair's demand among available travel modes
//! according to a discrete choice model.
//!
//! The probability of choosing mode `k` for OD pair `(i, j)` is:
//!
//! ```text
//! P(k | i,j) = exp(V_k(i,j)) / sum_m( exp(V_m(i,j)) )
//! ```
//!
//! where `V_k` is the systematic utility of mode `k`, computed from
//! travel time, distance, cost, and an alternative-specific constant (ASC).
//!
//! ## Components
//!
//! - [`ModeUtility`] - Utility function coefficients for a single mode
//! - [`ModeUtilityBuilder`] - Builder for constructing [`ModeUtility`]
//! - [`ModeSkim`] - Skim data (time, distance, cost matrices) for a mode
//! - [`MultinomialLogit`] - Logit model that performs the split
//!
//! ## Examples
//!
//! ### Building utility functions
//!
//! ```
//! use macro_traffic_sim_core::gmns::types::AgentType;
//! use macro_traffic_sim_core::mode_choice::ModeUtility;
//!
//! let auto_utility = ModeUtility::new(AgentType::Auto)
//!     .with_asc(0.0)
//!     .with_coeff_time(-0.03)
//!     .with_coeff_cost(-0.01)
//!     .build();
//!
//! // V = 0.0 + (-0.03)*20.0 + (-0.01)*5.0 = -0.65
//! let v = auto_utility.compute(20.0, 0.0, 5.0);
//! assert!((v - (-0.65)).abs() < 1e-10);
//! ```
//!
//! ### Using the default AUTO/BIKE/WALK model
//!
//! ```
//! use macro_traffic_sim_core::mode_choice::MultinomialLogit;
//!
//! let model = MultinomialLogit::default_auto_bike_walk();
//! assert_eq!(model.utilities.len(), 3);
//! ```
pub mod error;
pub mod logit;
pub mod utility;

pub use self::{logit::*, utility::*};
