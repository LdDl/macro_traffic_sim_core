//! # Location Records
//!
//! A point along a link, following the GMNS `location` table:
//! <https://github.com/zephyr-data-specs/GMNS/blob/develop/docs/spec/location.md>
//!
//! "A location is a vertex that is associated with a specific location
//! along a link. Locations may be used to represent places where
//! activities occur (e.g., driveways and bus stops)."
//!
//! The road graph itself is untouched: nodes stay intersections, links
//! stay streets. A location references its link and a linear offset
//! along it, so transit stops live mid-link without splitting the link.
//!
//! Like the GMNS schema itself, locations are resolution-agnostic: they
//! reference the links of whatever network layer they annotate. The
//! network container of each layer stores its own locations (currently
//! [`Network`](crate::gmns::meso::network::Network) at the meso level).

use crate::gmns::types::*;

/// A point along a link (GMNS `location` record).
///
/// The primary transit use case: a bus stop on a street link, with
/// `gtfs_stop_id` linking it to the GTFS feed. The user provides this
/// linkage; no map matching is performed.
#[derive(Debug, Clone)]
pub struct Location {
    /// Unique location identifier.
    pub id: i64,
    /// Link this location lies on.
    pub link_id: LinkID,
    /// Start node of that link; the offset is measured from it.
    pub ref_node_id: NodeID,
    /// Linear reference: distance along the link from `ref_node_id`, meters.
    pub lr: f64,
    /// Location classification (e.g. "bus_stop").
    pub loc_type: Option<String>,
    /// Associated zone ID. -1 if none.
    pub zone_id: ZoneID,
    /// GTFS stop this location represents, if any.
    pub gtfs_stop_id: Option<String>,
    /// WGS84 longitude.
    pub longitude: f64,
    /// WGS84 latitude.
    pub latitude: f64,
}

impl Location {
    /// Create a new builder with the required fields.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique location identifier
    /// * `link_id` - Link this location lies on
    /// * `ref_node_id` - Start node of the link (offset origin)
    /// * `lr` - Distance along the link from the reference node, meters
    ///
    /// # Example
    /// ```
    /// use macro_traffic_sim_core::gmns::location::Location;
    ///
    /// let stop = Location::new(500, 100, 1, 120.0)
    ///     .with_loc_type("bus_stop")
    ///     .with_gtfs_stop_id("stop_A")
    ///     .build();
    /// assert_eq!(stop.gtfs_stop_id.as_deref(), Some("stop_A"));
    /// ```
    pub fn new(id: i64, link_id: LinkID, ref_node_id: NodeID, lr: f64) -> LocationBuilder {
        LocationBuilder {
            instance: Location {
                id,
                link_id,
                ref_node_id,
                lr,
                loc_type: None,
                zone_id: -1,
                gtfs_stop_id: None,
                longitude: 0.0,
                latitude: 0.0,
            },
        }
    }
}

/// Builder for [`Location`], returned by [`Location::new`].
///
/// Optional fields are set through the `with_*` methods; unset fields
/// keep their defaults (no `loc_type`, no `gtfs_stop_id`, `zone_id = -1`,
/// zero coordinates). Finish with [`build`](LocationBuilder::build).
pub struct LocationBuilder {
    instance: Location,
}

impl LocationBuilder {
    /// Sets the location classification, e.g. `"bus_stop"`.
    /// GMNS recommends OpenStreetMap feature names.
    ///
    /// # Arguments
    ///
    /// * `loc_type` - Classification of what the location represents
    pub fn with_loc_type(mut self, loc_type: &str) -> Self {
        self.instance.loc_type = Some(loc_type.to_string());
        self
    }

    /// Associates the location with a zone.
    ///
    /// # Arguments
    ///
    /// * `zone_id` - Zone the location belongs to
    pub fn with_zone_id(mut self, zone_id: ZoneID) -> Self {
        self.instance.zone_id = zone_id;
        self
    }

    /// Links the location to a GTFS stop. This is the user-provided
    /// linkage consumed by
    /// [`Network::gtfs_stop_mapping`](crate::gmns::meso::network::Network::gtfs_stop_mapping);
    /// no map matching is performed.
    ///
    /// # Arguments
    ///
    /// * `gtfs_stop_id` - `stop_id` from the GTFS `stops.txt` this location represents
    pub fn with_gtfs_stop_id(mut self, gtfs_stop_id: &str) -> Self {
        self.instance.gtfs_stop_id = Some(gtfs_stop_id.to_string());
        self
    }

    /// Sets WGS84 coordinates. Optional: per GMNS they may also be
    /// derived from the link geometry and the linear reference.
    ///
    /// # Arguments
    ///
    /// * `latitude` - WGS84 latitude
    /// * `longitude` - WGS84 longitude
    pub fn with_coordinates(mut self, latitude: f64, longitude: f64) -> Self {
        self.instance.latitude = latitude;
        self.instance.longitude = longitude;
        self
    }

    /// Construct the final `Location`.
    pub fn build(self) -> Location {
        self.instance
    }
}
