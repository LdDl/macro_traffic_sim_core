//! # Mesoscopic Network
//!
//! Node, link, and network structures for the meso-level graph.
//!
//! Turn restrictions are encoded in the topology: connection links
//! exist only for allowed movements. Routing algorithms respect
//! restrictions automatically by following existing links.
//!
//! ## Components
//!
//! - [`node::Node`] -- network node (intersection point or mid-link point)
//! - [`link::Link`] -- directed link (road segment or connection/turn)
//! - [`network::Network`] -- graph container with adjacency and zone lookups
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::gmns::meso::node::Node;
//! use macro_traffic_sim_core::gmns::meso::link::Link;
//! use macro_traffic_sim_core::gmns::meso::network::Network;
//!
//! let mut net = Network::new();
//! net.add_node(Node::new(1).with_zone_id(0).build()).unwrap();
//! net.add_node(Node::new(2).build()).unwrap();
//! net.add_link(
//!     Link::new(100, 1, 2)
//!         .with_length_meters(500.0)
//!         .with_lanes_num(2)
//!         .build()
//! ).unwrap();
//!
//! let link = net.get_link(100).unwrap();
//! assert_eq!(link.length_meters, 500.0);
//! assert_eq!(link.lanes_num, 2);
//! ```

pub mod link;
pub mod network;
pub mod node;

pub use self::{link::*, network::*, node::*};
