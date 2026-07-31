//! # GTFS to Transit Network Conversion
//!
//! Builds a [`TransitNetwork`] from an in-memory GTFS Schedule dataset
//! ([`gtfs-rs`](https://crates.io/crates/gtfs-rs) crate). The linkage
//! between GTFS stops and network
//! nodes/locations is provided by the user (no map matching):
//! either an explicit map or [`Network::gtfs_stop_mapping`]
//! built from GMNS locations with `gtfs_stop_id`.
//!
//! ## Why patterns
//!
//! GTFS has no entity that matches what an assignment model calls a
//! "line". A `routes.txt` record is an administrative registry entry
//! (number, name, agency) with no stops, no times and no directions. A
//! `trips.txt` record is a single one-way run on given service days
//! (`block_id` chains the runs served by one physical vehicle) - or,
//! when `frequencies.txt` applies, a repeating template of such runs.
//! The actual structure lives only in `stop_times.txt`: the ordered stop
//! sequence of each trip. Directions, short-turns and weekend variants
//! are simply trips with different stop sequences under one route id.
//!
//! The modeling entity - the *pattern* (NeTEx calls it a
//! ServiceJourneyPattern; GTFS deliberately has no such table) - must
//! therefore be reconstructed by grouping the trips of a route by their
//! stop sequence. That reconstruction is exactly what this converter
//! does: one [`TransitRoute`] per distinct pattern, with the service
//! frequency aggregated over all trips of that pattern.
//!
//! Conversion rules:
//!
//! - Trips of a route are grouped by their **stop pattern** (the mapped
//!   stop sequence). Each distinct pattern becomes one [`TransitRoute`]:
//!   opposite directions are independent patterns even when the feed
//!   omits `direction_id`, and branch/short-turn variants are kept as
//!   separate lines instead of being dropped.
//! - Route ids are `"{route_id}:{direction}"` where direction is the
//!   numeric `direction_id` code of the pattern's first trip (0 when
//!   absent); additional patterns with the same label get a `#k`
//!   suffix (`"R:0#1"`).
//! - The headway of a pattern aggregates the `frequencies.txt` windows
//!   of **all** its trips (time-weighted: total covered time divided by
//!   total departures), so per-period template trips (`trip_am`,
//!   `trip_pm`, ...) produce the correct combined frequency. Both
//!   `exact_times` variants count: either way one vehicle departs every
//!   `headway_secs`. Trips without frequency windows are skipped
//!   (schedule-based service is out of scope).
//! - Segment travel times are `arrival[i+1] - departure[i]` from
//!   `stop_times`, in seconds, averaged over the pattern's trips
//!   weighted by their departure counts. Missing intermediate times
//!   (non-timepoint rows, spec-legal) are linearly interpolated;
//!   missing times at the first or last stop are an error.
//! - Trips containing GTFS-Flex rows (`location_id`/`location_group_id`
//!   instead of `stop_id`) are skipped: on-demand zones have no
//!   headway/stop semantics a frequency-based assignment could consume.
//!
//! All resulting times are in seconds (GTFS native unit); the transit
//! assignment is unit-agnostic.
//!
//! [`Network::gtfs_stop_mapping`]: crate::gmns::meso::network::Network::gtfs_stop_mapping

use std::collections::HashMap;

use gtfs_rs::{GtfsReference, StopTime};

use crate::transit::error::TransitError;
use crate::transit::route::{TransitNetwork, TransitRoute};

/// Builds a transit network from a GTFS Schedule dataset and a stop mapping.
///
/// # Arguments
///
/// * `gtfs` - In-memory GTFS dataset (stops, routes, trips, stop times, frequencies)
/// * `stop_to_node` - Mapping from GTFS `stop_id` to network stop ID
///   (node or location ID); the same IDs are then used as zones of the
///   transit OD matrix
///
/// # Errors
///
/// Returns [`TransitError::UnmappedGtfsStop`] when a stop of a usable
/// trip is missing from the mapping, and [`TransitError::InvalidGtfsData`]
/// when boundary stop times are missing, times are non-monotonic, or no
/// usable trips exist.
///
/// # Examples
///
/// ```
/// use gtfs_rs::{Frequency, GtfsReference, Route, RouteType, Stop, StopTime, Trip};
/// use macro_traffic_sim_core::transit::from_gtfs::transit_network_from_gtfs;
/// use std::collections::HashMap;
///
/// let mut gtfs = GtfsReference::new();
/// gtfs.stops.push(Stop::new("A"));
/// gtfs.stops.push(Stop::new("B"));
/// gtfs.routes.push(Route::new("L1", RouteType::Bus));
/// gtfs.trips.push(Trip::new("L1_t0", "L1", "daily"));
/// gtfs.stop_times.push(StopTime::new("L1_t0", "A", 0, 8 * 3600));
/// gtfs.stop_times.push(StopTime::new("L1_t0", "B", 1, 8 * 3600 + 600));
/// gtfs.frequencies.push(Frequency::new("L1_t0", 7 * 3600, 10 * 3600, 300));
///
/// let mapping = HashMap::from([("A".to_string(), 1_i64), ("B".to_string(), 2_i64)]);
/// let network = transit_network_from_gtfs(&gtfs, &mapping).unwrap();
///
/// assert_eq!(network.routes.len(), 1);
/// assert_eq!(network.routes[0].id, "L1:0");
/// // 600 seconds of riding, 300 seconds headway
/// assert_eq!(network.routes[0].segment_times, vec![600.0]);
/// assert_eq!(network.routes[0].headway, 300.0);
/// ```
pub fn transit_network_from_gtfs(
    gtfs: &GtfsReference,
    stop_to_node: &HashMap<String, i64>,
) -> Result<TransitNetwork, TransitError> {
    let mut network = TransitNetwork::new();

    for route in &gtfs.routes {
        // Trips grouped by identical mapped stop pattern, in first-seen
        // order. Each group accumulates frequency windows and
        // departure-weighted segment times over all its trips.
        let mut groups: Vec<PatternGroup> = Vec::new();

        for trip in gtfs.trips_of_route(&route.route_id) {
            let frequencies = gtfs.frequencies_of_trip(&trip.trip_id);
            if frequencies.is_empty() {
                continue;
            }
            let pattern = gtfs.stop_times_of_trip(&trip.trip_id);
            if pattern.len() < 2 {
                continue;
            }
            // GTFS-Flex rows reference an on-demand location/location
            // group instead of a stop; such trips have no fixed-route
            // semantics and are skipped
            if pattern.iter().any(|st| st.stop_id.is_none()) {
                continue;
            }

            let mut stops: Vec<i64> = Vec::with_capacity(pattern.len());
            for stop_time in &pattern {
                let stop_id = stop_time.stop_id.as_deref().unwrap();
                let node = stop_to_node.get(stop_id).copied().ok_or_else(|| {
                    TransitError::UnmappedGtfsStop {
                        stop_id: stop_id.to_string(),
                    }
                })?;
                stops.push(node);
            }

            let (window_time, departures) = window_stats(&frequencies);
            if departures <= 0.0 {
                continue;
            }
            let segment_times = trip_segment_times(&trip.trip_id, &pattern)?;

            let direction = trip.direction_id.map(|d| d.code()).unwrap_or(0);
            let group = match groups.iter_mut().find(|g| g.stops == stops) {
                Some(existing) => existing,
                None => {
                    groups.push(PatternGroup::new(stops, direction, pattern.len() - 1));
                    groups.last_mut().unwrap()
                }
            };
            group.window_time += window_time;
            group.departures += departures;
            for (acc, seg) in group.weighted_segments.iter_mut().zip(&segment_times) {
                *acc += seg * departures;
            }
        }

        // Emit one TransitRoute per pattern; disambiguate duplicate
        // "{route_id}:{direction}" labels with a #k suffix
        let mut used_ids: Vec<String> = Vec::new();
        for group in groups {
            let base = format!("{}:{}", route.route_id, group.direction);
            let mut id = base.clone();
            let mut k = 1;
            while used_ids.contains(&id) {
                id = format!("{}#{}", base, k);
                k += 1;
            }
            used_ids.push(id.clone());

            let headway = group.window_time / group.departures;
            let segment_times: Vec<f64> = group
                .weighted_segments
                .iter()
                .map(|w| w / group.departures)
                .collect();
            network.add_route(TransitRoute::new(&id, group.stops, segment_times, headway));
        }
    }

    if network.routes.is_empty() {
        return Err(TransitError::InvalidGtfsData(
            "dataset contains no trips with both stop times and frequencies".to_string(),
        ));
    }

    Ok(network)
}

/// Accumulator for one stop pattern of a route.
struct PatternGroup {
    stops: Vec<i64>,
    direction: i32,
    /// Sum of frequency window lengths over all trips, seconds
    window_time: f64,
    /// Total departures over all trips
    departures: f64,
    /// Departure-weighted segment time sums
    weighted_segments: Vec<f64>,
}

impl PatternGroup {
    fn new(stops: Vec<i64>, direction: i32, segments: usize) -> Self {
        PatternGroup {
            stops,
            direction,
            window_time: 0.0,
            departures: 0.0,
            weighted_segments: vec![0.0; segments],
        }
    }
}

/// Total covered time and total departures over frequency windows.
fn window_stats(frequencies: &[&gtfs_rs::Frequency]) -> (f64, f64) {
    let mut total_time = 0.0;
    let mut total_departures = 0.0;
    for freq in frequencies {
        if freq.end_time <= freq.start_time || freq.headway_secs == 0 {
            continue;
        }
        let window = (freq.end_time - freq.start_time) as f64;
        total_time += window;
        total_departures += window / freq.headway_secs as f64;
    }
    (total_time, total_departures)
}

/// Segment travel times of one trip, in seconds.
///
/// When every row carries both arrival and departure, segments are the
/// exact `arrival[i+1] - departure[i]`. Missing intermediate times
/// (non-timepoint rows) are linearly interpolated between the nearest
/// known times; missing boundary times are an error, matching the spec
/// requirement that first and last stop times be present.
fn trip_segment_times(trip_id: &str, pattern: &[&StopTime]) -> Result<Vec<f64>, TransitError> {
    let n = pattern.len();

    let fully_timed = pattern
        .iter()
        .all(|st| st.arrival_time.is_some() && st.departure_time.is_some());
    if fully_timed {
        let mut segment_times = Vec::with_capacity(n - 1);
        for i in 0..n - 1 {
            let departure = pattern[i].departure_time.unwrap();
            let arrival = pattern[i + 1].arrival_time.unwrap();
            if arrival < departure {
                return Err(TransitError::InvalidGtfsData(format!(
                    "trip '{}' has non-monotonic times at sequence {}",
                    trip_id,
                    pattern[i + 1].stop_sequence
                )));
            }
            segment_times.push((arrival - departure) as f64);
        }
        return Ok(segment_times);
    }

    // Representative instant per stop; unknown interior values are
    // linearly interpolated between the surrounding known ones
    let mut instants: Vec<Option<f64>> = pattern
        .iter()
        .map(|st| st.departure_time.or(st.arrival_time).map(|t| t as f64))
        .collect();
    if instants[0].is_none() || instants[n - 1].is_none() {
        return Err(TransitError::InvalidGtfsData(format!(
            "trip '{}' has no time at its first or last stop",
            trip_id
        )));
    }
    let mut prev_known = 0;
    for i in 1..n {
        if instants[i].is_none() {
            continue;
        }
        let gap = i - prev_known;
        if gap > 1 {
            let start = instants[prev_known].unwrap();
            let step = (instants[i].unwrap() - start) / gap as f64;
            for (offset, instant) in instants[prev_known + 1..i].iter_mut().enumerate() {
                *instant = Some(start + step * (offset + 1) as f64);
            }
        }
        prev_known = i;
    }

    let mut segment_times = Vec::with_capacity(n - 1);
    for i in 0..n - 1 {
        let from = instants[i].unwrap();
        let to = instants[i + 1].unwrap();
        if to < from {
            return Err(TransitError::InvalidGtfsData(format!(
                "trip '{}' has non-monotonic times at sequence {}",
                trip_id,
                pattern[i + 1].stop_sequence
            )));
        }
        segment_times.push(to - from);
    }
    Ok(segment_times)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::od::{DenseOdMatrix, OdMatrix};
    use crate::transit::assignment::assign_transit;
    use gtfs_rs::{Direction, Frequency, Route, RouteType, Stop, Trip};

    /// The Spiess & Florian (1989) example network as a GTFS dataset.
    /// Times in seconds: L1 ride 1500 (25 min), headway 360 (6 min), etc.
    fn paper_gtfs() -> GtfsReference {
        let mut gtfs = GtfsReference::new();
        for stop in ["A", "X", "Y", "B"] {
            gtfs.stops.push(Stop::new(stop));
        }

        let base = 8 * 3600;
        let mut add_line = |route_id: &str, stops: &[&str], seg_secs: &[u32], headway_secs: u32| {
            gtfs.routes.push(Route::new(route_id, RouteType::Bus));
            let trip_id = format!("{}_t0", route_id);
            gtfs.trips
                .push(Trip::new(&trip_id, route_id, "daily").with_direction(Direction::Outbound));
            let mut t = base;
            for (seq, stop) in stops.iter().enumerate() {
                if seq > 0 {
                    t += seg_secs[seq - 1];
                }
                gtfs.stop_times
                    .push(StopTime::new(&trip_id, stop, seq as u32, t));
            }
            gtfs.frequencies.push(Frequency::new(
                &trip_id,
                base,
                base + 4 * 3600,
                headway_secs,
            ));
        };

        add_line("L1", &["A", "B"], &[1500], 360);
        add_line("L2", &["A", "X", "Y"], &[420, 360], 360);
        add_line("L3", &["X", "Y", "B"], &[240, 240], 900);
        add_line("L4", &["Y", "B"], &[600], 180);
        gtfs
    }

    fn paper_mapping() -> HashMap<String, i64> {
        HashMap::from([
            ("A".to_string(), 1_i64),
            ("X".to_string(), 2_i64),
            ("Y".to_string(), 3_i64),
            ("B".to_string(), 4_i64),
        ])
    }

    #[test]
    fn test_paper_gtfs_conversion_and_assignment() {
        let gtfs = paper_gtfs();
        let network = transit_network_from_gtfs(&gtfs, &paper_mapping()).unwrap();

        assert_eq!(network.routes.len(), 4);
        let l2 = network.routes.iter().find(|r| r.id == "L2:0").unwrap();
        assert_eq!(l2.stops, vec![1, 2, 3]);
        assert_eq!(l2.segment_times, vec![420.0, 360.0]);
        assert!((l2.headway - 360.0).abs() < 1e-9);

        // Full chain: GTFS -> TransitNetwork -> assignment.
        // Paper expected cost 27.75 min = 1665 seconds.
        let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
        od.set(1, 4, 1.0);
        let result = assign_transit(&network, &od).unwrap();
        assert!(
            (result.od_costs[&(1, 4)] - 1665.0).abs() <= 1e-6,
            "u_A = {} s, want 1665 s",
            result.od_costs[&(1, 4)]
        );
        assert!((result.route_boardings["L1:0"] - 0.5).abs() <= 1e-9);
        assert!((result.route_boardings["L4:0"] - 5.0 / 12.0).abs() <= 1e-9);
    }

    #[test]
    fn test_average_headway_multiple_windows() {
        let mut gtfs = paper_gtfs();
        // Second window for L1 with a different headway:
        // 4h at 360s (40 departures) + 2h at 720s (10 departures)
        // average = 21600 / 50 = 432 seconds
        gtfs.frequencies
            .push(Frequency::new("L1_t0", 12 * 3600, 14 * 3600, 720));
        let network = transit_network_from_gtfs(&gtfs, &paper_mapping()).unwrap();
        let l1 = network.routes.iter().find(|r| r.id == "L1:0").unwrap();
        assert!(
            (l1.headway - 432.0).abs() < 1e-9,
            "headway = {}",
            l1.headway
        );
    }

    #[test]
    fn test_template_trips_aggregate_headway() {
        // Per-period template trips of the same pattern: the combined
        // headway must aggregate windows across ALL trips, not just the
        // first one. 3h at 300s (36 departures) + 3h at 600s (18
        // departures) = 21600 / 54 = 400 seconds.
        let mut gtfs = paper_gtfs();
        gtfs.trips
            .push(Trip::new("L1_t1", "L1", "daily").with_direction(Direction::Outbound));
        let base = 15 * 3600;
        gtfs.stop_times.push(StopTime::new("L1_t1", "A", 0, base));
        gtfs.stop_times
            .push(StopTime::new("L1_t1", "B", 1, base + 1500));
        // Rewrite L1 windows: t0 covers 06:00-09:00@300, t1 15:00-18:00@600
        gtfs.frequencies.retain(|f| f.trip_id != "L1_t0");
        gtfs.frequencies
            .push(Frequency::new("L1_t0", 6 * 3600, 9 * 3600, 300));
        gtfs.frequencies
            .push(Frequency::new("L1_t1", 15 * 3600, 18 * 3600, 600));

        let network = transit_network_from_gtfs(&gtfs, &paper_mapping()).unwrap();
        // Same pattern -> still one L1 route
        assert_eq!(
            network
                .routes
                .iter()
                .filter(|r| r.id.starts_with("L1"))
                .count(),
            1
        );
        let l1 = network.routes.iter().find(|r| r.id == "L1:0").unwrap();
        assert!(
            (l1.headway - 400.0).abs() < 1e-9,
            "headway = {}",
            l1.headway
        );
        assert_eq!(l1.segment_times, vec![1500.0]);
    }

    #[test]
    fn test_directionless_reverse_kept() {
        // Feeds without direction_id: forward and reverse trips are
        // distinct stop patterns and both must survive.
        let mut gtfs = GtfsReference::new();
        gtfs.routes.push(Route::new("R", RouteType::Bus));
        gtfs.trips.push(Trip::new("f", "R", "daily"));
        gtfs.trips.push(Trip::new("r", "R", "daily"));
        for (trip, stops) in [("f", ["A", "B"]), ("r", ["B", "A"])] {
            for (seq, stop) in stops.iter().enumerate() {
                gtfs.stop_times.push(StopTime::new(
                    trip,
                    stop,
                    seq as u32,
                    3600 + seq as u32 * 300,
                ));
            }
            gtfs.frequencies.push(Frequency::new(trip, 0, 3600, 600));
        }
        let mapping = HashMap::from([("A".to_string(), 1_i64), ("B".to_string(), 2_i64)]);
        let network = transit_network_from_gtfs(&gtfs, &mapping).unwrap();
        assert_eq!(network.routes.len(), 2);
        assert!(
            network
                .routes
                .iter()
                .any(|r| r.id == "R:0" && r.stops == vec![1, 2])
        );
        assert!(
            network
                .routes
                .iter()
                .any(|r| r.id == "R:0#1" && r.stops == vec![2, 1])
        );
    }

    #[test]
    fn test_flex_trip_skipped() {
        // A trip with a GTFS-Flex row (no stop_id) is on-demand service
        // and must be skipped without failing the whole conversion
        let mut gtfs = paper_gtfs();
        gtfs.routes.push(Route::new("F", RouteType::Bus));
        gtfs.trips.push(Trip::new("F_t0", "F", "daily"));
        gtfs.stop_times.push(StopTime::new("F_t0", "A", 0, 0));
        let mut flex_row = StopTime::new("F_t0", "ignored", 1, 600);
        flex_row.stop_id = None;
        gtfs.stop_times.push(flex_row);
        gtfs.frequencies.push(Frequency::new("F_t0", 0, 3600, 600));

        let network = transit_network_from_gtfs(&gtfs, &paper_mapping()).unwrap();
        assert_eq!(network.routes.len(), 4);
        assert!(!network.routes.iter().any(|r| r.id.starts_with("F")));
    }

    #[test]
    fn test_interpolated_intermediate_times() {
        // Spec-legal non-timepoint rows: middle stop has no times and
        // must be linearly interpolated, not rejected
        let mut gtfs = GtfsReference::new();
        gtfs.routes.push(Route::new("I", RouteType::Bus));
        gtfs.trips.push(Trip::new("i0", "I", "daily"));
        gtfs.stop_times.push(StopTime::new("i0", "A", 0, 3600));
        let mut untimed = StopTime::new("i0", "X", 1, 0);
        untimed.arrival_time = None;
        untimed.departure_time = None;
        gtfs.stop_times.push(untimed);
        gtfs.stop_times
            .push(StopTime::new("i0", "B", 2, 3600 + 600));
        gtfs.frequencies.push(Frequency::new("i0", 0, 3600, 300));

        let mapping = HashMap::from([
            ("A".to_string(), 1_i64),
            ("X".to_string(), 2_i64),
            ("B".to_string(), 3_i64),
        ]);
        let network = transit_network_from_gtfs(&gtfs, &mapping).unwrap();
        let route = &network.routes[0];
        // 600 seconds split equally across the two segments
        assert_eq!(route.segment_times, vec![300.0, 300.0]);
    }

    #[test]
    fn test_unmapped_stop() {
        let gtfs = paper_gtfs();
        let mut mapping = paper_mapping();
        mapping.remove("Y");
        assert!(matches!(
            transit_network_from_gtfs(&gtfs, &mapping),
            Err(TransitError::UnmappedGtfsStop { .. })
        ));
    }

    #[test]
    fn test_dataset_without_frequencies() {
        let mut gtfs = paper_gtfs();
        gtfs.frequencies.clear();
        assert!(matches!(
            transit_network_from_gtfs(&gtfs, &paper_mapping()),
            Err(TransitError::InvalidGtfsData(_))
        ));
    }

    #[test]
    fn test_direction_split() {
        let mut gtfs = GtfsReference::new();
        gtfs.routes.push(Route::new("R", RouteType::Bus));
        // Forward and reverse patterns of the same route
        gtfs.trips
            .push(Trip::new("f", "R", "daily").with_direction(Direction::Outbound));
        gtfs.trips
            .push(Trip::new("r", "R", "daily").with_direction(Direction::Inbound));
        for (trip, stops) in [("f", ["A", "B"]), ("r", ["B", "A"])] {
            for (seq, stop) in stops.iter().enumerate() {
                gtfs.stop_times.push(StopTime::new(
                    trip,
                    stop,
                    seq as u32,
                    3600 + seq as u32 * 300,
                ));
            }
            gtfs.frequencies.push(Frequency::new(trip, 0, 3600, 600));
        }
        let mapping = HashMap::from([("A".to_string(), 1_i64), ("B".to_string(), 2_i64)]);
        let network = transit_network_from_gtfs(&gtfs, &mapping).unwrap();
        assert_eq!(network.routes.len(), 2);
        assert!(network.routes.iter().any(|r| r.id == "R:0"));
        assert!(network.routes.iter().any(|r| r.id == "R:1"));
    }
}
