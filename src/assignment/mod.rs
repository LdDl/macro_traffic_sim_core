//! # Traffic Assignment Module (Step 4)
//!
//! Assigns OD demand to the network to find User Equilibrium
//! (Wardrop's first principle: no traveller can reduce their travel
//! time by unilaterally changing routes).
//!
//! ## Methods
//!
//! | Method | Step size | Path storage | Convergence |
//! |---|---|---|---|
//! | [`FrankWolfe`] | Bisection line search | None (link-based) | Typically good |
//! | [`Msa`] | Fixed 1/n | None (link-based) | Slower, simpler |
//! | [`GradientProjection`] | Gradient-scaled | Explicit path sets | Path-level detail |
//!
//! All three implement [`AssignmentMethod`], so they are interchangeable.
//!
//! ## Volume-delay function
//!
//! The BPR function models congestion:
//!
//! ```text
//! t(x) = t0 * (1 + alpha * (x / c) ^ beta)
//! ```
//!
//! Default parameters: `alpha = 0.15`, `beta = 4.0`.
//!
//! ## Components
//!
//! - [`AssignmentMethod`] - Trait for assignment algorithms
//! - [`VolumeDelayFunction`] - Trait for volume-delay functions
//! - [`BprFunction`] - BPR implementation
//! - [`AssignmentConfig`] - Iteration/convergence settings
//! - [`AssignmentResult`] - Output volumes, costs, convergence info
//! - [`shortest_path`] - Dijkstra, all-or-nothing, skim computation
//! - [`error`] - Assignment error types
//!
//! ## Examples
//!
//! ### BPR volume-delay function
//!
//! ```
//! use macro_traffic_sim_core::assignment::{BprFunction, VolumeDelayFunction};
//!
//! let bpr = BprFunction::default();
//!
//! // Free-flow: 10 min, no volume
//! assert_eq!(bpr.travel_time(10.0, 0.0, 1000.0), 10.0);
//!
//! // At capacity: t = 10 * (1 + 0.15 * 1^4) = 11.5
//! assert!((bpr.travel_time(10.0, 1000.0, 1000.0) - 11.5).abs() < 1e-10);
//!
//! // Over capacity: congestion grows rapidly
//! assert!(bpr.travel_time(10.0, 2000.0, 1000.0) > 11.5);
//! ```
//!
//! ### Computing the relative gap
//!
//! ```
//! use std::collections::HashMap;
//! use macro_traffic_sim_core::assignment::compute_relative_gap;
//!
//! let volumes = HashMap::from([(1, 100.0), (2, 200.0)]);
//! let costs = HashMap::from([(1, 5.0), (2, 10.0)]);
//!
//! // If auxiliary volumes equal current volumes, gap = 0
//! let gap = compute_relative_gap(&volumes, &costs, &volumes);
//! assert_eq!(gap, 0.0);
//! ```

mod assignment;
pub mod error;
pub mod frank_wolfe;
pub mod gradient_projection;
pub mod indexed_graph;
pub mod msa;
pub mod shortest_path;

pub use self::assignment::*;
pub use self::indexed_graph::IndexedGraph;
pub use self::{frank_wolfe::*, gradient_projection::*, msa::*, shortest_path::*};
