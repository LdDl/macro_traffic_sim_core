//! # Pipeline Orchestrator
//!
//! Implementation of the 4-step traffic demand model pipeline.
//!
//! See the [module-level documentation](super) for the full description
//! of steps, feedback loop, and skim computation.

use std::collections::HashMap;

use super::error::PipelineError;
use crate::assignment::{
    AssignmentMethod, AssignmentResult, compute_link_costs, frank_wolfe::FrankWolfe,
    gradient_projection::GradientProjection, msa::Msa, shortest_path::compute_skim_matrix,
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

    log_main!(
        EVENT_PIPELINE,
        "Starting 4-step model pipeline",
        zones = zones.len(),
        links = network.link_count(),
        method = config.assignment_method.to_string()
    );

    let zone_ids: Vec<ZoneID> = zones.iter().map(|z| z.id).collect();

    // Step 1: Trip Generation
    let (productions, attractions) = trip_generator.generate(zones)?;

    log_main!(
        EVENT_PIPELINE,
        "Trip generation complete",
        total_productions = format!("{:.0}", productions.iter().sum::<f64>()),
        total_attractions = format!("{:.0}", attractions.iter().sum::<f64>())
    );

    // Initial skim from free-flow costs
    let initial_costs = compute_link_costs(network, &HashMap::new(), &config.bpr);
    let mut skim = compute_skim_matrix(network, &initial_costs, &zone_ids);

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
        total_od = gravity.distribute(&productions, &attractions, &skim, impedance, &zone_ids)?;

        log_main!(
            EVENT_PIPELINE,
            "Trip distribution complete",
            total_trips = format!("{:.0}", total_od.total())
        );

        // Step 3: Mode Choice
        let mut mode_skims: HashMap<AgentType, ModeSkim> = HashMap::new();
        let auto_time = time_skim_in_minutes(&skim, &zone_ids);
        let auto_distance = distance_skim(network, &zone_ids);

        mode_skims.insert(
            AgentType::Auto,
            ModeSkim {
                time: auto_time,
                distance: auto_distance.clone(),
                cost: DenseOdMatrix::new(zone_ids.clone()),
            },
        );

        mode_skims.insert(
            AgentType::Bike,
            ModeSkim {
                time: speed_based_time_skim(&auto_distance, &zone_ids, 15.0),
                distance: auto_distance.clone(),
                cost: DenseOdMatrix::new(zone_ids.clone()),
            },
        );

        let walk_time = speed_based_time_skim(&auto_distance, &zone_ids, 5.0);
        mode_skims.insert(
            AgentType::Walk,
            ModeSkim {
                time: walk_time,
                distance: auto_distance,
                cost: DenseOdMatrix::new(zone_ids.clone()),
            },
        );

        mode_od = logit_model.split(&total_od, &mode_skims)?;

        // Step 4: Traffic Assignment (AUTO only)
        let auto_od = mode_od.get(&AgentType::Auto).ok_or_else(|| {
            PipelineError::MissingResult("no AUTO OD matrix from mode choice".to_string())
        })?;

        assignment_result = run_assignment(network, auto_od, &config)?;

        log_main!(
            EVENT_PIPELINE,
            "Assignment complete",
            iterations = assignment_result.iterations,
            gap = format!("{:.8}", assignment_result.relative_gap),
            converged = assignment_result.converged
        );

        // Update skim from assignment costs for next feedback iteration
        if fb_iter + 1 < max_feedback {
            skim = compute_skim_matrix(network, &assignment_result.link_costs, &zone_ids);
        }

        // If this is the last iteration, return results
        if fb_iter + 1 == max_feedback {
            return Ok(PipelineResult {
                productions,
                attractions,
                total_od,
                mode_od,
                assignment: assignment_result,
                feedback_iterations_done: feedback_done,
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
    od_matrix: &dyn OdMatrix,
    config: &ModelConfig,
) -> Result<AssignmentResult, crate::assignment::error::AssignmentError> {
    match config.assignment_method {
        AssignmentMethodType::FrankWolfe => {
            let method = FrankWolfe::new();
            method.assign(network, od_matrix, &config.bpr, &config.assignment_config)
        }
        AssignmentMethodType::Msa => {
            let method = Msa::new();
            method.assign(network, od_matrix, &config.bpr, &config.assignment_config)
        }
        AssignmentMethodType::GradientProjection => {
            let method = GradientProjection::with_step_scale(config.gp_step_scale);
            method.assign(network, od_matrix, &config.bpr, &config.assignment_config)
        }
    }
}

/// Convert a skim matrix from hours to minutes.
fn time_skim_in_minutes(skim_hours: &DenseOdMatrix, zone_ids: &[ZoneID]) -> DenseOdMatrix {
    let n = zone_ids.len();
    let mut result = DenseOdMatrix::new(zone_ids.to_vec());
    for i in 0..n {
        for j in 0..n {
            let hours = skim_hours.get(zone_ids[i], zone_ids[j]);
            result.set(zone_ids[i], zone_ids[j], hours * 60.0);
        }
    }
    result
}

/// Compute a distance skim matrix (km) using Haversine distance
/// between zone centroids.
fn distance_skim(network: &Network, zone_ids: &[ZoneID]) -> DenseOdMatrix {
    let mut result = DenseOdMatrix::new(zone_ids.to_vec());

    for &oz in zone_ids {
        let o_node = match network.get_zone_centroid(oz) {
            Ok(n) => n,
            Err(_) => continue,
        };
        let o = match network.get_node(o_node) {
            Ok(n) => n,
            Err(_) => continue,
        };

        for &dz in zone_ids {
            if oz == dz {
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

            result.set(oz, dz, haversine_km(o.latitude, o.longitude, d.latitude, d.longitude));
        }
    }

    result
}

/// Haversine great-circle distance between two points in kilometers.
///
/// Coordinates are in degrees (WGS-84).
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
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
            let dist = distance.get(zone_ids[i], zone_ids[j]);
            if dist > 0.0 {
                result.set(zone_ids[i], zone_ids[j], dist / speed_kmh * 60.0);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(dist.abs() < 1e-10);
    }

    #[test]
    fn haversine_antipodal() {
        // North pole to south pole ~20015 km (half circumference)
        let dist = haversine_km(90.0, 0.0, -90.0, 0.0);
        assert!((dist - 20015.0).abs() < 100.0, "got {:.1} km", dist);
    }
}
