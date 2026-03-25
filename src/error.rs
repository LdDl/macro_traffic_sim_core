//! # Error Module
//!
//! Top-level error type that aggregates domain-specific errors
//! from each module.
//!
//! ## Variants
//!
//! Each variant wraps a domain-specific error:
//!
//! - [`SimError::Graph`] -- network structure errors
//! - [`SimError::TripGeneration`] -- step 1 errors
//! - [`SimError::TripDistribution`] -- step 2 errors
//! - [`SimError::ModeChoice`] -- step 3 errors
//! - [`SimError::Assignment`] -- step 4 errors
//! - [`SimError::Pipeline`] -- orchestration errors
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::error::SimError;
//! use macro_traffic_sim_core::gmns::error::GraphError;
//!
//! let graph_err = GraphError::NodeNotFound { node_id: 99 };
//! let sim_err: SimError = graph_err.into();
//! assert_eq!(sim_err.to_string(), "graph error: node not found: 99");
//! ```

use std::fmt;

use crate::assignment::error::AssignmentError;
use crate::gmns::error::GraphError;
use crate::mode_choice::error::ModeChoiceError;
use crate::pipeline::error::PipelineError;
use crate::trip_distribution::error::TripDistributionError;
use crate::trip_generation::error::TripGenerationError;

/// Top-level error type for the macro traffic simulation.
///
/// Each variant wraps a domain-specific error defined in its own module.
///
/// All domain errors implement `From<T> for SimError`, so the `?`
/// operator works transparently across module boundaries.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::error::SimError;
/// use macro_traffic_sim_core::pipeline::error::PipelineError;
///
/// let err = PipelineError::InvalidConfig("bad param".to_string());
/// let sim_err: SimError = err.into();
/// assert_eq!(
///     sim_err.to_string(),
///     "pipeline error: invalid config: bad param"
/// );
/// ```
#[derive(Debug, Clone)]
pub enum SimError {
    /// Graph-related errors (nodes, links, zones).
    Graph(GraphError),
    /// Trip generation errors (Step 1).
    TripGeneration(TripGenerationError),
    /// Trip distribution errors (Step 2).
    TripDistribution(TripDistributionError),
    /// Mode choice errors (Step 3).
    ModeChoice(ModeChoiceError),
    /// Traffic assignment errors (Step 4).
    Assignment(AssignmentError),
    /// Pipeline orchestration errors.
    Pipeline(PipelineError),
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimError::Graph(e) => write!(f, "graph error: {}", e),
            SimError::TripGeneration(e) => write!(f, "trip generation error: {}", e),
            SimError::TripDistribution(e) => write!(f, "trip distribution error: {}", e),
            SimError::ModeChoice(e) => write!(f, "mode choice error: {}", e),
            SimError::Assignment(e) => write!(f, "assignment error: {}", e),
            SimError::Pipeline(e) => write!(f, "pipeline error: {}", e),
        }
    }
}

impl From<GraphError> for SimError {
    fn from(e: GraphError) -> Self {
        SimError::Graph(e)
    }
}

impl From<TripGenerationError> for SimError {
    fn from(e: TripGenerationError) -> Self {
        SimError::TripGeneration(e)
    }
}

impl From<TripDistributionError> for SimError {
    fn from(e: TripDistributionError) -> Self {
        SimError::TripDistribution(e)
    }
}

impl From<ModeChoiceError> for SimError {
    fn from(e: ModeChoiceError) -> Self {
        SimError::ModeChoice(e)
    }
}

impl From<AssignmentError> for SimError {
    fn from(e: AssignmentError) -> Self {
        SimError::Assignment(e)
    }
}

impl From<PipelineError> for SimError {
    fn from(e: PipelineError) -> Self {
        SimError::Pipeline(e)
    }
}
