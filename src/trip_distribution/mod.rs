//! # Trip Distribution Module (Step 2)
//!
//! Distributes generated trips between origin and destination zones
//! using the gravity model with configurable impedance functions.
//!
//! ## How it works
//!
//! The gravity model computes an initial trip matrix:
//!
//! ```text
//! T_ij = O_i * D_j * f(c_ij) / sum_k( D_k * f(c_ik) )
//! ```
//!
//! where:
//! - `O_i` - productions at zone `i`
//! - `D_j` - attractions at zone `j`
//! - `f(c)` - impedance function applied to the travel cost `c`
//!
//! The initial matrix is then balanced using the Furness method (IPF)
//! so that row sums match productions and column sums match attractions
//! (doubly-constrained).
//!
//! ## Impedance functions
//!
//! | Function | Formula | Best for |
//! |---|---|---|
//! | [`ExponentialImpedance`] | `exp(-beta * c)` | Travel time as cost |
//! | [`PowerImpedance`] | `c^(-alpha)` | Distance as cost |
//! | [`CombinedImpedance`] | `c^(-alpha) * exp(-beta * c)` | Flexible calibration |
//!
//! ## Components
//!
//! - [`impedance`] - Impedance function trait and implementations
//! - [`gravity`] - Gravity model with doubly-constrained balancing
//! - [`furness`] - Iterative Proportional Fitting (Furness method)
//! - [`error`] - Trip distribution error types
//!
//! ## Examples
//!
//! ### Full gravity model distribution
//!
//! ```
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//! use macro_traffic_sim_core::trip_distribution::{
//!     GravityModel, ExponentialImpedance, ImpedanceFunction,
//! };
//!
//! let zones = vec![1, 2, 3];
//! let productions = vec![100.0, 200.0, 150.0];
//! let attractions = vec![120.0, 180.0, 150.0];
//!
//! // Cost matrix (e.g. travel time in hours)
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
//!     &productions,
//!     &attractions,
//!     &cost,
//!     &impedance,
//!     &zones,
//! ).unwrap();
//!
//! // Row sums should approximate productions (doubly-constrained)
//! let row1 = od.row_sum(1);
//! assert!((row1 - 100.0).abs() < 1.0);
//! ```
pub mod error;
pub mod furness;
pub mod gravity;
pub mod impedance;

pub use self::{furness::*, gravity::*, impedance::*};
