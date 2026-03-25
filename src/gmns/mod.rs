//! # GMNS Module
//!
//! Network data model based on GMNS (General Modeling Network Specification).
//!
//! ## Levels
//!
//! - [`meso`] -- mesoscopic network (the working graph for all computations)
//!
//! ## Shared
//!
//! - [`types`] -- ID aliases and enumeration types
//! - [`defaults`] -- default lanes/speed/capacity by link type
//! - [`error`] -- graph error types
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::gmns::meso::network::Network;
//! use macro_traffic_sim_core::gmns::meso::node::Node;
//! use macro_traffic_sim_core::gmns::meso::link::Link;
//!
//! let mut net = Network::new();
//! net.add_node(Node::new(1).build()).unwrap();
//! net.add_node(Node::new(2).build()).unwrap();
//! net.add_link(Link::new(10, 1, 2).build()).unwrap();
//!
//! assert_eq!(net.node_count(), 2);
//! assert_eq!(net.link_count(), 1);
//! ```
pub mod defaults;
pub mod error;
pub mod meso;
pub mod types;

pub use self::{defaults::*, meso::*, types::*};
