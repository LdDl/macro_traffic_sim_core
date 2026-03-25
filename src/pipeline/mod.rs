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
//!    (AUTO, BIKE, WALK) via a multinomial logit model.
//! 4. **Traffic Assignment** -- assigns the AUTO OD matrix to the meso
//!    network using the configured method (Frank-Wolfe, MSA, or Gradient
//!    Projection) to find User Equilibrium link volumes. Only AUTO is
//!    assigned because BIKE and WALK do not contribute to road congestion.
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
//! ```text
//! Trip Generation
//!       |
//!       v
//! +---> Trip Distribution  <-- skim (travel time)
//! |           |
//! |           v
//! |     Mode Choice
//! |           |
//! |           v
//! +---- Assignment ------> update skim from congested costs
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

pub mod error;
mod pipeline;

pub use self::pipeline::*;
