//! # Diagonalization (Relaxation) Algorithm
//!
//! Multi-class traffic assignment with per-class volume-delay
//! functions using the Gauss-Seidel relaxation method.
//! See the ref: https://en.wikipedia.org/wiki/Gauss%E2%80%93Seidel_method
//!
//! Unlike the Beckmann-based multi-class FW/MSA (which requires
//! `ff_time_multiplier / pcu = const`), diagonalization supports
//! per-class VDFs where each class may perceive link costs
//! differently. This covers:
//!
//! - Class-specific volume-delay functions (trucks slower on grades)
//! - Per-link-per-class cost multipliers
//! - Link restrictions by class (modeled as high-cost VDF)
//!
//! ## Algorithm
//!
//! 1. Initialize: cold start (free-flow AoN) for each class.
//! 2. Outer loop (diagonalization):
//!    For each class i in Gauss-Seidel order:
//!    a. Compute background PCU from all other classes.
//!       Classes 1..i-1 use already-updated flows from this
//!       outer iteration, classes i+1..J use flows from the
//!       previous outer iteration (eq 3.11).
//!    b. Solve a single-class Frank-Wolfe where the VDF sees
//!       total PCU = background + class_i_volume * pcu_i.
//!    c. Check outer convergence: max relative change in class
//!       volumes across all classes.
//!
//! ## Inner Frank-Wolfe
//!
//! Each inner solve is a standard single-class Frank-Wolfe:
//! a. Compute per-class link costs: ff_mult * vdf(background + x * pcu).
//! b. AoN assignment at current costs to get auxiliary volumes.
//! c. Golden section line search on Beckmann integral with background.
//! d. Update class volumes: x = x + lambda * (y - x).
//!
//! ## Convergence
//!
//! Convergence is linear when cross-class cost interaction is weak
//! relative to own-class sensitivity (Theorem 3.1, conditions 3.8-3.9).
//! Not guaranteed for arbitrary cost structures.
//!
//! ## Trade-offs
//!
//! - Supports per-class VDFs (asymmetric costs).
//! - Each outer iteration runs a full Frank-Wolfe per class -
//!   computational cost is substantially higher than Beckmann FW.
//! - Per-class Dijkstra for path extraction (SPTs may differ).
//!
//! ## Ref
//!
//! Dafermos, S.C. (1982) "Relaxation Algorithms for the General
//! Asymmetric Traffic Equilibrium Problem",
//! Transportation Science, 16(2), 231-240.
//! DOI: 10.1287/trsc.16.2.231

use std::collections::HashMap;

use super::error::AssignmentError;
use super::indexed_graph::IndexedGraph;
use super::multiclass::UserClass;
use super::od_path::OdPath;
use super::{AssignmentConfig, AssignmentResult, VolumeDelayFunction};
use crate::log_additional;
use crate::log_main;
use crate::od::OdMatrix;
use crate::verbose::{EVENT_ASSIGNMENT, EVENT_ASSIGNMENT_ITERATION, EVENT_CONVERGENCE};

const INV_PHI: f64 = 0.618_033_988_749_895;
const TOL: f64 = 1e-8;

/// Diagonalization multi-class assignment.
///
/// Each class gets its own VDF. The outer loop iterates in
/// Gauss-Seidel order; the inner loop is Frank-Wolfe for a single
/// class with background PCU from all other classes.
///
/// # Arguments
///
/// * `class_vdfs` - one VDF per class (same length as `classes`).
///   For shared VDF, pass the same reference for every class.
/// * `max_outer_iterations` - outer diagonalization iterations.
/// * `outer_convergence_gap` - stop when max relative change in
///   per-class volumes drops below this threshold.
/// * `config` - controls the inner FW solver (max_iterations,
///   convergence_gap, store_paths).
pub fn assign_diagonalization(
    graph: &IndexedGraph,
    classes: &[UserClass],
    od_matrices: &[&dyn OdMatrix],
    class_vdfs: &[&dyn VolumeDelayFunction],
    config: &AssignmentConfig,
    max_outer_iterations: usize,
    outer_convergence_gap: f64,
) -> Result<AssignmentResult, AssignmentError> {
    validate_inputs(classes, od_matrices, class_vdfs)?;

    log_main!(
        EVENT_ASSIGNMENT,
        "Starting diagonalization",
        classes = classes.len(),
        outer_iters = max_outer_iterations
    );

    let n = graph.num_links;
    let m = classes.len();

    let mut class_volumes: Vec<Vec<f64>> = vec![vec![0.0; n]; m];
    let mut costs = vec![0.0; n];
    let mut aux = vec![0.0; n];
    let mut background_pcu = vec![0.0; n];
    let mut running_pcu = vec![0.0; n];

    // Cold start: AON for each class at free-flow
    for ci in 0..m {
        compute_class_costs(
            graph,
            &class_volumes[ci],
            classes[ci].pcu,
            classes[ci].ff_time_multiplier,
            &background_pcu,
            class_vdfs[ci],
            &mut costs,
        );
        #[cfg(feature = "parallel")]
        graph.all_or_nothing_parallel(od_matrices[ci], &costs, &mut class_volumes[ci]);
        #[cfg(not(feature = "parallel"))]
        graph.all_or_nothing(od_matrices[ci], &costs, &mut class_volumes[ci]);
    }

    let mut outer_converged = false;
    let mut outer_gap = f64::MAX;
    let mut total_inner_iterations = 0_usize;
    let mut outer_iteration = 0;

    let mut prev_class_volumes: Vec<Vec<f64>> = class_volumes.clone();

    for outer in 0..max_outer_iterations {
        outer_iteration = outer + 1;

        // Running PCU total across all classes (rebuilt each outer iteration)
        running_pcu.fill(0.0);
        for ci in 0..m {
            let p = classes[ci].pcu;
            for i in 0..n {
                running_pcu[i] += class_volumes[ci][i] * p;
            }
        }

        for ci in 0..m {
            let p = classes[ci].pcu;
            for i in 0..n {
                background_pcu[i] = running_pcu[i] - class_volumes[ci][i] * p;
            }

            let inner_iters = inner_fw(
                graph,
                od_matrices[ci],
                class_vdfs[ci],
                p,
                classes[ci].ff_time_multiplier,
                &background_pcu,
                &mut class_volumes[ci],
                config,
                &mut costs,
                &mut aux,
            );
            total_inner_iterations += inner_iters;

            // Update running total with new class volumes
            for i in 0..n {
                running_pcu[i] = background_pcu[i] + class_volumes[ci][i] * p;
            }
        }

        outer_gap = max_relative_change(&prev_class_volumes, &class_volumes, n);

        log_additional!(
            EVENT_ASSIGNMENT_ITERATION,
            "Diagonalization outer iteration",
            iteration = outer_iteration,
            outer_gap = format!("{:.8}", outer_gap)
        );

        if outer_gap < outer_convergence_gap {
            outer_converged = true;
            break;
        }

        for ci in 0..m {
            prev_class_volumes[ci].copy_from_slice(&class_volumes[ci]);
        }
    }

    let pcu_total = compute_pcu_total(&class_volumes, classes, n);
    let mut shared_costs = vec![0.0; n];
    // Final costs: use class 0's VDF on PCU total (for link_costs output).
    // Per-class costs differ - link_costs is an approximation.
    graph.compute_costs(&pcu_total, class_vdfs[0], &mut shared_costs);

    log_main!(
        EVENT_CONVERGENCE,
        "Diagonalization complete",
        outer_iterations = outer_iteration,
        total_inner_iterations = total_inner_iterations,
        outer_gap = format!("{:.8}", outer_gap),
        converged = outer_converged
    );

    build_result(
        graph,
        classes,
        od_matrices,
        class_vdfs,
        &class_volumes,
        &pcu_total,
        config,
        total_inner_iterations,
        outer_gap,
        outer_converged,
    )
}

/// Inner Frank-Wolfe for a single class with background PCU.
///
/// Returns the number of inner iterations performed.
fn inner_fw(
    graph: &IndexedGraph,
    od_matrix: &dyn OdMatrix,
    vdf: &dyn VolumeDelayFunction,
    pcu: f64,
    ff_time_multiplier: f64,
    background_pcu: &[f64],
    class_volumes: &mut [f64],
    config: &AssignmentConfig,
    costs: &mut [f64],
    aux: &mut [f64],
) -> usize {
    let n = graph.num_links;
    let mut iteration = 0;

    for iter in 0..config.max_iterations {
        iteration = iter + 1;

        compute_class_costs(
            graph,
            class_volumes,
            pcu,
            ff_time_multiplier,
            background_pcu,
            vdf,
            costs,
        );

        #[cfg(feature = "parallel")]
        graph.all_or_nothing_parallel(od_matrix, costs, aux);
        #[cfg(not(feature = "parallel"))]
        graph.all_or_nothing(od_matrix, costs, aux);

        let mut numerator = 0.0;
        let mut denominator = 0.0;
        for i in 0..n {
            let current_pcu = background_pcu[i] + class_volumes[i] * pcu;
            let aux_pcu = background_pcu[i] + aux[i] * pcu;
            numerator += current_pcu * costs[i];
            denominator += aux_pcu * costs[i];
        }
        let gap = if numerator > 0.0 {
            (numerator - denominator) / numerator
        } else {
            0.0
        };

        if gap.abs() < config.convergence_gap {
            break;
        }

        let lambda = line_search_with_background(
            graph, class_volumes, aux, pcu, background_pcu, vdf,
        );

        for i in 0..n {
            class_volumes[i] += lambda * (aux[i] - class_volumes[i]);
        }
    }

    iteration
}

/// Compute per-class costs: ff_mult * vdf(background + volume * pcu).
fn compute_class_costs(
    graph: &IndexedGraph,
    class_volumes: &[f64],
    pcu: f64,
    ff_time_multiplier: f64,
    background_pcu: &[f64],
    vdf: &dyn VolumeDelayFunction,
    out: &mut [f64],
) {
    for i in 0..graph.num_links {
        let total_vol = background_pcu[i] + class_volumes[i] * pcu;
        out[i] = ff_time_multiplier
            * vdf.travel_time(graph.link_ff_time[i], total_vol, graph.link_capacity[i]);
    }
}

fn compute_pcu_total(
    class_volumes: &[Vec<f64>],
    classes: &[UserClass],
    n: usize,
) -> Vec<f64> {
    let mut total = vec![0.0; n];
    for (ci, vols) in class_volumes.iter().enumerate() {
        let pcu = classes[ci].pcu;
        for i in 0..n {
            total[i] += vols[i] * pcu;
        }
    }
    total
}

/// Golden section line search on Beckmann integral with background.
fn line_search_with_background(
    graph: &IndexedGraph,
    current: &[f64],
    aux_vols: &[f64],
    pcu: f64,
    background_pcu: &[f64],
    vdf: &dyn VolumeDelayFunction,
) -> f64 {
    let eval = |lambda: f64| -> f64 {
        let mut objective = 0.0;
        for i in 0..graph.num_links {
            let vol = current[i] + lambda * (aux_vols[i] - current[i]);
            let total = background_pcu[i] + vol * pcu;
            objective +=
                vdf.integral(graph.link_ff_time[i], total, graph.link_capacity[i]);
        }
        objective
    };

    let mut a = 0.0_f64;
    let mut b = 1.0_f64;
    let mut x1 = b - INV_PHI * (b - a);
    let mut x2 = a + INV_PHI * (b - a);
    let mut f1 = eval(x1);
    let mut f2 = eval(x2);

    for _ in 0..20 {
        if (b - a) < TOL {
            break;
        }
        if f1 < f2 {
            b = x2;
            x2 = x1;
            f2 = f1;
            x1 = b - INV_PHI * (b - a);
            f1 = eval(x1);
        } else {
            a = x1;
            x1 = x2;
            f1 = f2;
            x2 = a + INV_PHI * (b - a);
            f2 = eval(x2);
        }
    }

    (a + b) / 2.0
}

/// Max relative change across all class volumes between iterations.
fn max_relative_change(
    prev: &[Vec<f64>],
    curr: &[Vec<f64>],
    n: usize,
) -> f64 {
    let mut max_change = 0.0_f64;
    for ci in 0..prev.len() {
        let mut sum_diff_sq = 0.0;
        let mut sum_sq = 0.0;
        for i in 0..n {
            let d = curr[ci][i] - prev[ci][i];
            sum_diff_sq += d * d;
            sum_sq += curr[ci][i] * curr[ci][i];
        }
        if sum_sq > 0.0 {
            let rel = (sum_diff_sq / sum_sq).sqrt();
            max_change = max_change.max(rel);
        }
    }
    max_change
}

fn validate_inputs(
    classes: &[UserClass],
    od_matrices: &[&dyn OdMatrix],
    class_vdfs: &[&dyn VolumeDelayFunction],
) -> Result<(), AssignmentError> {
    if classes.len() != od_matrices.len() {
        return Err(AssignmentError::InvalidConfig(format!(
            "classes.len() ({}) != od_matrices.len() ({})",
            classes.len(),
            od_matrices.len()
        )));
    }
    if classes.len() != class_vdfs.len() {
        return Err(AssignmentError::InvalidConfig(format!(
            "classes.len() ({}) != class_vdfs.len() ({})",
            classes.len(),
            class_vdfs.len()
        )));
    }
    if classes.is_empty() {
        return Err(AssignmentError::InvalidConfig(
            "at least one user class is required".to_string(),
        ));
    }
    for c in classes {
        if c.pcu <= 0.0 {
            return Err(AssignmentError::InvalidConfig(format!(
                "class '{}': pcu must be positive, got {}",
                c.name, c.pcu
            )));
        }
        if c.ff_time_multiplier <= 0.0 {
            return Err(AssignmentError::InvalidConfig(format!(
                "class '{}': ff_time_multiplier must be positive, got {}",
                c.name, c.ff_time_multiplier
            )));
        }
    }
    Ok(())
}

fn build_result(
    graph: &IndexedGraph,
    classes: &[UserClass],
    od_matrices: &[&dyn OdMatrix],
    class_vdfs: &[&dyn VolumeDelayFunction],
    class_volumes: &[Vec<f64>],
    pcu_total: &[f64],
    config: &AssignmentConfig,
    iterations: usize,
    relative_gap: f64,
    converged: bool,
) -> Result<AssignmentResult, AssignmentError> {
    let mut cv = HashMap::with_capacity(classes.len());
    for (ci, class) in classes.iter().enumerate() {
        cv.insert(
            class.name.clone(),
            graph.volumes_to_hashmap(&class_volumes[ci]),
        );
    }

    let path_flows = if config.store_paths {
        Some(extract_paths_per_class(
            graph,
            classes,
            od_matrices,
            class_vdfs,
            pcu_total,
        ))
    } else {
        None
    };

    // link_costs: per-class costs differ, so we store the base shared
    // cost t_a(V_a) using class 0's VDF.
    let mut shared_costs = vec![0.0; graph.num_links];
    graph.compute_costs(pcu_total, class_vdfs[0], &mut shared_costs);

    Ok(AssignmentResult {
        link_volumes: graph.volumes_to_hashmap(pcu_total),
        link_costs: graph.costs_to_hashmap(&shared_costs),
        iterations,
        relative_gap,
        converged,
        class_volumes: Some(cv),
        path_flows,
    })
}

/// Per-class path extraction with per-class Dijkstra.
///
/// Unlike the Beckmann multiclass (shared SPT), diagonalization may
/// produce different shortest paths per class because VDFs differ.
/// Runs M Dijkstra per origin.
fn extract_paths_per_class(
    graph: &IndexedGraph,
    classes: &[UserClass],
    od_matrices: &[&dyn OdMatrix],
    class_vdfs: &[&dyn VolumeDelayFunction],
    pcu_total: &[f64],
) -> Vec<OdPath> {
    let zone_ids = graph.zone_ids().to_vec();
    let zone_node_idxs: Vec<Option<usize>> =
        zone_ids.iter().map(|&z| graph.zone_node_idx(z)).collect();

    let mut paths = Vec::new();
    let mut costs = vec![0.0; graph.num_links];
    let mut dist = vec![f64::INFINITY; graph.num_nodes];
    let mut pred: Vec<Option<usize>> = vec![None; graph.num_nodes];
    let mut visited = vec![false; graph.num_nodes];

    for (ci, class) in classes.iter().enumerate() {
        // Per-class costs on converged volumes
        for i in 0..graph.num_links {
            costs[i] = class.ff_time_multiplier
                * class_vdfs[ci].travel_time(
                    graph.link_ff_time[i],
                    pcu_total[i],
                    graph.link_capacity[i],
                );
        }

        for (oi, &origin_zone) in zone_ids.iter().enumerate() {
            let origin_idx = match zone_node_idxs[oi] {
                Some(i) => i,
                None => continue,
            };

            graph.dijkstra_into(origin_idx, &costs, &mut dist, &mut pred, &mut visited);

            for (di, &dest_zone) in zone_ids.iter().enumerate() {
                if oi == di {
                    continue;
                }

                let demand = od_matrices[ci].get(origin_zone, dest_zone);
                if demand <= 0.0 {
                    continue;
                }

                let dest_idx = match zone_node_idxs[di] {
                    Some(i) => i,
                    None => continue,
                };

                if dist[dest_idx] == f64::INFINITY {
                    continue;
                }

                let mut link_ids = Vec::new();
                let mut path_cost = 0.0;
                let mut current = dest_idx;
                loop {
                    match pred[current] {
                        Some(li) => {
                            link_ids.push(graph.link_id(li));
                            path_cost += costs[li];
                            current = graph.link_source(li);
                        }
                        None => break,
                    }
                }
                link_ids.reverse();

                if link_ids.is_empty() {
                    continue;
                }

                paths.push(OdPath {
                    origin_zone,
                    dest_zone,
                    path_index: 0,
                    flow: demand,
                    cost: path_cost,
                    link_ids,
                    class_index: Some(ci as u16),
                });
            }
        }
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignment::BprFunction;
    use crate::gmns::meso::link::Link;
    use crate::gmns::meso::network::Network;
    use crate::gmns::meso::node::Node;
    use crate::od::dense::DenseOdMatrix;
    use crate::od::OdMatrix;

    const EPS: f64 = 1e-4;

    fn two_link_network() -> (Network, IndexedGraph) {
        let mut net = Network::new();
        net.add_node(
            Node::new(1)
                .with_zone_id(1)
                .with_coordinates(0.0, 0.0)
                .build(),
        )
        .unwrap();
        net.add_node(
            Node::new(2)
                .with_zone_id(2)
                .with_coordinates(0.0, 1.0)
                .build(),
        )
        .unwrap();
        net.add_link(
            Link::new(100, 1, 2)
                .with_length_meters(1000.0)
                .with_free_speed(60.0)
                .with_capacity(1000.0)
                .build(),
        )
        .unwrap();
        net.add_link(
            Link::new(101, 2, 1)
                .with_length_meters(1000.0)
                .with_free_speed(60.0)
                .with_capacity(1000.0)
                .build(),
        )
        .unwrap();
        let graph = IndexedGraph::from_network(&net);
        (net, graph)
    }

    #[test]
    fn shared_vdf_matches_beckmann() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let car = UserClass::new("car", 1.0, 1.0);
        let truck = UserClass::new("truck", 2.5, 2.5);
        let classes = [car, truck];

        let mut od_car = DenseOdMatrix::new(vec![1, 2]);
        od_car.set(1, 2, 450.0);
        let mut od_truck = DenseOdMatrix::new(vec![1, 2]);
        od_truck.set(1, 2, 50.0);
        let od_matrices: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];

        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr, &bpr];

        let config = AssignmentConfig {
            max_iterations: 50,
            convergence_gap: 1e-6,
            store_paths: false,
        };

        let result = assign_diagonalization(
            &graph, &classes, &od_matrices, &vdfs, &config, 20, 1e-4,
        )
        .unwrap();

        assert!(result.converged);

        let cv = result.class_volumes.as_ref().unwrap();
        let car_vol: f64 = cv["car"].values().sum();
        let truck_vol: f64 = cv["truck"].values().sum();
        assert!((car_vol - 450.0).abs() < EPS);
        assert!((truck_vol - 50.0).abs() < EPS);
    }

    #[test]
    fn per_class_vdf_different_costs() {
        let (_net, graph) = two_link_network();
        // Car: normal BPR
        let bpr_car = BprFunction::new(0.15, 4.0);
        // Truck: steeper BPR (more sensitive to congestion)
        let bpr_truck = BprFunction::new(0.30, 4.0);

        let car = UserClass::new("car", 1.0, 1.0);
        let truck = UserClass::new("truck", 2.5, 1.0);
        let classes = [car, truck];

        let mut od_car = DenseOdMatrix::new(vec![1, 2]);
        od_car.set(1, 2, 450.0);
        let mut od_truck = DenseOdMatrix::new(vec![1, 2]);
        od_truck.set(1, 2, 50.0);
        let od_matrices: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];

        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];

        let config = AssignmentConfig {
            max_iterations: 50,
            convergence_gap: 1e-6,
            store_paths: false,
        };

        let result = assign_diagonalization(
            &graph, &classes, &od_matrices, &vdfs, &config, 20, 1e-4,
        )
        .unwrap();

        assert!(result.converged);

        // Truck sees higher alpha (0.30 vs 0.15), so truck cost > car cost
        // on the same link at the same volume
        let pcu_total: f64 = result.link_volumes.values().sum();
        assert!(pcu_total > 0.0);
    }

    #[test]
    fn store_paths_per_class() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let car = UserClass::new("car", 1.0, 1.0);
        let truck = UserClass::new("truck", 2.5, 2.5);

        let mut od_car = DenseOdMatrix::new(vec![1, 2]);
        od_car.set(1, 2, 450.0);
        let mut od_truck = DenseOdMatrix::new(vec![1, 2]);
        od_truck.set(1, 2, 50.0);
        let od_matrices: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];

        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr, &bpr];

        let config = AssignmentConfig {
            max_iterations: 50,
            convergence_gap: 1e-6,
            store_paths: true,
        };

        let result = assign_diagonalization(
            &graph,
            &[car, truck],
            &od_matrices,
            &vdfs,
            &config,
            20,
            1e-4,
        )
        .unwrap();

        let paths = result.path_flows.as_ref().unwrap();
        assert_eq!(paths.len(), 2);

        let car_paths: Vec<_> = paths.iter().filter(|p| p.class_index == Some(0)).collect();
        let truck_paths: Vec<_> = paths.iter().filter(|p| p.class_index == Some(1)).collect();
        assert_eq!(car_paths.len(), 1);
        assert_eq!(truck_paths.len(), 1);

        assert!((car_paths[0].flow - 450.0).abs() < EPS);
        assert!((truck_paths[0].flow - 50.0).abs() < EPS);
    }

    #[test]
    fn validation_mismatched_vdfs() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let car = UserClass::new("car", 1.0, 1.0);

        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 100.0);
        let od_matrices: Vec<&dyn OdMatrix> = vec![&od];
        let vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr, &bpr];

        let config = AssignmentConfig::default();
        let result = assign_diagonalization(
            &graph, &[car], &od_matrices, &vdfs, &config, 10, 1e-4,
        );
        assert!(result.is_err());
    }
}
