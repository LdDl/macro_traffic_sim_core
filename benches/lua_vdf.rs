use criterion::{Criterion, criterion_group, criterion_main};

use macro_traffic_sim_core::assignment::diagonalization::assign_diagonalization;
use macro_traffic_sim_core::assignment::lua_vdf::LuaVdf;
use macro_traffic_sim_core::assignment::multiclass::UserClass;
use macro_traffic_sim_core::assignment::{
    AssignmentConfig, BprFunction, IndexedGraph, VolumeDelayFunction,
};
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::od::dense::DenseOdMatrix;
use macro_traffic_sim_core::od::OdMatrix;
use macro_traffic_sim_core::pipeline::haversine_km;

fn lua_bpr_script(alpha: f64, beta: f64) -> String {
    format!(
        r#"
local alpha = {alpha}
local beta = {beta}

function travel_time(ff, vol, cap)
    if cap <= 0 then return math.huge end
    return ff * (1.0 + alpha * (vol / cap) ^ beta)
end

function integral(ff, vol, cap)
    if cap <= 0 then return math.huge end
    if vol <= 0 then return 0.0 end
    local ratio = vol / cap
    return ff * (vol + alpha * cap * ratio ^ (beta + 1.0) / (beta + 1.0))
end
"#
    )
}

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

fn bench_vdf_micro(c: &mut Criterion) {
    let native = BprFunction::default();
    let lua_vdf = LuaVdf::new(&lua_bpr_script(0.15, 4.0)).unwrap();

    for vol in [0.0, 500.0, 1000.0, 2000.0] {
        let n = native.travel_time(10.0, vol, 1000.0);
        let l = lua_vdf.travel_time(10.0, vol, 1000.0);
        assert!(
            (n - l).abs() < 1e-10,
            "travel_time mismatch at vol={}: native={}, lua={}",
            vol,
            n,
            l
        );
    }
    println!("Lua VDF correctness verified");

    let mut group = c.benchmark_group("vdf_micro");

    group.bench_function("native_1000_calls", |b| {
        b.iter(|| {
            let mut sum = 0.0;
            for i in 0..1000 {
                sum += native.travel_time(10.0, i as f64 * 2.0, 1000.0);
            }
            sum
        });
    });

    group.bench_function("lua_1000_calls", |b| {
        b.iter(|| {
            let mut sum = 0.0;
            for i in 0..1000 {
                sum += lua_vdf.travel_time(10.0, i as f64 * 2.0, 1000.0);
            }
            sum
        });
    });

    group.finish();
}

fn bench_diag_lua(c: &mut Criterion) {
    let (network, zone_ids) = generated_grid(20, 2);
    let graph = IndexedGraph::from_network(&network);

    println!(
        "lua_vdf 100z: {} zones, {} nodes, {} links",
        zone_ids.len(),
        network.nodes.len(),
        network.links.len()
    );

    let bpr_car = BprFunction::new(0.15, 4.0);
    let bpr_truck = BprFunction::new(0.30, 4.0);
    let lua_car = LuaVdf::new(&lua_bpr_script(0.15, 4.0)).unwrap();
    let lua_truck = LuaVdf::new(&lua_bpr_script(0.30, 4.0)).unwrap();

    let car = UserClass::new("car", 1.0, 1.0);
    let truck = UserClass::new("truck", 2.5, 1.0);

    let od_car = uniform_od(&zone_ids, 10.0);
    let od_truck = uniform_od(&zone_ids, 2.0);

    let config = AssignmentConfig {
        max_iterations: 20,
        convergence_gap: 1e-3,
        store_paths: false,
    };

    let mut group = c.benchmark_group("diag_lua_100z");

    group.bench_function("native_both", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car.clone(), truck.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];
        b.iter(|| {
            assign_diagonalization(&graph, &classes, &od_refs, &vdfs, &config, 10, 1e-3).unwrap()
        });
    });

    group.bench_function("lua_truck_only", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car.clone(), truck.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &lua_truck];
        b.iter(|| {
            assign_diagonalization(&graph, &classes, &od_refs, &vdfs, &config, 10, 1e-3).unwrap()
        });
    });

    group.bench_function("lua_both", |b| {
        let od_refs: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];
        let classes = vec![car.clone(), truck.clone()];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&lua_car, &lua_truck];
        b.iter(|| {
            assign_diagonalization(&graph, &classes, &od_refs, &vdfs, &config, 10, 1e-3).unwrap()
        });
    });

    group.finish();
}

criterion_group!(benches, bench_vdf_micro, bench_diag_lua);
criterion_main!(benches);
