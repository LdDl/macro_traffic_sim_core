//! # Logging Module
//!
//! Structured logging system for traffic simulation debugging and monitoring.
//!
//! This module provides hierarchical logging levels and structured event tracking
//! using the `tracing` crate with JSON output format.
//!
//! ## Components
//!
//! - [`VerboseLevel`] -- hierarchical debug levels (None, Main, Additional, All)
//! - [`verbose_log`] -- global logging function
//! - Event constants -- predefined event types for 4-step model phases
//! - Macros -- `log_main!`, `log_additional!`, `log_all!`
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use macro_traffic_sim_core::verbose::{set_verbose_level, VerboseLevel, EVENT_PIPELINE};
//! use macro_traffic_sim_core::log_main;
//!
//! set_verbose_level(VerboseLevel::Main);
//! log_main!(EVENT_PIPELINE, "Starting 4-step model",);
//! ```
//!
//! ## Logging Levels
//!
//! - `None` -- no logging
//! - `Main` -- major simulation phases only
//! - `Additional` -- nested function details
//! - `All` -- everything (trace level)
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::verbose::VerboseLevel;
//!
//! // Levels have a total ordering
//! assert!(VerboseLevel::Main > VerboseLevel::None);
//! assert!(VerboseLevel::All > VerboseLevel::Additional);
//!
//! // Display shows the level name
//! assert_eq!(VerboseLevel::Main.to_string(), "main");
//! ```
pub mod logger;
pub mod verbose;

pub use self::{logger::*, verbose::*};

use std::sync::Once;

static INIT: Once = Once::new();

pub fn ensure_logger_init() {
    INIT.call_once(|| {
        init_logger();
    });
}
