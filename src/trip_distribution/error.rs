//! # Trip Distribution Errors
//!
//! Error types for the trip distribution step.

use std::fmt;

/// Trip distribution errors.
#[derive(Debug, Clone)]
pub enum TripDistributionError {
    /// Productions/attractions length mismatch with zone IDs.
    DimensionMismatch(String),
    /// Furness balancing did not converge.
    FurnessNotConverged { max_iterations: usize },
}

impl fmt::Display for TripDistributionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TripDistributionError::DimensionMismatch(msg) => {
                write!(f, "dimension mismatch: {}", msg)
            }
            TripDistributionError::FurnessNotConverged { max_iterations } => {
                write!(
                    f,
                    "Furness did not converge after {} iterations",
                    max_iterations
                )
            }
        }
    }
}
