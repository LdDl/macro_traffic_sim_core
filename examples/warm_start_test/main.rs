use macro_traffic_sim_core::config::{AssignmentMethodType, ModelConfig};
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::mode_choice::MultinomialLogit;
use macro_traffic_sim_core::pipeline::{haversine_km, run_four_step_model};
use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
use macro_traffic_sim_core::trip_generation::RegressionGenerator;
use macro_traffic_sim_core::verbose::VerboseLevel;
use macro_traffic_sim_core::zone::Zone;
use std::time::Instant;

fn generated_grid(grid_side: usize, zone_step: usize) -> (Network, Vec<Zone>) {
    let mut net = Network::new();
    let mut link_id: i64 = 1;
    let base_lat = 55.75;
    let base_lon = 37.60;
    let step_deg = 0.002;
    let node_id = |r: usize, c: usize| -> i64 { (r * grid_side + c) as i64 + 1 };
    let mut zone_node_ids: Vec<i64> = Vec::new();
    for r in 0..grid_side {
        for c in 0..grid_side {
            let nid = node_id(r, c);
            let lat = base_lat + r as f64 * step_deg;
            let lon = base_lon + c as f64 * step_deg;
            let is_zone = r % zone_step == 0 && c % zone_step == 0;
            let mut builder = Node::new(nid).with_coordinates(lat, lon);
            if is_zone {
                builder = builder.with_zone_id(nid);
                zone_node_ids.push(nid);
            }
            net.add_node(builder.build()).unwrap();
        }
    }
    for r in 0..grid_side {
        for c in 0..grid_side {
            let from = node_id(r, c);
            let (lat1, lon1) = (
                base_lat + r as f64 * step_deg,
                base_lon + c as f64 * step_deg,
            );
            let mut neigh: Vec<(i64, f64, f64)> = Vec::new();
            if c + 1 < grid_side {
                neigh.push((
                    node_id(r, c + 1),
                    base_lat + r as f64 * step_deg,
                    base_lon + (c + 1) as f64 * step_deg,
                ));
            }
            if r + 1 < grid_side {
                neigh.push((
                    node_id(r + 1, c),
                    base_lat + (r + 1) as f64 * step_deg,
                    base_lon + c as f64 * step_deg,
                ));
            }
            for (to, lat2, lon2) in neigh {
                let dist = haversine_km(lat1, lon1, lat2, lon2) * 1000.0;
                for &(a, b) in &[(from, to), (to, from)] {
                    net.add_link(
                        Link::new(link_id, a, b)
                            .with_length_meters(dist)
                            .with_free_speed(60.0)
                            .with_capacity(1800.0)
                            .with_lanes_num(2)
                            .build(),
                    )
                    .unwrap();
                    link_id += 1;
                }
            }
        }
    }
    let zones: Vec<Zone> = zone_node_ids
        .iter()
        .enumerate()
        .map(|(i, &nid)| {
            Zone::new(nid)
                .with_population(1000.0 + (i as f64 * 137.0) % 5000.0)
                .with_employment(500.0 + (i as f64 * 271.0) % 4000.0)
                .build()
        })
        .collect();
    (net, zones)
}

fn main() {
    let (network, zones) = generated_grid(30, 2);
    let trip_gen = RegressionGenerator::new();
    let impedance = ExponentialImpedance::new(0.3);
    let logit = MultinomialLogit::default_auto_bike_walk();

    println!(
        "Network: {} zones, {} nodes, {} links",
        zones.len(),
        network.nodes.len(),
        network.links.len()
    );
    println!("FrankWolfe, max_iterations=500, convergence_gap=5e-4, feedback=5\n");

    for &warm in &[false, true] {
        let label = if warm { "WARM" } else { "COLD" };
        let config = ModelConfig::new()
            .with_assignment_method(AssignmentMethodType::FrankWolfe)
            .with_max_iterations(500)
            .with_convergence_gap(5e-4)
            .with_feedback_iterations(5)
            .with_warm_start(warm)
            .with_verbose_level(VerboseLevel::None)
            .build();

        let start = Instant::now();
        let result = run_four_step_model(
            &network, &zones, &trip_gen, &impedance, &logit, &config, None,
        )
        .unwrap();
        let elapsed = start.elapsed();

        println!("=== {} ===", label);
        for (i, ar) in result.per_feedback_assignments.iter().enumerate() {
            println!(
                "  feedback {}: {:>4} iters, gap={:.8}, converged={}",
                i + 1,
                ar.iterations,
                ar.relative_gap,
                ar.converged
            );
        }
        let total_iters: usize = result
            .per_feedback_assignments
            .iter()
            .map(|a| a.iterations)
            .sum();
        println!(
            "  TOTAL: {} iters, {:.1}ms\n",
            total_iters,
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
