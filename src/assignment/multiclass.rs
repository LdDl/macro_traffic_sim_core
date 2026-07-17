//! Multi-class PCU-based traffic assignment.
//!
//! All vehicle classes share the same VDF (Volume-Delay Function,
//! e.g. BPR or custom).
//! Link flow is aggregated in PCU (Passenger Car Units, also known
//! as PCE - Passenger Car Equivalents):
//!
//!   V_a = sum_m(x_a^m * pcu_m)
//!
//! Per-class cost applies a class-specific multiplier to the shared cost:
//!
//!   c_a^m = ff_time_multiplier_m * t_a(V_a)
//!
//! The Beckmann convex program structure is preserved when the symmetry
//! condition holds: ff_time_multiplier_m / pcu_m = const for all classes.
//! This is validated at runtime. When the condition holds, Frank-Wolfe
//! and MSA converge to the multi-class User Equilibrium. The simplest
//! valid configuration is ff_time_multiplier = pcu for every class.
//!
//! Note: under any valid ratio k, the per-class cost is
//! c_a^m = k * pcu_m * t_a(V_a). Since k * pcu_m is a constant
//! multiplier applied equally to all links for a given class, it does
//! not change shortest paths. All classes route identically -- they
//! differ only in PCU weight and OD patterns, not routing.
//!
//! An alternative formulation uses class-specific VDFs (each class has its
//! own volume-delay function). That turns the problem into a variational
//! inequality (VI) - the Beckmann objective no longer applies, and solvers
//! like diagonalization (Dafermos, 1982) are needed. Each outer
//! iteration of diagonalization solves a full single-class assignment.
//! Convergence is not guaranteed for general cost structures, and
//! computational cost is substantially higher. We do not implement this.
//!
//! Ref:
//! 1. Dafermos, S.C. (1972) "The Traffic Assignment Problem for
//! Multiclass-User Transportation Networks",
//! Transportation Science, 6(1), 73-87.
//! DOI: 10.1287/trsc.6.1.73
//! 2. Dafermos, S.C. (1982) "Relaxation Algorithms for the General
//! Asymmetric Traffic Equilibrium Problem",
//! Transportation Science, 16(2), 231-240.
//! DOI: 10.1287/trsc.16.2.231
//! Diagonalization for class-specific VDFs (NOT implemented due model limitations).
//! 
//! Model limitations are:
//! * All classes share the same link cost `t_a(V_a)``, computed on PCU-total volume.
//! * Per-class cost is a scalar multiple: `c_a^m = ff_mult_m * t_a(V_a)`.
//! * Classes differ in volumes (demand split + PCU weight), not in per-link costs.
//! * This guarantees a shared SPT across all classes.
//! Per-link-per-class costs (truck bans, class-specific tolls,
//! class-specific VDFs) require a different formulation (variational
//! inequality) and a different solver (e.g. diagonalization with
//! FW/MSA as inner solver). This can be added as a separate
//! assignment method without modifying the existing ones.

use std::collections::HashMap;

use super::error::AssignmentError;
use super::indexed_graph::IndexedGraph;
use super::{AssignmentConfig, AssignmentResult, VolumeDelayFunction};
use crate::gmns::types::LinkID;
use crate::log_additional;
use crate::log_main;
use crate::od::OdMatrix;
use crate::verbose::{EVENT_ASSIGNMENT, EVENT_ASSIGNMENT_ITERATION, EVENT_CONVERGENCE};

const INV_PHI: f64 = 0.618_033_988_749_895;
const LINE_SEARCH_TOL: f64 = 1e-8;

/// User class for multi-class assignment.
///
/// Each class has a PCU factor (how many passenger car equivalents
/// one vehicle of this class represents) and a free-flow time
/// multiplier (scaling factor for perceived travel time).
#[derive(Debug, Clone)]
pub struct UserClass {
    pub name: String,
    /// Passenger Car Units per vehicle. Cars = 1.0, trucks typically 2.0-3.0.
    pub pcu: f64,
    /// Multiplier applied to link travel time for this class.
    /// 1.0 = same as base, >1.0 = slower (e.g. trucks).
    pub ff_time_multiplier: f64,
}

impl UserClass {
    pub fn new(name: &str, pcu: f64, ff_time_multiplier: f64) -> Self {
        UserClass {
            name: name.to_string(),
            pcu,
            ff_time_multiplier,
        }
    }

    pub fn car() -> Self {
        UserClass::new("car", 1.0, 1.0)
    }

    pub fn truck() -> Self {
        UserClass::new("truck", 2.5, 2.5)
    }
}

/// Multi-class Frank-Wolfe assignment.
///
/// Each class has its own OD matrix. Link volumes are aggregated in
/// PCU for the shared VDF evaluation. Line search and convergence
/// check use PCU-total volumes.
///
/// Warm start: pass per-class volumes from a previous
/// [`AssignmentResult::class_volumes`] to skip the AON initialization.
pub fn assign_multiclass_fw(
    graph: &IndexedGraph,
    classes: &[UserClass],
    od_matrices: &[&dyn OdMatrix],
    vdf: &dyn VolumeDelayFunction,
    config: &AssignmentConfig,
    initial_class_volumes: Option<&HashMap<String, HashMap<LinkID, f64>>>,
) -> Result<AssignmentResult, AssignmentError> {
    validate_inputs(classes, od_matrices)?;

    let warm = initial_class_volumes.is_some();
    log_main!(
        EVENT_ASSIGNMENT,
        "Starting multi-class Frank-Wolfe",
        classes = classes.len(),
        warm_start = warm
    );

    let n = graph.num_links;
    let m = classes.len();

    let mut class_volumes = init_class_volumes(graph, classes, initial_class_volumes, n, m);
    let mut class_aux = vec![vec![0.0; n]; m];
    let mut shared_costs = vec![0.0; n];
    let mut class_costs = vec![0.0; n];

    if initial_class_volumes.is_none() {
        let pcu_total = compute_pcu_total(&class_volumes, classes, n);
        graph.compute_costs(&pcu_total, vdf, &mut shared_costs);
        for ci in 0..m {
            scale_costs(&shared_costs, classes[ci].ff_time_multiplier, &mut class_costs);
            #[cfg(feature = "parallel")]
            graph.all_or_nothing_parallel(od_matrices[ci], &class_costs, &mut class_volumes[ci]);
            #[cfg(not(feature = "parallel"))]
            graph.all_or_nothing(od_matrices[ci], &class_costs, &mut class_volumes[ci]);
        }
    }

    let mut converged = false;
    let mut relative_gap = f64::MAX;
    let mut iteration = 0;

    for iter in 0..config.max_iterations {
        iteration = iter + 1;

        let pcu_total = compute_pcu_total(&class_volumes, classes, n);
        graph.compute_costs(&pcu_total, vdf, &mut shared_costs);

        for ci in 0..m {
            scale_costs(&shared_costs, classes[ci].ff_time_multiplier, &mut class_costs);
            #[cfg(feature = "parallel")]
            graph.all_or_nothing_parallel(od_matrices[ci], &class_costs, &mut class_aux[ci]);
            #[cfg(not(feature = "parallel"))]
            graph.all_or_nothing(od_matrices[ci], &class_costs, &mut class_aux[ci]);
        }

        let pcu_aux = compute_pcu_total(&class_aux, classes, n);
        relative_gap = graph.relative_gap(&pcu_total, &shared_costs, &pcu_aux);

        log_additional!(
            EVENT_ASSIGNMENT_ITERATION,
            "Multi-class FW iteration",
            iteration = iteration,
            gap = format!("{:.8}", relative_gap)
        );

        if relative_gap.abs() < config.convergence_gap {
            converged = true;
            break;
        }

        let lambda = line_search(graph, &pcu_total, &pcu_aux, vdf);

        for ci in 0..m {
            for i in 0..n {
                class_volumes[ci][i] += lambda * (class_aux[ci][i] - class_volumes[ci][i]);
            }
        }
    }

    let pcu_total = compute_pcu_total(&class_volumes, classes, n);
    graph.compute_costs(&pcu_total, vdf, &mut shared_costs);

    log_main!(
        EVENT_CONVERGENCE,
        "Multi-class Frank-Wolfe complete",
        iterations = iteration,
        gap = format!("{:.8}", relative_gap),
        converged = converged
    );

    Ok(build_result(
        graph,
        classes,
        od_matrices,
        &class_volumes,
        &pcu_total,
        &shared_costs,
        config,
        iteration,
        relative_gap,
        converged,
    ))
}

/// Multi-class MSA assignment.
///
/// Same as [`assign_multiclass_fw`] but with fixed step size 1/(n+1).
pub fn assign_multiclass_msa(
    graph: &IndexedGraph,
    classes: &[UserClass],
    od_matrices: &[&dyn OdMatrix],
    vdf: &dyn VolumeDelayFunction,
    config: &AssignmentConfig,
    initial_class_volumes: Option<&HashMap<String, HashMap<LinkID, f64>>>,
) -> Result<AssignmentResult, AssignmentError> {
    validate_inputs(classes, od_matrices)?;

    let warm = initial_class_volumes.is_some();
    log_main!(
        EVENT_ASSIGNMENT,
        "Starting multi-class MSA",
        classes = classes.len(),
        warm_start = warm
    );

    let n = graph.num_links;
    let m = classes.len();

    let mut class_volumes = init_class_volumes(graph, classes, initial_class_volumes, n, m);
    let mut class_aux = vec![vec![0.0; n]; m];
    let mut shared_costs = vec![0.0; n];
    let mut class_costs = vec![0.0; n];

    if initial_class_volumes.is_none() {
        let pcu_total = compute_pcu_total(&class_volumes, classes, n);
        graph.compute_costs(&pcu_total, vdf, &mut shared_costs);
        for ci in 0..m {
            scale_costs(&shared_costs, classes[ci].ff_time_multiplier, &mut class_costs);
            #[cfg(feature = "parallel")]
            graph.all_or_nothing_parallel(od_matrices[ci], &class_costs, &mut class_volumes[ci]);
            #[cfg(not(feature = "parallel"))]
            graph.all_or_nothing(od_matrices[ci], &class_costs, &mut class_volumes[ci]);
        }
    }

    let mut converged = false;
    let mut relative_gap = f64::MAX;
    let mut iteration = 0;

    for iter in 0..config.max_iterations {
        iteration = iter + 1;

        let pcu_total = compute_pcu_total(&class_volumes, classes, n);
        graph.compute_costs(&pcu_total, vdf, &mut shared_costs);

        for ci in 0..m {
            scale_costs(&shared_costs, classes[ci].ff_time_multiplier, &mut class_costs);
            #[cfg(feature = "parallel")]
            graph.all_or_nothing_parallel(od_matrices[ci], &class_costs, &mut class_aux[ci]);
            #[cfg(not(feature = "parallel"))]
            graph.all_or_nothing(od_matrices[ci], &class_costs, &mut class_aux[ci]);
        }

        let pcu_aux = compute_pcu_total(&class_aux, classes, n);
        relative_gap = graph.relative_gap(&pcu_total, &shared_costs, &pcu_aux);

        log_additional!(
            EVENT_ASSIGNMENT_ITERATION,
            "Multi-class MSA iteration",
            iteration = iteration,
            gap = format!("{:.8}", relative_gap)
        );

        if relative_gap.abs() < config.convergence_gap {
            converged = true;
            break;
        }

        let lambda = 1.0 / (iteration as f64 + 1.0);

        for ci in 0..m {
            for i in 0..n {
                class_volumes[ci][i] += lambda * (class_aux[ci][i] - class_volumes[ci][i]);
            }
        }
    }

    let pcu_total = compute_pcu_total(&class_volumes, classes, n);
    graph.compute_costs(&pcu_total, vdf, &mut shared_costs);

    log_main!(
        EVENT_CONVERGENCE,
        "Multi-class MSA complete",
        iterations = iteration,
        gap = format!("{:.8}", relative_gap),
        converged = converged
    );

    Ok(build_result(
        graph,
        classes,
        od_matrices,
        &class_volumes,
        &pcu_total,
        &shared_costs,
        config,
        iteration,
        relative_gap,
        converged,
    ))
}

fn validate_inputs(
    classes: &[UserClass],
    od_matrices: &[&dyn OdMatrix],
) -> Result<(), AssignmentError> {
    if classes.len() != od_matrices.len() {
        return Err(AssignmentError::InvalidConfig(format!(
            "classes.len() ({}) != od_matrices.len() ({})",
            classes.len(),
            od_matrices.len()
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
    let ratio0 = classes[0].ff_time_multiplier / classes[0].pcu;
    for c in &classes[1..] {
        let ratio = c.ff_time_multiplier / c.pcu;
        if (ratio - ratio0).abs() > 1e-10 {
            return Err(AssignmentError::InvalidConfig(format!(
                "Beckmann symmetry condition violated: \
                 ff_time_multiplier/pcu must be the same for all classes. \
                 Class '{}' has {}/{} = {:.6}, class '{}' has {}/{} = {:.6}",
                classes[0].name,
                classes[0].ff_time_multiplier,
                classes[0].pcu,
                ratio0,
                c.name,
                c.ff_time_multiplier,
                c.pcu,
                ratio
            )));
        }
    }
    Ok(())
}

fn init_class_volumes(
    graph: &IndexedGraph,
    classes: &[UserClass],
    initial: Option<&HashMap<String, HashMap<LinkID, f64>>>,
    n: usize,
    m: usize,
) -> Vec<Vec<f64>> {
    match initial {
        Some(prev) => classes
            .iter()
            .map(|c| {
                prev.get(&c.name)
                    .map(|v| graph.hashmap_to_vec(v))
                    .unwrap_or_else(|| vec![0.0; n])
            })
            .collect(),
        None => vec![vec![0.0; n]; m],
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

fn scale_costs(shared: &[f64], multiplier: f64, out: &mut [f64]) {
    if (multiplier - 1.0).abs() < 1e-15 {
        out.copy_from_slice(shared);
    } else {
        for i in 0..shared.len() {
            out[i] = shared[i] * multiplier;
        }
    }
}

fn line_search(
    graph: &IndexedGraph,
    current_pcu: &[f64],
    aux_pcu: &[f64],
    vdf: &dyn VolumeDelayFunction,
) -> f64 {
    let mut a = 0.0_f64;
    let mut b = 1.0_f64;

    let mut x1 = b - INV_PHI * (b - a);
    let mut x2 = a + INV_PHI * (b - a);
    let mut f1 = eval_beckmann_at_step(graph, current_pcu, aux_pcu, vdf, x1);
    let mut f2 = eval_beckmann_at_step(graph, current_pcu, aux_pcu, vdf, x2);

    for _ in 0..20 {
        if (b - a) < LINE_SEARCH_TOL {
            break;
        }
        if f1 < f2 {
            b = x2;
            x2 = x1;
            f2 = f1;
            x1 = b - INV_PHI * (b - a);
            f1 = eval_beckmann_at_step(graph, current_pcu, aux_pcu, vdf, x1);
        } else {
            a = x1;
            x1 = x2;
            f1 = f2;
            x2 = a + INV_PHI * (b - a);
            f2 = eval_beckmann_at_step(graph, current_pcu, aux_pcu, vdf, x2);
        }
    }

    (a + b) / 2.0
}

fn eval_beckmann_at_step(
    graph: &IndexedGraph,
    current_pcu: &[f64],
    aux_pcu: &[f64],
    vdf: &dyn VolumeDelayFunction,
    lambda: f64,
) -> f64 {
    let mut objective = 0.0;
    for i in 0..graph.num_links {
        let vol = current_pcu[i] + lambda * (aux_pcu[i] - current_pcu[i]);
        objective += vdf.integral(graph.link_ff_time[i], vol, graph.link_capacity[i]);
    }
    objective
}

fn build_result(
    graph: &IndexedGraph,
    classes: &[UserClass],
    od_matrices: &[&dyn OdMatrix],
    class_volumes: &[Vec<f64>],
    pcu_total: &[f64],
    shared_costs: &[f64],
    config: &AssignmentConfig,
    iterations: usize,
    relative_gap: f64,
    converged: bool,
) -> AssignmentResult {
    let mut cv = HashMap::with_capacity(classes.len());
    for (ci, class) in classes.iter().enumerate() {
        cv.insert(
            class.name.clone(),
            graph.volumes_to_hashmap(&class_volumes[ci]),
        );
    }

    let path_flows = if config.store_paths {
        let multipliers: Vec<f64> = classes.iter().map(|c| c.ff_time_multiplier).collect();
        Some(graph.extract_shortest_paths_multiclass(
            od_matrices,
            &multipliers,
            shared_costs,
        ))
    } else {
        None
    };

    AssignmentResult {
        link_volumes: graph.volumes_to_hashmap(pcu_total),
        link_costs: graph.costs_to_hashmap(shared_costs),
        iterations,
        relative_gap,
        converged,
        class_volumes: Some(cv),
        path_flows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignment::BprFunction;
    use crate::gmns::meso::link::Link;
    use crate::gmns::meso::network::Network;
    use crate::gmns::meso::node::Node;
    use crate::od::dense::DenseOdMatrix;

    const EPS: f64 = 1e-10;

    fn two_link_network() -> (Network, IndexedGraph) {
        let mut net = Network::new();
        net.add_node(Node::new(1).with_zone_id(1).with_coordinates(0.0, 0.0).build())
            .unwrap();
        net.add_node(Node::new(2).with_zone_id(2).with_coordinates(0.0, 1.0).build())
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
    fn single_class_matches_regular_fw() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 100,
            convergence_gap: 1e-4,
            store_paths: false,
        };

        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 500.0);

        let classes = vec![UserClass::car()];
        let od_refs: Vec<&dyn OdMatrix> = vec![&od];
        let result = assign_multiclass_fw(&graph, &classes, &od_refs, &bpr, &config, None).unwrap();

        assert!(result.converged);
        assert!(result.class_volumes.is_some());
        let cv = result.class_volumes.as_ref().unwrap();
        assert!(cv.contains_key("car"));

        let car_vol: f64 = cv["car"].values().sum();
        assert!((car_vol - 500.0).abs() < EPS);

        let pcu_total: f64 = result.link_volumes.values().sum();
        assert!((pcu_total - 500.0).abs() < EPS);
    }

    #[test]
    fn two_classes_pcu_aggregation() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 100,
            convergence_gap: 1e-4,
            store_paths: false,
        };

        let mut car_od = DenseOdMatrix::new(vec![1, 2]);
        car_od.set(1, 2, 400.0);
        let mut truck_od = DenseOdMatrix::new(vec![1, 2]);
        truck_od.set(1, 2, 100.0);

        let classes = vec![UserClass::car(), UserClass::truck()];
        let od_refs: Vec<&dyn OdMatrix> = vec![&car_od, &truck_od];
        let result = assign_multiclass_fw(&graph, &classes, &od_refs, &bpr, &config, None).unwrap();

        assert!(result.converged);
        let cv = result.class_volumes.as_ref().unwrap();

        let car_vol: f64 = cv["car"].values().sum();
        let truck_vol: f64 = cv["truck"].values().sum();
        assert!((car_vol - 400.0).abs() < EPS);
        assert!((truck_vol - 100.0).abs() < EPS);

        let pcu_total: f64 = result.link_volumes.values().sum();
        assert!((pcu_total - 650.0).abs() < EPS);
    }

    #[test]
    fn warm_start_multiclass() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 100,
            convergence_gap: 1e-4,
            store_paths: false,
        };

        let mut car_od = DenseOdMatrix::new(vec![1, 2]);
        car_od.set(1, 2, 400.0);
        let mut truck_od = DenseOdMatrix::new(vec![1, 2]);
        truck_od.set(1, 2, 100.0);

        let classes = vec![UserClass::car(), UserClass::truck()];
        let od_refs: Vec<&dyn OdMatrix> = vec![&car_od, &truck_od];

        let cold = assign_multiclass_fw(&graph, &classes, &od_refs, &bpr, &config, None).unwrap();
        let warm = assign_multiclass_fw(
            &graph,
            &classes,
            &od_refs,
            &bpr,
            &config,
            cold.class_volumes.as_ref(),
        )
        .unwrap();

        assert!(warm.converged);
        assert!(warm.iterations <= cold.iterations);
    }

    #[test]
    fn msa_multiclass_converges() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 200,
            convergence_gap: 1e-3,
            store_paths: false,
        };

        let mut car_od = DenseOdMatrix::new(vec![1, 2]);
        car_od.set(1, 2, 400.0);
        let mut truck_od = DenseOdMatrix::new(vec![1, 2]);
        truck_od.set(1, 2, 100.0);

        let classes = vec![UserClass::car(), UserClass::truck()];
        let od_refs: Vec<&dyn OdMatrix> = vec![&car_od, &truck_od];
        let result =
            assign_multiclass_msa(&graph, &classes, &od_refs, &bpr, &config, None).unwrap();

        assert!(result.converged);
    }

    #[test]
    fn mismatched_classes_and_ods_errors() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig::default();

        let od = DenseOdMatrix::new(vec![1, 2]);
        let classes = vec![UserClass::car(), UserClass::truck()];
        let od_refs: Vec<&dyn OdMatrix> = vec![&od];

        let err = assign_multiclass_fw(&graph, &classes, &od_refs, &bpr, &config, None);
        assert!(err.is_err());
    }

    #[test]
    fn symmetry_condition_violated_errors() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig::default();

        let mut car_od = DenseOdMatrix::new(vec![1, 2]);
        car_od.set(1, 2, 400.0);
        let mut truck_od = DenseOdMatrix::new(vec![1, 2]);
        truck_od.set(1, 2, 100.0);

        let classes = vec![
            UserClass::new("car", 1.0, 1.0),
            UserClass::new("truck", 2.5, 1.2),
        ];
        let od_refs: Vec<&dyn OdMatrix> = vec![&car_od, &truck_od];

        let err = assign_multiclass_fw(&graph, &classes, &od_refs, &bpr, &config, None);
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("symmetry"));
    }

    #[test]
    fn zero_pcu_errors() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig::default();

        let mut od = DenseOdMatrix::new(vec![1, 2]);
        od.set(1, 2, 100.0);

        let classes = vec![UserClass::new("bad", 0.0, 1.0)];
        let od_refs: Vec<&dyn OdMatrix> = vec![&od];

        let err = assign_multiclass_fw(&graph, &classes, &od_refs, &bpr, &config, None);
        assert!(err.is_err());
    }

    #[test]
    fn store_paths_multiclass_fw() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 100,
            convergence_gap: 1e-4,
            store_paths: true,
        };

        let mut car_od = DenseOdMatrix::new(vec![1, 2]);
        car_od.set(1, 2, 400.0);
        let mut truck_od = DenseOdMatrix::new(vec![1, 2]);
        truck_od.set(1, 2, 100.0);

        let classes = vec![UserClass::car(), UserClass::truck()];
        let od_refs: Vec<&dyn OdMatrix> = vec![&car_od, &truck_od];
        let result = assign_multiclass_fw(&graph, &classes, &od_refs, &bpr, &config, None).unwrap();

        let paths = result.path_flows.as_ref().expect("paths should be Some");
        assert_eq!(paths.len(), 2);

        let car_paths: Vec<_> = paths.iter()
            .filter(|p| p.class_index == Some(0))
            .collect();
        assert_eq!(car_paths.len(), 1);
        assert!((car_paths[0].flow - 400.0).abs() < EPS);
        assert_eq!(car_paths[0].link_ids, vec![100]);

        let truck_paths: Vec<_> = paths.iter()
            .filter(|p| p.class_index == Some(1))
            .collect();
        assert_eq!(truck_paths.len(), 1);
        assert!((truck_paths[0].flow - 100.0).abs() < EPS);
        assert_eq!(truck_paths[0].link_ids, vec![100]);

        // Truck cost > car cost (ff_time_multiplier 2.5 vs 1.0)
        assert!(truck_paths[0].cost > car_paths[0].cost);
        assert!((truck_paths[0].cost / car_paths[0].cost - 2.5).abs() < 1e-6);
    }

    #[test]
    fn store_paths_multiclass_msa() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 200,
            convergence_gap: 1e-3,
            store_paths: true,
        };

        let mut car_od = DenseOdMatrix::new(vec![1, 2]);
        car_od.set(1, 2, 400.0);
        let mut truck_od = DenseOdMatrix::new(vec![1, 2]);
        truck_od.set(1, 2, 100.0);

        let classes = vec![UserClass::car(), UserClass::truck()];
        let od_refs: Vec<&dyn OdMatrix> = vec![&car_od, &truck_od];
        let result =
            assign_multiclass_msa(&graph, &classes, &od_refs, &bpr, &config, None).unwrap();

        let paths = result.path_flows.as_ref().expect("paths should be Some");
        assert_eq!(paths.len(), 2);

        let car_flow: f64 = paths.iter()
            .filter(|p| p.class_index == Some(0))
            .map(|p| p.flow)
            .sum();
        let truck_flow: f64 = paths.iter()
            .filter(|p| p.class_index == Some(1))
            .map(|p| p.flow)
            .sum();
        assert!((car_flow - 400.0).abs() < EPS);
        assert!((truck_flow - 100.0).abs() < EPS);
    }

    #[test]
    fn store_paths_false_multiclass() {
        let (_net, graph) = two_link_network();
        let bpr = BprFunction::default();
        let config = AssignmentConfig {
            max_iterations: 100,
            convergence_gap: 1e-4,
            store_paths: false,
        };

        let mut car_od = DenseOdMatrix::new(vec![1, 2]);
        car_od.set(1, 2, 400.0);

        let classes = vec![UserClass::car()];
        let od_refs: Vec<&dyn OdMatrix> = vec![&car_od];
        let result = assign_multiclass_fw(&graph, &classes, &od_refs, &bpr, &config, None).unwrap();

        assert!(result.path_flows.is_none());
    }
}
