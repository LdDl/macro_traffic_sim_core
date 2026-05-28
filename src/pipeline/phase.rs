//! # Pipeline phase reporting
//!
//! Types used by the optional progress callback in [`run_four_step_model`].
//!
//! [`run_four_step_model`]: super::pipeline::run_four_step_model

use std::fmt;

/// Current phase of the 4-step model pipeline.
///
/// Passed to the `on_progress` callback each time the pipeline
/// transitions to a new step. Phases inside the feedback loop
/// (Distribution, ModeChoice, Assignment) fire once per iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelinePhase {
    /// Input validation before any computation starts.
    Preflight,
    /// Trip generation (step 1 -- runs once).
    Generation,
    /// Trip distribution via gravity model (step 2).
    Distribution,
    /// Mode choice split (step 3).
    ModeChoice,
    /// Traffic assignment (step 4).
    Assignment,
}

impl fmt::Display for PipelinePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            PipelinePhase::Preflight => "preflight",
            PipelinePhase::Generation => "generation",
            PipelinePhase::Distribution => "distribution",
            PipelinePhase::ModeChoice => "mode_choice",
            PipelinePhase::Assignment => "assignment",
        };
        write!(f, "{}", s)
    }
}

/// Progress event passed to the `on_progress` callback.
///
/// `feedback_iter` and `feedback_total` are both 0 for phases that
/// run outside the feedback loop (Preflight, Generation).
#[derive(Debug, Clone, Copy)]
pub struct ProgressEvent {
    /// Phase that is about to start.
    pub phase: PipelinePhase,
    /// Feedback iteration index (1-based). 0 outside the feedback loop.
    pub feedback_iter: usize,
    /// Total number of feedback iterations configured. 0 outside the loop.
    pub feedback_total: usize,
}

impl ProgressEvent {
    pub fn single(phase: PipelinePhase) -> Self {
        ProgressEvent { phase, feedback_iter: 0, feedback_total: 0 }
    }

    pub fn feedback(phase: PipelinePhase, iter: usize, total: usize) -> Self {
        ProgressEvent { phase, feedback_iter: iter, feedback_total: total }
    }
}
