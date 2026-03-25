//! # Meso Node
//!
//! A node in the mesoscopic network. Can represent either a point
//! along a road segment (mid-link) or a point at an intersection.

use crate::gmns::types::*;

/// A node in the mesoscopic network.
///
/// Meso nodes fall into two categories:
/// - Segment nodes (mid-link): created where lane count changes
///   within a macro link. These have `macro_link_id` set.
/// - Intersection nodes: at macro node locations. These have
///   `macro_node_id` set.
///
/// The `incoming_links` and `outcoming_links` fields are maintained
/// by [`Network::add_link`](super::network::Network::add_link).
#[derive(Debug, Clone)]
pub struct Node {
    /// Unique node identifier.
    pub id: NodeID,
    /// Reference to the parent macro node (intersection).
    /// -1 if this is a mid-link segment node.
    pub macro_node_id: NodeID,
    /// Reference to the parent macro link (road segment).
    /// -1 if this is an intersection node.
    pub macro_link_id: LinkID,
    /// Associated zone ID. -1 if not a centroid.
    pub zone_id: ZoneID,
    /// Traffic control type.
    pub control_type: ControlType,
    /// Boundary classification.
    pub boundary_type: BoundaryType,
    /// WGS84 longitude.
    pub longitude: f64,
    /// WGS84 latitude.
    pub latitude: f64,
    /// IDs of incoming links.
    pub incoming_links: Vec<LinkID>,
    /// IDs of outgoing links.
    pub outcoming_links: Vec<LinkID>,
}

impl Node {
    /// Create a new builder with the required node ID.
    ///
    /// # Example
    /// ```
    /// use macro_traffic_sim_core::gmns::meso::node::Node;
    /// use macro_traffic_sim_core::gmns::types::ControlType;
    ///
    /// let node = Node::new(1)
    ///     .with_control_type(ControlType::IsSignal)
    ///     .with_coordinates(55.7558, 37.6173)
    ///     .build();
    /// ```
    pub fn new(id: NodeID) -> NodeBuilder {
        NodeBuilder {
            instance: Node {
                id,
                macro_node_id: -1,
                macro_link_id: -1,
                zone_id: -1,
                control_type: ControlType::NotSignal,
                boundary_type: BoundaryType::None,
                longitude: 0.0,
                latitude: 0.0,
                incoming_links: Vec::new(),
                outcoming_links: Vec::new(),
            },
        }
    }
}

/// Builder for `Node`.
pub struct NodeBuilder {
    instance: Node,
}

impl NodeBuilder {
    pub fn with_macro_node_id(mut self, macro_node_id: NodeID) -> Self {
        self.instance.macro_node_id = macro_node_id;
        self
    }

    pub fn with_macro_link_id(mut self, macro_link_id: LinkID) -> Self {
        self.instance.macro_link_id = macro_link_id;
        self
    }

    pub fn with_zone_id(mut self, zone_id: ZoneID) -> Self {
        self.instance.zone_id = zone_id;
        self
    }

    pub fn with_control_type(mut self, control_type: ControlType) -> Self {
        self.instance.control_type = control_type;
        self
    }

    pub fn with_boundary_type(mut self, boundary_type: BoundaryType) -> Self {
        self.instance.boundary_type = boundary_type;
        self
    }

    pub fn with_coordinates(mut self, latitude: f64, longitude: f64) -> Self {
        self.instance.latitude = latitude;
        self.instance.longitude = longitude;
        self
    }

    /// Construct the final `Node`.
    pub fn build(self) -> Node {
        self.instance
    }
}
