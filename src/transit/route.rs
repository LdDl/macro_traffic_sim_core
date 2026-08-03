//! # Transit Network Data Model
//!
//! Frequency-based transit network: routes (lines) with stop sequences,
//! per-segment travel times, and service headways, plus optional walk links
//! between stops.
//!
//! All times use one consistent unit: the assignment is unit-agnostic,
//! costs and headways only have to share the same unit. Hand-built
//! networks typically use minutes (as the examples do); networks produced
//! by [`from_gtfs`](crate::transit::from_gtfs) are in seconds, the GTFS
//! native unit.

use std::collections::HashSet;

use crate::transit::error::TransitError;

/// A transit route (line): an ordered sequence of stops served with a fixed
/// headway.
///
/// Stops are opaque `i64` identifiers - typically GMNS node or location
/// IDs; any ID space works as long as the transit OD matrix uses the same
/// one. A route is one-way; model the opposite direction as a separate
/// route.
#[derive(Debug, Clone)]
pub struct TransitRoute {
    /// Unique route identifier
    pub id: String,
    /// Ordered stop sequence (node IDs)
    pub stops: Vec<i64>,
    /// Travel time of each segment between consecutive stops.
    /// Must contain exactly `stops.len() - 1` entries.
    pub segment_times: Vec<f64>,
    /// Service headway (time between consecutive vehicles). Must be > 0.
    /// Frequency is `1 / headway`.
    pub headway: f64,
    /// Per-route boarding penalty. When `Some`, overrides the assignment's
    /// global `boarding_penalty` for this route only. Must be non-negative.
    pub boarding_penalty: Option<f64>,
    /// Per-route alighting penalty. When `Some`, overrides the global
    /// `alighting_penalty` for this route only. Must be non-negative.
    pub alighting_penalty: Option<f64>,
    /// Per-route dwell time. When `Some`, overrides the global `dwell_time`
    /// for this route only (a positive value activates the two-node stop
    /// scheme for this route). Must be non-negative.
    pub dwell_time: Option<f64>,
    /// Road links each segment runs on, for the road interaction: the
    /// vehicles load those links (background traffic) and the road
    /// congestion on them drives the segment's in-vehicle time.
    ///
    /// `None` (default) means the whole route runs on its own right-of-way
    /// (e.g. a metro): it neither loads the road nor is slowed by it, and
    /// keeps its fixed `segment_times`. When `Some`, it holds one entry per
    /// segment (`stops.len() - 1` of them); the user supplies the link
    /// sequence each segment traverses (no map matching). An empty inner
    /// list marks a segment on dedicated right-of-way.
    pub segment_links: Option<Vec<Vec<i64>>>,
    /// Road space of one vehicle of this route in passenger-car equivalents,
    /// used for the background load it puts on the links in `segment_links`.
    /// Ignored for a route with no road links. Default
    /// [`DEFAULT_TRANSIT_PCE`].
    pub pce: f64,
}

/// Default passenger-car-equivalent road space of a transit vehicle (a
/// typical standard bus), used when a route does not set its own `pce`.
pub const DEFAULT_TRANSIT_PCE: f64 = 2.0;

impl TransitRoute {
    /// Creates a new transit route.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique route identifier
    /// * `stops` - Ordered stop sequence (node IDs)
    /// * `segment_times` - Travel times between consecutive stops, `stops.len() - 1` entries
    /// * `headway` - Service headway in the same time unit as `segment_times`
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::TransitRoute;
    ///
    /// // Line 2 from the Spiess & Florian (1989) example:
    /// // A -> X (7 min) -> Y (6 min), headway 6 min
    /// let route = TransitRoute::new("L2", vec![1, 2, 3], vec![7.0, 6.0], 6.0);
    /// assert_eq!(route.stops.len(), 3);
    /// ```
    pub fn new(id: &str, stops: Vec<i64>, segment_times: Vec<f64>, headway: f64) -> Self {
        TransitRoute {
            id: id.to_string(),
            stops,
            segment_times,
            headway,
            boarding_penalty: None,
            alighting_penalty: None,
            dwell_time: None,
            segment_links: None,
            pce: DEFAULT_TRANSIT_PCE,
        }
    }

    /// Override the boarding penalty for this route.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::TransitRoute;
    ///
    /// // an express line with a higher perceived boarding cost
    /// let route = TransitRoute::new("EXP", vec![1, 2], vec![10.0], 12.0)
    ///     .with_boarding_penalty(5.0);
    /// assert_eq!(route.boarding_penalty, Some(5.0));
    /// ```
    pub fn with_boarding_penalty(mut self, penalty: f64) -> Self {
        self.boarding_penalty = Some(penalty);
        self
    }

    /// Override the alighting penalty for this route.
    pub fn with_alighting_penalty(mut self, penalty: f64) -> Self {
        self.alighting_penalty = Some(penalty);
        self
    }

    /// Override the dwell time for this route. A positive value activates
    /// the two-node stop scheme for this route only.
    pub fn with_dwell_time(mut self, dwell: f64) -> Self {
        self.dwell_time = Some(dwell);
        self
    }

    /// Set the road links each segment runs on (one list per segment, in
    /// order). Puts the route in mixed traffic: it loads those links and
    /// takes its in-vehicle time from their congestion. An empty list for a
    /// segment keeps that segment on dedicated right-of-way.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::TransitRoute;
    ///
    /// // a bus over stops 1 -> 2 -> 3, its two segments running on links
    /// // [100] and [104] respectively
    /// let route = TransitRoute::new("B1", vec![1, 2, 3], vec![6.0, 7.0], 6.0)
    ///     .with_segment_links(vec![vec![100], vec![104]]);
    /// assert_eq!(route.segment_links.as_ref().unwrap().len(), 2);
    /// ```
    pub fn with_segment_links(mut self, segment_links: Vec<Vec<i64>>) -> Self {
        self.segment_links = Some(segment_links);
        self
    }

    /// Set the vehicle road space in passenger-car equivalents (default
    /// [`DEFAULT_TRANSIT_PCE`]). Only used when the route has road links.
    pub fn with_pce(mut self, pce: f64) -> Self {
        self.pce = pce;
        self
    }
}

/// A walk connection between two stops (transfer or access link).
/// One-way; add the reverse direction explicitly if needed.
///
/// Besides stop-to-stop transfers, walk links model zone access: a zone
/// centroid may appear as a pseudo-stop connected to nearby stops (see
/// the module docs of [`crate::transit`], section "Zone access").
#[derive(Debug, Clone)]
pub struct WalkLink {
    /// Source stop (node ID)
    pub from_stop: i64,
    /// Target stop (node ID)
    pub to_stop: i64,
    /// Walking time in the same unit as route segment times
    pub time: f64,
}

/// Frequency-based transit network: a set of routes and optional walk links.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::transit::{TransitNetwork, TransitRoute};
///
/// let mut network = TransitNetwork::new();
/// network.add_route(TransitRoute::new("L1", vec![1, 4], vec![25.0], 6.0));
/// network.add_route(TransitRoute::new("L2", vec![1, 2, 3], vec![7.0, 6.0], 6.0));
/// network.add_walk_link(2, 3, 10.0);
/// assert_eq!(network.routes.len(), 2);
/// ```
#[derive(Debug, Clone, Default)]
pub struct TransitNetwork {
    /// Transit routes (lines)
    pub routes: Vec<TransitRoute>,
    /// Walk connections between stops
    pub walk_links: Vec<WalkLink>,
}

impl TransitNetwork {
    /// Creates an empty transit network.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::TransitNetwork;
    ///
    /// let network = TransitNetwork::new();
    /// assert!(network.routes.is_empty());
    /// assert!(network.walk_links.is_empty());
    /// ```
    pub fn new() -> Self {
        TransitNetwork::default()
    }

    /// Adds a route to the network.
    ///
    /// # Arguments
    ///
    /// * `route` - The route to add
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::{TransitNetwork, TransitRoute};
    ///
    /// let mut network = TransitNetwork::new();
    /// // two stops, one 10-minute segment, a vehicle every 5 minutes
    /// network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
    /// assert_eq!(network.routes.len(), 1);
    /// assert_eq!(network.routes[0].id, "L1");
    /// ```
    pub fn add_route(&mut self, route: TransitRoute) {
        self.routes.push(route);
    }

    /// Adds a one-way walk link between two stops.
    ///
    /// # Arguments
    ///
    /// * `from_stop` - Source stop (node ID)
    /// * `to_stop` - Target stop (node ID)
    /// * `time` - Walking time in the same unit as route segment times
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::TransitNetwork;
    ///
    /// let mut network = TransitNetwork::new();
    /// // 4-minute walk from stop 2 to stop 3; add (3, 2, 4.0) for the
    /// // opposite direction if needed
    /// network.add_walk_link(2, 3, 4.0);
    /// assert_eq!(network.walk_links.len(), 1);
    /// assert_eq!(network.walk_links[0].to_stop, 3);
    /// ```
    pub fn add_walk_link(&mut self, from_stop: i64, to_stop: i64, time: f64) {
        self.walk_links.push(WalkLink {
            from_stop,
            to_stop,
            time,
        });
    }

    /// Generates and adds zone access walk links from coordinates.
    ///
    /// Convenience wrapper over
    /// [`generate_access_connectors`](crate::transit::generate_access_connectors):
    /// it connects each centroid to its nearest candidate stops (both
    /// directions) and appends the links to this network. Returns the
    /// number of links added.
    ///
    /// # Arguments
    ///
    /// * `centroids` - `(id, lat, lon)` of the zone centroids
    /// * `stops` - `(id, lat, lon)` of the transit stops
    /// * `params` - Walking speed, search radius, and candidate count
    ///
    /// # Errors
    ///
    /// Returns [`TransitError`] when the connector parameters are invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::{
    ///     AccessConnectorParams, TransitNetwork, TransitRoute,
    /// };
    ///
    /// let mut network = TransitNetwork::new();
    /// network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));
    ///
    /// let centroids = [(900, 55.7500, 37.6200)];
    /// let stops = [(1, 55.7500, 37.6222)];
    /// let params = AccessConnectorParams {
    ///     walking_speed: 80.0,
    ///     max_radius_meters: 400.0,
    ///     max_connectors: 2,
    /// };
    /// let added = network.add_access_connectors(&centroids, &stops, &params).unwrap();
    /// assert_eq!(added, 2); // one pair, both directions
    /// ```
    pub fn add_access_connectors(
        &mut self,
        centroids: &[(i64, f64, f64)],
        stops: &[(i64, f64, f64)],
        params: &crate::transit::connectors::AccessConnectorParams,
    ) -> Result<usize, TransitError> {
        let links =
            crate::transit::connectors::generate_access_connectors(centroids, stops, params)?;
        let added = links.len();
        self.walk_links.extend(links);
        Ok(added)
    }

    /// Validates the network structure.
    ///
    /// Checks that the network is non-empty, every route has at least two
    /// stops, segment times match the stop count, headways are positive,
    /// and route ids are unique.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::{TransitError, TransitNetwork, TransitRoute};
    ///
    /// let mut network = TransitNetwork::new();
    /// network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
    /// assert!(network.validate().is_ok());
    ///
    /// // three stops need exactly two segment times
    /// network.add_route(TransitRoute::new("L2", vec![1, 2, 3], vec![10.0], 5.0));
    /// assert!(matches!(
    ///     network.validate(),
    ///     Err(TransitError::SegmentTimesMismatch { .. })
    /// ));
    /// ```
    pub fn validate(&self) -> Result<(), TransitError> {
        if self.routes.is_empty() {
            return Err(TransitError::EmptyNetwork);
        }
        let mut seen_ids: HashSet<&str> = HashSet::with_capacity(self.routes.len());
        for route in &self.routes {
            if !seen_ids.insert(route.id.as_str()) {
                return Err(TransitError::DuplicateRouteId {
                    route_id: route.id.clone(),
                });
            }
            if route.stops.len() < 2 {
                return Err(TransitError::RouteTooShort {
                    route_id: route.id.clone(),
                });
            }
            if route.segment_times.len() != route.stops.len() - 1 {
                return Err(TransitError::SegmentTimesMismatch {
                    route_id: route.id.clone(),
                    stops: route.stops.len(),
                    segments: route.segment_times.len(),
                });
            }
            if route.headway <= 0.0 {
                return Err(TransitError::NonPositiveHeadway {
                    route_id: route.id.clone(),
                });
            }
            for (name, value) in [
                ("boarding_penalty", route.boarding_penalty),
                ("alighting_penalty", route.alighting_penalty),
                ("dwell_time", route.dwell_time),
                ("pce", Some(route.pce)),
            ] {
                if let Some(value) = value
                    && (value.is_nan() || value < 0.0)
                {
                    return Err(TransitError::InvalidPenalty { name, value });
                }
            }
            if let Some(links) = &route.segment_links
                && links.len() != route.stops.len() - 1
            {
                return Err(TransitError::SegmentTimesMismatch {
                    route_id: route.id.clone(),
                    stops: route.stops.len(),
                    segments: links.len(),
                });
            }
        }
        Ok(())
    }

    /// Returns the set of physical stops referenced by routes and walk links.
    ///
    /// Walk-link endpoints count as stops, which is what allows zone
    /// centroids connected only by walk links to act as OD zones.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::transit::{TransitNetwork, TransitRoute};
    ///
    /// let mut network = TransitNetwork::new();
    /// network.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 5.0));
    /// // centroid 100 reaches stop 1 on foot
    /// network.add_walk_link(100, 1, 3.0);
    ///
    /// let stops = network.stop_ids();
    /// assert_eq!(stops.len(), 3);
    /// assert!(stops.contains(&100));
    /// ```
    pub fn stop_ids(&self) -> HashSet<i64> {
        let mut stops = HashSet::new();
        for route in &self.routes {
            stops.extend(route.stops.iter().copied());
        }
        for walk in &self.walk_links {
            stops.insert(walk.from_stop);
            stops.insert(walk.to_stop);
        }
        stops
    }
}
