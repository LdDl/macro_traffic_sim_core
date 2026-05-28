//! Example: ring network -- unbalanced trip ends, auto-normalized by pipeline.
//!
//! 4-node one-way ring where each node is a zone centroid.
//! Reproduces a real request where the pipeline previously failed with
//! "Furness did not converge after 100 iterations".
//!
//! ```text
//!   (1) --link 1--> (2) --link 7--> (3) --link 2--> (4) --link 12--> (1)
//!  zone 1          zone 2          zone 4           zone 3
//! ```
//!
//! The graph is fully connected (one strongly-connected component).
//! The failure was caused by unbalanced trip ends:
//!
//! ```text
//!   sum(productions) = 233.6
//!   sum(attractions) = 308.8   (imbalance 24.4%)
//! ```
//!
//! The pipeline normalizes attractions to match production total before
//! distribution. This step is visible in the pipeline progress log between
//! the Generation and Distribution phases.
//!
//! Usage:
//!   cargo run --example ring_network

use macro_traffic_sim_core::config::{AssignmentMethodType, ModelConfig};
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::gmns::types::AgentType;
use macro_traffic_sim_core::mode_choice::MultinomialLogit;
use macro_traffic_sim_core::od::OdMatrix;
use macro_traffic_sim_core::pipeline::phase::ProgressEvent;
use macro_traffic_sim_core::pipeline::run_four_step_model;
use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
use macro_traffic_sim_core::trip_generation::{RegressionGenerator, TripGenerator};
use macro_traffic_sim_core::verbose::VerboseLevel;
use macro_traffic_sim_core::zone::Zone;

fn main() {
    let network = build_network();
    let zones = build_zones();
    let trip_gen = RegressionGenerator::new();
    let impedance = ExponentialImpedance::new(0.1);
    let logit = MultinomialLogit::default_auto_bike_walk();

    let config = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(100)
        .with_convergence_gap(0.001)
        .with_furness_max_iterations(100)
        .with_furness_tolerance(0.001)
        .with_feedback_iterations(3)
        .with_verbose_level(VerboseLevel::Main)
        .build();

    println!("nodes: {}, links: {}, zones: {}",
        network.node_count(), network.link_count(), zones.len());

    let (productions, attractions) = trip_gen.generate(&zones)
        .expect("trip generation failed");

    println!();
    println!("trip generation (regression):");
    println!("  {:<8} {:>12} {:>12}", "zone", "production", "attraction");
    for (i, zone) in zones.iter().enumerate() {
        println!("  {:<8} {:>12.1} {:>12.1}", zone.id, productions[i], attractions[i]);
    }
    let sum_p: f64 = productions.iter().sum();
    let sum_a: f64 = attractions.iter().sum();
    println!("  {:<8} {:>12.1} {:>12.1}", "TOTAL", sum_p, sum_a);
    println!();

    if (sum_p - sum_a).abs() > 1e-6 * sum_p.max(sum_a) {
        let factor = sum_p / sum_a;
        let imbalance = (sum_p - sum_a).abs() / sum_p.max(sum_a) * 100.0;
        println!("NOTE: imbalance {:.1}% -- pipeline normalizes A *= {:.6}", imbalance, factor);
        println!("  (visible in pipeline log between Generation and Distribution)");
        println!();
    }

    // on_progress prints each phase transition as it happens.
    // Normalization appears in the tracing log (JSON) between generation
    // complete and the first distribution phase below.
    let on_progress = |event: ProgressEvent| {
        if event.feedback_total > 0 {
            println!("[progress] phase={} iter={}/{}", event.phase, event.feedback_iter, event.feedback_total);
        } else {
            println!("[progress] phase={}", event.phase);
        }
    };

    match run_four_step_model(&network, &zones, &trip_gen, &impedance, &logit, &config, Some(&on_progress)) {
        Ok(result) => {
            println!();
            println!("feedback iterations: {}", result.feedback_iterations_done);
            println!("assignment converged: {}, gap: {:.6}", result.assignment.converged, result.assignment.relative_gap);

            println!();
            println!("OD matrix (all modes, total {:.1} trips):", result.total_od.total());
            let zone_ids: Vec<i64> = zones.iter().map(|z| z.id).collect();
            print!("  {:>6}", "");
            for &d in &zone_ids { print!("  {:>8}", format!("-> z{}", d)); }
            println!("  {:>8}", "row sum");
            for &o in &zone_ids {
                print!("  z{:<5}", o);
                for &d in &zone_ids {
                    print!("  {:>8.1}", result.total_od.get(o, d));
                }
                println!("  {:>8.1}", result.total_od.row_sum(o));
            }
            print!("  {:>6}", "col");
            for &d in &zone_ids { print!("  {:>8.1}", result.total_od.col_sum(d)); }
            println!();

            println!();
            println!("mode split:");
            for mode in &[AgentType::Auto, AgentType::Bike, AgentType::Walk] {
                if let Some(od) = result.mode_od.get(mode) {
                    let pct = od.total() / result.total_od.total() * 100.0;
                    println!("  {:5}  {:7.1} trips  ({:.1}%)", mode.to_string(), od.total(), pct);
                }
            }

            println!();
            println!("link volumes:");
            println!("  {:>8}  {:>8}  {:>12}", "link_id", "volume", "cost (h)");
            let mut vols: Vec<_> = result.assignment.link_volumes.iter().collect();
            vols.sort_by_key(|&(&id, _)| id);
            for &(&id, &vol) in &vols {
                if vol > 0.01 {
                    println!("  {:>8}  {:>8.1}  {:>12.4}",
                        id, vol,
                        result.assignment.link_costs.get(&id).copied().unwrap_or(0.0));
                }
            }
        }
        Err(e) => {
            println!("pipeline failed: {}", e);
        }
    }
}

fn build_network() -> Network {
    let mut net = Network::new();

    net.add_node(Node::new(1).with_zone_id(1).with_coordinates(56.8861353328808,  35.901006907224655).build()).unwrap();
    net.add_node(Node::new(2).with_zone_id(2).with_coordinates(56.88693592938617, 35.90406060218811 ).build()).unwrap();
    net.add_node(Node::new(3).with_zone_id(4).with_coordinates(56.88649799368358, 35.904288589954376).build()).unwrap();
    net.add_node(Node::new(4).with_zone_id(3).with_coordinates(56.88577504969939, 35.901316702365875).build()).unwrap();

    // One-way ring: 1 -> 2 -> 3 -> 4 -> 1
    net.add_link(Link::new( 1, 1, 2).with_length_meters(205.754).with_free_speed(60.0).with_capacity(5400.0).with_lanes_num(3).build()).unwrap();
    net.add_link(Link::new( 2, 3, 4).with_length_meters(197.620).with_free_speed(60.0).with_capacity(7200.0).with_lanes_num(4).build()).unwrap();
    net.add_link(Link::new( 7, 2, 3).with_length_meters( 42.475).with_free_speed(60.0).with_capacity(5400.0).with_lanes_num(3).with_is_connection(true).build()).unwrap();
    net.add_link(Link::new(12, 4, 1).with_length_meters( 35.825).with_free_speed(60.0).with_capacity(7200.0).with_lanes_num(4).with_is_connection(true).build()).unwrap();

    net
}

fn build_zones() -> Vec<Zone> {
    vec![
        Zone::new(1).with_population(100.0).with_employment( 15.0).build(),
        Zone::new(2).with_population( 10.0).with_employment( 99.0).build(),
        Zone::new(3).with_population(100.0).with_employment(200.0).build(),
        Zone::new(4).with_population(190.0).with_employment( 22.0).build(),
    ]
}
