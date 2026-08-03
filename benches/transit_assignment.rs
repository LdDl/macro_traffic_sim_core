use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
use macro_traffic_sim_core::transit::{TransitNetwork, TransitRoute, assign_transit};

fn build_grid(rows: i64, cols: i64) -> (TransitNetwork, DenseOdMatrix) {
    let mut network = TransitNetwork::new();
    let stop = |r: i64, c: i64| r * cols + c + 1;

    // A route per row in each direction; shared grid stops create transfers.
    for r in 0..rows {
        let mut stops: Vec<i64> = (0..cols).map(|c| stop(r, c)).collect();
        let times = vec![3.0; (cols - 1) as usize];
        network.add_route(TransitRoute::new(&format!("R{}e", r), stops.clone(), times.clone(), 6.0));
        stops.reverse();
        network.add_route(TransitRoute::new(&format!("R{}w", r), stops, times, 6.0));
    }
    // A route per column in each direction.
    for c in 0..cols {
        let mut stops: Vec<i64> = (0..rows).map(|r| stop(r, c)).collect();
        let times = vec![3.0; (rows - 1) as usize];
        network.add_route(TransitRoute::new(&format!("C{}s", c), stops.clone(), times.clone(), 6.0));
        stops.reverse();
        network.add_route(TransitRoute::new(&format!("C{}n", c), stops, times, 6.0));
    }

    let zones: Vec<i64> = (1..=rows * cols).collect();
    let mut od = DenseOdMatrix::new(zones.clone());
    for &o in &zones {
        for &d in &zones {
            if o != d {
                od.set(o, d, 1.0);
            }
        }
    }
    (network, od)
}

fn bench_assign_transit(c: &mut Criterion) {
    let mut group = c.benchmark_group("assign_transit");
    for &(rows, cols) in &[(8i64, 8i64), (12, 12)] {
        let (network, od) = build_grid(rows, cols);
        let id = BenchmarkId::from_parameter(format!("{}x{}", rows, cols));
        group.bench_function(id, |b| {
            b.iter(|| assign_transit(&network, &od).unwrap());
        });
    }
    group.finish();
}

criterion_group!(benches, bench_assign_transit);
criterion_main!(benches);
