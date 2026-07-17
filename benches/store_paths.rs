use criterion::{Criterion, criterion_group, criterion_main};

use macro_traffic_sim_core::assignment::OdPath;
use macro_traffic_sim_core::config::{AssignmentMethodType, ModelConfig, UserClassConfig};
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::mode_choice::MultinomialLogit;
use macro_traffic_sim_core::pipeline::{haversine_km, run_four_step_model};
use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
use macro_traffic_sim_core::trip_generation::RegressionGenerator;
use macro_traffic_sim_core::verbose::VerboseLevel;
use macro_traffic_sim_core::zone::Zone;

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
            let (lat1, lon1) = (base_lat + r as f64 * step_deg, base_lon + c as f64 * step_deg);

            let mut neigh: Vec<(i64, f64, f64)> = Vec::new();
            if c + 1 < grid_side {
                let to = node_id(r, c + 1);
                neigh.push((
                    to,
                    base_lat + r as f64 * step_deg,
                    base_lon + (c + 1) as f64 * step_deg,
                ));
            }
            if r + 1 < grid_side {
                let to = node_id(r + 1, c);
                neigh.push((
                    to,
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
            let pop = 1000.0 + (i as f64 * 137.0) % 5000.0;
            let emp = 500.0 + (i as f64 * 271.0) % 4000.0;
            Zone::new(nid)
                .with_population(pop)
                .with_employment(emp)
                .build()
        })
        .collect();

    (net, zones)
}

fn print_path_memory(paths: &[OdPath], label: &str) {
    let struct_size = std::mem::size_of::<OdPath>();
    let total_links: usize = paths.iter().map(|p| p.link_ids.len()).sum();
    let avg_links = total_links as f64 / paths.len() as f64;

    // Vec<LinkID> heap: each link_ids Vec allocates capacity * size_of::<LinkID>() on heap.
    // OdPath struct itself is stored inline in the outer Vec.
    let struct_total = paths.len() * struct_size;
    let heap_total = total_links * std::mem::size_of::<i64>();
    let total_bytes = struct_total + heap_total;

    println!("  [{label}] paths: {}", paths.len());
    println!("  [{label}] avg links/path: {:.1}", avg_links);
    println!(
        "  [{label}] sizeof(OdPath): {} bytes",
        struct_size
    );
    println!(
        "  [{label}] memory: structs={:.2} MB, heap(link_ids)={:.2} MB, total={:.2} MB",
        struct_total as f64 / 1_048_576.0,
        heap_total as f64 / 1_048_576.0,
        total_bytes as f64 / 1_048_576.0,
    );
}

fn bench_store_paths_single_class(c: &mut Criterion) {
    let (network, zones) = generated_grid(50, 2);
    let trip_gen = RegressionGenerator::new();
    let impedance = ExponentialImpedance::new(0.3);
    let logit = MultinomialLogit::default_auto_bike_walk();

    println!(
        "store_paths single-class: {} zones, {} nodes, {} links",
        zones.len(),
        network.nodes.len(),
        network.links.len()
    );

    let config_no_paths = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(20)
        .with_convergence_gap(1e-3)
        .with_feedback_iterations(1)
        .with_verbose_level(VerboseLevel::None)
        .build();

    let config_with_paths = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(20)
        .with_convergence_gap(1e-3)
        .with_feedback_iterations(1)
        .with_verbose_level(VerboseLevel::None)
        .with_store_paths(true)
        .build();

    let mut group = c.benchmark_group("store_paths_single_class");

    group.bench_function("without_paths", |b| {
        b.iter(|| {
            run_four_step_model(
                &network,
                &zones,
                &trip_gen,
                &impedance,
                &logit,
                &config_no_paths,
                None,
            )
            .unwrap()
        });
    });

    group.bench_function("with_paths", |b| {
        b.iter(|| {
            run_four_step_model(
                &network,
                &zones,
                &trip_gen,
                &impedance,
                &logit,
                &config_with_paths,
                None,
            )
            .unwrap()
        });
    });

    let result = run_four_step_model(
        &network,
        &zones,
        &trip_gen,
        &impedance,
        &logit,
        &config_with_paths,
        None,
    )
    .unwrap();
    if let Some(ref paths) = result.assignment.path_flows {
        print_path_memory(paths, "single-class");
    }

    group.finish();
}

fn bench_store_paths_multi_class(c: &mut Criterion) {
    let (network, zones) = generated_grid(50, 2);
    let trip_gen = RegressionGenerator::new();
    let impedance = ExponentialImpedance::new(0.3);
    let logit = MultinomialLogit::default_auto_bike_walk();

    println!(
        "store_paths multi-class: {} zones, {} nodes, {} links",
        zones.len(),
        network.nodes.len(),
        network.links.len()
    );

    let classes = vec![
        UserClassConfig::new("car", 1.0, 1.0, 0.9),
        UserClassConfig::new("truck", 2.5, 2.5, 0.1),
    ];

    let config_no_paths = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(20)
        .with_convergence_gap(1e-3)
        .with_feedback_iterations(1)
        .with_verbose_level(VerboseLevel::None)
        .with_user_classes(classes.clone())
        .build();

    let config_with_paths = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(20)
        .with_convergence_gap(1e-3)
        .with_feedback_iterations(1)
        .with_verbose_level(VerboseLevel::None)
        .with_user_classes(classes)
        .with_store_paths(true)
        .build();

    let mut group = c.benchmark_group("store_paths_multi_class");

    group.bench_function("without_paths", |b| {
        b.iter(|| {
            run_four_step_model(
                &network,
                &zones,
                &trip_gen,
                &impedance,
                &logit,
                &config_no_paths,
                None,
            )
            .unwrap()
        });
    });

    group.bench_function("with_paths", |b| {
        b.iter(|| {
            run_four_step_model(
                &network,
                &zones,
                &trip_gen,
                &impedance,
                &logit,
                &config_with_paths,
                None,
            )
            .unwrap()
        });
    });

    let result = run_four_step_model(
        &network,
        &zones,
        &trip_gen,
        &impedance,
        &logit,
        &config_with_paths,
        None,
    )
    .unwrap();
    if let Some(ref paths) = result.assignment.path_flows {
        let car_paths = paths.iter().filter(|p| p.class_index == Some(0)).count();
        let truck_paths = paths.iter().filter(|p| p.class_index == Some(1)).count();
        println!(
            "  [multi-class] car={}, truck={}",
            car_paths, truck_paths
        );
        print_path_memory(paths, "multi-class");
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_store_paths_single_class,
    bench_store_paths_multi_class
);
criterion_main!(benches);
