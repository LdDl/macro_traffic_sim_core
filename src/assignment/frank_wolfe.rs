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
    AssignmentConfig, AssignmentMethod, AssignmentResult, VolumeDelayFunction,
    compute_link_costs, compute_link_costs_into, compute_relative_gap,
    shortest_path::{all_or_nothing, all_or_nothing_into},
};

/// Pre-extracted immutable link properties for hot-loop iteration.
/// Avoids repeated HashMap lookups and method calls on Link.
struct LinkProps {
    id: LinkID,
    ff_time: f64,
    capacity: f64,
}

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
    /// Uses pre-extracted link properties to avoid repeated HashMap
    /// lookups and method calls (~40 eval calls per line search).
    fn line_search(
        link_props: &[LinkProps],
        current: &HashMap<LinkID, f64>,
        auxiliary: &HashMap<LinkID, f64>,
        vdf: &dyn VolumeDelayFunction,
    ) -> f64 {
        let mut lo = 0.0_f64;
        let mut hi = 1.0_f64;

        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;

            let eps = 1e-6;
            let obj_mid = Self::eval_at_step(link_props, current, auxiliary, vdf, mid);
            let obj_mid_plus = Self::eval_at_step(link_props, current, auxiliary, vdf, mid + eps);

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
    /// Uses pre-extracted link properties instead of iterating
    /// the network HashMap and calling methods on each Link.
    fn eval_at_step(
        link_props: &[LinkProps],
        current: &HashMap<LinkID, f64>,
        auxiliary: &HashMap<LinkID, f64>,
        vdf: &dyn VolumeDelayFunction,
        lambda: f64,
    ) -> f64 {
        let mut objective = 0.0;
        for lp in link_props {
            let cur = current.get(&lp.id).copied().unwrap_or(0.0);
            let aux = auxiliary.get(&lp.id).copied().unwrap_or(0.0);
            let vol = cur + lambda * (aux - cur);
            objective += vdf.integral(lp.ff_time, vol, lp.capacity);
        }
        objective
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

        // Pre-extract link properties once (immutable during assignment)
        let link_props: Vec<LinkProps> = network
            .links
            .iter()
            .map(|(&id, link)| LinkProps {
                id,
                ff_time: link.get_free_flow_time_hours(),
                capacity: link.get_total_capacity(),
            })
            .collect();

        // Step 0: Initialize with free-flow costs
        let mut costs = compute_link_costs(network, &HashMap::new(), vdf);

        // Initial all-or-nothing assignment
        let mut volumes = all_or_nothing(network, od_matrix, &costs)?;

        // Pre-allocate auxiliary volumes (reused every iteration)
        let mut aux_volumes: HashMap<LinkID, f64> = HashMap::with_capacity(network.links.len());
        for &link_id in network.links.keys() {
            aux_volumes.insert(link_id, 0.0);
        }

        let mut converged = false;
        let mut relative_gap = f64::MAX;
        let mut iteration = 0;

        for iter in 0..config.max_iterations {
            iteration = iter + 1;

            // Update link costs with current volumes (reuse allocation)
            compute_link_costs_into(network, &volumes, vdf, &mut costs);

            // All-or-nothing with current costs (reuse allocation)
            all_or_nothing_into(network, od_matrix, &costs, &mut aux_volumes)?;

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
            let lambda = Self::line_search(&link_props, &volumes, &aux_volumes, vdf);

            // Update volumes: x = x + lambda * (y - x)
            for (&link_id, vol) in volumes.iter_mut() {
                let aux_vol = aux_volumes.get(&link_id).copied().unwrap_or(0.0);
                *vol += lambda * (aux_vol - *vol);
            }
        }

        // Final cost computation (reuse allocation)
        compute_link_costs_into(network, &volumes, vdf, &mut costs);

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
