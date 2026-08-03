//! # Transit Assignment Errors
//!
//! Error types for the transit assignment step.

use std::fmt;

/// Transit assignment errors.
#[derive(Debug, Clone)]
pub enum TransitError {
    /// The transit network has no routes.
    EmptyNetwork,
    /// A route has fewer than two stops.
    RouteTooShort { route_id: String },
    /// The number of segment times does not match the number of stops.
    /// A route with N stops must have exactly N-1 segment times.
    SegmentTimesMismatch {
        route_id: String,
        stops: usize,
        segments: usize,
    },
    /// A route has a zero or negative headway.
    NonPositiveHeadway { route_id: String },
    /// The waiting time factor is not strictly positive.
    InvalidWaitFactor { wait_factor: f64 },
    /// A boarding or alighting penalty is negative (or NaN).
    InvalidPenalty { name: &'static str, value: f64 },
    /// Access connector parameters are invalid.
    InvalidConnectorParams { reason: &'static str },
    /// The analysis period is not strictly positive.
    InvalidAnalysisPeriod { value: f64 },
    /// Two routes share the same id.
    DuplicateRouteId { route_id: String },
    /// An OD zone with demand is not a stop of any route or walk link.
    UnknownStop { zone: i64 },
    /// No transit path exists between an OD pair with positive demand.
    Unreachable { origin: i64, destination: i64 },
    /// A GTFS stop has no mapping to a network node/location.
    UnmappedGtfsStop { stop_id: String },
    /// The GTFS data cannot be converted (missing times, no usable
    /// trips, etc.).
    InvalidGtfsData(String),
}

impl fmt::Display for TransitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransitError::EmptyNetwork => {
                write!(f, "transit network has no routes")
            }
            TransitError::RouteTooShort { route_id } => {
                write!(f, "route '{}' has fewer than two stops", route_id)
            }
            TransitError::SegmentTimesMismatch {
                route_id,
                stops,
                segments,
            } => {
                write!(
                    f,
                    "route '{}' has {} stops but {} segment times (expected {})",
                    route_id,
                    stops,
                    segments,
                    stops - 1
                )
            }
            TransitError::NonPositiveHeadway { route_id } => {
                write!(f, "route '{}' has non-positive headway", route_id)
            }
            TransitError::InvalidWaitFactor { wait_factor } => {
                write!(
                    f,
                    "waiting time factor must be strictly positive, got {}",
                    wait_factor
                )
            }
            TransitError::InvalidPenalty { name, value } => {
                write!(f, "{} must be non-negative, got {}", name, value)
            }
            TransitError::InvalidConnectorParams { reason } => {
                write!(f, "invalid access connector parameters: {}", reason)
            }
            TransitError::InvalidAnalysisPeriod { value } => {
                write!(
                    f,
                    "analysis period must be strictly positive, got {}",
                    value
                )
            }
            TransitError::DuplicateRouteId { route_id } => {
                write!(f, "duplicate route id '{}'", route_id)
            }
            TransitError::UnknownStop { zone } => {
                write!(f, "zone {} is not a stop of any route or walk link", zone)
            }
            TransitError::Unreachable {
                origin,
                destination,
            } => {
                write!(
                    f,
                    "no transit path from zone {} to zone {}",
                    origin, destination
                )
            }
            TransitError::UnmappedGtfsStop { stop_id } => {
                write!(
                    f,
                    "GTFS stop '{}' has no mapping to a network node/location",
                    stop_id
                )
            }
            TransitError::InvalidGtfsData(msg) => {
                write!(f, "invalid GTFS data: {}", msg)
            }
        }
    }
}

impl std::error::Error for TransitError {}
