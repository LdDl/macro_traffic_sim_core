//! Example: 4-step model on a 3x3 grid with a proper mesoscopic graph.
//!
//! Each intersection is "split" into approach/departure sub-nodes
//! connected by connection links that encode allowed turning movements.
//! Turn restrictions and one-way streets are enforced by topology.
//!
//! 6 zone centroids sit outside the grid and connect to nearby
//! intersections via bidirectional connector links.
//!
//! ```text
//!            (101)---[1]====[2]====[3]---(102)
//!                     |     ||      |
//!                    [4]-->[5]--->[6]
//!                     |     ||      |
//!            (104)---[7]<--[8]<---[9]---(105)
//!                          ||
//!                        (103)
//!
//!   (106) connects to [2] and [5]
//!
//!   ====  bidirectional, 2 lanes, 60 km/h
//!   ---   bidirectional, 1 lane,  40 km/h
//!   -->   one-way eastbound,  2 lanes, 50 km/h
//!   <--   one-way westbound,  1 lane,  40 km/h
//!   ||    bidirectional, 2 lanes, 60 km/h  (central avenue)
//! ```
//!
//! Turn restrictions:
//!   - Intersection 5: no left from south (8) to east (6)
//!   - Intersection 2: no left from east (3) to south (5)
//!
//! Usage:
//!   cargo run --example grid_city

use std::collections::{BTreeSet, HashMap, HashSet};

use macro_traffic_sim_core::config::{AssignmentMethodType, ModelConfig};
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::gmns::types::AgentType;
use macro_traffic_sim_core::mode_choice::MultinomialLogit;
use macro_traffic_sim_core::od::OdMatrix;
use macro_traffic_sim_core::pipeline::{haversine_km, run_four_step_model};
use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
use macro_traffic_sim_core::trip_generation::RegressionGenerator;
use macro_traffic_sim_core::verbose::VerboseLevel;
use macro_traffic_sim_core::zone::Zone;
use tracing::info;

/// Link category for separating output by type.
#[derive(Clone)]
enum LinkKind {
    /// Road segment between two macro-intersections: (from, to, lanes, capacity)
    Road {
        from: i64,
        to: i64,
        lanes: i32,
        total_capacity: f64,
    },
    /// Connection link (turn) at an intersection: (from_neighbor, via_intersection, to_neighbor)
    Turn {
        from: i64,
        via: i64,
        to: i64,
    },
    /// Centroid connector: (centroid, intersection)
    Connector {
        from: i64,
        to: i64,
    },
}

fn main() {
    let (network, link_info) = build_network();
    let zones = build_zones();

    info!(
        event = "network_built",
        nodes = network.node_count(),
        links = network.link_count(),
        zones = zones.len(),
        "Network ready",
    );

    let trip_gen = RegressionGenerator::new();
    let impedance = ExponentialImpedance::new(0.15);
    let logit = MultinomialLogit::default_auto_bike_walk();

    let config = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(200)
        .with_convergence_gap(1e-3)
        .with_feedback_iterations(3)
        .with_verbose_level(VerboseLevel::Main)
        .build();

    let result = run_four_step_model(&network, &zones, &trip_gen, &impedance, &logit, &config)
        .expect("pipeline failed");

    for (i, zone) in zones.iter().enumerate() {
        info!(
            event = "trip_generation",
            zone_id = zone.id,
            name = zone.name.as_str(),
            productions = format!("{:.0}", result.productions[i]),
            attractions = format!("{:.0}", result.attractions[i]),
            "Zone trips",
        );
    }

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

    info!(
        event = "assignment_result",
        iterations = result.assignment.iterations,
        gap = format!("{:.6}", result.assignment.relative_gap),
        converged = result.assignment.converged,
        feedback_iterations = result.feedback_iterations_done,
        "Assignment complete",
    );

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

    // Road segments: volumes, V/C, congestion
    let mut road_volumes: Vec<(i64, f64, &LinkKind)> = result
        .assignment
        .link_volumes
        .iter()
        .filter_map(|(&id, &vol)| {
            link_info.get(&id).and_then(|kind| match kind {
                LinkKind::Road { .. } if vol > 0.1 => Some((id, vol, kind)),
                _ => None,
            })
        })
        .collect();
    road_volumes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    for (id, vol, kind) in &road_volumes {
        if let LinkKind::Road {
            from,
            to,
            lanes,
            total_capacity,
        } = kind
        {
            let cost = result
                .assignment
                .link_costs
                .get(id)
                .copied()
                .unwrap_or(0.0);
            let vc = if *total_capacity > 0.0 {
                vol / total_capacity
            } else {
                0.0
            };
            info!(
                event = "road_volume",
                link_id = id,
                segment = format!("{} -> {}", from, to),
                lanes = lanes,
                volume = format!("{:.0}", vol),
                capacity = format!("{:.0}", total_capacity),
                vc_ratio = format!("{:.2}", vc),
                cost_hours = format!("{:.4}", cost),
                "Road segment",
            );
        }
    }

    // Turning volumes at intersections
    let mut turn_volumes: Vec<(i64, f64, &LinkKind)> = result
        .assignment
        .link_volumes
        .iter()
        .filter_map(|(&id, &vol)| {
            link_info.get(&id).and_then(|kind| match kind {
                LinkKind::Turn { .. } if vol > 0.1 => Some((id, vol, kind)),
                _ => None,
            })
        })
        .collect();
    turn_volumes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    for (id, vol, kind) in turn_volumes.iter().take(10) {
        if let LinkKind::Turn { from, via, to } = kind {
            info!(
                event = "turn_volume",
                link_id = id,
                intersection = via,
                movement = format!("{} -> {}", from, to),
                volume = format!("{:.0}", vol),
                "Turn movement",
            );
        }
    }

    // Zone connector totals
    let mut connector_volumes: Vec<(i64, f64, &LinkKind)> = result
        .assignment
        .link_volumes
        .iter()
        .filter_map(|(&id, &vol)| {
            link_info.get(&id).and_then(|kind| match kind {
                LinkKind::Connector { .. } if vol > 0.1 => Some((id, vol, kind)),
                _ => None,
            })
        })
        .collect();
    connector_volumes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    for (id, vol, kind) in &connector_volumes {
        if let LinkKind::Connector { from, to } = kind {
            info!(
                event = "connector_volume",
                link_id = id,
                segment = format!("{} -> {}", from, to),
                volume = format!("{:.0}", vol),
                "Zone connector",
            );
        }
    }
}

/// Build the mesoscopic 3x3 grid network.
///
/// Each intersection macro-node is split into sub-nodes:
///   - incoming sub-node  at intersection N from neighbor A:  N*1000 + A
///   - outgoing sub-node  at intersection N toward neighbor B: N*1000 + 500 + B
///
/// A connection link from (in-sub) to (out-sub) encodes one allowed turn.
/// Missing connection links = forbidden turns.
///
/// Returns `(network, link_info)` where `link_info` maps link_id
/// to its [`LinkKind`] for categorized output.
fn build_network() -> (Network, HashMap<i64, LinkKind>) {
    let mut net = Network::new();
    let mut link_id: i64 = 1000;
    let mut info: HashMap<i64, LinkKind> = HashMap::new();

    // Macro intersection coordinates (used for distances and sub-node placement)
    //
    //   [1]====[2]====[3]       row 0: lat 55.760
    //    |      ||      |
    //   [4]-->[5]--->[6]       row 1: lat 55.751
    //    |      ||      |
    //   [7]<--[8]<---[9]       row 2: lat 55.742
    //
    //   col 0: lon 37.600   col 1: lon 37.613   col 2: lon 37.626
    let grid: [(i64, f64, f64); 9] = [
        (1, 55.760, 37.600),
        (2, 55.760, 37.613),
        (3, 55.760, 37.626),
        (4, 55.751, 37.600),
        (5, 55.751, 37.613),
        (6, 55.751, 37.626),
        (7, 55.742, 37.600),
        (8, 55.742, 37.613),
        (9, 55.742, 37.626),
    ];

    // Zone centroid nodes (node_id, zone_id, lat, lon)
    let centroids: [(i64, i64, f64, f64); 6] = [
        (101, 1, 55.764, 37.593),
        (102, 2, 55.764, 37.633),
        (103, 3, 55.736, 37.613),
        (104, 4, 55.746, 37.593),
        (105, 5, 55.746, 37.633),
        (106, 6, 55.764, 37.613),
    ];

    // All coordinates for haversine distance
    let coords: HashMap<i64, (f64, f64)> = grid
        .iter()
        .map(|&(id, lat, lon)| (id, (lat, lon)))
        .chain(centroids.iter().map(|&(id, _, lat, lon)| (id, (lat, lon))))
        .collect();

    // Directed road list
    // (from, to, lanes, speed_kmh, capacity_per_lane)
    let mut roads: Vec<(i64, i64, i32, f64, f64)> = Vec::new();

    // Bidirectional grid roads
    let bidir = [
        (1, 2, 2, 60.0, 1800.0),
        (2, 3, 2, 60.0, 1800.0),
        (1, 4, 1, 40.0, 1200.0),
        (4, 7, 1, 40.0, 1200.0),
        (2, 5, 2, 60.0, 1800.0),
        (5, 8, 2, 60.0, 1800.0),
        (3, 6, 1, 40.0, 1200.0),
        (6, 9, 1, 40.0, 1200.0),
    ];
    for &(a, b, l, s, c) in &bidir {
        roads.push((a, b, l, s, c));
        roads.push((b, a, l, s, c));
    }

    // One-way: middle row eastbound
    roads.push((4, 5, 2, 50.0, 1500.0));
    roads.push((5, 6, 2, 50.0, 1500.0));
    // One-way: bottom row westbound
    roads.push((9, 8, 1, 40.0, 1200.0));
    roads.push((8, 7, 1, 40.0, 1200.0));

    // Centroid connectors (bidirectional, high capacity)
    let connector_pairs: [(i64, i64); 9] = [
        (101, 1),
        (101, 4),
        (102, 3),
        (102, 6),
        (103, 8),
        (104, 7),
        (105, 9),
        (106, 2),
        (106, 5),
    ];
    for &(c, n) in &connector_pairs {
        roads.push((c, n, 1, 40.0, 9999.0));
        roads.push((n, c, 1, 40.0, 9999.0));
    }

    // Turn restrictions: (from, via, to)
    let banned: HashSet<(i64, i64, i64)> = HashSet::from([
        (8, 5, 6), // no left at intersection 5: northbound from 8 turning east to 6
        (3, 2, 5), // no left at intersection 2: westbound from 3 turning south to 5
    ]);

    // Which macro-node IDs are intersections (to be split)
    let intersections: HashSet<i64> = HashSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);

    // Sub-node ID scheme
    let in_sub = |n: i64, from: i64| -> i64 { n * 1000 + from };
    let out_sub = |n: i64, to: i64| -> i64 { n * 1000 + 500 + to };

    // Split each intersection into sub-nodes + connection links
    for &(n, lat, lon) in &grid {
        // Incoming neighbors (unique)
        let incoming: BTreeSet<i64> = roads
            .iter()
            .filter(|&&(_, to, ..)| to == n)
            .map(|&(from, ..)| from)
            .collect();

        // Outgoing neighbors (unique)
        let outgoing: BTreeSet<i64> = roads
            .iter()
            .filter(|&&(from, ..)| from == n)
            .map(|&(_, to, ..)| to)
            .collect();

        // Create approach sub-nodes
        for &from in &incoming {
            net.add_node(Node::new(in_sub(n, from)).with_coordinates(lat, lon).build())
                .unwrap();
        }

        // Create departure sub-nodes
        for &to in &outgoing {
            net.add_node(
                Node::new(out_sub(n, to))
                    .with_coordinates(lat, lon)
                    .build(),
            )
            .unwrap();
        }

        // Connection links: approach -> departure for each allowed turn
        for &from in &incoming {
            for &to in &outgoing {
                if from == to {
                    continue;
                }
                if banned.contains(&(from, n, to)) {
                    continue;
                }
                net.add_link(
                    Link::new(link_id, in_sub(n, from), out_sub(n, to))
                        .with_is_connection(true)
                        .with_length_meters(10.0)
                        .with_free_speed(30.0)
                        .with_capacity(1800.0)
                        .build(),
                )
                .unwrap();
                info.insert(
                    link_id,
                    LinkKind::Turn {
                        from,
                        via: n,
                        to,
                    },
                );
                link_id += 1;
            }
        }
    }

    // Centroid nodes (not split, just endpoints)
    for &(node_id, zone_id, lat, lon) in &centroids {
        net.add_node(
            Node::new(node_id)
                .with_zone_id(zone_id)
                .with_coordinates(lat, lon)
                .build(),
        )
        .unwrap();
    }

    // Road / connector links between sub-nodes
    for &(from, to, lanes, speed, cap) in &roads {
        // Source: if 'from' is an intersection, use its outgoing sub-node;
        //         otherwise (centroid), use the node directly.
        let source = if intersections.contains(&from) {
            out_sub(from, to)
        } else {
            from
        };
        // Target: if 'to' is an intersection, use its incoming sub-node;
        //         otherwise (centroid), use the node directly.
        let target = if intersections.contains(&to) {
            in_sub(to, from)
        } else {
            to
        };

        let (lat1, lon1) = coords[&from];
        let (lat2, lon2) = coords[&to];
        let length = haversine_km(lat1, lon1, lat2, lon2) * 1000.0;

        net.add_link(
            Link::new(link_id, source, target)
                .with_length_meters(length)
                .with_free_speed(speed)
                .with_capacity(cap)
                .with_lanes_num(lanes)
                .build(),
        )
        .unwrap();

        let is_connector =
            !intersections.contains(&from) || !intersections.contains(&to);
        if is_connector {
            info.insert(link_id, LinkKind::Connector { from, to });
        } else {
            let total_capacity = cap * (lanes.max(1) as f64);
            info.insert(
                link_id,
                LinkKind::Road {
                    from,
                    to,
                    lanes,
                    total_capacity,
                },
            );
        }
        link_id += 1;
    }

    (net, info)
}

/// Build 6 zones balanced for Furness convergence.
///
/// Default regression coefficients:
///   production = 0.5 * pop + 0.1 * emp
///   attraction = 0.1 * pop + 0.8 * emp
///
/// Balance requires sum(prod) = sum(attr):
///   0.5*P + 0.1*E = 0.1*P + 0.8*E  =>  P/E = 1.75
///
/// Totals: pop=52500, emp=30000 -> prod=29250, attr=29250
fn build_zones() -> Vec<Zone> {
    vec![
        Zone::new(1)
            .with_name("Residential NW")
            .with_population(15000.0)
            .with_employment(1500.0)
            .build(),
        Zone::new(2)
            .with_name("Residential NE")
            .with_population(12000.0)
            .with_employment(1500.0)
            .build(),
        Zone::new(3)
            .with_name("CBD South")
            .with_population(3500.0)
            .with_employment(12000.0)
            .build(),
        Zone::new(4)
            .with_name("Residential SW")
            .with_population(8000.0)
            .with_employment(2000.0)
            .build(),
        Zone::new(5)
            .with_name("Industrial SE")
            .with_population(7000.0)
            .with_employment(8000.0)
            .build(),
        Zone::new(6)
            .with_name("University N")
            .with_population(7000.0)
            .with_employment(5000.0)
            .build(),
    ]
}
