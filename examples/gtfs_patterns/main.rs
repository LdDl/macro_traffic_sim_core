//! Example: how GTFS trips are grouped into patterns by
//! `transit_network_from_gtfs`.
//!
//! GTFS has no "line" entity an assignment model could use directly:
//! `routes.txt` is an administrative registry record, a trip is a single
//! run (or a repeating template when `frequencies.txt` applies), and the
//! actual structure lives in `stop_times.txt`. The converter
//! reconstructs the modeling entity - the *pattern* - by grouping the
//! trips of each route by their stop sequence.
//!
//! Five scenarios are demonstrated:
//!
//! 1. Route "10": per-period template trips (morning + evening) with the
//!    same stop sequence - ONE pattern, headway and segment times
//!    aggregated over both trips.
//! 2. Route "20": forward and reverse trips without `direction_id` -
//!    two patterns (the reverse direction survives).
//! 3. Route "30": a full run and a short-turn variant - two patterns.
//! 4. Route "40": a non-timepoint intermediate stop without times -
//!    linear interpolation.
//! 5. Route "50": a GTFS-Flex trip and a schedule-based trip (no
//!    frequencies) - both skipped, the route vanishes.
//!
//! Usage:
//!   cargo run --example gtfs_patterns

use std::collections::HashMap;

use gtfs_rs::{
    Direction, Frequency, GtfsReference, Route, RouteType, Stop, StopTime, Trip, parse_gtfs_time,
};
use macro_traffic_sim_core::transit::transit_network_from_gtfs;

fn t(hms: &str) -> u32 {
    parse_gtfs_time(hms).expect("valid GTFS time")
}

fn build_gtfs() -> GtfsReference {
    let mut gtfs = GtfsReference::new();

    for stop in ["A", "B", "C", "D"] {
        gtfs.stops.push(Stop::new(stop));
    }

    // Scenario 1: route "10" - two template trips of the SAME pattern.
    // Morning: every 300 s for 3 h (36 departures), segments 120 s + 180 s.
    // Evening: every 600 s for 3 h (18 departures), segments 180 s + 240 s.
    // Expected: one pattern, headway = 21600 / 54 = 400 s,
    // segments = departure-weighted means [140, 200].
    gtfs.routes.push(Route::new("10", RouteType::Bus));
    gtfs.trips
        .push(Trip::new("10_am", "10", "daily").with_direction(Direction::Outbound));
    gtfs.stop_times
        .push(StopTime::new("10_am", "A", 0, t("08:00:00")));
    gtfs.stop_times
        .push(StopTime::new("10_am", "B", 1, t("08:02:00")));
    gtfs.stop_times
        .push(StopTime::new("10_am", "C", 2, t("08:05:00")));
    gtfs.frequencies
        .push(Frequency::new("10_am", t("06:00:00"), t("09:00:00"), 300));
    gtfs.trips
        .push(Trip::new("10_pm", "10", "daily").with_direction(Direction::Outbound));
    gtfs.stop_times
        .push(StopTime::new("10_pm", "A", 0, t("17:00:00")));
    gtfs.stop_times
        .push(StopTime::new("10_pm", "B", 1, t("17:03:00")));
    gtfs.stop_times
        .push(StopTime::new("10_pm", "C", 2, t("17:07:00")));
    gtfs.frequencies
        .push(Frequency::new("10_pm", t("15:00:00"), t("18:00:00"), 600));

    // Scenario 2: route "20" - forward and reverse runs, NO direction_id.
    // Different stop sequences => two patterns: "20:0" and "20:0#1".
    gtfs.routes.push(Route::new("20", RouteType::Bus));
    gtfs.trips.push(Trip::new("20_f", "20", "daily"));
    gtfs.stop_times
        .push(StopTime::new("20_f", "A", 0, t("08:00:00")));
    gtfs.stop_times
        .push(StopTime::new("20_f", "D", 1, t("08:10:00")));
    gtfs.frequencies
        .push(Frequency::new("20_f", t("07:00:00"), t("10:00:00"), 360));
    gtfs.trips.push(Trip::new("20_r", "20", "daily"));
    gtfs.stop_times
        .push(StopTime::new("20_r", "D", 0, t("08:00:00")));
    gtfs.stop_times
        .push(StopTime::new("20_r", "A", 1, t("08:10:00")));
    gtfs.frequencies
        .push(Frequency::new("20_r", t("07:00:00"), t("10:00:00"), 360));

    // Scenario 3: route "30" - a full run and a short-turn variant.
    // Both direction 0, different sequences => "30:0" and "30:0#1".
    gtfs.routes.push(Route::new("30", RouteType::Tram));
    gtfs.trips
        .push(Trip::new("30_full", "30", "daily").with_direction(Direction::Outbound));
    for (seq, (stop, time)) in [
        ("A", "08:00:00"),
        ("B", "08:04:00"),
        ("C", "08:09:00"),
        ("D", "08:15:00"),
    ]
    .iter()
    .enumerate()
    {
        gtfs.stop_times
            .push(StopTime::new("30_full", stop, seq as u32, t(time)));
    }
    gtfs.frequencies
        .push(Frequency::new("30_full", t("06:00:00"), t("22:00:00"), 480));
    gtfs.trips
        .push(Trip::new("30_short", "30", "daily").with_direction(Direction::Outbound));
    gtfs.stop_times
        .push(StopTime::new("30_short", "A", 0, t("08:00:00")));
    gtfs.stop_times
        .push(StopTime::new("30_short", "B", 1, t("08:04:00")));
    gtfs.frequencies.push(Frequency::new(
        "30_short",
        t("07:00:00"),
        t("09:00:00"),
        480,
    ));

    // Scenario 4: route "40" - middle stop has no times (non-timepoint).
    // A dep 08:00:00, C arr 08:10:00 => interpolated segments [300, 300].
    gtfs.routes.push(Route::new("40", RouteType::Bus));
    gtfs.trips
        .push(Trip::new("40_t", "40", "daily").with_direction(Direction::Outbound));
    gtfs.stop_times
        .push(StopTime::new("40_t", "A", 0, t("08:00:00")));
    let mut untimed = StopTime::new("40_t", "B", 1, 0);
    untimed.arrival_time = None;
    untimed.departure_time = None;
    gtfs.stop_times.push(untimed);
    gtfs.stop_times
        .push(StopTime::new("40_t", "C", 2, t("08:10:00")));
    gtfs.frequencies
        .push(Frequency::new("40_t", t("07:00:00"), t("10:00:00"), 300));

    // Scenario 5: route "50" - nothing usable.
    // 50_flex serves an on-demand location instead of a stop; 50_sched
    // has no frequencies (schedule-based). Both are skipped.
    gtfs.routes.push(Route::new("50", RouteType::Bus));
    gtfs.trips
        .push(Trip::new("50_flex", "50", "daily").with_direction(Direction::Outbound));
    gtfs.stop_times
        .push(StopTime::new("50_flex", "A", 0, t("08:00:00")));
    let mut flex_row = StopTime::new("50_flex", "ignored", 1, t("08:20:00"));
    flex_row.stop_id = None;
    gtfs.stop_times.push(flex_row);
    gtfs.frequencies
        .push(Frequency::new("50_flex", t("07:00:00"), t("10:00:00"), 600));
    gtfs.trips
        .push(Trip::new("50_sched", "50", "daily").with_direction(Direction::Outbound));
    gtfs.stop_times
        .push(StopTime::new("50_sched", "A", 0, t("08:00:00")));
    gtfs.stop_times
        .push(StopTime::new("50_sched", "B", 1, t("08:05:00")));

    gtfs
}

fn main() {
    let gtfs = build_gtfs();

    println!("GTFS registry view:");
    for route in &gtfs.routes {
        let trips = gtfs.trips_of_route(&route.route_id);
        println!("  route {}: {} trip(s)", route.route_id, trips.len());
        for trip in trips {
            let stops: Vec<&str> = gtfs
                .stop_times_of_trip(&trip.trip_id)
                .iter()
                .map(|st| st.stop_id.as_deref().unwrap_or("<flex>"))
                .collect();
            let windows = gtfs.frequencies_of_trip(&trip.trip_id).len();
            println!(
                "    trip {}: [{}], {} frequency window(s)",
                trip.trip_id,
                stops.join(" -> "),
                windows
            );
        }
    }

    let mapping = HashMap::from([
        ("A".to_string(), 1_i64),
        ("B".to_string(), 2_i64),
        ("C".to_string(), 3_i64),
        ("D".to_string(), 4_i64),
    ]);
    let network = transit_network_from_gtfs(&gtfs, &mapping).expect("conversion failed");

    println!("\nReconstructed patterns ({} total):", network.routes.len());
    let mut routes: Vec<_> = network.routes.iter().collect();
    routes.sort_by(|a, b| a.id.cmp(&b.id));
    for route in routes {
        println!(
            "  {}: stops {:?}, headway {:.0} s, segments {:?}",
            route.id, route.stops, route.headway, route.segment_times
        );
    }

    println!("\nNotes:");
    println!("  10:0    - one pattern from two template trips; 21600 s / 54 departures = 400 s");
    println!("  20:0#1  - reverse direction survives even without direction_id");
    println!("  30:0#1  - short-turn variant kept as its own line");
    println!("  40:0    - middle stop interpolated: [300, 300]");
    println!("  route 50 absent - flex and schedule-based trips are out of scope");
}
