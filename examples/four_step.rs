//! Example: 4-step traffic demand model with CSV input.
//!
//! Demonstrates loading a meso network and zones from CSV,
//! configuring the model, and running the pipeline.
//!
//! Usage:
//!   cargo run --example four_step -- \
//!     --nodes data/mesonode.csv \
//!     --links data/mesolink.csv \
//!     --zones data/zone.csv \
//!     --od data/od.csv

use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::process;

use macro_traffic_sim_core::config::ModelConfig;
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::gmns::types::*;
use macro_traffic_sim_core::mode_choice::MultinomialLogit;
use macro_traffic_sim_core::od::OdMatrix;
use macro_traffic_sim_core::od::dense::DenseOdMatrix;
use macro_traffic_sim_core::pipeline::run_four_step_model;
use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
use macro_traffic_sim_core::trip_generation::RegressionGenerator;
use macro_traffic_sim_core::verbose::VerboseLevel;
use macro_traffic_sim_core::zone::Zone;

fn load_nodes(path: &Path) -> Vec<Node> {
    let mut rdr = csv::Reader::from_path(path).unwrap_or_else(|e| {
        eprintln!("cannot open {}: {}", path.display(), e);
        process::exit(1);
    });

    let mut nodes = Vec::new();
    for result in rdr.records() {
        let row = result.expect("csv row error");
        let id: i64 = row.get(0).unwrap().parse().unwrap();
        let macro_node_id: i64 = row.get(1).and_then(|s| s.parse().ok()).unwrap_or(-1);
        let macro_link_id: i64 = row.get(2).and_then(|s| s.parse().ok()).unwrap_or(-1);
        let longitude: f64 = row.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let latitude: f64 = row.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let zone_id: i64 = row.get(5).and_then(|s| s.parse().ok()).unwrap_or(-1);

        let node = Node::new(id)
            .with_macro_node_id(macro_node_id)
            .with_macro_link_id(macro_link_id)
            .with_coordinates(latitude, longitude)
            .with_zone_id(zone_id)
            .build();
        nodes.push(node);
    }
    nodes
}

fn load_links(path: &Path) -> Vec<Link> {
    let mut rdr = csv::Reader::from_path(path).unwrap_or_else(|e| {
        eprintln!("cannot open {}: {}", path.display(), e);
        process::exit(1);
    });

    let mut links = Vec::new();
    for result in rdr.records() {
        let row = result.expect("csv row error");
        let id: i64 = row.get(0).unwrap().parse().unwrap();
        let source: i64 = row.get(1).unwrap().parse().unwrap();
        let target: i64 = row.get(2).unwrap().parse().unwrap();
        let is_connection: bool = row.get(3).and_then(|s| s.parse().ok()).unwrap_or(false);
        let macro_link_id: i64 = row.get(4).and_then(|s| s.parse().ok()).unwrap_or(-1);
        let movement_id: i64 = row.get(5).and_then(|s| s.parse().ok()).unwrap_or(-1);
        let lanes: i32 = row.get(6).and_then(|s| s.parse().ok()).unwrap_or(1);
        let free_speed: f64 = row.get(7).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let capacity: f64 = row.get(8).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let length: f64 = row.get(9).and_then(|s| s.parse().ok()).unwrap_or(0.0);

        let link = Link::new(id, source, target)
            .with_is_connection(is_connection)
            .with_macro_link_id(macro_link_id)
            .with_movement_id(movement_id)
            .with_lanes_num(lanes)
            .with_free_speed(free_speed)
            .with_capacity(capacity)
            .with_length_meters(length)
            .build();
        links.push(link);
    }
    links
}

fn load_zones(path: &Path) -> Vec<Zone> {
    let mut rdr = csv::Reader::from_path(path).unwrap_or_else(|e| {
        eprintln!("cannot open {}: {}", path.display(), e);
        process::exit(1);
    });

    let mut zones = Vec::new();
    for result in rdr.records() {
        let row = result.expect("csv row error");
        let id: i64 = row.get(0).unwrap().parse().unwrap();
        let population: f64 = row.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let employment: f64 = row.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);

        let zone = Zone::new(id)
            .with_population(population)
            .with_employment(employment)
            .build();
        zones.push(zone);
    }
    zones
}

#[allow(dead_code)]
fn load_od(path: &Path, zone_ids: &[ZoneID]) -> DenseOdMatrix {
    let mut rdr = csv::Reader::from_path(path).unwrap_or_else(|e| {
        eprintln!("cannot open {}: {}", path.display(), e);
        process::exit(1);
    });

    let mut od = DenseOdMatrix::new(zone_ids.to_vec());
    for result in rdr.records() {
        let row = result.expect("csv row error");
        let origin: i64 = row.get(0).unwrap().parse().unwrap();
        let dest: i64 = row.get(1).unwrap().parse().unwrap();
        let demand: f64 = row.get(2).unwrap().parse().unwrap();
        od.set(origin, dest, demand);
    }
    od
}

fn save_link_volumes(path: &Path, volumes: &HashMap<LinkID, f64>, costs: &HashMap<LinkID, f64>) {
    let mut wtr = csv::Writer::from_path(path).unwrap_or_else(|e| {
        eprintln!("cannot create {}: {}", path.display(), e);
        process::exit(1);
    });

    wtr.write_record(["link_id", "volume", "travel_time"])
        .unwrap();
    let mut ids: Vec<&LinkID> = volumes.keys().collect();
    ids.sort();
    for id in ids {
        let vol = volumes.get(id).copied().unwrap_or(0.0);
        let cost = costs.get(id).copied().unwrap_or(0.0);
        wtr.write_record(&[
            id.to_string(),
            format!("{:.4}", vol),
            format!("{:.6}", cost),
        ])
        .unwrap();
    }
    wtr.flush().unwrap();
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let nodes_path = get_arg(&args, "--nodes").unwrap_or_else(|| "data/mesonode.csv".to_string());
    let links_path = get_arg(&args, "--links").unwrap_or_else(|| "data/mesolink.csv".to_string());
    let zones_path = get_arg(&args, "--zones").unwrap_or_else(|| "data/zone.csv".to_string());
    let output_path =
        get_arg(&args, "--output").unwrap_or_else(|| "output_volumes.csv".to_string());

    // Load network
    let nodes = load_nodes(Path::new(&nodes_path));
    let links = load_links(Path::new(&links_path));

    let mut network = Network::new();
    for node in nodes {
        network.add_node(node).expect("duplicate node");
    }
    for link in links {
        network.add_link(link).expect("duplicate link");
    }

    println!(
        "Network: {} nodes, {} links",
        network.node_count(),
        network.link_count()
    );

    // Load zones
    let zones = load_zones(Path::new(&zones_path));
    println!("Zones: {}", zones.len());

    // Trip generation + distribution + mode choice + assignment
    let trip_gen = RegressionGenerator::new();
    let impedance = ExponentialImpedance::new(0.1);
    let logit = MultinomialLogit::default_auto_bike_walk();

    let config = ModelConfig::new()
        .with_verbose_level(VerboseLevel::Main)
        .with_feedback_iterations(2)
        .build();

    let result = run_four_step_model(&network, &zones, &trip_gen, &impedance, &logit, &config)
        .expect("pipeline failed");

    println!(
        "Assignment: {} iterations, gap={:.6}, converged={}",
        result.assignment.iterations, result.assignment.relative_gap, result.assignment.converged,
    );

    // Save results
    save_link_volumes(
        Path::new(&output_path),
        &result.assignment.link_volumes,
        &result.assignment.link_costs,
    );
    println!("Volumes saved to {}", output_path);
}

fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
