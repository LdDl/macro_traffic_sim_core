//! Example: crowding on de Cea and Fernandez's (1993) "Modified Network G"
//! (their Figure 1), as a paper-grounded debugging scaffold.
//!
//! This reproduces the *topology* of the network de Cea use for their worked
//! example: nodes N1..N4 and six lines
//!
//! ```text
//!            L1 (N1-N2-N3)
//!        +-------------------------------+
//!        |         L2          L5, L6    |
//!       N1 ---------------> N2 --------> N3
//!        \                 ^
//!      L3 \               / L4
//!          \             /
//!           +--> N4 ----+
//! ```
//!
//! IMPORTANT - what this does and does NOT reproduce:
//!
//! - de Cea's Table 1 gives ROUTE-SECTION aggregates (S1..S6), not per-line
//!   data, and two of them are frequency-weighted "expected" times, so the
//!   individual line frequencies and times cannot be recovered uniquely. The
//!   per-line values below are therefore ILLUSTRATIVE (magnitudes taken from
//!   Table 1), not the paper's exact inputs. See papers/decea1993_transcription.tex.
//! - Our tool computes the Spiess-Florian optimal-strategies solution (plus
//!   the de Cea / Cominetti-Correa effective-frequency crowding). de Cea note
//!   that Spiess "spreads the flows over more paths, using line L1 [...] due
//!   to the concept of optimum strategy", so on this network we match their
//!   Figure 7 (Spiess), NOT their route-section Figure 6. Do not expect the
//!   Fig. 6 numbers.
//! - Crowding here uses each line's own load (v1: the cross-section coupling
//!   V_hat of their Eq. 7 is dropped).
//!
//! The point of the example is to watch, on the paper's own network, (a) the
//! optimal strategy spread demand over several lines and (b) crowding push
//! flow off a capacity-limited line.
//!
//! Usage:
//!   cargo run --example transit_decea

use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
use macro_traffic_sim_core::transit::{
    CrowdingParams, TransitAssignmentOptions, TransitNetwork, TransitRoute, assign_transit,
    assign_transit_crowded,
};

const N1: i64 = 1;
const N2: i64 = 2;
const N3: i64 = 3;
const N4: i64 = 4;
const ANALYSIS_PERIOD: f64 = 60.0;
const DEMAND: f64 = 600.0;

fn build_network() -> TransitNetwork {
    let mut net = TransitNetwork::new();
    // Illustrative per-line data (see the module note): segment in-vehicle
    // times ~ Table 1 section times, headways ~ the section waits, and a
    // small capacity on the through line L1 so it crowds.
    net.add_route(
        TransitRoute::new("L1", vec![N1, N2, N3], vec![7.0, 13.0], 6.0).with_capacity(30.0),
    );
    net.add_route(TransitRoute::new("L2", vec![N1, N2], vec![7.0], 6.0).with_capacity(60.0));
    net.add_route(TransitRoute::new("L3", vec![N1, N4], vec![5.0], 6.0).with_capacity(40.0));
    net.add_route(TransitRoute::new("L4", vec![N4, N2], vec![9.0], 6.0).with_capacity(40.0));
    net.add_route(TransitRoute::new("L5", vec![N2, N3], vec![13.0], 6.0).with_capacity(60.0));
    net.add_route(TransitRoute::new("L6", vec![N2, N3], vec![8.0], 15.0).with_capacity(40.0));
    net
}

fn print_boardings(title: &str, boardings: &std::collections::HashMap<String, f64>) {
    println!("{title}");
    let mut rows: Vec<(&String, &f64)> = boardings.iter().collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    for (line, vol) in rows {
        println!("  {line}: {vol:.1}");
    }
}

fn main() {
    let network = build_network();
    let mut od = DenseOdMatrix::new(vec![N1, N2, N3, N4]);
    od.set(N1, N3, DEMAND);

    println!("de Cea and Fernandez (1993) Modified Network G, O-D N1 -> N3");
    println!("Demand: {DEMAND:.0} passengers/hour (illustrative per-line data)\n");

    let options = TransitAssignmentOptions::default();

    // Uncongested optimal strategies: this is the Spiess-Florian solution
    // (the paper's Fig. 7), spreading demand over several lines.
    let plain = assign_transit(&network, &od).expect("assignment failed");
    print_boardings(
        "Uncongested optimal strategies (Spiess, = paper Fig. 7):",
        &plain.route_boardings,
    );
    println!(
        "  expected N1 -> N3 travel time: {:.2} min",
        plain.od_costs[&(N1, N3)]
    );

    // Crowded: L1 (capacity 30/veh -> line capacity (60/6)*30 = 300/hour)
    // fills and its effective frequency drops, so riders shift.
    let crowding = CrowdingParams::new(ANALYSIS_PERIOD);
    let crowded =
        assign_transit_crowded(&network, &od, &options, &crowding).expect("assignment failed");
    println!();
    print_boardings(
        "With crowding (de Cea Eq. 16 effective frequency):",
        &crowded.route_boardings,
    );
    println!(
        "  expected N1 -> N3 travel time: {:.2} min",
        crowded.od_costs[&(N1, N3)]
    );

    let l1_line_cap = (ANALYSIS_PERIOD / 6.0) * 30.0;
    println!(
        "\nL1 line capacity = (60/6) * 30 = {l1_line_cap:.0}/hour. Uncrowded it carries"
    );
    println!(
        "{:.0}; crowding drops it to {:.0}, and the freed demand moves onto the",
        plain.route_boardings["L1"], crowded.route_boardings["L1"]
    );
    println!("N1 -> N2 -> N3 lines (L2 then L5/L6). This is the Spiess solution under");
    println!("de Cea's effective frequency, not de Cea's route-section Fig. 6.");
}
