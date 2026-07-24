use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use macro_traffic_sim_core::assignment::diagonalization::assign_diagonalization;
use macro_traffic_sim_core::assignment::multiclass::{UserClass, assign_multiclass_fw};
use macro_traffic_sim_core::assignment::{AssignmentConfig, BprFunction, IndexedGraph, OdPath, VolumeDelayFunction};
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::od::dense::DenseOdMatrix;
use macro_traffic_sim_core::od::OdMatrix;
use macro_traffic_sim_core::pipeline::haversine_km;

fn generated_grid(grid_side: usize, zone_step: usize) -> (Network, Vec<i64>) {
    let mut net = Network::new();
    let mut link_id: i64 = 1;

    let base_lat = 55.75;
    let base_lon = 37.60;
    let step_deg = 0.002;

    let node_id = |r: usize, c: usize| -> i64 { (r * grid_side + c) as i64 + 1 };

    let mut zone_ids: Vec<i64> = Vec::new();

    for r in 0..grid_side {
        for c in 0..grid_side {
            let nid = node_id(r, c);
            let lat = base_lat + r as f64 * step_deg;
            let lon = base_lon + c as f64 * step_deg;

            let is_zone = r % zone_step == 0 && c % zone_step == 0;
            let mut builder = Node::new(nid).with_coordinates(lat, lon);
            if is_zone {
                builder = builder.with_zone_id(nid);
                zone_ids.push(nid);
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

    (net, zone_ids)
}

fn uniform_od(zone_ids: &[i64], demand_per_pair: f64) -> DenseOdMatrix {
    let n = zone_ids.len();
    let mut data = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            if i != j {
                data[i * n + j] = demand_per_pair;
            }
        }
    }
    DenseOdMatrix::from_data(zone_ids.to_vec(), data)
}

fn gravity_od(
    zone_ids: &[i64],
    grid_side: usize,
    zone_step: usize,
    base_demand: f64,
    beta: f64,
) -> DenseOdMatrix {
    let n = zone_ids.len();
    let zones_per_row = grid_side / zone_step;
    let mut data = vec![0.0; n * n];
    for i in 0..n {
        let ri = (i / zones_per_row) as f64;
        let ci = (i % zones_per_row) as f64;
        for j in 0..n {
            if i == j {
                continue;
            }
            let rj = (j / zones_per_row) as f64;
            let cj = (j % zones_per_row) as f64;
            let dist = ((ri - rj).powi(2) + (ci - cj).powi(2)).sqrt();
            data[i * n + j] = base_demand * (-beta * dist).exp();
        }
    }
    DenseOdMatrix::from_data(zone_ids.to_vec(), data)
}

fn print_path_memory(paths: &[OdPath], label: &str) {
    let struct_size = std::mem::size_of::<OdPath>();
    let total_links: usize = paths.iter().map(|p| p.link_ids.len()).sum();
    let avg_links = total_links as f64 / paths.len() as f64;

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

// Small grid (100 zones): quick criterion bench for relative comparison
fn bench_diagonalization_100z(c: &mut Criterion) {
    let (network, zone_ids) = generated_grid(20, 2);
    let graph = IndexedGraph::from_network(&network);

    println!(
        "diag 100z: {} zones, {} nodes, {} links",
        zone_ids.len(),
        network.nodes.len(),
        network.links.len()
    );

    let bpr = BprFunction::default();
    let bpr_car = BprFunction::new(0.15, 4.0);
    let bpr_truck = BprFunction::new(0.30, 4.0);

    let car_sym = UserClass::new("car", 1.0, 1.0);
    let truck_sym = UserClass::new("truck", 2.5, 2.5);
    let car_asym = UserClass::new("car", 1.0, 1.0);
    let truck_asym = UserClass::new("truck", 2.5, 1.0);

    let od_car = uniform_od(&zone_ids, 10.0);
    let od_truck = uniform_od(&zone_ids, 2.0);

    let config = AssignmentConfig {
        max_iterations: 20,
        convergence_gap: 1e-3,
        store_paths: false,
    };

    let mut group = c.benchmark_group("diag_100z");

    group.bench_function("beckmann_fw", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car_sym.clone(), truck_sym.clone()];
        b.iter(|| {
            assign_multiclass_fw(
                &graph, &classes, &od_refs, &bpr, &config, None,
            )
            .unwrap()
        });
    });

    group.bench_function("diag_shared_vdf", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car_sym.clone(), truck_sym.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr, &bpr];
        b.iter(|| {
            assign_diagonalization(
                &graph, &classes, &od_refs, &vdfs, &config, 10, 1e-3,
            )
            .unwrap()
        });
    });

    group.bench_function("diag_per_class_vdf", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car_asym.clone(), truck_asym.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];
        b.iter(|| {
            assign_diagonalization(
                &graph, &classes, &od_refs, &vdfs, &config, 10, 1e-3,
            )
            .unwrap()
        });
    });

    let config_paths = AssignmentConfig {
        max_iterations: 20,
        convergence_gap: 1e-3,
        store_paths: true,
    };

    group.bench_function("diag_per_class_vdf_paths", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car_asym.clone(), truck_asym.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];
        b.iter(|| {
            assign_diagonalization(
                &graph, &classes, &od_refs, &vdfs, &config_paths, 10, 1e-3,
            )
            .unwrap()
        });
    });

    // Memory stats
    let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
    let classes = vec![car_asym.clone(), truck_asym.clone()];
    let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];
    let result = assign_diagonalization(
        &graph, &classes, &od_refs, &vdfs, &config_paths, 10, 1e-3,
    )
    .unwrap();
    if let Some(ref paths) = result.path_flows {
        print_path_memory(paths, "diag-100z");
    }

    group.finish();
}

// Large grid (625 zones): gravity-model demand for realistic load
fn bench_diagonalization_625z(c: &mut Criterion) {
    let grid_side = 50;
    let zone_step = 2;
    let (network, zone_ids) = generated_grid(grid_side, zone_step);
    let graph = IndexedGraph::from_network(&network);

    let od_car = gravity_od(&zone_ids, grid_side, zone_step, 50.0, 0.15);
    let od_truck = gravity_od(&zone_ids, grid_side, zone_step, 10.0, 0.15);

    println!(
        "diag 625z: {} zones, {} nodes, {} links, car_demand={:.0}, truck_demand={:.0}",
        zone_ids.len(),
        network.nodes.len(),
        network.links.len(),
        od_car.total(),
        od_truck.total(),
    );

    let bpr = BprFunction::default();
    let bpr_car = BprFunction::new(0.15, 4.0);
    let bpr_truck = BprFunction::new(0.30, 4.0);

    let car_sym = UserClass::new("car", 1.0, 1.0);
    let truck_sym = UserClass::new("truck", 2.5, 2.5);
    let car_asym = UserClass::new("car", 1.0, 1.0);
    let truck_asym = UserClass::new("truck", 2.5, 1.0);

    let config = AssignmentConfig {
        max_iterations: 20,
        convergence_gap: 1e-3,
        store_paths: false,
    };

    let config_paths = AssignmentConfig {
        max_iterations: 20,
        convergence_gap: 1e-3,
        store_paths: true,
    };

    let mut group = c.benchmark_group("diag_625z");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    group.bench_function("beckmann_fw", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car_sym.clone(), truck_sym.clone()];
        b.iter(|| {
            assign_multiclass_fw(
                &graph, &classes, &od_refs, &bpr, &config, None,
            )
            .unwrap()
        });
    });

    group.bench_function("diag_shared_vdf", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car_sym.clone(), truck_sym.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr, &bpr];
        b.iter(|| {
            assign_diagonalization(
                &graph, &classes, &od_refs, &vdfs, &config, 10, 1e-3,
            )
            .unwrap()
        });
    });

    group.bench_function("diag_per_class_vdf", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car_asym.clone(), truck_asym.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];
        b.iter(|| {
            assign_diagonalization(
                &graph, &classes, &od_refs, &vdfs, &config, 10, 1e-3,
            )
            .unwrap()
        });
    });

    group.bench_function("diag_per_class_vdf_paths", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car_asym.clone(), truck_asym.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];
        b.iter(|| {
            assign_diagonalization(
                &graph, &classes, &od_refs, &vdfs, &config_paths, 10, 1e-3,
            )
            .unwrap()
        });
    });

    // Memory stats
    let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
    let classes = vec![car_asym.clone(), truck_asym.clone()];
    let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];
    let result = assign_diagonalization(
        &graph, &classes, &od_refs, &vdfs, &config_paths, 10, 1e-3,
    )
    .unwrap();
    if let Some(ref paths) = result.path_flows {
        let car_paths = paths.iter().filter(|p| p.class_index == Some(0)).count();
        let truck_paths = paths.iter().filter(|p| p.class_index == Some(1)).count();
        println!(
            "  [diag-625z] car={}, truck={}",
            car_paths, truck_paths
        );
        print_path_memory(paths, "diag-625z");
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_diagonalization_100z,
    bench_diagonalization_625z
);
criterion_main!(benches);
