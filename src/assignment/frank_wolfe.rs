//! # Frank-Wolfe Algorithm
//!
//! Traffic assignment using the Frank-Wolfe (convex combinations) method
//! with bisection line search to find User Equilibrium.
//!
//! ## Algorithm
//!
//! 1. Initialize with free-flow costs and all-or-nothing (AoN) loading.
//! 2. At each iteration:
//!    a. Compute link costs from current volumes via the VDF.
//!    b. Perform AoN assignment at current costs to get auxiliary volumes `y`.
//!    c. Check the relative gap -- if below threshold, stop.
//!    d. Find step size `lambda` via bisection line search on the
//!       Beckmann objective: `min Z(x + lambda * (y - x))`, `lambda in [0, 1]`.
//!    e. Update volumes: `x = x + lambda * (y - x)`.
//!
//! ## Bisection line search
//!
//! The line search finds the `lambda` that minimizes the Beckmann
//! objective along the direction `(y - x)`. It evaluates the objective
//! at `mid` and `mid + eps` to determine the gradient sign, then
//! narrows the bracket. Runs up to 20 iterations or until the
//! bracket width falls below `1e-8`.
//!
//! ## Convergence
//!
//! Frank-Wolfe typically converges faster than MSA because the
//! step size is optimized at each iteration rather than fixed.
//! However, convergence slows near the optimum (sublinear rate).
//!
//! ## Trade-offs
//!
//! - No path storage required (link-based).
//! - Each iteration requires one shortest path computation (AoN)
//!   plus several Beckmann objective evaluations for the line search.
//! - Cannot extract path-level information (use [`GradientProjection`]
//!   if paths are needed).

use std::collections::HashMap;

use super::error::AssignmentError;
use crate::gmns::meso::network::Network;
use crate::gmns::types::LinkID;

use crate::log_additional;
use crate::log_main;
use crate::od::OdMatrix;
use crate::verbose::{EVENT_ASSIGNMENT, EVENT_ASSIGNMENT_ITERATION, EVENT_CONVERGENCE};

use super::{
    AssignmentConfig, AssignmentMethod, AssignmentResult, VolumeDelayFunction, beckmann_objective,
    compute_link_costs, compute_relative_gap, shortest_path::all_or_nothing,
};

/// Frank-Wolfe traffic assignment algorithm.
///
/// Uses convex combinations with bisection line search on the
/// Beckmann objective to find User Equilibrium.
///
/// This is the most common assignment method in practice. It is
/// link-based (does not store paths) and uses the Beckmann
/// transformation to cast UE as a convex optimization problem.
///
/// # Usage
///
/// ```text
/// let fw = FrankWolfe::new();
/// let result = fw.assign(&network, &od_matrix, &bpr, &config)?;
/// // result.link_volumes -- equilibrium volumes
/// // result.converged    -- true if gap < threshold
/// ```
#[derive(Debug)]
pub struct FrankWolfe;

impl FrankWolfe {
    /// Create a new Frank-Wolfe assignment instance.
    pub fn new() -> Self {
        FrankWolfe
    }

    /// Bisection line search to find the optimal step size.
    ///
    /// Minimizes `Z(x + lambda * (y - x))` over `lambda in [0, 1]`,
    /// where `Z` is the Beckmann objective function.
    ///
    /// The search evaluates the objective at two points (`mid` and
    /// `mid + eps`) to approximate the gradient sign, then narrows
    /// the bracket accordingly. Terminates after 20 iterations or
    /// when the bracket width is below `1e-8`.
    fn line_search(
        network: &Network,
        current: &HashMap<LinkID, f64>,
        auxiliary: &HashMap<LinkID, f64>,
        vdf: &dyn VolumeDelayFunction,
    ) -> f64 {
        let mut lo = 0.0_f64;
        let mut hi = 1.0_f64;

        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;

            let eps = 1e-6;
            let obj_mid = Self::eval_at_step(network, current, auxiliary, vdf, mid);
            let obj_mid_plus = Self::eval_at_step(network, current, auxiliary, vdf, mid + eps);

            if obj_mid_plus > obj_mid {
                hi = mid;
            } else {
                lo = mid;
            }

            if (hi - lo) < 1e-8 {
                break;
            }
        }

        (lo + hi) / 2.0
    }

    /// Evaluate the Beckmann objective at a given step size.
    ///
    /// Computes volumes as `x + lambda * (y - x)` and returns `Z(volumes)`.
    fn eval_at_step(
        network: &Network,
        current: &HashMap<LinkID, f64>,
        auxiliary: &HashMap<LinkID, f64>,
        vdf: &dyn VolumeDelayFunction,
        lambda: f64,
    ) -> f64 {
        let mut combined = HashMap::new();
        for (&link_id, &vol) in current {
            let aux_vol = auxiliary.get(&link_id).copied().unwrap_or(0.0);
            combined.insert(link_id, vol + lambda * (aux_vol - vol));
        }
        beckmann_objective(network, &combined, vdf)
    }
}

impl Default for FrankWolfe {
    fn default() -> Self {
        Self::new()
    }
}

impl AssignmentMethod for FrankWolfe {
    fn assign(
        &self,
        network: &Network,
        od_matrix: &dyn OdMatrix,
        vdf: &dyn VolumeDelayFunction,
        config: &AssignmentConfig,
    ) -> Result<AssignmentResult, AssignmentError> {
        log_main!(EVENT_ASSIGNMENT, "Starting Frank-Wolfe assignment",);

        // Step 0: Initialize with free-flow costs
        let mut costs = compute_link_costs(network, &HashMap::new(), vdf);

        // Initial all-or-nothing assignment
        let mut volumes = all_or_nothing(network, od_matrix, &costs)?;

        let mut converged = false;
        let mut relative_gap = f64::MAX;
        let mut iteration = 0;

        for iter in 0..config.max_iterations {
            iteration = iter + 1;

            // Update link costs with current volumes
            costs = compute_link_costs(network, &volumes, vdf);

            // All-or-nothing with current costs
            let aux_volumes = all_or_nothing(network, od_matrix, &costs)?;

            relative_gap = compute_relative_gap(&volumes, &costs, &aux_volumes);

            log_additional!(
                EVENT_ASSIGNMENT_ITERATION,
                "Frank-Wolfe iteration",
                iteration = iteration,
                gap = format!("{:.8}", relative_gap)
            );

            if relative_gap.abs() < config.convergence_gap {
                converged = true;
                break;
            }

            // Line search for optimal step size
            let lambda = Self::line_search(network, &volumes, &aux_volumes, vdf);

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
            "Frank-Wolfe complete",
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
