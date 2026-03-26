//! # Pipeline Orchestrator
//!
//! Implementation of the 4-step traffic demand model pipeline.
//!
//! See the [module-level documentation](super) for the full description
//! of steps, feedback loop, and skim computation.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::error::PipelineError;
use crate::assignment::{
    AssignmentMethod, AssignmentResult, IndexedGraph, frank_wolfe::FrankWolfe,
    gradient_projection::GradientProjection, msa::Msa,
};
use crate::config::{AssignmentMethodType, ModelConfig};
use crate::error::SimError;
use crate::gmns::meso::network::Network;
use crate::gmns::types::{AgentType, ZoneID};
use crate::log_main;
use crate::mode_choice::logit::{ModeSkim, MultinomialLogit};
use crate::od::OdMatrix;
use crate::od::dense::DenseOdMatrix;
use crate::trip_distribution::gravity::GravityModel;
use crate::trip_distribution::impedance::ImpedanceFunction;
use crate::trip_generation::TripGenerator;
use crate::verbose::{EVENT_FEEDBACK_LOOP, EVENT_PIPELINE, set_verbose_level};
use crate::zone::Zone;

const EARTH_RADIUS_KM: f64 = 6371.0;

/// Timing information for each pipeline step.
///
/// All durations are cumulative across feedback iterations.
/// For example, if 3 feedback loops ran, `distribution` is the
/// total time spent in all 3 distribution passes.
#[derive(Debug, Clone)]
pub struct PipelineTimings {
    /// Trip generation (step 1, runs once).
    pub generation: Duration,
    /// Trip distribution (step 2, cumulative across feedback loops).
    pub distribution: Duration,
    /// Mode choice (step 3, cumulative across feedback loops).
    pub mode_choice: Duration,
    /// Traffic assignment (step 4, cumulative across feedback loops).
    pub assignment: Duration,
    /// Total pipeline wall time.
    pub total: Duration,
}

/// Result of the complete 4-step model pipeline.
///
/// Contains all intermediate and final results so callers can
/// inspect any stage of the model.
#[derive(Debug)]
pub struct PipelineResult {
    /// Trip productions per zone (one entry per zone, same order as input).
    pub productions: Vec<f64>,
    /// Trip attractions per zone (one entry per zone, same order as input).
    pub attractions: Vec<f64>,
    /// Total OD matrix after trip distribution (all modes combined).
    pub total_od: DenseOdMatrix,
    /// Per-mode OD matrices after mode choice split.
    pub mode_od: HashMap<AgentType, DenseOdMatrix>,
    /// Traffic assignment result (AUTO mode only).
    /// Contains link volumes, link costs, iteration count, and
    /// convergence status.
    pub assignment: AssignmentResult,
    /// Number of feedback iterations actually performed.
    pub feedback_iterations_done: usize,
    /// Per-step timing breakdown.
    pub timings: PipelineTimings,
}

/// Run the complete 4-step traffic demand model.
///
/// This is the main entry point for the pipeline. It wires together
/// all four steps with an outer feedback loop that updates travel
/// time skims from congested assignment costs.
///
/// # Arguments
/// * `network` - Mesoscopic network with turn restrictions encoded
///   in the topology (connection links for allowed movements).
/// * `zones` - Transport analysis zones with socioeconomic data.
/// * `trip_generator` - Trip generation method (regression or
///   cross-classification).
/// * `impedance` - Impedance function for the gravity model
///   (exponential, power, or combined).
/// * `logit_model` - Multinomial logit mode choice model with
///   utility functions per mode.
/// * `config` - Model configuration (assignment method, BPR
///   parameters, convergence thresholds, feedback iterations).
///
/// # Returns
/// A [`PipelineResult`] with all intermediate and final results.
///
/// # Errors
/// Returns [`SimError`] if any step fails (e.g., no zones,
/// Furness not converged, assignment issues).
pub fn run_four_step_model(
    network: &Network,
    zones: &[Zone],
    trip_generator: &dyn TripGenerator,
    impedance: &dyn ImpedanceFunction,
    logit_model: &MultinomialLogit,
    config: &ModelConfig,
) -> Result<PipelineResult, SimError> {
    set_verbose_level(config.verbose_level);

    let pipeline_start = Instant::now();

    log_main!(
        EVENT_PIPELINE,
        "Starting 4-step model pipeline",
        zones = zones.len(),
        links = network.link_count(),
        method = config.assignment_method.to_string()
    );

    let zone_ids: Vec<ZoneID> = zones.iter().map(|z| z.id).collect();

    let t_generation;
    let mut t_distribution = Duration::ZERO;
    let mut t_mode_choice = Duration::ZERO;
    let mut t_assignment = Duration::ZERO;

    // Step 1: Trip Generation
    let step_start = Instant::now();
    let (productions, attractions) = trip_generator.generate(zones)?;
    t_generation = step_start.elapsed();

    log_main!(
        EVENT_PIPELINE,
        "Trip generation complete",
        total_productions = format!("{:.0}", productions.iter().sum::<f64>()),
        total_attractions = format!("{:.0}", attractions.iter().sum::<f64>()),
        elapsed_ms = format!("{:.3}", step_start.elapsed().as_secs_f64() * 1000.0)
    );

    // Build indexed graph once for skim computation
    let igraph = IndexedGraph::from_network(network);
    let mut skim_costs = vec![0.0; igraph.num_links];
    igraph.compute_costs(&vec![0.0; igraph.num_links], &config.bpr, &mut skim_costs);
    let mut skim = igraph.compute_skim(&skim_costs, &zone_ids);

    let gravity = GravityModel::with_furness_config(config.furness_config.clone());

    let mut total_od;
    let mut mode_od;
    let mut assignment_result;

    let max_feedback = config.feedback_iterations.max(1);
    let mut feedback_done;

    for fb_iter in 0..max_feedback {
        feedback_done = fb_iter + 1;

        log_main!(
            EVENT_FEEDBACK_LOOP,
            "Feedback iteration",
            iteration = feedback_done,
            total = max_feedback
        );

        // Step 2: Trip Distribution
        let step_start = Instant::now();
        total_od = gravity.distribute(&productions, &attractions, &skim, impedance, &zone_ids)?;
        t_distribution += step_start.elapsed();

        log_main!(
            EVENT_PIPELINE,
            "Trip distribution complete",
            total_trips = format!("{:.0}", total_od.total()),
            elapsed_ms = format!("{:.3}", step_start.elapsed().as_secs_f64() * 1000.0)
        );

        // Step 3: Mode Choice
        let step_start = Instant::now();
        let mut mode_skims: HashMap<AgentType, ModeSkim> = HashMap::with_capacity(3);
        let auto_time = time_skim_in_minutes(&skim, &zone_ids);
        let auto_distance = distance_skim(network, &zone_ids);
        let zero_cost = DenseOdMatrix::new(zone_ids.clone());

        // Compute time skims before moving auto_distance
        let bike_time = speed_based_time_skim(&auto_distance, &zone_ids, 15.0);
        let walk_time = speed_based_time_skim(&auto_distance, &zone_ids, 5.0);

        mode_skims.insert(
            AgentType::Walk,
            ModeSkim {
                time: walk_time,
                distance: auto_distance.clone(),
                cost: zero_cost.clone(),
            },
        );

        mode_skims.insert(
            AgentType::Bike,
            ModeSkim {
                time: bike_time,
                distance: auto_distance.clone(),
                cost: zero_cost.clone(),
            },
        );

        mode_skims.insert(
            AgentType::Auto,
            ModeSkim {
                time: auto_time,
                distance: auto_distance,
                cost: zero_cost,
            },
        );

        mode_od = logit_model.split(&total_od, &mode_skims)?;
        t_mode_choice += step_start.elapsed();

        log_main!(
            EVENT_PIPELINE,
            "Mode choice complete",
            elapsed_ms = format!("{:.3}", step_start.elapsed().as_secs_f64() * 1000.0)
        );

        // Step 4: Traffic Assignment (AUTO only)
        let auto_od = mode_od.get(&AgentType::Auto).ok_or_else(|| {
            PipelineError::MissingResult("no AUTO OD matrix from mode choice".to_string())
        })?;

        let step_start = Instant::now();
        assignment_result = run_assignment(network, &igraph, auto_od, &config)?;
        t_assignment += step_start.elapsed();

        log_main!(
            EVENT_PIPELINE,
            "Assignment complete",
            iterations = assignment_result.iterations,
            gap = format!("{:.8}", assignment_result.relative_gap),
            converged = assignment_result.converged,
            elapsed_ms = format!("{:.3}", step_start.elapsed().as_secs_f64() * 1000.0)
        );

        // Update skim from assignment costs for next feedback iteration
        if fb_iter + 1 < max_feedback {
            // Convert assignment link_costs HashMap to indexed Vec
            for i in 0..igraph.num_links {
                let lid = igraph.link_id(i);
                skim_costs[i] = assignment_result.link_costs.get(&lid).copied().unwrap_or(0.0);
            }
            skim = igraph.compute_skim(&skim_costs, &zone_ids);
        }

        // If this is the last iteration, return results
        if fb_iter + 1 == max_feedback {
            log_main!(
                EVENT_PIPELINE,
                "Pipeline complete",
                feedback_iterations = feedback_done,
                elapsed_ms = format!("{:.3}", pipeline_start.elapsed().as_secs_f64() * 1000.0)
            );

            return Ok(PipelineResult {
                productions,
                attractions,
                total_od,
                mode_od,
                assignment: assignment_result,
                feedback_iterations_done: feedback_done,
                timings: PipelineTimings {
                    generation: t_generation,
                    distribution: t_distribution,
                    mode_choice: t_mode_choice,
                    assignment: t_assignment,
                    total: pipeline_start.elapsed(),
                },
            });
        }
    }

    unreachable!()
}

/// Run traffic assignment with the configured method.
///
/// Dispatches to the appropriate algorithm based on
/// [`AssignmentMethodType`](crate::config::AssignmentMethodType)
/// in the model config.
fn run_assignment(
    network: &Network,
    graph: &IndexedGraph,
    od_matrix: &dyn OdMatrix,
    config: &ModelConfig,
) -> Result<AssignmentResult, crate::assignment::error::AssignmentError> {
    match config.assignment_method {
        AssignmentMethodType::FrankWolfe => {
            let method = FrankWolfe::new();
            method.assign(network, graph, od_matrix, &config.bpr, &config.assignment_config)
        }
        AssignmentMethodType::Msa => {
            let method = Msa::new();
            method.assign(network, graph, od_matrix, &config.bpr, &config.assignment_config)
        }
        AssignmentMethodType::GradientProjection => {
            let method = GradientProjection::with_step_scale(config.gp_step_scale);
            method.assign(network, graph, od_matrix, &config.bpr, &config.assignment_config)
        }
    }
}

/// Convert a skim matrix from hours to minutes.
fn time_skim_in_minutes(skim_hours: &DenseOdMatrix, zone_ids: &[ZoneID]) -> DenseOdMatrix {
    let n = zone_ids.len();
    let mut result = DenseOdMatrix::new(zone_ids.to_vec());
    for i in 0..n {
        for j in 0..n {
            result.set_by_index(i, j, skim_hours.get_by_index(i, j) * 60.0);
        }
    }
    result
}

/// Compute a distance skim matrix (km) using Haversine distance
/// between zone centroids.
fn distance_skim(network: &Network, zone_ids: &[ZoneID]) -> DenseOdMatrix {
    let mut result = DenseOdMatrix::new(zone_ids.to_vec());

    for (i, &oz) in zone_ids.iter().enumerate() {
        let o_node = match network.get_zone_centroid(oz) {
            Ok(n) => n,
            Err(_) => continue,
        };
        let o = match network.get_node(o_node) {
            Ok(n) => n,
            Err(_) => continue,
        };

        for (j, &dz) in zone_ids.iter().enumerate() {
            if i == j {
                continue;
            }
            let d_node = match network.get_zone_centroid(dz) {
                Ok(n) => n,
                Err(_) => continue,
            };
            let d = match network.get_node(d_node) {
                Ok(n) => n,
                Err(_) => continue,
            };

            result.set_by_index(i, j, haversine_km(o.latitude, o.longitude, d.latitude, d.longitude));
        }
    }

    result
}

/// Haversine great-circle distance between two points in kilometers.
///
/// Coordinates are in degrees (WGS-84).
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::pipeline::haversine_km;
///
/// // Moscow to Saint Petersburg: ~634 km
/// let dist = haversine_km(55.7558, 37.6173, 59.9343, 30.3351);
/// assert!((dist - 634.0).abs() < 5.0);
/// ```
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    EARTH_RADIUS_KM * c
}

/// Compute time skim from distance skim and a fixed speed (km/h).
///
/// Result is in minutes. Used for non-motorized modes
/// (BIKE at 15 km/h, WALK at 5 km/h).
fn speed_based_time_skim(
    distance: &DenseOdMatrix,
    zone_ids: &[ZoneID],
    speed_kmh: f64,
) -> DenseOdMatrix {
    let n = zone_ids.len();
    let mut result = DenseOdMatrix::new(zone_ids.to_vec());
    for i in 0..n {
        for j in 0..n {
            let dist = distance.get_by_index(i, j);
            if dist > 0.0 {
                result.set_by_index(i, j, dist / speed_kmh * 60.0);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-10;

    #[test]
    fn haversine_moscow_to_saint_petersburg() {
        // Moscow (55.7558, 37.6173) -> Saint Petersburg (59.9343, 30.3351)
        // Expected ~634 km
        let dist = haversine_km(55.7558, 37.6173, 59.9343, 30.3351);
        assert!((dist - 634.0).abs() < 5.0, "got {:.1} km", dist);
    }

    #[test]
    fn haversine_same_point_is_zero() {
        let dist = haversine_km(48.8566, 2.3522, 48.8566, 2.3522);
        assert!(dist.abs() < EPS);
    }

    #[test]
    fn haversine_antipodal() {
        // North pole to south pole ~20015 km (half circumference)
        let dist = haversine_km(90.0, 0.0, -90.0, 0.0);
        assert!((dist - 20015.0).abs() < 100.0, "got {:.1} km", dist);
    }
}
