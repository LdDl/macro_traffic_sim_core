//! # Default Values by Link Type
//!
//! Default lanes, speed, and capacity values for each link type,
//! ported from Go GMNS codebase.

use super::types::LinkType;

/// Default parameters for a given link type.
#[derive(Debug, Clone, Copy)]
pub struct LinkDefaults {
    /// Number of lanes.
    pub lanes: i32,
    /// Free-flow speed in km/h.
    pub free_speed: f64,
    /// Capacity in vehicles per hour per lane.
    pub capacity: i32,
    /// Whether the link is one-way by default.
    pub one_way: bool,
}

/// Returns default parameters for a given link type.
///
/// # Arguments
/// * `link_type` - The link type to get defaults for.
///
/// # Returns
/// A `LinkDefaults` instance with appropriate values.
///
/// # Example
/// ```
/// use macro_traffic_sim_core::gmns::defaults::get_link_defaults;
/// use macro_traffic_sim_core::gmns::types::LinkType;
///
/// let defaults = get_link_defaults(LinkType::Motorway);
/// assert_eq!(defaults.lanes, 4);
/// assert_eq!(defaults.capacity, 2300);
/// ```
pub fn get_link_defaults(link_type: LinkType) -> LinkDefaults {
    match link_type {
        LinkType::Motorway => LinkDefaults {
            lanes: 4,
            free_speed: 120.0,
            capacity: 2300,
            one_way: false,
        },
        LinkType::Trunk => LinkDefaults {
            lanes: 3,
            free_speed: 100.0,
            capacity: 2200,
            one_way: false,
        },
        LinkType::Primary => LinkDefaults {
            lanes: 3,
            free_speed: 80.0,
            capacity: 1800,
            one_way: false,
        },
        LinkType::Secondary => LinkDefaults {
            lanes: 2,
            free_speed: 60.0,
            capacity: 1600,
            one_way: false,
        },
        LinkType::Tertiary => LinkDefaults {
            lanes: 2,
            free_speed: 40.0,
            capacity: 1200,
            one_way: false,
        },
        LinkType::Residential => LinkDefaults {
            lanes: 1,
            free_speed: 30.0,
            capacity: 1000,
            one_way: false,
        },
        LinkType::LivingStreet => LinkDefaults {
            lanes: 1,
            free_speed: 15.0,
            capacity: 800,
            one_way: false,
        },
        LinkType::Service => LinkDefaults {
            lanes: 1,
            free_speed: 30.0,
            capacity: 800,
            one_way: false,
        },
        LinkType::Cycleway => LinkDefaults {
            lanes: 1,
            free_speed: 5.0,
            capacity: 800,
            one_way: true,
        },
        LinkType::Footway => LinkDefaults {
            lanes: 1,
            free_speed: 5.0,
            capacity: 800,
            one_way: true,
        },
        LinkType::Track => LinkDefaults {
            lanes: 1,
            free_speed: 30.0,
            capacity: 800,
            one_way: true,
        },
        LinkType::Unclassified => LinkDefaults {
            lanes: 1,
            free_speed: 30.0,
            capacity: 800,
            one_way: false,
        },
        LinkType::Connector => LinkDefaults {
            lanes: 2,
            free_speed: 120.0,
            capacity: 9999,
            one_way: false,
        },
        LinkType::Railway => LinkDefaults {
            lanes: 1,
            free_speed: 50.0,
            capacity: 1000,
            one_way: true,
        },
        LinkType::Aeroway => LinkDefaults {
            lanes: 1,
            free_speed: 50.0,
            capacity: 1000,
            one_way: true,
        },
        LinkType::Undefined => LinkDefaults {
            lanes: 1,
            free_speed: 30.0,
            capacity: 800,
            one_way: false,
        },
    }
}
