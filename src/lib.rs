//! # Macro Traffic Simulation Core
//!
//! Implementation of the classical 4-step macroscopic traffic demand model:
//!
//! 1. **Trip Generation** - Produces trip productions/attractions per zone
//! 2. **Trip Distribution** - Gravity model with Furness balancing
//! 3. **Mode Choice** - Multinomial logit (AUTO, BIKE, WALK)
//! 4. **Traffic Assignment** - User Equilibrium via Frank-Wolfe, MSA, or Gradient Projection
//!
//! ## Network Format
//!
//! Based on GMNS (General Modeling Network Specification) at the mesoscopic level.
//! Turn restrictions are encoded in the graph topology: connection links
//! exist only for allowed movements. Routing algorithms respect
//! restrictions automatically.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use macro_traffic_sim_core::config::{ModelConfig, AssignmentMethodType};
//! use macro_traffic_sim_core::pipeline::run_four_step_model;
//! use macro_traffic_sim_core::trip_generation::RegressionGenerator;
//! use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
//! use macro_traffic_sim_core::mode_choice::MultinomialLogit;
//!
//! // Acquire meso network (load from CSV/JSON/REST API, whatever), configure, and run
//! ```
pub mod assignment;
pub mod config;
pub mod error;
pub mod gmns;
pub mod mode_choice;
pub mod od;
pub mod pipeline;
pub mod trip_distribution;
pub mod trip_generation;
pub mod verbose;
pub mod zone;
