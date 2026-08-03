//! Example: congested (strict-capacity) transit assignment, reproducing the
//! worked example of Cepeda, Cominetti and Florian (2006), Section 4.1.1.
//!
//! A tiny network with an express and a local line between A and C shows the
//! strict-capacity effect: as the express fills, its effective frequency
//! drops, its waiting time rises, and passengers spill onto the slower local
//! line. The method of successive averages drives the Cepeda-Cominetti-Florian
//! gap function to zero at the equilibrium.
//!
//! ```text
//!                   express (A-C, 16 bus/h, 320 pax/h)
//!          +----------------------------------------------+
//!          |                                              v
//!        [A] --- local-AB ---> [B] --- local-BC ---> [C]
//!                    (local: A-B-C, 6 bus/h, 120 pax/h)
//! ```
//!
//! The A-to-C demand can use the express (E), the local (L), or board the
//! first of the two (EL). At low demand everyone takes the faster express; as
//! demand grows the express saturates and the combined strategy EL takes over.
//!
//! Paper equilibria (beta = 0.2):
//!   - demand 100: express 84.3, A-to-C time 40.02 min
//!   - demand 350: express 260.5, local 99.5, A-to-C time 97.36 min
//!
//! Usage:
//!   cargo run --example transit_congested

use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
use macro_traffic_sim_core::transit::{
    CongestedParams, TransitAssignmentOptions, TransitNetwork, TransitRoute,
    assign_transit_congested,
};

const A: i64 = 1;
const B: i64 = 2;
const C: i64 = 3;
const ANALYSIS_PERIOD: f64 = 60.0;

fn build_network() -> TransitNetwork {
    let mut network = TransitNetwork::new();
    // Express A-C: 16 bus/h (headway 3.75 min), 20 pax/bus -> 320 pax/h.
    // In-vehicle 24 min + 0.01 dwell = 24.01, as in the paper.
    network.add_route(
        TransitRoute::new("Express", vec![A, C], vec![24.01], 60.0 / 16.0).with_capacity(20.0),
    );
    // Local A-B-C: 6 bus/h (headway 10 min), 20 pax/bus -> 120 pax/h.
    network.add_route(
        TransitRoute::new("Local", vec![A, B, C], vec![20.01, 20.01], 10.0).with_capacity(20.0),
    );
    network
}

fn run_case(demand_ac: f64, paper_express: f64, paper_time: f64) {
    let network = build_network();
    let mut od = DenseOdMatrix::new(vec![A, B, C]);
    od.set(A, B, 10.0);
    od.set(B, C, 10.0);
    od.set(A, C, demand_ac);

    let result = assign_transit_congested(
        &network,
        &od,
        &TransitAssignmentOptions::default(),
        &CongestedParams::new(ANALYSIS_PERIOD).with_limits(20000, 1e-8),
    )
    .expect("congested assignment failed");
    let a = &result.assignment;

    println!("Demand A -> C: {demand_ac:.0} passengers/hour");
    println!(
        "  express boardings: {:>7.2}   (paper {:.1})",
        a.route_boardings["Express"], paper_express
    );
    println!("  local boardings:   {:>7.2}", a.route_boardings["Local"]);
    println!(
        "  A -> C time:       {:>7.2}   (paper {:.2})",
        a.od_costs[&(A, C)],
        paper_time
    );
    println!(
        "  relative gap:      {:>7.1e}   ({} MSA iterations)",
        result.relative_gap, result.iterations
    );
}

fn main() {
    println!("Cepeda, Cominetti and Florian (2006) example, Section 4.1.1");
    println!("Express A-C: 16 bus/h, 320 pax/h capacity; in-vehicle 24 min");
    println!("Local A-B-C: 6 bus/h, 120 pax/h capacity; in-vehicle 20+20 min\n");

    // Low demand: the express has spare capacity but its rising wait makes the
    // local competitive, splitting the A-to-C demand.
    run_case(100.0, 84.3, 40.02);
    println!();
    // High demand: all-or-nothing 350 on the express would exceed its 320
    // capacity, so the express saturates and the local carries the overflow.
    run_case(350.0, 260.5, 97.36);

    println!("\nThe express settles within its 320 pax/hour capacity; the strict-");
    println!("capacity effective frequency prevents it from being overloaded, and");
    println!("the gap function certifies the distance to equilibrium.");
}
