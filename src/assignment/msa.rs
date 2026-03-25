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

use std::collections::HashMap;

use super::error::AssignmentError;
use crate::gmns::meso::network::Network;
use crate::log_additional;
use crate::log_main;
use crate::od::OdMatrix;
use crate::verbose::{EVENT_ASSIGNMENT, EVENT_ASSIGNMENT_ITERATION, EVENT_CONVERGENCE};

use super::{
    AssignmentConfig, AssignmentMethod, AssignmentResult, VolumeDelayFunction, compute_link_costs,
    compute_relative_gap, shortest_path::all_or_nothing,
};

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
        network: &Network,
        od_matrix: &dyn OdMatrix,
        vdf: &dyn VolumeDelayFunction,
        config: &AssignmentConfig,
    ) -> Result<AssignmentResult, AssignmentError> {
        log_main!(EVENT_ASSIGNMENT, "Starting MSA assignment",);

        // Step 0: Initialize with free-flow costs
        let mut costs = compute_link_costs(network, &HashMap::new(), vdf);

        // Initial all-or-nothing assignment
        let mut volumes = all_or_nothing(network, od_matrix, &costs)?;

        let mut converged = false;
        let mut relative_gap = f64::MAX;
        let mut iteration = 0;

        for iter in 0..config.max_iterations {
            iteration = iter + 1;

            // Update link costs
            costs = compute_link_costs(network, &volumes, vdf);

            // All-or-nothing with current costs
            let aux_volumes = all_or_nothing(network, od_matrix, &costs)?;

            relative_gap = compute_relative_gap(&volumes, &costs, &aux_volumes);

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
            // +1 because iteration starts at 1, and first AoN was iteration 0
            let lambda = 1.0 / (iteration as f64 + 1.0);

            // Update volumes: x = x + lambda * (y - x)
            for (&link_id, vol) in volumes.iter_mut() {
                let aux_vol = aux_volumes.get(&link_id).copied().unwrap_or(0.0);
                *vol += lambda * (aux_vol - *vol);
            }
        }

        // Final cost computation
        costs = compute_link_costs(network, &volumes, vdf);

        log_main!(
            EVENT_CONVERGENCE,
            "MSA complete",
            iterations = iteration,
            gap = format!("{:.8}", relative_gap),
            converged = converged
        );

        Ok(AssignmentResult {
            link_volumes: volumes,
            link_costs: costs,
            iterations: iteration,
            relative_gap,
            converged,
        })
    }
}
