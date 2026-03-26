//! # Method of Successive Averages (MSA)
//!
//! Traffic assignment using MSA with fixed step size `1/n`.
//! Simpler than Frank-Wolfe but potentially slower convergence.
//!
//! ## Algorithm
//!
//! 1. Initialize with free-flow costs and all-or-nothing (AoN) loading.
//! 2. At each iteration `n`:
//!    a. Compute link costs from current volumes via the VDF.
//!    b. Perform AoN assignment at current costs to get auxiliary volumes `y`.
//!    c. Check the relative gap -- if below threshold, stop.
//!    d. Compute step size `lambda = 1 / (n + 1)`.
//!    e. Update volumes: `x = x + lambda * (y - x)`.
//!
//! ## Step size
//!
//! The step size `1/(n+1)` guarantees convergence under mild conditions
//! (the sequence satisfies `sum(lambda_n) = inf` and
//! `sum(lambda_n^2) < inf`). However, the fixed schedule cannot adapt
//! to the problem geometry, so convergence is typically slower than
//! Frank-Wolfe's optimized line search.
//!
//! ## Trade-offs
//!
//! - Simplest assignment method -- no line search, no path storage.
//! - Each iteration requires exactly one shortest path computation (AoN).
//! - Convergence is guaranteed but slower than Frank-Wolfe.
//! - Good for quick prototyping or when line search cost is a concern.

use super::error::AssignmentError;
use super::indexed_graph::IndexedGraph;
use crate::gmns::meso::network::Network;
use crate::log_additional;
use crate::log_main;
use crate::od::OdMatrix;
use crate::verbose::{EVENT_ASSIGNMENT, EVENT_ASSIGNMENT_ITERATION, EVENT_CONVERGENCE};

use super::{AssignmentConfig, AssignmentMethod, AssignmentResult, VolumeDelayFunction};

/// Method of Successive Averages traffic assignment.
///
/// Step size at iteration `n` is `1/(n+1)` (deterministic averaging).
/// This is the simplest UE assignment method -- no line search or
/// path storage is needed.
///
/// # Usage
///
/// ```text
/// let msa = Msa::new();
/// let result = msa.assign(&network, &od_matrix, &bpr, &config)?;
/// ```
#[derive(Debug)]
pub struct Msa;

impl Msa {
    /// Create a new MSA assignment instance.
    pub fn new() -> Self {
        Msa
    }
}

impl Default for Msa {
    fn default() -> Self {
        Self::new()
    }
}

impl AssignmentMethod for Msa {
    fn assign(
        &self,
        _network: &Network,
        graph: &IndexedGraph,
        od_matrix: &dyn OdMatrix,
        vdf: &dyn VolumeDelayFunction,
        config: &AssignmentConfig,
    ) -> Result<AssignmentResult, AssignmentError> {
        log_main!(EVENT_ASSIGNMENT, "Starting MSA assignment",);

        let n = graph.num_links;

        let mut costs = vec![0.0; n];
        let mut volumes = vec![0.0; n];
        let mut aux_volumes = vec![0.0; n];

        // Step 0: Initialize with free-flow costs
        graph.compute_costs(&volumes, vdf, &mut costs);

        // Initial all-or-nothing assignment
        graph.all_or_nothing(od_matrix, &costs, &mut volumes);

        let mut converged = false;
        let mut relative_gap = f64::MAX;
        let mut iteration = 0;

        for iter in 0..config.max_iterations {
            iteration = iter + 1;

            // Update link costs
            graph.compute_costs(&volumes, vdf, &mut costs);

            // All-or-nothing with current costs
            graph.all_or_nothing(od_matrix, &costs, &mut aux_volumes);

            relative_gap = graph.relative_gap(&volumes, &costs, &aux_volumes);

            log_additional!(
                EVENT_ASSIGNMENT_ITERATION,
                "MSA iteration",
                iteration = iteration,
                gap = format!("{:.8}", relative_gap)
            );

            if relative_gap.abs() < config.convergence_gap {
                converged = true;
                break;
            }

            // MSA step: lambda = 1 / (iteration + 1)
            let lambda = 1.0 / (iteration as f64 + 1.0);

            // Update volumes: x = x + lambda * (y - x)
            for i in 0..n {
                volumes[i] += lambda * (aux_volumes[i] - volumes[i]);
            }
        }

        // Final cost computation
        graph.compute_costs(&volumes, vdf, &mut costs);

        log_main!(
            EVENT_CONVERGENCE,
            "MSA complete",
            iterations = iteration,
            gap = format!("{:.8}", relative_gap),
            converged = converged
        );

        Ok(AssignmentResult {
            link_volumes: graph.volumes_to_hashmap(&volumes),
            link_costs: graph.costs_to_hashmap(&costs),
            iterations: iteration,
            relative_gap,
            converged,
        })
    }
}
