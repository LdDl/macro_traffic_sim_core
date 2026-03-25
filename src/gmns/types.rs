//! # GMNS Type Aliases and Enumerations
//!
//! Core type aliases and classification enums for the mesoscopic network.
//!
//! ## Type aliases
//!
//! - [`NodeID`], [`LinkID`], [`MovementID`], [`ZoneID`] -- `i64` identifiers
//!
//! ## Enumerations
//!
//! - [`AgentType`] -- mode of travel (Auto, Bike, Walk)
//! - [`LinkType`] -- road classification from OSM highway tags
//! - [`ControlType`] -- traffic signal presence
//! - [`BoundaryType`] -- network edge classification
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::gmns::types::{AgentType, LinkType};
//!
//! let mode = AgentType::Auto;
//! assert_eq!(mode.to_string(), "auto");
//!
//! let road = LinkType::Primary;
//! assert_eq!(road.to_string(), "primary");
//! ```

use std::fmt;

/// Unique identifier for a node.
pub type NodeID = i64;

/// Unique identifier for a link.
pub type LinkID = i64;

/// Unique identifier for a movement (turn maneuver).
///
/// Stored as a reference attribute on connection links.
/// The core does not interpret this value -- it exists for
/// user-side aggregation and reporting.
pub type MovementID = i64;

/// Unique identifier for a zone.
pub type ZoneID = i64;

/// Agent type classification (no public transit).
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::gmns::types::AgentType;
///
/// assert_eq!(AgentType::Auto.to_string(), "auto");
/// assert_eq!(AgentType::Bike.to_string(), "bike");
/// assert_eq!(AgentType::Walk.to_string(), "walk");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentType {
    Undefined,
    Auto,
    Bike,
    Walk,
}

impl fmt::Display for AgentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            AgentType::Undefined => "undefined",
            AgentType::Auto => "auto",
            AgentType::Bike => "bike",
            AgentType::Walk => "walk",
        };
        write!(f, "{}", s)
    }
}

/// Road type classification derived from OSM highway tags.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::gmns::types::LinkType;
///
/// assert_eq!(LinkType::Motorway.to_string(), "motorway");
/// assert_eq!(LinkType::Residential.to_string(), "residential");
/// assert_eq!(LinkType::Connector.to_string(), "connector");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkType {
    Undefined,
    Motorway,
    Trunk,
    Primary,
    Secondary,
    Tertiary,
    Residential,
    LivingStreet,
    Service,
    Cycleway,
    Footway,
    Track,
    Unclassified,
    Connector,
    Railway,
    Aeroway,
}

impl fmt::Display for LinkType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            LinkType::Undefined => "undefined",
            LinkType::Motorway => "motorway",
            LinkType::Trunk => "trunk",
            LinkType::Primary => "primary",
            LinkType::Secondary => "secondary",
            LinkType::Tertiary => "tertiary",
            LinkType::Residential => "residential",
            LinkType::LivingStreet => "living_street",
            LinkType::Service => "service",
            LinkType::Cycleway => "cycleway",
            LinkType::Footway => "footway",
            LinkType::Track => "track",
            LinkType::Unclassified => "unclassified",
            LinkType::Connector => "connector",
            LinkType::Railway => "railway",
            LinkType::Aeroway => "aeroway",
        };
        write!(f, "{}", s)
    }
}

/// Traffic control type at a node or downstream end of a link.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::gmns::types::ControlType;
///
/// assert_eq!(ControlType::NotSignal.to_string(), "not_signal");
/// assert_eq!(ControlType::IsSignal.to_string(), "is_signal");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlType {
    NotSignal,
    IsSignal,
}

impl fmt::Display for ControlType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ControlType::NotSignal => "not_signal",
            ControlType::IsSignal => "is_signal",
        };
        write!(f, "{}", s)
    }
}

/// Boundary classification for nodes at network edges.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::gmns::types::BoundaryType;
///
/// assert_eq!(BoundaryType::None.to_string(), "none");
/// assert_eq!(BoundaryType::IncomeOnly.to_string(), "income_only");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryType {
    None,
    IncomeOnly,
    OutcomeOnly,
    IncomeOutcome,
}

impl fmt::Display for BoundaryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BoundaryType::None => "none",
            BoundaryType::IncomeOnly => "income_only",
            BoundaryType::OutcomeOnly => "outcome_only",
            BoundaryType::IncomeOutcome => "income_outcome",
        };
        write!(f, "{}", s)
    }
}
