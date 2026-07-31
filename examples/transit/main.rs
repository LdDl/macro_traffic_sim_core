//! Example: frequency-based transit assignment with optimal strategies
//! (Spiess & Florian, 1989).
//!
//! Reproduces the example network from the paper (pages 96-97):
//! four stops A, X, Y, B and four transit lines. One unit of demand
//! travels from A to B; the optimal strategy splits it across lines
//! according to their frequencies.
//!
//! The expanded route graph, redrawn after Fig. 7 of the paper
//! (labels are `(travel time, frequency)`; `[..]` = stop, `(..)` =
//! route node, i.e. a line platform; `inf` frequency = no waiting):
//!
//! ```text
//!    +--------------------- (25, 1/6) ---------------------+
//!    |                                                     v
//!  [A] --(7, 1/6)--> (X2) ------(6, inf)------> [Y] --(10, 1/3)--> [B]
//!                     | ^                        | ^                 ^
//!            (0, inf) | | (0, 1/6)    (0, 1/15)  | | (0, inf)        |
//!                     v |                        v |                 |
//!                    [X] -------(4, 1/15)------> (Y3) ---(4, inf)----+
//! ```
//!
//! In line terms: Line 1 runs A-B directly, Line 2 runs A-X-Y, Line 3
//! runs X-Y-B, Line 4 runs Y-B. X2 is the Line 2 platform at X, Y3 the
//! Line 3 platform at Y - the paper built these route nodes by hand;
//! `expand_route_graph` derives them automatically from the four
//! `TransitRoute` definitions below.
//!
//! Usage:
//!   cargo run --example transit

use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
use macro_traffic_sim_core::transit::{
    TransitLinkKind, TransitNetwork, TransitRoute, assign_transit,
};

const STOP_NAMES: [(i64, &str); 4] = [(1, "A"), (2, "X"), (3, "Y"), (4, "B")];

fn stop_label(stop: i64) -> &'static str {
    STOP_NAMES
        .iter()
        .find(|(id, _)| *id == stop)
        .map(|(_, name)| *name)
        .unwrap_or("?")
}

fn main() {
    // Stops: A=1, X=2, Y=3, B=4 (times in minutes)
    let mut network = TransitNetwork::new();
    network.add_route(TransitRoute::new("Line 1", vec![1, 4], vec![25.0], 6.0));
    network.add_route(TransitRoute::new(
        "Line 2",
        vec![1, 2, 3],
        vec![7.0, 6.0],
        6.0,
    ));
    network.add_route(TransitRoute::new(
        "Line 3",
        vec![2, 3, 4],
        vec![4.0, 4.0],
        15.0,
    ));
    network.add_route(TransitRoute::new("Line 4", vec![3, 4], vec![10.0], 3.0));

    println!("Transit network: {} routes", network.routes.len());
    for route in &network.routes {
        let stops: Vec<&str> = route.stops.iter().map(|&s| stop_label(s)).collect();
        println!(
            "  {}: {} (headway {} min)",
            route.id,
            stops.join(" -> "),
            route.headway
        );
    }

    let mut od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
    od.set(1, 4, 1.0);
    println!("\nDemand: {} trip(s), A -> B", od.total());

    let result = assign_transit(&network, &od).expect("transit assignment failed");

    println!("\nExpected travel times (waiting + in-vehicle):");
    let mut od_pairs: Vec<(&(i64, i64), &f64)> = result.od_costs.iter().collect();
    od_pairs.sort_by_key(|((o, d), _)| (*o, *d));
    for ((origin, destination), cost) in od_pairs {
        println!(
            "  {} -> {}: {:.2} min",
            stop_label(*origin),
            stop_label(*destination),
            cost
        );
    }

    println!("\nRiding volumes per segment:");
    for lv in &result.link_volumes {
        if lv.kind == TransitLinkKind::Riding && lv.volume > 0.0 {
            println!(
                "  {}: {} -> {}: {:.4}",
                lv.route_id.as_deref().unwrap_or("-"),
                stop_label(lv.from_stop),
                stop_label(lv.to_stop),
                lv.volume
            );
        }
    }

    println!("\nBoardings per route:");
    let mut boardings: Vec<(&String, &f64)> = result.route_boardings.iter().collect();
    boardings.sort_by(|a, b| a.0.cmp(b.0));
    for (route_id, volume) in boardings {
        println!("  {}: {:.4}", route_id, volume);
    }
}
