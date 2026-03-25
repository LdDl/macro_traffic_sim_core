//! # Trip Generation Errors
//!
//! Error types for the trip generation step.

use std::fmt;

/// Trip generation errors.
#[derive(Debug, Clone)]
pub enum TripGenerationError {
    /// No zones provided.
    NoZones,
    /// Invalid coefficients or parameters.
    InvalidParams(String),
}

impl fmt::Display for TripGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TripGenerationError::NoZones => write!(f, "no zones provided"),
            TripGenerationError::InvalidParams(msg) => {
                write!(f, "invalid parameters: {}", msg)
            }
        }
    }
}
