//! Example: transit assignment from a GTFS dataset linked to a GMNS network.
//!
//! Demonstrates the full chain:
//!
//! 1. A GMNS road network (nodes = intersections, links = streets).
//! 2. Bus stops as GMNS locations: points along links (link_id + offset),
//!    each carrying its `gtfs_stop_id`. The road graph is not modified.
//! 3. A hardcoded frequency-based GTFS Schedule dataset (stops, routes,
//!    trips, stop times, frequencies) - the Spiess & Florian (1989)
//!    example expressed as GTFS.
//! 4. `Network::gtfs_stop_mapping()` links the dataset to the network,
//!    `transit_network_from_gtfs()` builds transit routes, and
//!    `assign_transit()` runs the optimal strategies assignment.
//!
//! All GTFS times are in seconds; results are printed in minutes.
//!
//! Usage:
//!   cargo run --example transit_gtfs

use gtfs_rs::{Direction, Frequency, GtfsReference, Route, RouteType, Stop, StopTime, Trip};
use macro_traffic_sim_core::gmns::location::Location;
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
use macro_traffic_sim_core::transit::{TransitLinkKind, assign_transit, transit_network_from_gtfs};

/// Road network: 4 intersections connected in a chain, bus stops as
/// locations on the links between them.
fn build_road_network() -> Network {
    let mut net = Network::new();

    let coords = [
        (1, 55.760, 37.600),
        (2, 55.758, 37.615),
        (3, 55.756, 37.630),
        (4, 55.754, 37.645),
    ];
    for &(id, lat, lon) in &coords {
        net.add_node(Node::new(id).with_coordinates(lat, lon).build())
            .unwrap();
    }

    let edges = [(100, 1, 2), (101, 2, 3), (102, 3, 4)];
    for &(link_id, a, b) in &edges {
        net.add_link(
            Link::new(link_id, a, b)
                .with_length_meters(1200.0)
                .with_free_speed(50.0)
                .with_capacity(1800.0)
                .with_lanes_num(2)
                .build(),
        )
        .unwrap();
    }

    // Bus stops: mid-link locations with GTFS linkage provided by the
    // user (no map matching). Stop ids 501..504 become transit zones.
    let stops = [
        (501, 100, 1, 150.0, "stop_A"),
        (502, 100, 1, 900.0, "stop_X"),
        (503, 101, 2, 600.0, "stop_Y"),
        (504, 102, 3, 300.0, "stop_B"),
    ];
    for &(loc_id, link_id, ref_node, offset, gtfs_id) in &stops {
        net.add_location(
            Location::new(loc_id, link_id, ref_node, offset)
                .with_loc_type("bus_stop")
                .with_gtfs_stop_id(gtfs_id)
                .build(),
        )
        .unwrap();
    }

    net
}

/// The Spiess & Florian (1989) example expressed as a GTFS dataset.
/// Segment times and headways in seconds (25 min = 1500 s, etc).
fn build_gtfs_reference() -> GtfsReference {
    let mut gtfs = GtfsReference::new();

    for (id, name) in [
        ("stop_A", "Stop A"),
        ("stop_X", "Stop X"),
        ("stop_Y", "Stop Y"),
        ("stop_B", "Stop B"),
    ] {
        gtfs.stops.push(Stop::new(id).with_name(name));
    }

    let service_start = 7 * 3600;
    let service_end = 11 * 3600;
    let base = 8 * 3600;
    let mut add_line = |route_id: &str, stops: &[&str], seg_secs: &[u32], headway: u32| {
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
            service_start,
            service_end,
            headway,
        ));
    };

    add_line("L1", &["stop_A", "stop_B"], &[1500], 360);
    add_line("L2", &["stop_A", "stop_X", "stop_Y"], &[420, 360], 360);
    add_line("L3", &["stop_X", "stop_Y", "stop_B"], &[240, 240], 900);
    add_line("L4", &["stop_Y", "stop_B"], &[600], 180);

    gtfs
}

fn main() {
    let road_network = build_road_network();
    println!(
        "GMNS network: {} nodes, {} links, {} locations (bus stops)",
        road_network.node_count(),
        road_network.link_count(),
        road_network.locations.len()
    );

    let gtfs = build_gtfs_reference();
    println!(
        "GTFS: {} stops, {} routes, {} trips, {} stop_times, {} frequencies",
        gtfs.stops.len(),
        gtfs.routes.len(),
        gtfs.trips.len(),
        gtfs.stop_times.len(),
        gtfs.frequencies.len()
    );

    // The user-provided linkage: gtfs_stop_id -> location id
    let stop_mapping = road_network.gtfs_stop_mapping();
    println!("\nGTFS stop -> location mapping:");
    let mut mapping_sorted: Vec<(&String, &i64)> = stop_mapping.iter().collect();
    mapping_sorted.sort();
    for (gtfs_id, loc_id) in &mapping_sorted {
        let loc = road_network.get_location(**loc_id).unwrap();
        println!(
            "  {} -> location {} (link {}, offset {} m)",
            gtfs_id, loc_id, loc.link_id, loc.lr
        );
    }

    let transit_network =
        transit_network_from_gtfs(&gtfs, &stop_mapping).expect("GTFS conversion failed");
    println!("\nTransit routes from GTFS:");
    for route in &transit_network.routes {
        println!(
            "  {}: stops {:?}, headway {:.0} s",
            route.id, route.stops, route.headway
        );
    }

    // Demand: 1000 passengers from stop A to stop B (by location ids)
    let stop_ids: Vec<i64> = mapping_sorted.iter().map(|&(_, &id)| id).collect();
    let mut od = DenseOdMatrix::new(stop_ids);
    od.set(501, 504, 1000.0);
    println!("\nDemand: {:.0} passengers, stop_A -> stop_B", od.total());

    let result = assign_transit(&transit_network, &od).expect("transit assignment failed");

    println!("\nExpected travel times:");
    let mut od_pairs: Vec<(&(i64, i64), &f64)> = result.od_costs.iter().collect();
    od_pairs.sort_by_key(|((o, d), _)| (*o, *d));
    for ((origin, destination), cost) in od_pairs {
        println!(
            "  location {} -> {}: {:.2} min",
            origin,
            destination,
            cost / 60.0
        );
    }

    println!("\nRiding volumes per segment:");
    for lv in &result.link_volumes {
        if lv.kind == TransitLinkKind::Riding && lv.volume > 0.0 {
            println!(
                "  {}: {} -> {}: {:.1} passengers",
                lv.route_id.as_deref().unwrap_or("-"),
                lv.from_stop,
                lv.to_stop,
                lv.volume
            );
        }
    }

    println!("\nBoardings per route:");
    let mut boardings: Vec<(&String, &f64)> = result.route_boardings.iter().collect();
    boardings.sort_by(|a, b| a.0.cmp(b.0));
    for (route_id, volume) in boardings {
        println!("  {}: {:.1}", route_id, volume);
    }
}
