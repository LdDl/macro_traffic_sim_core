//! Example: crowded (congested) transit assignment - feeling the moment a
//! line runs out of seats.
//!
//! Two parallel lines connect A to B with the *same* nominal frequency, so
//! the uncrowded optimal-strategies model always splits demand 50/50
//! between them. But the "Small" line runs little vehicles: once the flow
//! it attracts approaches its capacity, its effective frequency drops
//! (Cominetti & Correa 2001; De Cea & Fernandez 1993) and it starts to shed
//! riders onto the roomy "Big" line.
//!
//! Sweeping the demand upward shows the tipping point: while both lines
//! have spare seats the split stays 50/50, but the instant the Small line
//! fills, the extra passengers pile onto the Big line instead.
//!
//! ```text
//!            Big  (headway 10 min, 100 seats/veh -> 600 seats/hour)
//!          +----------------------------------------------------+
//!          |                                                    v
//!        [A]                                                  [B]
//!          |                                                    ^
//!          +----------------------------------------------------+
//!            Small(headway 10 min,  30 seats/veh -> 180 seats/hour)
//! ```
//!
//! Usage:
//!   cargo run --example transit_crowding

use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
use macro_traffic_sim_core::transit::{
    CrowdingParams, TransitAssignmentOptions, TransitNetwork, TransitRoute, assign_transit,
    assign_transit_crowded,
};

const STOP_A: i64 = 1;
const STOP_B: i64 = 2;
const HEADWAY: f64 = 10.0;
const ANALYSIS_PERIOD: f64 = 60.0;
const BIG_SEATS: f64 = 100.0;
const SMALL_SEATS: f64 = 30.0;

fn build_network() -> TransitNetwork {
    let mut network = TransitNetwork::new();
    network.add_route(
        TransitRoute::new("Big", vec![STOP_A, STOP_B], vec![10.0], HEADWAY)
            .with_capacity(BIG_SEATS),
    );
    network.add_route(
        TransitRoute::new("Small", vec![STOP_A, STOP_B], vec![10.0], HEADWAY)
            .with_capacity(SMALL_SEATS),
    );
    network
}

fn line_capacity(seats: f64) -> f64 {
    (ANALYSIS_PERIOD / HEADWAY) * seats
}

fn main() {
    let network = build_network();
    let big_cap = line_capacity(BIG_SEATS);
    let small_cap = line_capacity(SMALL_SEATS);

    println!("Two parallel lines A -> B, identical {HEADWAY}-minute headway:");
    println!("  Big:   {BIG_SEATS:.0} seats/vehicle -> {big_cap:.0} passengers/hour capacity");
    println!("  Small: {SMALL_SEATS:.0} seats/vehicle -> {small_cap:.0} passengers/hour capacity");
    println!("\nUncrowded, the two equal frequencies always split demand 50/50.");
    println!("Crowded, the Small line stops growing once it fills.\n");

    let crowding = CrowdingParams::new(ANALYSIS_PERIOD);
    let options = TransitAssignmentOptions::default();

    println!(
        "{:>8} | {:>16} | {:>26} | {:>10}",
        "demand", "uncrowded (B/S)", "crowded (Big / Small)", "Small load"
    );
    println!("{}", "-".repeat(72));

    for demand in [100.0, 200.0, 300.0, 360.0, 500.0, 700.0, 900.0] {
        let mut od = DenseOdMatrix::new(vec![STOP_A, STOP_B]);
        od.set(STOP_A, STOP_B, demand);

        let plain = assign_transit(&network, &od).expect("assignment failed");
        let crowded =
            assign_transit_crowded(&network, &od, &options, &crowding).expect("assignment failed");

        let plain_small = plain.route_boardings["Small"];
        let big = crowded.route_boardings["Big"];
        let small = crowded.route_boardings["Small"];
        let load_pct = 100.0 * small / small_cap;

        println!(
            "{:>8.0} | {:>7.0} / {:>6.0} | {:>11.0} / {:>10.0} | {:>8.0}%",
            demand,
            demand - plain_small,
            plain_small,
            big,
            small,
            load_pct
        );
    }

    println!(
        "\nBelow ~{:.0} passengers/hour (2 x Small capacity) both lines have",
        2.0 * small_cap
    );
    println!("spare seats and the split stays even. Above it the Small line's growth");
    println!("flattens out near its {small_cap:.0}-passenger capacity and the Big line takes");
    println!("most of the extra demand. The effective-frequency law is a soft BPR-like");
    println!("penalty, not a hard cutoff, so the Small line still creeps over capacity");
    println!("under very heavy demand rather than refusing passengers outright.");
}
