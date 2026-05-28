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
//! - [`PipelineError::InvalidInput`] -- input data is unusable before
//!   any computation starts (see [`InvalidInputReason`] for details).

use std::fmt;

use crate::gmns::types::ZoneID;

/// Structured reason for [`PipelineError::InvalidInput`].
///
/// Each variant carries the data needed to produce a precise diagnostic
/// message and allows callers to react programmatically (e.g., highlight
/// zones that lack centroids in the UI).
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::pipeline::error::InvalidInputReason;
///
/// let reason = InvalidInputReason::NoZones;
/// assert_eq!(reason.to_string(), "no zones provided: at least one zone is required");
/// ```
///
/// ```
/// use macro_traffic_sim_core::pipeline::error::InvalidInputReason;
///
/// let reason = InvalidInputReason::NoCentroids { zone_count: 3, missing_ids: vec![1, 2, 3] };
/// let msg = reason.to_string();
/// assert!(msg.contains("3 zones provided"));
/// assert!(msg.contains("[1, 2, 3]"));
/// ```
///
/// ```
/// use macro_traffic_sim_core::pipeline::error::InvalidInputReason;
///
/// let reason = InvalidInputReason::DisconnectedComponents {
///     components: vec![vec![1, 2], vec![3, 4]],
/// };
/// let msg = reason.to_string();
/// assert!(msg.contains("2 disconnected"));
/// assert!(msg.contains("[1, 2]"));
/// assert!(msg.contains("[3, 4]"));
/// ```
#[derive(Debug, Clone)]
pub enum InvalidInputReason {
    /// No zones were provided at all.
    NoZones,
    /// Zones exist but none of them have a centroid node in the network.
    ///
    /// `zone_count` is the total number of zones; `missing_ids` lists
    /// every zone ID that has no corresponding centroid node.
    NoCentroids {
        zone_count: usize,
        missing_ids: Vec<ZoneID>,
    },
    /// All zones have zero population, employment, and households.
    ///
    /// `zone_ids` lists every zone ID with all-zero attributes.
    ZeroAttributes { zone_ids: Vec<ZoneID> },
    /// Trip generation produced zero total productions.
    ///
    /// Carries the aggregate zone attributes used by the generator so
    /// the caller can tell which coefficients to check.
    ZeroProductions {
        total_pop: f64,
        total_emp: f64,
        total_hh: f64,
    },
    /// Trip generation produced zero total attractions.
    ZeroAttractions {
        total_pop: f64,
        total_emp: f64,
        total_hh: f64,
    },
    /// Zone centroids form multiple strongly-connected components.
    ///
    /// Furness (IPF) requires every zone to be able to exchange trips
    /// with every other zone. If the centroid graph splits into
    /// isolated groups, the within-component productions and attractions
    /// must balance independently — a constraint the model cannot
    /// enforce automatically.
    ///
    /// `components` lists the zone IDs in each component, sorted by
    /// the smallest zone ID in each group.
    DisconnectedComponents { components: Vec<Vec<ZoneID>> },
}

impl fmt::Display for InvalidInputReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidInputReason::NoZones => {
                write!(f, "no zones provided: at least one zone is required")
            }
            InvalidInputReason::NoCentroids {
                zone_count,
                missing_ids,
            } => {
                write!(
                    f,
                    "no zone centroids found in network: {} zones provided but none have a \
                     corresponding centroid node (zone_id on a network node); missing zone IDs: {:?}",
                    zone_count, missing_ids
                )
            }
            InvalidInputReason::ZeroAttributes { zone_ids } => {
                write!(
                    f,
                    "all zones have zero population, employment, and households: \
                     trip generation will produce zero demand; affected zone IDs: {:?}",
                    zone_ids
                )
            }
            InvalidInputReason::ZeroProductions {
                total_pop,
                total_emp,
                total_hh,
            } => {
                write!(
                    f,
                    "trip generation produced zero total productions: \
                     check production coefficients \
                     (total_pop={:.1}, total_emp={:.1}, total_hh={:.1})",
                    total_pop, total_emp, total_hh
                )
            }
            InvalidInputReason::ZeroAttractions {
                total_pop,
                total_emp,
                total_hh,
            } => {
                write!(
                    f,
                    "trip generation produced zero total attractions: \
                     check attraction coefficients \
                     (total_pop={:.1}, total_emp={:.1}, total_hh={:.1})",
                    total_pop, total_emp, total_hh
                )
            }
            InvalidInputReason::DisconnectedComponents { components } => {
                let parts: Vec<String> = components
                    .iter()
                    .map(|c| format!("{:?}", c))
                    .collect();
                write!(
                    f,
                    "{} disconnected zone components detected: Furness requires every zone \
                     to exchange trips with every other zone; each component must be \
                     modeled separately; components: [{}]",
                    components.len(),
                    parts.join(", ")
                )
            }
        }
    }
}

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
///
/// ```
/// use macro_traffic_sim_core::pipeline::error::InvalidInputReason;
/// use macro_traffic_sim_core::pipeline::error::PipelineError;
///
/// let err = PipelineError::InvalidInput(InvalidInputReason::NoZones);
/// assert_eq!(err.to_string(), "invalid input: no zones provided: at least one zone is required");
/// ```
#[derive(Debug, Clone)]
pub enum PipelineError {
    /// Missing result from a pipeline step.
    MissingResult(String),
    /// Invalid pipeline configuration.
    InvalidConfig(String),
    /// Invalid input data (zones, network, coefficients).
    InvalidInput(InvalidInputReason),
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
            PipelineError::InvalidInput(reason) => {
                write!(f, "invalid input: {}", reason)
            }
        }
    }
}
