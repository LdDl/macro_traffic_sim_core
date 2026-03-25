//! # Assignment Errors
//!
//! Error types for the traffic assignment step.

use std::fmt;

/// Traffic assignment errors.
#[derive(Debug, Clone)]
pub enum AssignmentError {
    /// Algorithm did not converge within max iterations.
    NotConverged {
        method: String,
        iterations: usize,
        relative_gap: f64,
    },
    /// No path found between origin and destination.
    NoPath {
        origin_zone: i64,
        destination_zone: i64,
    },
    /// Invalid configuration parameter.
    InvalidConfig(String),
}

impl fmt::Display for AssignmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssignmentError::NotConverged {
                method,
                iterations,
                relative_gap,
            } => {
                write!(
                    f,
                    "{} did not converge after {} iterations (gap: {:.6})",
                    method, iterations, relative_gap
                )
            }
            AssignmentError::NoPath {
                origin_zone,
                destination_zone,
            } => {
                write!(
                    f,
                    "no path from zone {} to zone {}",
                    origin_zone, destination_zone
                )
            }
            AssignmentError::InvalidConfig(msg) => {
                write!(f, "invalid config: {}", msg)
            }
        }
    }
}
