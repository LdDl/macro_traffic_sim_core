//! Example: diagonalization multi-class assignment with per-class VDFs.
//!
//! Same 4-zone diamond network as `multiclass_network`, but each class
//! gets its own volume-delay function. Cars use standard BPR(0.15, 4.0),
//! trucks use a steeper BPR(0.30, 4.0) - more sensitive to congestion.
//!
//! Because VDFs differ per class, the Beckmann symmetry condition does
//! not apply and shortest path trees may differ between classes.
//! Diagonalization (Dafermos 1982) is required.
//!
//! Demonstrates:
//! 1. Per-class VDFs with different parameters
//! 2. Per-class path extraction (paths may differ across classes)
//! 3. Cost comparison between classes on the same links
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
//! Usage:
//!   cargo run --example diagonalization

use std::collections::HashMap;

use macro_traffic_sim_core::assignment::diagonalization::assign_diagonalization;
use macro_traffic_sim_core::assignment::multiclass::UserClass;
use macro_traffic_sim_core::assignment::{AssignmentConfig, BprFunction, IndexedGraph, VolumeDelayFunction};
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::od::dense::DenseOdMatrix;
use macro_traffic_sim_core::od::OdMatrix;
use macro_traffic_sim_core::pipeline::haversine_km;

fn main() {
    let network = build_network();
    let graph = IndexedGraph::from_network(&network);

    println!(
        "Network: {} nodes, {} links",
        network.nodes.len(),
        network.links.len()
    );

    // Two classes with DIFFERENT VDFs
    let car = UserClass::new("car", 1.0, 1.0);
    let truck = UserClass::new("truck", 2.5, 1.0);
    let classes = vec![car, truck];

    // Car: standard BPR
    let bpr_car = BprFunction::new(0.15, 4.0);
    // Truck: steeper BPR (more sensitive to congestion)
    let bpr_truck = BprFunction::new(0.30, 4.0);
    let class_vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];

    let class_names = ["car", "truck"];

    // OD matrices: same diamond network demand
    // Zone 1 (residential, high production) -> Zones 2,3,4
    // Zone 4 (industrial, moderate attraction) <- Zones 1,2,3
    let zones = vec![1_i64, 2, 3, 4];
    let od_car = DenseOdMatrix::from_data(
        zones.clone(),
        vec![
            0.0, 2000.0, 3000.0, 4000.0,
            1500.0, 0.0, 1000.0, 2500.0,
            2000.0, 1500.0, 0.0, 3500.0,
            1000.0, 2000.0, 2500.0, 0.0,
        ],
    );
    let od_truck = DenseOdMatrix::from_data(
        zones.clone(),
        vec![
            0.0, 200.0, 300.0, 500.0,
            150.0, 0.0, 100.0, 300.0,
            200.0, 150.0, 0.0, 400.0,
            100.0, 250.0, 300.0, 0.0,
        ],
    );
    let od_matrices: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];

    println!("Car demand: {:.0} trips", od_car.total());
    println!("Truck demand: {:.0} trips", od_truck.total());

    let config = AssignmentConfig {
        max_iterations: 50,
        convergence_gap: 1e-4,
        store_paths: true,
    };

    let result = assign_diagonalization(
        &graph,
        &classes,
        &od_matrices,
        &class_vdfs,
        &config,
        20,
        1e-4,
    )
    .expect("diagonalization failed");

    println!("\n--- Assignment result ---");
    println!(
        "Iterations: {}, gap: {:.6}, converged: {}",
        result.iterations, result.relative_gap, result.converged
    );

    // Per-class volumes
    if let Some(ref cv) = result.class_volumes {
        for (name, vols) in cv {
            let total: f64 = vols.values().sum();
            println!("{}: total vehicle-trips on network = {:.1}", name, total);
        }
    }

    let pcu_total: f64 = result.link_volumes.values().sum();
    println!("PCU total on network: {:.1}", pcu_total);

    // Top loaded links
    println!("\n--- Top 10 links by PCU volume ---");
    let mut volumes: Vec<(i64, f64)> = result
        .link_volumes
        .iter()
        .map(|(&id, &vol)| (id, vol))
        .collect();
    volumes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (id, vol) in volumes.iter().take(10) {
        let cost = result.link_costs.get(id).copied().unwrap_or(0.0);
        println!("  link {:>4}: volume={:.1} PCU, cost={:.4} h", id, vol, cost);
    }

    // Per-class cost comparison on the same link
    println!("\n--- Per-class cost comparison ---");
    println!("Car VDF:   BPR(alpha=0.15, beta=4.0)");
    println!("Truck VDF: BPR(alpha=0.30, beta=4.0)");
    if let Some(&(top_link, top_vol)) = volumes.first() {
        let ff_time = 0.014; // approximate for ~840m at 60 km/h
        let cap = 3600.0; // 2 lanes * 1800
        let car_cost = bpr_car.travel_time(ff_time, top_vol, cap);
        let truck_cost = bpr_truck.travel_time(ff_time, top_vol, cap);
        println!(
            "Link {}: car_cost={:.6} h, truck_cost={:.6} h, ratio={:.2}",
            top_link,
            car_cost,
            truck_cost,
            truck_cost / car_cost
        );
    }

    // Path analysis
    let paths = match result.path_flows.as_ref() {
        Some(p) => p,
        None => {
            println!("No paths (store_paths not enabled)");
            return;
        }
    };

    println!("\n--- Paths: {} total ---", paths.len());

    // Count paths per class
    for (ci, name) in class_names.iter().enumerate() {
        let count = paths.iter().filter(|p| p.class_index == Some(ci as u16)).count();
        println!("  {}: {} paths", name, count);
    }

    // OD pair query: Zone 1 -> Zone 4, per class
    let origin = 1;
    let dest = 4;
    println!("\n--- OD pair: Zone {} -> Zone {} ---", origin, dest);

    for (ci, name) in class_names.iter().enumerate() {
        let class_paths: Vec<_> = paths
            .iter()
            .filter(|p| {
                p.origin_zone == origin
                    && p.dest_zone == dest
                    && p.class_index == Some(ci as u16)
            })
            .collect();

        for p in &class_paths {
            println!(
                "  {}: flow={:.1}, cost={:.4} h, links={:?}",
                name, p.flow, p.cost, p.link_ids
            );
        }
    }

    // Check if paths differ between classes
    let car_links: Vec<_> = paths
        .iter()
        .filter(|p| {
            p.origin_zone == origin
                && p.dest_zone == dest
                && p.class_index == Some(0)
        })
        .map(|p| &p.link_ids)
        .collect();
    let truck_links: Vec<_> = paths
        .iter()
        .filter(|p| {
            p.origin_zone == origin
                && p.dest_zone == dest
                && p.class_index == Some(1)
        })
        .map(|p| &p.link_ids)
        .collect();

    if !car_links.is_empty() && !truck_links.is_empty() {
        if car_links[0] == truck_links[0] {
            println!("  -> Same route for both classes (VDF difference not large enough)");
        } else {
            println!("  -> Different routes! Car: {:?}, Truck: {:?}", car_links[0], truck_links[0]);
        }
    }

    // Select link analysis: which OD pairs use a specific link?
    let target_link: i64 = 102;
    println!("\n--- Select link analysis: link {} ---", target_link);

    let mut od_totals: HashMap<(i64, i64, Option<u16>), f64> = HashMap::new();
    for p in paths {
        *od_totals
            .entry((p.origin_zone, p.dest_zone, p.class_index))
            .or_insert(0.0) += p.flow;
    }

    let mut select_link: Vec<((i64, i64, Option<u16>), f64)> = Vec::new();
    for p in paths {
        if p.link_ids.contains(&target_link) {
            select_link.push(((p.origin_zone, p.dest_zone, p.class_index), p.flow));
        }
    }
    select_link.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    let mut total = 0.0;
    for ((o, d, ci), flow) in &select_link {
        let name = match ci {
            Some(i) => class_names[*i as usize],
            None => "all",
        };
        println!(
            "  {} -> {} [{}]: flow={:.1}",
            o, d, name, flow
        );
        total += flow;
    }
    println!("  Total through link {}: {:.1}", target_link, total);
}

fn build_network() -> Network {
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
        let (_, lat1, lon1) = coords.iter().find(|&&(id, _, _)| id == a).unwrap();
        let (_, lat2, lon2) = coords.iter().find(|&&(id, _, _)| id == b).unwrap();
        let dist = haversine_km(*lat1, *lon1, *lat2, *lon2) * 1000.0;

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
