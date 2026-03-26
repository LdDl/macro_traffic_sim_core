use std::collections::HashMap;

use criterion::{Criterion, criterion_group, criterion_main};

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

// simple_network setup (4 zones, diamond)
fn simple_network() -> Network {
    let mut net = Network::new();

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

    let edges = [(1, 2), (1, 3), (2, 4), (3, 4)];
    let mut link_id: i64 = 100;
    let mut road_links: HashMap<(i64, i64), i64> = HashMap::new();

    for &(a, b) in &edges {
        let (_, lat1, lon1) = coords.iter().find(|&&(id, _, _)| id == a).copied().unwrap();
        let (_, lat2, lon2) = coords.iter().find(|&&(id, _, _)| id == b).copied().unwrap();
        let dist = haversine_km(lat1, lon1, lat2, lon2) * 1000.0;

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

        for &(in_from, _) in &incoming {
            for &(out_to, _) in &outgoing {
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

fn simple_zones() -> Vec<Zone> {
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

// grid_city setup (6 zones, 3x3 grid with mesoscopic splitting)
fn grid_network() -> Network {
    use std::collections::{BTreeSet, HashSet};

    let mut net = Network::new();
    let mut link_id: i64 = 1000;

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

    let centroids: [(i64, i64, f64, f64); 6] = [
        (101, 1, 55.764, 37.593),
        (102, 2, 55.764, 37.633),
        (103, 3, 55.736, 37.613),
        (104, 4, 55.746, 37.593),
        (105, 5, 55.746, 37.633),
        (106, 6, 55.764, 37.613),
    ];

    let coords: HashMap<i64, (f64, f64)> = grid
        .iter()
        .map(|&(id, lat, lon)| (id, (lat, lon)))
        .chain(centroids.iter().map(|&(id, _, lat, lon)| (id, (lat, lon))))
        .collect();

    let mut roads: Vec<(i64, i64, i32, f64, f64)> = Vec::new();
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
    roads.push((4, 5, 2, 50.0, 1500.0));
    roads.push((5, 6, 2, 50.0, 1500.0));
    roads.push((9, 8, 1, 40.0, 1200.0));
    roads.push((8, 7, 1, 40.0, 1200.0));

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

    let banned: HashSet<(i64, i64, i64)> = HashSet::from([(8, 5, 6), (3, 2, 5)]);
    let intersections: HashSet<i64> = HashSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    let in_sub = |n: i64, from: i64| -> i64 { n * 1000 + from };
    let out_sub = |n: i64, to: i64| -> i64 { n * 1000 + 500 + to };

    for &(n, lat, lon) in &grid {
        let incoming: BTreeSet<i64> = roads
            .iter()
            .filter(|&&(_, to, ..)| to == n)
            .map(|&(from, ..)| from)
            .collect();
        let outgoing: BTreeSet<i64> = roads
            .iter()
            .filter(|&&(from, ..)| from == n)
            .map(|&(_, to, ..)| to)
            .collect();

        for &from in &incoming {
            net.add_node(Node::new(in_sub(n, from)).with_coordinates(lat, lon).build())
                .unwrap();
        }
        for &to in &outgoing {
            net.add_node(
                Node::new(out_sub(n, to))
                    .with_coordinates(lat, lon)
                    .build(),
            )
            .unwrap();
        }

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
                link_id += 1;
            }
        }
    }

    for &(node_id, zone_id, lat, lon) in &centroids {
        net.add_node(
            Node::new(node_id)
                .with_zone_id(zone_id)
                .with_coordinates(lat, lon)
                .build(),
        )
        .unwrap();
    }

    for &(from, to, lanes, speed, cap) in &roads {
        let source = if intersections.contains(&from) {
            out_sub(from, to)
        } else {
            from
        };
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
        link_id += 1;
    }

    net
}

fn grid_zones() -> Vec<Zone> {
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

// Benchmarks
fn bench_simple_network(c: &mut Criterion) {
    let network = simple_network();
    let zones = simple_zones();
    let trip_gen = RegressionGenerator::new();
    let impedance = ExponentialImpedance::new(0.1);
    let logit = MultinomialLogit::default_auto_bike_walk();
    let config = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(50)
        .with_convergence_gap(1e-4)
        .with_feedback_iterations(3)
        .with_verbose_level(VerboseLevel::None)
        .build();

    c.bench_function("simple_network_pipeline", |b| {
        b.iter(|| {
            run_four_step_model(&network, &zones, &trip_gen, &impedance, &logit, &config)
                .unwrap()
        });
    });
}

fn bench_grid_city(c: &mut Criterion) {
    let network = grid_network();
    let zones = grid_zones();
    let trip_gen = RegressionGenerator::new();
    let impedance = ExponentialImpedance::new(0.15);
    let logit = MultinomialLogit::default_auto_bike_walk();
    let config = ModelConfig::new()
        .with_assignment_method(AssignmentMethodType::FrankWolfe)
        .with_max_iterations(200)
        .with_convergence_gap(1e-3)
        .with_feedback_iterations(3)
        .with_verbose_level(VerboseLevel::None)
        .build();

    c.bench_function("grid_city_pipeline", |b| {
        b.iter(|| {
            run_four_step_model(&network, &zones, &trip_gen, &impedance, &logit, &config)
                .unwrap()
        });
    });
}

criterion_group!(benches, bench_simple_network, bench_grid_city);
criterion_main!(benches);
