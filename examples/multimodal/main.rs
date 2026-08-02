//! Example: a multimodal city - personal cars and public transit on
//! one GMNS network.
//!
//! Extends `path_analysis_multi_class` (full 4-step pipeline, car +
//! truck, path analysis) with a public transit layer:
//!
//! - two bus lines and an in-street tram line, defined over stops that
//!   are GMNS locations pinned to the road links (the road graph is never modified);
//! - zone centroids (the zone IDs themselves) connected to nearby stops
//!   by walk links - the algorithm itself picks the access stop per destination;
//! - transit as a fourth mode-choice alternative: the logit splits the
//!   total demand into auto/bike/walk/transit from a transit skim, and the
//!   transit share is assigned with the Spiess-Florian optimal strategies
//!   algorithm inside the same 4-step pipeline as the car/truck equilibrium.
//!
//! Both the road and transit demand come from the one 4-step run: mode
//! choice is where they part.
//!
//! ```text
//!                 Zone 1 (residential)
//!                        [1]
//!       tram T1        /     \        buses B1, B2
//!                   [3]       [2]
//!   Zone 3 (commercial)       Zone 2 (mixed)
//!                      \     /
//!                        [4]
//!                 Zone 4 (industrial)
//!
//!   transit: bus B1 (z1-z2-z4) and bus B2 (z1-z4 express) run down the
//!   eastern side, tram T1 (z1-z3-z4) down the western side; each line
//!   has a reverse twin
//! ```
//!
//! Usage:
//!   cargo run --example multimodal

use std::collections::HashMap;

use macro_traffic_sim_core::config::{AssignmentMethodType, ModelConfig, UserClassConfig};
use macro_traffic_sim_core::gmns::location::Location;
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::gmns::types::AgentType;
use macro_traffic_sim_core::mode_choice::MultinomialLogit;
use macro_traffic_sim_core::od::OdMatrix;
use macro_traffic_sim_core::pipeline::{TransitInput, haversine_km, run_four_step_model};
use macro_traffic_sim_core::transit::{
    TransitAssignmentOptions, TransitLinkKind, TransitNetwork, TransitRoute,
};
use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
use macro_traffic_sim_core::trip_generation::RegressionGenerator;
use macro_traffic_sim_core::verbose::VerboseLevel;
use macro_traffic_sim_core::zone::Zone;
use tracing::info;

// Transit stop locations: points on road links (IDs 8xx). Zone centroids
// are the zone IDs themselves (1-4) - a transit trip starts and ends at
// its zone, walking to a stop from there. Using the zone IDs as centroids
// is what lets one OD matrix split across road and transit modes.
const STOP_Z1_EAST: i64 = 811;
const STOP_Z2: i64 = 812;
const STOP_Z4_EAST: i64 = 814;
const STOP_Z1_WEST: i64 = 821;
const STOP_Z3: i64 = 823;
const STOP_Z4_WEST: i64 = 824;
const ZONE_1: i64 = 1;
const ZONE_2: i64 = 2;
const ZONE_3: i64 = 3;
const ZONE_4: i64 = 4;

fn main() {
    let network = build_network();
    let zones = build_zones();

    info!(
        event = "network_built",
        nodes = network.node_count(),
        links = network.link_count(),
        zones = zones.len(),
        "Network ready",
    );

    // Trip generation: regression with default coefficients
    //   production = 0.5 * population + 0.1 * employment
    //   attraction = 0.1 * population + 0.8 * employment
    let trip_gen = RegressionGenerator::new();

    // Trip distribution: exponential impedance f(t) = exp(-0.1 * t)
    let impedance = ExponentialImpedance::new(0.1);

    // Mode choice: multinomial logit (auto/bike/walk)
    let logit = MultinomialLogit::default_auto_bike_walk();

    // Config: Frank-Wolfe with store_paths, multi-class, 3 feedback iterations
    let class_names = ["car", "truck"];
    let config = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(50)
        .with_convergence_gap(1e-4)
        .with_feedback_iterations(3)
        .with_verbose_level(VerboseLevel::Main)
        .with_store_paths(true)
        .with_user_classes(vec![
            UserClassConfig::new("car", 1.0, 1.0, 0.9),
            UserClassConfig::new("truck", 2.5, 2.5, 0.1),
        ])
        .build();

    let result = run_four_step_model(
        &network, &zones, &trip_gen, &impedance, &logit, &config, None, None,
    )
    .expect("pipeline failed");

    // Trip generation results
    for (i, zone) in zones.iter().enumerate() {
        info!(
            event = "trip_generation",
            zone_id = zone.id,
            productions = format!("{:.0}", result.productions[i]),
            attractions = format!("{:.0}", result.attractions[i]),
            "Zone trips",
        );
    }

    // Mode split totals
    for mode in &[AgentType::Auto, AgentType::Bike, AgentType::Walk] {
        if let Some(od) = result.mode_od.get(mode) {
            info!(
                event = "mode_split",
                mode = mode.to_string(),
                trips = format!("{:.0}", od.total()),
                "Mode total",
            );
        }
    }

    // Assignment summary
    info!(
        event = "assignment_result",
        iterations = result.assignment.iterations,
        gap = format!("{:.6}", result.assignment.relative_gap),
        converged = result.assignment.converged,
        feedback_iterations = result.feedback_iterations_done,
        "Assignment complete",
    );

    // Per-class volumes (multi-class only)
    if let Some(ref cv) = result.assignment.class_volumes {
        for (class_name, volumes) in cv {
            let total: f64 = volumes.values().sum();
            info!(
                event = "class_volumes",
                class = class_name.as_str(),
                total_volume = format!("{:.1}", total),
                "Class total vehicle-trips on network",
            );
        }
        let pcu_total: f64 = result.assignment.link_volumes.values().sum();
        info!(
            event = "pcu_total",
            total = format!("{:.1}", pcu_total),
            "PCU-total on network",
        );
    }

    // Timings
    let t = &result.timings;
    info!(
        event = "timings",
        generation_ms = format!("{:.3}", t.generation.as_secs_f64() * 1000.0),
        distribution_ms = format!("{:.3}", t.distribution.as_secs_f64() * 1000.0),
        mode_choice_ms = format!("{:.3}", t.mode_choice.as_secs_f64() * 1000.0),
        assignment_ms = format!("{:.3}", t.assignment.as_secs_f64() * 1000.0),
        total_ms = format!("{:.3}", t.total.as_secs_f64() * 1000.0),
        "Pipeline timings",
    );

    // Top loaded links
    let mut volumes: Vec<(i64, f64)> = result
        .assignment
        .link_volumes
        .iter()
        .map(|(&id, &vol)| (id, vol))
        .collect();
    volumes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (id, vol) in volumes.iter().take(10) {
        let cost = result.assignment.link_costs.get(id).copied().unwrap_or(0.0);
        info!(
            event = "link_volume",
            link_id = id,
            volume = format!("{:.1}", vol),
            cost_hours = format!("{:.4}", cost),
            "Link load",
        );
    }

    // Path analysis (requires store_paths = true)
    let paths = match result.assignment.path_flows.as_ref() {
        Some(p) => p,
        None => {
            info!(
                event = "path_analysis",
                "No path data (store_paths not enabled)"
            );
            return;
        }
    };

    info!(
        event = "path_analysis",
        total_paths = paths.len(),
        "Paths extracted",
    );

    // 1) OD pair query: shortest route from Zone 1 to Zone 4, per class
    let origin = 1;
    let dest = 4;

    let od_paths: Vec<_> = paths
        .iter()
        .filter(|p| p.origin_zone == origin && p.dest_zone == dest)
        .collect();

    for (ci, class_name) in class_names.iter().enumerate() {
        let mut class_paths: Vec<_> = od_paths
            .iter()
            .filter(|p| p.class_index == Some(ci as u16))
            .collect();
        class_paths.sort_by(|a, b| b.flow.partial_cmp(&a.flow).unwrap());

        let total_flow: f64 = class_paths.iter().map(|p| p.flow).sum();

        for p in &class_paths {
            let share = if total_flow > 0.0 {
                p.flow / total_flow * 100.0
            } else {
                0.0
            };
            info!(
                event = "od_path",
                origin = origin,
                dest = dest,
                class = *class_name,
                path_index = p.path_index,
                flow = format!("{:.1}", p.flow),
                share_pct = format!("{:.0}", share),
                cost_hours = format!("{:.4}", p.cost),
                link_ids = format!("{:?}", p.link_ids),
                "OD pair route",
            );
        }

        info!(
            event = "od_path_summary",
            origin = origin,
            dest = dest,
            class = *class_name,
            total_flow = format!("{:.1}", total_flow),
            num_paths = class_paths.len(),
            "OD pair class total",
        );
    }

    // 2) Select link analysis: which OD pairs use link 102 (1->3)?
    let target_link: i64 = 102;

    let mut od_totals: HashMap<(i64, i64, Option<u16>), f64> = HashMap::new();
    for p in paths {
        *od_totals
            .entry((p.origin_zone, p.dest_zone, p.class_index))
            .or_insert(0.0) += p.flow;
    }

    let mut select_link: HashMap<(i64, i64, Option<u16>), (f64, u32, f64)> = HashMap::new();
    for p in paths {
        if p.link_ids.contains(&target_link) {
            let od_total = od_totals
                .get(&(p.origin_zone, p.dest_zone, p.class_index))
                .copied()
                .unwrap_or(0.0);
            let entry = select_link
                .entry((p.origin_zone, p.dest_zone, p.class_index))
                .or_insert((0.0, 0, od_total));
            entry.0 += p.flow;
            entry.1 += 1;
        }
    }

    let mut select_link_sorted: Vec<_> = select_link.into_iter().collect();
    select_link_sorted.sort_by(|a, b| b.1.0.partial_cmp(&a.1.0).unwrap());

    let mut total_through_link = 0.0;
    for ((o, d, ci), (flow_through, n_paths, flow_od)) in &select_link_sorted {
        let cname = match ci {
            Some(i) => class_names[*i as usize],
            None => "all",
        };
        info!(
            event = "select_link",
            link_id = target_link,
            origin = o,
            dest = d,
            class = cname,
            flow_through = format!("{:.1}", flow_through),
            paths = n_paths,
            od_flow = format!("{:.1}", flow_od),
            "Select link OD",
        );
        total_through_link += flow_through;
    }

    info!(
        event = "select_link_summary",
        link_id = target_link,
        total_flow = format!("{:.1}", total_through_link),
        od_pairs = select_link_sorted.len(),
        "Select link total",
    );

    // Public transit: optimal strategies assignment over the transit
    // layer (times in minutes). Demand is an exogenous OD between zone
    // centroids acting as pseudo-stops.
    let transit_network = build_transit_network();
    let transit_od = build_transit_od();

    info!(
        event = "transit_network",
        routes = transit_network.routes.len(),
        walk_links = transit_network.walk_links.len(),
        demand = format!("{:.0}", transit_od.total()),
        "Transit network ready",
    );

    let transit = assign_transit(&transit_network, &transit_od).expect("transit assignment failed");

    // Expected travel times per OD pair (waiting + in-vehicle + walking)
    let mut od_costs: Vec<(&(i64, i64), &f64)> = transit.od_costs.iter().collect();
    od_costs.sort_by_key(|((o, d), _)| (*o, *d));
    for ((origin, destination), cost) in od_costs {
        info!(
            event = "transit_od_cost",
            origin = origin,
            destination = destination,
            expected_min = format!("{:.2}", cost),
            "Transit expected travel time",
        );
    }

    // Access stop choice: walking volumes leaving the centroids show
    // which stop each zone uses, per destination mix
    for lv in &transit.link_volumes {
        if lv.kind == TransitLinkKind::Walking && lv.volume > 0.0 && lv.from_stop >= 900 {
            info!(
                event = "transit_access",
                centroid = lv.from_stop,
                stop = lv.to_stop,
                passengers = format!("{:.1}", lv.volume),
                "Access walk",
            );
        }
    }

    // Riding volumes per line segment
    for lv in &transit.link_volumes {
        if lv.kind == TransitLinkKind::Riding && lv.volume > 0.0 {
            info!(
                event = "transit_riding",
                route = lv.route_id.as_deref().unwrap_or("-"),
                from_stop = lv.from_stop,
                to_stop = lv.to_stop,
                passengers = format!("{:.1}", lv.volume),
                "Riding volume",
            );
        }
    }

    // Boardings per route
    let mut boardings: Vec<(&String, &f64)> = transit.route_boardings.iter().collect();
    boardings.sort_by(|a, b| a.0.cmp(b.0));
    for (route_id, volume) in boardings {
        if *volume > 0.0 {
            info!(
                event = "transit_boardings",
                route = route_id.as_str(),
                passengers = format!("{:.1}", volume),
                "Route boardings",
            );
        }
    }
}

/// Build the 4-zone diamond network.
///
/// Intersection nodes 1-4 are also zone centroids.
/// Road segments connect adjacent intersections (bidirectional).
/// Connection links at each node allow all turns except U-turns.
fn build_network() -> Network {
    let mut net = Network::new();

    // Intersection nodes (zone centroids)
    // Laid out as a diamond ~1 km apart:
    //   Node 1: top        (55.76, 37.62)
    //   Node 2: right      (55.755, 37.63)
    //   Node 3: left       (55.755, 37.61)
    //   Node 4: bottom     (55.75, 37.62)
    let coords = [
        (1, 55.760, 37.620),
        (2, 55.755, 37.630),
        (3, 55.755, 37.610),
        (4, 55.750, 37.620),
    ];

    for &(id, lat, lon) in &coords {
        net.add_node(
            Node::new(id)
                .with_zone_id(id)
                .with_coordinates(lat, lon)
                .build(),
        )
        .unwrap();
    }

    // Road segment links (bidirectional pairs)
    // Diamond edges: 1-2, 1-3, 2-4, 3-4
    let edges = [(1, 2), (1, 3), (2, 4), (3, 4)];

    let mut link_id: i64 = 100;
    let mut road_links: HashMap<(i64, i64), i64> = HashMap::new();

    for &(a, b) in &edges {
        let (_, lat1, lon1) = coords.iter().find(|&&(id, _, _)| id == a).unwrap();
        let (_, lat2, lon2) = coords.iter().find(|&&(id, _, _)| id == b).unwrap();
        let dist = haversine_km(*lat1, *lon1, *lat2, *lon2) * 1000.0;

        // Forward: a -> b
        net.add_link(
            Link::new(link_id, a, b)
                .with_length_meters(dist)
                .with_free_speed(60.0)
                .with_capacity(1800.0)
                .with_lanes_num(2)
                .build(),
        )
        .unwrap();
        road_links.insert((a, b), link_id);
        link_id += 1;

        // Reverse: b -> a
        net.add_link(
            Link::new(link_id, b, a)
                .with_length_meters(dist)
                .with_free_speed(60.0)
                .with_capacity(1800.0)
                .with_lanes_num(2)
                .build(),
        )
        .unwrap();
        road_links.insert((b, a), link_id);
        link_id += 1;
    }

    // Connection links at each intersection: allow all turns except U-turns.
    // For each node, connect every incoming road to every outgoing road
    // where the source and target differ.
    let node_ids: Vec<i64> = coords.iter().map(|&(id, _, _)| id).collect();
    for &n in &node_ids {
        let incoming: Vec<(i64, i64)> = road_links
            .iter()
            .filter(|&(&(_, to), _)| to == n)
            .map(|(&(from, _), &lid)| (from, lid))
            .collect();
        let outgoing: Vec<(i64, i64)> = road_links
            .iter()
            .filter(|&(&(from, _), _)| from == n)
            .map(|(&(_, to), &lid)| (to, lid))
            .collect();

        for &(in_from, _in_lid) in &incoming {
            for &(out_to, _out_lid) in &outgoing {
                // Skip U-turn
                if in_from == out_to {
                    continue;
                }
                net.add_link(
                    Link::new(link_id, n, n)
                        .with_is_connection(true)
                        .with_length_meters(10.0)
                        .with_free_speed(30.0)
                        .with_capacity(1800.0)
                        .build(),
                )
                .unwrap();
                link_id += 1;
            }
        }
    }

    // Transit stops as GMNS locations: points on the road links, the
    // road graph itself is untouched. Eastern side (links 1->2 and
    // 2->4) hosts the bus stops, western side (1->3 and 3->4) the
    // in-street tram stops. One platform per stop serves both
    // directions here; real models would split them per direction.
    let stops = [
        (STOP_Z1_EAST, road_links[&(1, 2)], 1, 100.0, "bus_stop"),
        (STOP_Z2, road_links[&(1, 2)], 1, 700.0, "bus_stop"),
        (STOP_Z4_EAST, road_links[&(2, 4)], 2, 750.0, "bus_stop"),
        (STOP_Z1_WEST, road_links[&(1, 3)], 1, 100.0, "tram_stop"),
        (STOP_Z3, road_links[&(1, 3)], 1, 700.0, "tram_stop"),
        (STOP_Z4_WEST, road_links[&(3, 4)], 3, 750.0, "tram_stop"),
    ];
    for &(loc_id, on_link, ref_node, offset, loc_type) in &stops {
        net.add_location(
            Location::new(loc_id, on_link, ref_node, offset)
                .with_loc_type(loc_type)
                .build(),
        )
        .unwrap();
    }

    net
}

/// Build 4 zones with different land-use profiles.
fn build_zones() -> Vec<Zone> {
    // Default regression coefficients:
    //   production = 0.5 * pop + 0.1 * emp
    //   attraction = 0.1 * pop + 0.8 * emp
    //
    // Furness (IPF) requires balanced totals:
    //   sum(prod) = sum(attr)  =>  0.4 * P_total = 0.7 * E_total
    //   =>  P_total / E_total = 1.75
    //
    // Total: pop=14000, emp=8000 -> prod=7800, attr=7800
    vec![
        Zone::new(1)
            .with_name("Residential North")
            .with_population(6000.0)
            .with_employment(500.0)
            .build(),
        Zone::new(2)
            .with_name("Mixed East")
            .with_population(4000.0)
            .with_employment(2500.0)
            .build(),
        Zone::new(3)
            .with_name("Commercial West")
            .with_population(2000.0)
            .with_employment(3000.0)
            .build(),
        Zone::new(4)
            .with_name("Industrial South")
            .with_population(2000.0)
            .with_employment(2000.0)
            .build(),
    ]
}

/// Build the transit layer over the stop locations (times in minutes).
///
/// Eastern side: bus B1 serves every stop (z1 - z2 - z4), bus B2 runs
/// express (z1 - z4, no intermediate stop). They share stops 811 and
/// 814, so for a z1 -> z4 passenger the optimal strategy is "board
/// whichever comes first" with the frequency-proportional split of
/// Spiess & Florian. Western side: tram T1 (z1 - z3 - z4). Each line
/// has a reverse twin (suffix "r") because `TransitRoute` is one-way.
///
/// Zone centroids (9xx) join as pseudo-stops via walk links; zones 1
/// and 4 reach both sides of the network, so the access stop is chosen
/// by the algorithm per destination, not hardwired.
fn build_transit_network() -> TransitNetwork {
    let mut net = TransitNetwork::new();

    // Eastern buses
    net.add_route(TransitRoute::new(
        "B1",
        vec![STOP_Z1_EAST, STOP_Z2, STOP_Z4_EAST],
        vec![6.0, 7.0],
        6.0,
    ));
    net.add_route(TransitRoute::new(
        "B1r",
        vec![STOP_Z4_EAST, STOP_Z2, STOP_Z1_EAST],
        vec![7.0, 6.0],
        6.0,
    ));
    net.add_route(TransitRoute::new(
        "B2",
        vec![STOP_Z1_EAST, STOP_Z4_EAST],
        vec![11.0],
        12.0,
    ));
    net.add_route(TransitRoute::new(
        "B2r",
        vec![STOP_Z4_EAST, STOP_Z1_EAST],
        vec![11.0],
        12.0,
    ));

    // Western tram
    net.add_route(TransitRoute::new(
        "T1",
        vec![STOP_Z1_WEST, STOP_Z3, STOP_Z4_WEST],
        vec![5.0, 5.0],
        8.0,
    ));
    net.add_route(TransitRoute::new(
        "T1r",
        vec![STOP_Z4_WEST, STOP_Z3, STOP_Z1_WEST],
        vec![5.0, 5.0],
        8.0,
    ));

    // Zone access: centroid <-> stop walk links, both directions.
    // Zones 1 and 4 have a choice between the bus and the tram side.
    let walks = [
        (CENTROID_Z1, STOP_Z1_EAST, 3.0),
        (CENTROID_Z1, STOP_Z1_WEST, 4.0),
        (CENTROID_Z2, STOP_Z2, 2.0),
        (CENTROID_Z3, STOP_Z3, 2.0),
        (CENTROID_Z4, STOP_Z4_EAST, 2.0),
        (CENTROID_Z4, STOP_Z4_WEST, 3.0),
    ];
    for &(centroid, stop, minutes) in &walks {
        net.add_walk_link(centroid, stop, minutes);
        net.add_walk_link(stop, centroid, minutes);
    }

    net
}

/// External (exogenous if to be correct) transit demand between zone centroids (passengers/hour).
///
/// Independent from the road OD produced by the 4-step pipeline: mode
/// choice with a transit alternative is future work, here the transit
/// riders are given directly.
fn build_transit_od() -> DenseOdMatrix {
    let mut od = DenseOdMatrix::new(vec![CENTROID_Z1, CENTROID_Z2, CENTROID_Z3, CENTROID_Z4]);
    od.set(CENTROID_Z1, CENTROID_Z4, 600.0);
    od.set(CENTROID_Z4, CENTROID_Z1, 400.0);
    od.set(CENTROID_Z1, CENTROID_Z3, 300.0);
    od.set(CENTROID_Z2, CENTROID_Z4, 200.0);
    od.set(CENTROID_Z3, CENTROID_Z1, 150.0);
    od
}
