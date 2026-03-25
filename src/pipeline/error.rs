//! # Pipeline Errors
//!
//! Error types for the 4-step model pipeline orchestrator.
//!
//! ## Variants
//!
//! - [`PipelineError::MissingResult`] -- a required intermediate result
//!   is absent (e.g., mode choice did not produce an AUTO matrix).
//! - [`PipelineError::InvalidConfig`] -- the pipeline configuration is
//!   inconsistent (e.g., zero feedback iterations).

use std::fmt;

/// Pipeline orchestration errors.
///
/// These errors indicate problems with the pipeline setup or
/// unexpected missing data between steps, not algorithm failures
/// (those are reported by the step-specific error types).
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::pipeline::error::PipelineError;
///
/// let err = PipelineError::MissingResult("no AUTO OD matrix".to_string());
/// assert_eq!(err.to_string(), "missing result: no AUTO OD matrix");
/// ```
///
/// ```
/// use macro_traffic_sim_core::pipeline::error::PipelineError;
///
/// let err = PipelineError::InvalidConfig("feedback_iterations must be > 0".to_string());
/// assert_eq!(err.to_string(), "invalid config: feedback_iterations must be > 0");
/// ```
#[derive(Debug, Clone)]
pub enum PipelineError {
    /// Missing result from a pipeline step.
    MissingResult(String),
    /// Invalid pipeline configuration.
    InvalidConfig(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineError::MissingResult(msg) => {
                write!(f, "missing result: {}", msg)
            }
            PipelineError::InvalidConfig(msg) => {
                write!(f, "invalid config: {}", msg)
            }
        }
    }
}
