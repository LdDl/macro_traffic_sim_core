//! # Pipeline Module
//!
//! Orchestrator for the classical 4-step traffic demand model.
//!
//! ## Steps
//!
//! 1. **Trip Generation** -- computes productions and attractions per zone
//!    using the provided [`TripGenerator`] implementation.
//! 2. **Trip Distribution** -- distributes trips between zone pairs using
//!    a gravity model with Furness (IPF) balancing. The impedance function
//!    and a zone-to-zone travel time skim drive the distribution.
//! 3. **Mode Choice** -- splits the total OD matrix into per-mode matrices
//!    (AUTO, BIKE, WALK, and optionally TRANSIT) via a multinomial logit
//!    model. When a transit layer is supplied, the transit alternative is
//!    fed by a transit level-of-service skim.
//! 4. **Traffic Assignment** -- assigns the AUTO OD to the meso network
//!    using the configured method (Frank-Wolfe, MSA, Gradient Projection,
//!    or Diagonalization) to find User Equilibrium link volumes (BIKE and
//!    WALK do not congest roads). The TRANSIT OD is assigned separately with
//!    the optimal strategies algorithm on the transit network.
//!
//! ## Feedback loop
//!
//! Steps 2-4 run inside a feedback loop controlled by
//! [`ModelConfig::feedback_iterations`](crate::config::ModelConfig::feedback_iterations).
//! After each assignment, the link costs (congested travel times)
//! are used to recompute the skim matrix, which feeds back into
//! trip distribution and mode choice. This captures the interaction
//! between congestion and route/mode choice.
//!
//! Road and transit are two-way coupled inside the loop: transit vehicles in
//! mixed traffic preload the road links they run on (helping congest them),
//! and that road congestion raises the in-vehicle time of those transit
//! segments, so the transit skim is recomputed each iteration. See
//! [`TransitInput`] and [`crate::transit::road_interaction`].
//!
//! ```text
//! Trip Generation
//!       |
//!       v
//! +---> Trip Distribution  <-- skim (road + transit travel time)
//! |           |
//! |           v
//! |     Mode Choice  (AUTO / BIKE / WALK / TRANSIT)
//! |         |      \
//! |         v       v
//! |   Road Assign    Transit Assign
//! |    ^   |               |
//! |    |   +-- buses preload the road
//! |    +------ congestion slows transit
//! +---- update skims from congested costs
//!       (repeat N times)
//! ```
//!
//! ## Components
//!
//! - [`run_four_step_model`] -- main entry point
//! - [`PipelineResult`] -- output: productions, attractions, OD matrices,
//!   assignment result, feedback iteration count
//! - [`error::PipelineError`] -- pipeline-specific error types
//!
//! ## Skim computation
//!
//! The pipeline computes three types of skim matrices:
//!
//! - **Time skim (AUTO)** -- shortest path travel times from the
//!   assignment network, converted from hours to minutes.
//! - **Distance skim** -- Haversine great-circle distance (km) between
//!   zone centroids. Used for BIKE and WALK time estimates.
//! - **Speed-based time skim** -- distance / fixed speed, used for
//!   non-motorized modes (BIKE at 15 km/h, WALK at 5 km/h).

mod connectivity;
pub mod error;
pub mod phase;
mod pipeline;

pub use self::phase::{PipelinePhase, ProgressEvent};
pub use self::pipeline::*;
