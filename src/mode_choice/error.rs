//! # Mode Choice Errors
//!
//! Error types for the mode choice step.

use std::fmt;

/// Mode choice errors.
#[derive(Debug, Clone)]
pub enum ModeChoiceError {
    /// No utility functions defined.
    NoModes,
    /// Missing skim data for a mode.
    MissingSkim(String),
}

impl fmt::Display for ModeChoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModeChoiceError::NoModes => write!(f, "no mode utilities defined"),
            ModeChoiceError::MissingSkim(mode) => {
                write!(f, "missing skim data for mode: {}", mode)
            }
        }
    }
}
