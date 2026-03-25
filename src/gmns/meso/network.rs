//! # Meso Network
//!
//! Container for the mesoscopic network: nodes and links
//! with adjacency and lookup support.
//!
//! Turn restrictions are encoded in the topology: connection links
//! exist only for allowed turns. Routing algorithms (Dijkstra, etc.)
//! respect restrictions automatically by following existing links.
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::gmns::meso::network::Network;
//! use macro_traffic_sim_core::gmns::meso::node::Node;
//! use macro_traffic_sim_core::gmns::meso::link::Link;
//!
//! let mut net = Network::new();
//!
//! net.add_node(Node::new(1).build()).unwrap();
//! net.add_node(Node::new(2).build()).unwrap();
//! net.add_link(Link::new(100, 1, 2).build()).unwrap();
//!
//! assert_eq!(net.node_count(), 2);
//! assert_eq!(net.link_count(), 1);
//! assert_eq!(net.link_target(100).unwrap(), 2);
//! ```

use std::collections::HashMap;

use super::link::Link;
use super::node::Node;
use crate::error::SimError;
use crate::gmns::error::GraphError;
use crate::gmns::types::*;

/// Container for the mesoscopic transport network.
///
/// The meso network is the sole graph format used by the computation
/// core. It consists of:
/// - Road segment links (pieces of road between nodes)
/// - Connection links (turn maneuvers at intersections)
///
/// Each link carries optional `macro_link_id` and `movement_id`
/// references for user-side aggregation.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::gmns::meso::network::Network;
/// use macro_traffic_sim_core::gmns::meso::node::Node;
/// use macro_traffic_sim_core::gmns::meso::link::Link;
///
/// let mut net = Network::new();
///
/// // Zone centroid node (zone_id = 1)
/// let node = Node::new(10).with_zone_id(1).build();
/// net.add_node(node).unwrap();
///
/// assert_eq!(net.get_zone_centroid(1).unwrap(), 10);
/// assert_eq!(net.zone_ids(), vec![1]);
/// ```
#[derive(Debug)]
pub struct Network {
    /// All meso nodes indexed by ID.
    pub nodes: HashMap<NodeID, Node>,
    /// All meso links indexed by ID.
    pub links: HashMap<LinkID, Link>,
    /// Mapping from zone ID to the centroid node ID.
    pub zone_centroids: HashMap<ZoneID, NodeID>,
}

impl Network {
    /// Create a new empty network.
    pub fn new() -> Self {
        Network {
            nodes: HashMap::new(),
            links: HashMap::new(),
            zone_centroids: HashMap::new(),
        }
    }

    /// Add a node to the network.
    ///
    /// If the node has a zone_id >= 0, it is registered as a zone centroid.
    ///
    /// # Returns
    /// Error if a node with the same ID already exists.
    pub fn add_node(&mut self, node: Node) -> Result<(), SimError> {
        let id = node.id;
        if self.nodes.contains_key(&id) {
            return Err(GraphError::DuplicateId {
                entity: "node".to_string(),
                id,
            }
            .into());
        }
        if node.zone_id >= 0 {
            self.zone_centroids.insert(node.zone_id, id);
        }
        self.nodes.insert(id, node);
        Ok(())
    }

    /// Add a link to the network. Updates source/target node adjacency.
    ///
    /// # Returns
    /// Error if a link with the same ID already exists.
    pub fn add_link(&mut self, link: Link) -> Result<(), SimError> {
        let id = link.id;
        if self.links.contains_key(&id) {
            return Err(GraphError::DuplicateId {
                entity: "link".to_string(),
                id,
            }
            .into());
        }
        let source_id = link.source_node_id;
        let target_id = link.target_node_id;
        self.links.insert(id, link);
        if let Some(source_node) = self.nodes.get_mut(&source_id) {
            source_node.outcoming_links.push(id);
        }
        if let Some(target_node) = self.nodes.get_mut(&target_id) {
            target_node.incoming_links.push(id);
        }
        Ok(())
    }

    /// Get a node by ID.
    pub fn get_node(&self, id: NodeID) -> Result<&Node, SimError> {
        self.nodes
            .get(&id)
            .ok_or_else(|| GraphError::NodeNotFound { node_id: id }.into())
    }

    /// Get a link by ID.
    pub fn get_link(&self, id: LinkID) -> Result<&Link, SimError> {
        self.links
            .get(&id)
            .ok_or_else(|| GraphError::LinkNotFound { link_id: id }.into())
    }

    /// Get outgoing links from a node.
    pub fn outgoing_links(&self, node_id: NodeID) -> Result<&[LinkID], SimError> {
        let node = self.get_node(node_id)?;
        Ok(&node.outcoming_links)
    }

    /// Get incoming links to a node.
    pub fn incoming_links(&self, node_id: NodeID) -> Result<&[LinkID], SimError> {
        let node = self.get_node(node_id)?;
        Ok(&node.incoming_links)
    }

    /// Get the target node ID of a link.
    pub fn link_target(&self, link_id: LinkID) -> Result<NodeID, SimError> {
        let link = self.get_link(link_id)?;
        Ok(link.target_node_id)
    }

    /// Get the source node ID of a link.
    pub fn link_source(&self, link_id: LinkID) -> Result<NodeID, SimError> {
        let link = self.get_link(link_id)?;
        Ok(link.source_node_id)
    }

    /// Returns all zone IDs that have centroids in the network.
    pub fn zone_ids(&self) -> Vec<ZoneID> {
        let mut zones: Vec<ZoneID> = self.zone_centroids.keys().copied().collect();
        zones.sort();
        zones
    }

    /// Returns the centroid node ID for a given zone.
    pub fn get_zone_centroid(&self, zone_id: ZoneID) -> Result<NodeID, SimError> {
        self.zone_centroids
            .get(&zone_id)
            .copied()
            .ok_or_else(|| GraphError::ZoneNotFound { zone_id }.into())
    }

    /// Returns total number of nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns total number of links.
    pub fn link_count(&self) -> usize {
        self.links.len()
    }
}

impl Default for Network {
    fn default() -> Self {
        Self::new()
    }
}
