//! Example: 4-step model on a small in-memory network.
//!
//! Builds a 4-zone diamond network entirely in code (no CSV files),
//! runs the full pipeline, and prints results to stdout as JSON.
//!
//! ```text
//!        Zone 1 (residential)
//!          |
//!     [1]--+--[2]
//!      |         |
//! Zone 2         Zone 3
//! (mixed)        (commercial)
//!      |         |
//!     [3]--+--[4]
//!          |
//!        Zone 4 (industrial)
//! ```
//!
//! Each edge is a pair of one-way road segment links (forward + reverse).
//! Connection links at each intersection allow all through-movements
//! and turns except U-turns.
//!
//! Usage:
//!   cargo run --example simple_network

use std::collections::HashMap;

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

    // Config: Frank-Wolfe assignment, 3 feedback iterations
    let config = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(50)
        .with_convergence_gap(1e-4)
        .with_feedback_iterations(3)
        .with_verbose_level(VerboseLevel::Main)
        .build();

    let result = run_four_step_model(&network, &zones, &trip_gen, &impedance, &logit, &config)
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
        let cost = result
            .assignment
            .link_costs
            .get(id)
            .copied()
            .unwrap_or(0.0);
        info!(
            event = "link_volume",
            link_id = id,
            volume = format!("{:.1}", vol),
            cost_hours = format!("{:.4}", cost),
            "Link load",
        );
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
    let edges = [
        (1, 2),
        (1, 3),
        (2, 4),
        (3, 4),
    ];

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
