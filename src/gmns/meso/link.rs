//! # Meso Link
//!
//! A directed link in the mesoscopic network. Can be either a road
//! segment or a connection link (turn maneuver at an intersection).

use crate::gmns::types::*;

/// A directed link in the mesoscopic network.
///
/// Two types of links:
/// - Road segment (`is_connection = false`): a piece of road between
///   two meso nodes, inheriting properties from its parent macro link.
/// - Connection link (`is_connection = true`): a turn maneuver at an
///   intersection, created from a movement. Turn restrictions are
///   encoded in the topology: if a turn is forbidden, no connection
///   link exists.
#[derive(Debug, Clone)]
pub struct Link {
    /// Unique link identifier.
    pub id: LinkID,
    /// Source node ID.
    pub source_node_id: NodeID,
    /// Target node ID.
    pub target_node_id: NodeID,
    /// Whether this is a connection (turn) link at an intersection.
    pub is_connection: bool,
    /// Reference to the parent macro link. -1 if not applicable.
    pub macro_link_id: LinkID,
    /// Reference to the movement that generated this connection link.
    /// -1 if this is a road segment (not a connection).
    pub movement_id: MovementID,
    /// Length in meters. 0 for connection links.
    pub length_meters: f64,
    /// Number of lanes.
    pub lanes_num: i32,
    /// Capacity in vehicles per hour per lane.
    pub capacity: f64,
    /// Free-flow speed in km/h.
    pub free_speed: f64,
    /// Speed limit in km/h.
    pub max_speed: f64,
    /// Road type classification.
    pub link_type: LinkType,
    /// Allowed agent types on this link.
    pub allowed_agent_types: Vec<AgentType>,
    /// WGS84 geometry as a sequence of (longitude, latitude) points.
    pub geom: Vec<[f64; 2]>,
}

impl Link {
    /// Create a new builder with the required link ID, source, and target.
    ///
    /// # Example
    /// ```
    /// use macro_traffic_sim_core::gmns::meso::link::Link;
    /// use macro_traffic_sim_core::gmns::types::LinkType;
    ///
    /// let link = Link::new(1, 10, 20)
    ///     .with_link_type(LinkType::Primary)
    ///     .with_free_speed(80.0)
    ///     .with_capacity(1800.0)
    ///     .with_length_meters(500.0)
    ///     .build();
    /// ```
    pub fn new(id: LinkID, source_node_id: NodeID, target_node_id: NodeID) -> LinkBuilder {
        LinkBuilder {
            instance: Link {
                id,
                source_node_id,
                target_node_id,
                is_connection: false,
                macro_link_id: -1,
                movement_id: -1,
                length_meters: 0.0,
                lanes_num: 1,
                capacity: 0.0,
                free_speed: 0.0,
                max_speed: 0.0,
                link_type: LinkType::Undefined,
                allowed_agent_types: Vec::new(),
                geom: Vec::new(),
            },
        }
    }

    /// Returns the free-flow travel time in hours for this link.
    /// Returns `f64::INFINITY` if free_speed or length_meters is not positive.
    pub fn get_free_flow_time_hours(&self) -> f64 {
        if self.free_speed <= 0.0 || self.length_meters <= 0.0 {
            return f64::INFINITY;
        }
        (self.length_meters / 1000.0) / self.free_speed
    }

    /// Returns the effective capacity considering lanes.
    pub fn get_total_capacity(&self) -> f64 {
        if self.capacity <= 0.0 {
            return 0.0;
        }
        if self.lanes_num > 0 {
            self.capacity * self.lanes_num as f64
        } else {
            self.capacity
        }
    }
}

/// Builder for `Link`.
pub struct LinkBuilder {
    instance: Link,
}

impl LinkBuilder {
    pub fn with_is_connection(mut self, is_connection: bool) -> Self {
        self.instance.is_connection = is_connection;
        self
    }

    pub fn with_macro_link_id(mut self, macro_link_id: LinkID) -> Self {
        self.instance.macro_link_id = macro_link_id;
        self
    }

    pub fn with_movement_id(mut self, movement_id: MovementID) -> Self {
        self.instance.movement_id = movement_id;
        self
    }

    pub fn with_length_meters(mut self, length: f64) -> Self {
        self.instance.length_meters = length;
        self
    }

    pub fn with_lanes_num(mut self, lanes: i32) -> Self {
        self.instance.lanes_num = lanes;
        self
    }

    pub fn with_capacity(mut self, capacity: f64) -> Self {
        self.instance.capacity = capacity;
        self
    }

    pub fn with_free_speed(mut self, speed: f64) -> Self {
        self.instance.free_speed = speed;
        self
    }

    pub fn with_max_speed(mut self, speed: f64) -> Self {
        self.instance.max_speed = speed;
        self
    }

    pub fn with_link_type(mut self, link_type: LinkType) -> Self {
        self.instance.link_type = link_type;
        self
    }

    pub fn with_allowed_agent_types(mut self, agents: Vec<AgentType>) -> Self {
        self.instance.allowed_agent_types = agents;
        self
    }

    pub fn with_geom(mut self, geom: Vec<[f64; 2]>) -> Self {
        self.instance.geom = geom;
        self
    }

    /// Construct the final `Link`.
    pub fn build(self) -> Link {
        self.instance
    }
}
