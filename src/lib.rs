//! # Macro Traffic Simulation Core
//!
//! Implementation of the classical 4-step macroscopic traffic demand model:
//!
//! 1. **Trip Generation** - Produces trip productions/attractions per zone
//! 2. **Trip Distribution** - Gravity model with Furness balancing
//! 3. **Mode Choice** - Multinomial logit (AUTO, BIKE, WALK)
//! 4. **Traffic Assignment** - User Equilibrium via Frank-Wolfe, MSA, or Gradient Projection
//!
//! Note on public transport:
//! Assignment (frequency-based, Spiess & Florian optimal strategies) is available in the [`transit`] module.
//!
//! ## Network Format
//!
//! Based on GMNS (General Modeling Network Specification) at the mesoscopic level.
//! Turn restrictions are encoded in the graph topology: connection links
//! exist only for allowed movements. Routing algorithms respect
//! restrictions automatically.
//!
//! ## Transit Network (public transport)
//!
//! Transit network lives in the [`transit`] module and rests on two graphs joined
//! by one bridge:
//!
//! - The **ROAD** network ([`gmns::meso`]) is never modified by transit:
//!   stops do not split links. A stop is a GMNS [`gmns::location`]
//!   record - a point on a link (`link_id` + linear offset) with an
//!   optional `gtfs_stop_id` linkage provided by the user (no map
//!   matching).
//! - The **TRANSIT** network ([`transit::TransitNetwork`]) is the data
//!   model: [`transit::TransitRoute`]s (ordered stop sequence,
//!   per-segment travel times, service headway; one-way - the opposite
//!   direction is a separate route) plus [`transit::WalkLink`]s for
//!   transfers and zone access. Stops are opaque `i64` IDs, typically
//!   location IDs.
//! - At assignment time ([`transit::assign_transit`]) the routes are
//!   expanded into an ephemeral route graph - one route node per
//!   (route, stop) - solved by the [`hyperpaths-rs`](https://crates.io/crates/hyperpaths-rs) crate:
//!
//! ```text
//! (A) --board: wait 1/f--> [L1@A] --ride: t--> [L1@B] --alight--> (B)
//! (B) --walk: t--> (C)
//! ```
//!
//! Boarding links carry the waiting (frequency = 1/headway); riding,
//! alighting and walk links have none. Zones of the transit OD matrix
//! are stop IDs; a zone centroid can act as a pseudo-stop connected to
//! several candidate stops by walk links - the algorithm itself picks
//! the access stop per destination. Networks are built by hand or
//! reconstructed from a GTFS Schedule dataset
//! ([`transit::transit_network_from_gtfs`]: trips grouped into patterns,
//! headways from `frequencies.txt`).
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
pub mod transit;
pub mod trip_distribution;
pub mod trip_generation;
pub mod verbose;
pub mod zone;
