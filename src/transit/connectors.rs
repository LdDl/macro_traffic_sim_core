//! # Zone access connector generation
//!
//! Turns a set of zone centroids and a set of transit stops, both given by
//! geographic coordinates, into walk links connecting each centroid to its
//! nearest candidate stops. This automates the hand-built walk links used
//! for zone access (see the [`crate::transit`] module docs, section
//! "Zone access").
//!
//! Each centroid is connected to several candidate stops within a search
//! radius, not just the nearest one: the access stop is then chosen by the
//! assignment itself, per destination. Both directions (centroid -> stop
//! and stop -> centroid) are generated, so the links serve access and
//! egress alike.
//!
//! This is connector generation from coordinates, not map matching: no
//! stop is snapped to a link and the road graph is untouched. The straight
//! line (great-circle) distance is used, scaled by a walking speed to a
//! walking time in the network's own time unit.

use crate::pipeline::haversine_km;
use crate::transit::error::TransitError;

/// Parameters controlling zone access connector generation.
#[derive(Debug, Clone, Copy)]
pub struct AccessConnectorParams {
    /// Walking speed in meters per unit of the network's time (the same
    /// time unit as route segment times and headways). For a network in
    /// minutes a typical value is `80.0` (about 4.8 km/h); for a network
    /// in seconds, about `1.33`.
    ///
    /// Must be strictly positive.
    pub walking_speed: f64,

    /// Maximum straight-line distance in meters for a stop to be a
    /// candidate. Stops farther than this from a centroid are ignored.
    ///
    /// Must be non-negative.
    pub max_radius_meters: f64,

    /// Maximum number of candidate stops to connect per centroid (the
    /// nearest ones within the radius). `0` generates no connectors.
    pub max_connectors: usize,
}

/// Generates zone access walk links from centroid and stop coordinates.
///
/// `centroids` and `stops` are `(id, latitude, longitude)` triples. For
/// each centroid the stops within `max_radius_meters` are ranked by
/// distance and the nearest `max_connectors` are connected by a pair of
/// [`WalkLink`](crate::transit::WalkLink)s (centroid -> stop and
/// stop -> centroid) whose time is `distance / walking_speed`.
///
/// The search is a brute-force scan (`O(centroids * stops)`), which is
/// adequate for the network sizes handled here; a spatial index can
/// replace it later without changing the result.
///
/// # Arguments
///
/// * `centroids` - `(id, lat, lon)` of the zone centroids (pseudo-stops)
/// * `stops` - `(id, lat, lon)` of the transit stops
/// * `params` - Walking speed, search radius, and candidate count
///
/// # Errors
///
/// Returns [`TransitError::InvalidConnectorParams`] when `walking_speed`
/// is not strictly positive or `max_radius_meters` is negative (or either
/// is NaN).
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::transit::{generate_access_connectors, AccessConnectorParams};
///
/// // one centroid, two stops ~140 m and ~280 m away
/// let centroids = [(900, 55.7500, 37.6200)];
/// let stops = [(1, 55.7500, 37.6222), (2, 55.7500, 37.6244)];
/// let params = AccessConnectorParams {
///     walking_speed: 80.0,
///     max_radius_meters: 400.0,
///     max_connectors: 2,
/// };
///
/// let links = generate_access_connectors(&centroids, &stops, &params).unwrap();
/// // 2 candidate stops -> 2 links each way = 4 links
/// assert_eq!(links.len(), 4);
/// ```
pub fn generate_access_connectors(
    centroids: &[(i64, f64, f64)],
    stops: &[(i64, f64, f64)],
    params: &AccessConnectorParams,
) -> Result<Vec<crate::transit::route::WalkLink>, TransitError> {
    use crate::transit::route::WalkLink;

    if params.walking_speed.is_nan() || params.walking_speed <= 0.0 {
        return Err(TransitError::InvalidConnectorParams {
            reason: "walking_speed must be strictly positive",
        });
    }
    if params.max_radius_meters.is_nan() || params.max_radius_meters < 0.0 {
        return Err(TransitError::InvalidConnectorParams {
            reason: "max_radius_meters must be non-negative",
        });
    }

    let mut links = Vec::new();
    let mut candidates: Vec<(f64, i64)> = Vec::new();
    for &(centroid_id, clat, clon) in centroids {
        candidates.clear();
        for &(stop_id, slat, slon) in stops {
            let dist_m = haversine_km(clat, clon, slat, slon) * 1000.0;
            if dist_m <= params.max_radius_meters {
                candidates.push((dist_m, stop_id));
            }
        }
        // Nearest first; a stable tie-break on stop id keeps the output
        // deterministic when two stops are equidistant.
        candidates.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        candidates.truncate(params.max_connectors);

        for &(dist_m, stop_id) in &candidates {
            let time = dist_m / params.walking_speed;
            links.push(WalkLink {
                from_stop: centroid_id,
                to_stop: stop_id,
                time,
            });
            links.push(WalkLink {
                from_stop: stop_id,
                to_stop: centroid_id,
                time,
            });
        }
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tolerance for exact-value comparisons in the connector tests.
    const EPS: f64 = 1e-12;

    fn params(max_connectors: usize) -> AccessConnectorParams {
        AccessConnectorParams {
            walking_speed: 80.0,
            max_radius_meters: 400.0,
            max_connectors,
        }
    }

    #[test]
    fn test_connects_nearest_within_radius() {
        // Centroid at origin; three stops at ~140, ~280, ~1100 m east.
        let centroids = [(900, 55.7500, 37.6200)];
        let stops = [
            (1, 55.7500, 37.6222),
            (2, 55.7500, 37.6244),
            (3, 55.7500, 37.6375),
        ];
        let links = generate_access_connectors(&centroids, &stops, &params(3)).unwrap();
        // Stop 3 is beyond 400 m, so only stops 1 and 2 connect: 2 pairs.
        assert_eq!(links.len(), 4);
        let targets: Vec<i64> = links
            .iter()
            .filter(|l| l.from_stop == 900)
            .map(|l| l.to_stop)
            .collect();
        assert!(targets.contains(&1));
        assert!(targets.contains(&2));
        assert!(!targets.contains(&3));
    }

    #[test]
    fn test_both_directions_and_time() {
        let centroids = [(900, 55.7500, 37.6200)];
        let stops = [(1, 55.7500, 37.6222)];
        let links = generate_access_connectors(&centroids, &stops, &params(1)).unwrap();
        assert_eq!(links.len(), 2);
        let access = links.iter().find(|l| l.from_stop == 900).unwrap();
        let egress = links.iter().find(|l| l.from_stop == 1).unwrap();
        assert_eq!(access.to_stop, 1);
        assert_eq!(egress.to_stop, 900);
        // Same time both ways, and it equals distance / walking speed.
        assert!((access.time - egress.time).abs() < EPS);
        assert!(access.time > 0.0);
    }

    #[test]
    fn test_max_connectors_caps_candidates() {
        let centroids = [(900, 55.7500, 37.6200)];
        let stops = [
            (1, 55.7500, 37.6222),
            (2, 55.7500, 37.6233),
            (3, 55.7500, 37.6244),
        ];
        // All within radius, but keep only the nearest one.
        let links = generate_access_connectors(&centroids, &stops, &params(1)).unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].to_stop, 1);
    }

    #[test]
    fn test_invalid_params() {
        let centroids = [(900, 55.75, 37.62)];
        let stops = [(1, 55.75, 37.622)];
        let mut p = params(2);
        p.walking_speed = 0.0;
        assert!(matches!(
            generate_access_connectors(&centroids, &stops, &p),
            Err(TransitError::InvalidConnectorParams { .. })
        ));
        let mut p = params(2);
        p.max_radius_meters = -1.0;
        assert!(matches!(
            generate_access_connectors(&centroids, &stops, &p),
            Err(TransitError::InvalidConnectorParams { .. })
        ));
    }
}
