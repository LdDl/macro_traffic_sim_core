//! # Graph Errors
//!
//! Error types for the GMNS graph structure.
//!
//! ## Variants
//!
//! - [`GraphError::NodeNotFound`] -- missing node lookup
//! - [`GraphError::LinkNotFound`] -- missing link lookup
//! - [`GraphError::ZoneNotFound`] -- missing zone centroid
//! - [`GraphError::DuplicateId`] -- duplicate entity insertion
//! - [`GraphError::InvalidTopology`] -- structural inconsistency
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::gmns::error::GraphError;
//!
//! let err = GraphError::NodeNotFound { node_id: 42 };
//! assert_eq!(err.to_string(), "node not found: 42");
//!
//! let err = GraphError::DuplicateId {
//!     entity: "link".to_string(),
//!     id: 7,
//! };
//! assert_eq!(err.to_string(), "duplicate link id: 7");
//! ```

use std::fmt;

/// Graph-related errors (nodes, links, zones).
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::gmns::error::GraphError;
///
/// let err = GraphError::ZoneNotFound { zone_id: 10 };
/// assert_eq!(err.to_string(), "zone not found: 10");
///
/// let err = GraphError::InvalidTopology("dangling link".to_string());
/// assert_eq!(err.to_string(), "invalid topology: dangling link");
/// ```
#[derive(Debug, Clone)]
pub enum GraphError {
    /// Node not found by ID.
    NodeNotFound { node_id: i64 },
    /// Link not found by ID.
    LinkNotFound { link_id: i64 },
    /// Zone not found by ID.
    ZoneNotFound { zone_id: i64 },
    /// Duplicate entity ID.
    DuplicateId { entity: String, id: i64 },
    /// Invalid network topology.
    InvalidTopology(String),
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::NodeNotFound { node_id } => {
                write!(f, "node not found: {}", node_id)
            }
            GraphError::LinkNotFound { link_id } => {
                write!(f, "link not found: {}", link_id)
            }
            GraphError::ZoneNotFound { zone_id } => {
                write!(f, "zone not found: {}", zone_id)
            }
            GraphError::DuplicateId { entity, id } => {
                write!(f, "duplicate {} id: {}", entity, id)
            }
            GraphError::InvalidTopology(msg) => {
                write!(f, "invalid topology: {}", msg)
            }
        }
    }
}
