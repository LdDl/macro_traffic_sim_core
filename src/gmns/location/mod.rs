//! # GMNS Locations
//!
//! The GMNS `location` table: points along links representing places
//! where activities occur (bus stops, driveways). Resolution-agnostic,
//! like the GMNS schema itself: a location references the link table of
//! whatever network layer it annotates, so the module lives beside the
//! layers rather than inside one of them.
//!
//! ## Components
//!
//! - [`locations::Location`] -- a point on a link (link_id + linear
//!   offset from the reference node), with optional `gtfs_stop_id`
//!   linkage for transit stops
//!
//! ## Examples
//!
//! ```
//! use macro_traffic_sim_core::gmns::location::Location;
//!
//! let stop = Location::new(500, 100, 1, 120.0)
//!     .with_loc_type("bus_stop")
//!     .with_gtfs_stop_id("stop_A")
//!     .build();
//! assert_eq!(stop.link_id, 100);
//! ```

pub mod locations;

pub use self::locations::*;
