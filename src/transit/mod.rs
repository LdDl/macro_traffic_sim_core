//! # Transit Assignment Module
//!
//! Frequency-based public transit assignment using the optimal strategies
//! algorithm (Spiess & Florian, 1989, DOI: <https://doi.org/10.1016/0191-2615(89)90034-9>).
//! The strategy computation and demand loading are provided by the
//! [`hyperpaths-rs`](https://crates.io/crates/hyperpaths-rs) crate;
//! this module adds the transit network data model (routes, headways, walk links),
//! the route graph expansion, and the per-destination assignment loop.
//!
//! ## Components
//!
//! - [`route`] -- the transit network data model:
//!   [`TransitRoute`], [`WalkLink`], [`TransitNetwork`]
//! - [`assignment`] -- route graph expansion and [`assign_transit`],
//!   producing [`TransitAssignmentResult`]
//! - [`from_gtfs`] -- [`transit_network_from_gtfs`]: pattern
//!   reconstruction from a GTFS Schedule dataset
//! - [`error`] -- [`TransitError`]
//!
//! ## Model
//!
//! A transit network is a set of [`TransitRoute`]s (ordered stop sequences
//! with per-segment travel times and a service headway) plus optional
//! [`WalkLink`]s between stops. Zones of the transit OD matrix are stop node IDs.
//!
//! Passengers at a stop choose a set of attractive lines (a strategy) and
//! board whichever attractive vehicle arrives first. Expected waiting time
//! at a stop is `1 / (combined frequency of attractive lines)`; links
//! without waiting (walking, alighting, riding) are handled exactly via
//! the paper's modified algorithm (p. 96), not with a big-M frequency.
//!
//! ## Zone access
//!
//! By default zones coincide with stops: demand is loaded directly at the
//! stop nodes. Walk-based access is modeled by adding a zone centroid as
//! a pseudo-stop connected to several candidate stops with [`WalkLink`]s (walking times);
//! the OD matrix then uses the centroid IDs. The algorithm itself picks
//! the access stop per destination - the one minimizing walking time
//! plus the expected remaining journey - so connect all candidates within
//! walking range rather than only the nearest one.
//!
//! These walk links can be built by hand or generated from coordinates
//! with [`generate_access_connectors`] (or the
//! [`TransitNetwork::add_access_connectors`] convenience): each centroid is
//! connected to its nearest candidate stops within a search radius, both
//! directions, with walking times derived from a walking speed. This is
//! generation from coordinates, not map matching.
//!
//! ## Example
//!
//! ```
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//! use macro_traffic_sim_core::transit::{assign_transit, TransitNetwork, TransitRoute};
//!
//! // Spiess & Florian (1989) example network: stops A=1, X=2, Y=3, B=4
//! let mut network = TransitNetwork::new();
//! network.add_route(TransitRoute::new("L1", vec![1, 4], vec![25.0], 6.0));
//! network.add_route(TransitRoute::new("L2", vec![1, 2, 3], vec![7.0, 6.0], 6.0));
//! network.add_route(TransitRoute::new("L3", vec![2, 3, 4], vec![4.0, 4.0], 15.0));
//! network.add_route(TransitRoute::new("L4", vec![3, 4], vec![10.0], 3.0));
//!
//! let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
//! od.set(1, 4, 1.0);
//!
//! let result = assign_transit(&network, &od).unwrap();
//! // expected travel time from the paper: 27.75 minutes
//! assert!((result.od_costs[&(1, 4)] - 27.75).abs() < 1e-9);
//! ```
//!
//! ## Reference
//!
//! Spiess, H. and Florian, M. (1989) "Optimal strategies: A new assignment
//! model for transit networks". Transportation Research Part B 23(2), 83-102.
//! DOI: <https://doi.org/10.1016/0191-2615(89)90034-9>
//!
//! Solver: [`hyperpaths-rs`](https://crates.io/crates/hyperpaths-rs)

pub mod assignment;
pub mod connectors;
pub mod error;
pub mod from_gtfs;
pub mod road_interaction;
pub mod route;

pub use self::assignment::{
    TransitAssignmentOptions, TransitAssignmentResult, TransitLinkKind, TransitLinkVolume,
    assign_transit, assign_transit_with_options, transit_skim, transit_skim_with_options,
};
pub use self::connectors::{AccessConnectorParams, generate_access_connectors};
pub use self::error::TransitError;
pub use self::from_gtfs::transit_network_from_gtfs;
pub use self::road_interaction::transit_road_preload;
pub use self::route::{DEFAULT_TRANSIT_PCE, TransitNetwork, TransitRoute, WalkLink};
