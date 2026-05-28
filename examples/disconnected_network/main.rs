//! Example: disconnected network - demonstrates Furness failure.
//!
//! This example reproduces a real request sent to the macro pipeline
//! that results in "Furness did not converge". The network has 4 nodes
//! and only 2 one-way links forming two isolated chains:
//!
//! ```text
//!   Node 1 (zone 1) ---link 1---> Node 2 (zone 2)
//!
//!   Node 3 (zone 4) ---link 2---> Node 4 (zone 3)
//! ```
//!
//! Nodes 2 and 4 have no outgoing links. Zones 2 and 3 (whose centroids
//! are nodes 2 and 4) have positive productions but zero reachable
//! destinations. Furness cannot balance their rows and fails.
//!
//! Usage:
//!   cargo run --example disconnected_network

use macro_traffic_sim_core::config::{AssignmentMethodType, ModelConfig};
use macro_traffic_sim_core::error::SimError;
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::gmns::types::AgentType;
use macro_traffic_sim_core::mode_choice::{ModeUtility, MultinomialLogit};
use macro_traffic_sim_core::pipeline::error::{InvalidInputReason, PipelineError};
use macro_traffic_sim_core::pipeline::run_four_step_model;
use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
use macro_traffic_sim_core::trip_generation::{RegressionCoefficients, RegressionGenerator};
use macro_traffic_sim_core::verbose::VerboseLevel;
use macro_traffic_sim_core::zone::Zone;

fn main() {
    let network = build_network();
    let zones = build_zones();
    let trip_gen = build_trip_gen();
    let impedance = ExponentialImpedance::new(0.1);
    let logit = build_logit();
    let config = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(100)
        .with_convergence_gap(0.001)
        .with_feedback_iterations(3)
        .with_verbose_level(VerboseLevel::None)
        .build();

    println!("nodes: {}, links: {}, zones: {}",
        network.node_count(), network.link_count(), zones.len());

    match run_four_step_model(&network, &zones, &trip_gen, &impedance, &logit, &config) {
        Ok(result) => {
            println!("pipeline succeeded (unexpected)");
            println!("  feedback iterations done: {}", result.feedback_iterations_done);
            println!("  assignment converged: {}", result.assignment.converged);
            println!("  relative gap: {:.6}", result.assignment.relative_gap);
        }
        Err(e) => {
            println!("pipeline failed (expected):");
            println!("  {}", e);
            if let SimError::Pipeline(PipelineError::InvalidInput(
                InvalidInputReason::DisconnectedComponents { components },
            )) = &e
            {
                println!();
                println!("detected {} isolated zone groups:", components.len());
                for (i, comp) in components.iter().enumerate() {
                    println!("  component {}: zones {:?}", i + 1, comp);
                }
                println!();
                println!("fix: add bidirectional links so every zone centroid can reach");
                println!("every other zone centroid. see simple_network for a working example.");
            }
        }
    }
}

fn build_network() -> Network {
    let mut net = Network::new();

    // Exact coordinates from the original request.
    // zone_id is set so preflight validation passes.
    // Note: node 3 is centroid of zone 4, node 4 is centroid of zone 3.
    let nodes = [
        (1, 56.8861353328808,  35.901006907224655, 1_i64),
        (2, 56.88693592938617, 35.90406060218811,  2_i64),
        (3, 56.88649799368358, 35.904288589954376, 4_i64),
        (4, 56.88577504969939, 35.901316702365875, 3_i64),
    ];
    for (id, lat, lon, zone_id) in nodes {
        net.add_node(
            Node::new(id)
                .with_coordinates(lat, lon)
                .with_zone_id(zone_id)
                .build(),
        )
        .unwrap();
    }

    // Two one-way links - no cross-connection between the chains.
    // Chain 1: node 1 -> node 2 (node 2 has no outgoing links)
    // Chain 2: node 3 -> node 4 (node 4 has no outgoing links)
    net.add_link(
        Link::new(1, 1, 2)
            .with_length_meters(205.7543447516241)
            .with_free_speed(60.0)
            .with_capacity(5400.0)
            .with_lanes_num(3)
            .build(),
    )
    .unwrap();
    net.add_link(
        Link::new(2, 3, 4)
            .with_length_meters(197.62034517984398)
            .with_free_speed(60.0)
            .with_capacity(7200.0)
            .with_lanes_num(4)
            .build(),
    )
    .unwrap();

    net
}

fn build_zones() -> Vec<Zone> {
    vec![
        Zone::new(1).with_population(100.0).with_employment(15.0).build(),
        Zone::new(2).with_population(10.0).with_employment(99.0).build(),
        Zone::new(3).with_population(100.0).with_employment(200.0).build(),
        Zone::new(4).with_population(190.0).with_employment(22.0).build(),
    ]
}

fn build_trip_gen() -> RegressionGenerator {
    RegressionGenerator::with_coefficients(
        RegressionCoefficients {
            intercept: 0.0,
            pop_coeff: 0.5,
            emp_coeff: 0.1,
            hh_coeff: 0.0,
            income_coeff: 0.0,
        },
        RegressionCoefficients {
            intercept: 0.0,
            pop_coeff: 0.1,
            emp_coeff: 0.8,
            hh_coeff: 0.0,
            income_coeff: 0.0,
        },
    )
}

fn build_logit() -> MultinomialLogit {
    MultinomialLogit::new(vec![
        ModeUtility::new(AgentType::Auto)
            .with_asc(0.0)
            .with_coeff_time(-0.03)
            .with_coeff_cost(-0.05)
            .build(),
        ModeUtility::new(AgentType::Bike)
            .with_asc(-1.5)
            .with_coeff_time(-0.04)
            .with_coeff_distance(-0.1)
            .build(),
        ModeUtility::new(AgentType::Walk)
            .with_asc(-2.0)
            .with_coeff_time(-0.05)
            .with_coeff_distance(-0.2)
            .build(),
    ])
}
