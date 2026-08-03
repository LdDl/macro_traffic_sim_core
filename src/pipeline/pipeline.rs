//! # Pipeline Orchestrator
//!
//! Implementation of the 4-step traffic demand model pipeline.
//!
//! See the [module-level documentation](super) for the full description
//! of steps, feedback loop, and skim computation.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::connectivity::zone_scc;
use super::error::{InvalidInputReason, PipelineError};
use super::phase::{PipelinePhase, ProgressEvent};
use crate::assignment::{
    AssignmentMethod, AssignmentResult, IndexedGraph, frank_wolfe::FrankWolfe,
    gradient_projection::GradientProjection, msa::Msa, multiclass,
};
use crate::config::{AssignmentMethodType, ModelConfig};
use crate::error::SimError;
use crate::gmns::meso::network::Network;
use crate::gmns::types::{AgentType, LinkID, ZoneID};
use crate::mode_choice::logit::{ModeSkim, MultinomialLogit};
use crate::od::OdMatrix;
use crate::od::dense::DenseOdMatrix;
use crate::transit::{
    TransitAssignmentOptions, TransitAssignmentResult, TransitNetwork, assign_transit_with_options,
    congest_transit_network, transit_road_preload, transit_skim_with_options,
};
use crate::trip_distribution::gravity::GravityModel;
use crate::trip_distribution::impedance::ImpedanceFunction;
use crate::trip_generation::TripGenerator;
use crate::verbose::{EVENT_FEEDBACK_LOOP, EVENT_PIPELINE, EVENT_PREFLIGHT, set_verbose_level};
use crate::zone::Zone;
use crate::{log_additional, log_main};

const EARTH_RADIUS_KM: f64 = 6371.0;
const EPS_DEMAND_FRACTION: f64 = 1e-6;

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

/// Optional public transit layer for the pipeline.
///
/// When supplied to [`run_four_step_model`], mode choice gains a TRANSIT
/// alternative (the caller's `logit_model` must include a
/// [`AgentType::Transit`](crate::gmns::types::AgentType) utility), and the
/// resulting transit demand is assigned with the optimal strategies
/// algorithm.
///
/// The transit `network` must use the same zone IDs as the road zones for
/// its access points (zone centroid = zone ID), so a single OD matrix
/// splits cleanly across road and transit modes.
pub struct TransitInput<'a> {
    /// Transit routes and (zone-access) walk links.
    pub network: &'a TransitNetwork,
    /// Assignment options (waiting factor, penalties, dwell).
    pub options: TransitAssignmentOptions,
    /// Fixed exogenous transit demand added on top of the mode-choice
    /// share before assignment, `None` for none. Use it for demand that
    /// must not go through mode choice - captive riders with no car, a
    /// known external matrix, a scenario constraint. It is added to (not
    /// substituted for) the endogenous transit OD, so a logit without a
    /// transit alternative simply assigns this matrix alone. Must use the
    /// road zone IDs; entries on other zones are ignored.
    pub fixed_od: Option<&'a dyn OdMatrix>,
    /// Analysis period length (same time unit as the route headways) that
    /// turns on the transit vehicle load on the road: buses on their
    /// `segment_links` become a fixed background PCU in the road
    /// assignment, congesting the roads they share with cars. `None`
    /// leaves the road assignment unaffected by transit. Only routes that
    /// declare `segment_links` contribute; see
    /// [`transit_road_preload`](crate::transit::transit_road_preload).
    pub analysis_period: Option<f64>,
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
    /// convergence status. This is the result of the LAST feedback
    /// iteration.
    pub assignment: AssignmentResult,
    /// Assignment result per feedback iteration (index 0 = first iteration).
    /// Useful for comparing convergence with/without warm start.
    pub per_feedback_assignments: Vec<AssignmentResult>,
    /// Number of feedback iterations actually performed.
    pub feedback_iterations_done: usize,
    /// Public transit assignment result, `None` when no transit layer was
    /// supplied. Contains the optimal-strategies volumes, OD costs and
    /// boardings for the final transit demand from mode choice.
    pub transit: Option<TransitAssignmentResult>,
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
///   utility functions per mode. Include an
///   [`AgentType::Transit`](crate::gmns::types::AgentType) utility to
///   enable the transit alternative (requires `transit` to be `Some`).
/// * `config` - Model configuration (assignment method, BPR
///   parameters, convergence thresholds, feedback iterations).
/// * `transit` - Optional public transit layer. When `Some`, mode choice
///   gains a transit alternative fed by a transit skim, and the resulting
///   transit demand is assigned with the optimal strategies algorithm.
///
/// # Returns
/// A [`PipelineResult`] with all intermediate and final results (including
/// `transit` when a transit layer was supplied).
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
    transit: Option<TransitInput>,
    on_progress: Option<&dyn Fn(ProgressEvent)>,
) -> Result<PipelineResult, SimError> {
    set_verbose_level(config.verbose_level);

    let notify = |event: ProgressEvent| {
        if let Some(cb) = on_progress {
            cb(event);
        }
    };

    // Catch bad inputs before any computation.
    notify(ProgressEvent::single(PipelinePhase::Preflight));
    preflight_check(network, zones, trip_generator)?;

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
    notify(ProgressEvent::single(PipelinePhase::Generation));
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

    // Normalize attractions to match production total before Furness.
    // Regression coefficients for productions and attractions are calibrated
    // independently, so sum(P) != sum(A) is normal. Furness requires equal totals.
    let sum_p: f64 = productions.iter().sum();
    let sum_a: f64 = attractions.iter().sum();
    let attractions: Vec<f64> = if (sum_p - sum_a).abs() > 1e-6 * sum_p.max(sum_a) {
        let factor = sum_p / sum_a;
        log_main!(
            EVENT_PIPELINE,
            "Normalizing attractions to match production total",
            sum_productions = format!("{:.3}", sum_p),
            sum_attractions = format!("{:.3}", sum_a),
            normalization_factor = format!("{:.6}", factor)
        );
        attractions.iter().map(|a| a * factor).collect()
    } else {
        attractions
    };

    // Build indexed graph once for skim computation
    let mut igraph = IndexedGraph::from_network(network);
    // Transit vehicles as a fixed background load on the road they share
    // with cars. Flow-independent (headways are inputs), so set once here
    // and reused by every assignment iteration.
    if let Some(t) = &transit
        && let Some(period) = t.analysis_period
    {
        let preload = transit_road_preload(t.network, period)?;
        igraph.set_background_pcu(&preload);
    }
    let mut skim_costs = vec![0.0; igraph.num_links];
    igraph.compute_costs(&vec![0.0; igraph.num_links], &config.bpr, &mut skim_costs)?;
    #[cfg(feature = "parallel")]
    let mut skim = igraph.compute_skim_parallel(&skim_costs, &zone_ids);
    #[cfg(not(feature = "parallel"))]
    let mut skim = igraph.compute_skim(&skim_costs, &zone_ids);

    let gravity = GravityModel::with_furness_config(config.furness_config.clone());

    // Distance skim is invariant across feedback iterations (geometry doesn't change).
    // Wrapped in Rc to share across all 3 mode skims without cloning.
    let distance_skim_rc = Rc::new(distance_skim(network, &zone_ids));

    // When any transit route runs on road links, its in-vehicle times react
    // to road congestion, so the transit skim is recomputed each feedback
    // iteration from the congested link costs. Otherwise the Spiess-Florian
    // costs are flow-independent and the skim is computed once and reused.
    let transit_congested = transit
        .as_ref()
        .is_some_and(|t| t.network.routes.iter().any(|r| r.segment_links.is_some()));
    // Free-flow road link times, used to scale transit segment times by the
    // road congestion factor (only when congested transit is active).
    let ff_link_time: HashMap<LinkID, f64> = if transit_congested {
        (0..igraph.num_links)
            .map(|i| (igraph.link_id(i), igraph.link_ff_time[i]))
            .collect()
    } else {
        HashMap::new()
    };
    // The transit network with congested in-vehicle times, rebuilt each
    // iteration; `None` until the first road assignment (or when transit
    // runs on its own right-of-way and never congests).
    let mut congested_transit_net: Option<TransitNetwork> = None;
    // Transit skim, recomputed each iteration when congested transit is on.
    let mut transit_skim_map = match &transit {
        Some(t) => Some(transit_skim_with_options(t.network, &zone_ids, &t.options)?),
        None => None,
    };

    let mut total_od;
    let mut mode_od;
    let mut assignment_result;
    let mut prev_volumes: Option<HashMap<LinkID, f64>> = None;
    let mut prev_class_volumes: Option<HashMap<String, HashMap<LinkID, f64>>> = None;
    let mut per_feedback_assignments: Vec<AssignmentResult> = Vec::new();

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
        notify(ProgressEvent::feedback(
            PipelinePhase::Distribution,
            feedback_done,
            max_feedback,
        ));
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
        notify(ProgressEvent::feedback(
            PipelinePhase::ModeChoice,
            feedback_done,
            max_feedback,
        ));
        let step_start = Instant::now();
        let mut mode_skims: HashMap<AgentType, ModeSkim> = HashMap::with_capacity(3);
        let auto_time = time_skim_in_minutes(&skim, &zone_ids);
        let zero_cost = Rc::new(DenseOdMatrix::new(zone_ids.clone()));

        let bike_time = speed_based_time_skim(&distance_skim_rc, &zone_ids, 15.0);
        let walk_time = speed_based_time_skim(&distance_skim_rc, &zone_ids, 5.0);

        mode_skims.insert(
            AgentType::Walk,
            ModeSkim {
                time: walk_time,
                distance: Rc::clone(&distance_skim_rc),
                cost: Rc::clone(&zero_cost),
            },
        );

        mode_skims.insert(
            AgentType::Bike,
            ModeSkim {
                time: bike_time,
                distance: Rc::clone(&distance_skim_rc),
                cost: Rc::clone(&zero_cost),
            },
        );

        mode_skims.insert(
            AgentType::Auto,
            ModeSkim {
                time: auto_time,
                distance: Rc::clone(&distance_skim_rc),
                cost: Rc::clone(&zero_cost),
            },
        );

        // Transit alternative: the flow-independent skim built once above.
        // Missing pairs are unavailable (infinite time -> zero share).
        if let Some(map) = &transit_skim_map {
            mode_skims.insert(
                AgentType::Transit,
                ModeSkim::from_time_map(
                    &zone_ids,
                    map,
                    Rc::clone(&distance_skim_rc),
                    Rc::clone(&zero_cost),
                ),
            );
        }

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

        notify(ProgressEvent::feedback(
            PipelinePhase::Assignment,
            feedback_done,
            max_feedback,
        ));
        let step_start = Instant::now();

        assignment_result = if let Some(ref uc) = config.user_classes {
            let fraction_sum: f64 = uc.iter().map(|c| c.demand_fraction).sum();
            if (fraction_sum - 1.0).abs() > EPS_DEMAND_FRACTION {
                return Err(SimError::from(
                    crate::assignment::error::AssignmentError::InvalidConfig(format!(
                        "demand_fraction values must sum to 1.0, got {:.6}",
                        fraction_sum,
                    )),
                ));
            }
            let classes: Vec<_> = uc.iter().map(|c| c.to_user_class()).collect();
            let class_ods: Vec<DenseOdMatrix> = uc
                .iter()
                .map(|c| scale_od(auto_od, c.demand_fraction))
                .collect();
            let od_refs: Vec<&dyn OdMatrix> =
                class_ods.iter().map(|o| o as &dyn OdMatrix).collect();
            let warm_cv = if config.warm_start {
                prev_class_volumes.as_ref()
            } else {
                None
            };
            let result = match config.assignment_method {
                AssignmentMethodType::FrankWolfe => multiclass::assign_multiclass_fw(
                    &igraph,
                    &classes,
                    &od_refs,
                    &config.bpr,
                    &config.assignment_config,
                    warm_cv,
                )?,
                AssignmentMethodType::Msa => multiclass::assign_multiclass_msa(
                    &igraph,
                    &classes,
                    &od_refs,
                    &config.bpr,
                    &config.assignment_config,
                    warm_cv,
                )?,
                AssignmentMethodType::GradientProjection => {
                    return Err(SimError::from(
                        crate::assignment::error::AssignmentError::InvalidConfig(
                            "gradient projection does not support multi-class assignment"
                                .to_string(),
                        ),
                    ));
                }
            };
            prev_class_volumes = result.class_volumes.clone();
            result
        } else {
            let warm_vols = if config.warm_start {
                prev_volumes.as_ref()
            } else {
                None
            };
            run_assignment(network, &igraph, auto_od, &config, warm_vols)?
        };

        prev_volumes = Some(assignment_result.link_volumes.clone());
        per_feedback_assignments.push(assignment_result.clone());
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
                skim_costs[i] = assignment_result
                    .link_costs
                    .get(&lid)
                    .copied()
                    .unwrap_or(0.0);
            }
            #[cfg(feature = "parallel")]
            {
                skim = igraph.compute_skim_parallel(&skim_costs, &zone_ids);
            }
            #[cfg(not(feature = "parallel"))]
            {
                skim = igraph.compute_skim(&skim_costs, &zone_ids);
            }
        }

        // Recompute the transit skim from the congested road times: buses on
        // those links are slowed, shifting the transit level of service.
        // Runs every iteration - the updated skim feeds the next mode choice,
        // and the congested network is used for the final transit assignment.
        if transit_congested && let Some(t) = &transit {
            let net =
                congest_transit_network(t.network, &ff_link_time, &assignment_result.link_costs);
            transit_skim_map = Some(transit_skim_with_options(&net, &zone_ids, &t.options)?);
            congested_transit_net = Some(net);
        }

        // If this is the last iteration, return results
        if fb_iter + 1 == max_feedback {
            // Assign the final transit demand with optimal strategies. Done
            // once here (not per feedback iteration) since only the last
            // split matters. The demand is the mode-choice transit share
            // (if any) plus the optional fixed exogenous matrix; skipped
            // when both are empty.
            let transit_result = match &transit {
                Some(t) => {
                    let mut transit_od = mode_od
                        .get(&AgentType::Transit)
                        .cloned()
                        .unwrap_or_else(|| DenseOdMatrix::new(zone_ids.clone()));
                    if let Some(fixed) = t.fixed_od {
                        for &o in &zone_ids {
                            for &d in &zone_ids {
                                let extra = fixed.get(o, d);
                                if extra != 0.0 {
                                    let current = transit_od.get(o, d);
                                    transit_od.set(o, d, current + extra);
                                }
                            }
                        }
                    }
                    if transit_od.total() > 0.0 {
                        let step_start = Instant::now();
                        // Use the congested transit network if it was built
                        // (transit runs on roads), else the free-flow one.
                        let net = congested_transit_net.as_ref().unwrap_or(t.network);
                        let result = assign_transit_with_options(net, &transit_od, &t.options)?;
                        t_assignment += step_start.elapsed();
                        Some(result)
                    } else {
                        None
                    }
                }
                None => None,
            };

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
                per_feedback_assignments,
                feedback_iterations_done: feedback_done,
                transit: transit_result,
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

/// Validate inputs before any computation starts.
///
/// Checks possible failure cases that would cause e.g. "furness did not converge"
/// or other cryptic downstream errors.
fn preflight_check(
    network: &Network,
    zones: &[Zone],
    trip_generator: &dyn TripGenerator,
) -> Result<(), SimError> {
    // Case 1: no zones at all.
    if zones.is_empty() {
        return Err(PipelineError::InvalidInput(InvalidInputReason::NoZones).into());
    }

    // Case 2: no zone centroid nodes in the network.
    let missing_ids: Vec<ZoneID> = zones
        .iter()
        .filter(|z| network.get_zone_centroid(z.id).is_err())
        .map(|z| z.id)
        .collect();
    if missing_ids.len() == zones.len() {
        return Err(
            PipelineError::InvalidInput(InvalidInputReason::NoCentroids {
                zone_count: zones.len(),
                missing_ids,
            })
            .into(),
        );
    }

    // Case 3: all zone socioeconomic attributes are zero.
    let zero_attr_ids: Vec<ZoneID> = zones
        .iter()
        .filter(|z| z.population <= 0.0 && z.employment <= 0.0 && z.households <= 0.0)
        .map(|z| z.id)
        .collect();
    if zero_attr_ids.len() == zones.len() {
        return Err(
            PipelineError::InvalidInput(InvalidInputReason::ZeroAttributes {
                zone_ids: zero_attr_ids,
            })
            .into(),
        );
    }

    // Case 4: trip generator produces all-zero productions or attractions.
    // Run a trial generation to catch zero-coefficient configurations before
    // entering the feedback loop.
    let total_pop: f64 = zones.iter().map(|z| z.population).sum();
    let total_emp: f64 = zones.iter().map(|z| z.employment).sum();
    let total_hh: f64 = zones.iter().map(|z| z.households).sum();
    let (productions, attractions) = trip_generator.generate(zones).map_err(SimError::from)?;
    let total_p: f64 = productions.iter().sum();
    let total_a: f64 = attractions.iter().sum();
    if total_p <= 0.0 {
        return Err(
            PipelineError::InvalidInput(InvalidInputReason::ZeroProductions {
                total_pop,
                total_emp,
                total_hh,
            })
            .into(),
        );
    }
    if total_a <= 0.0 {
        return Err(
            PipelineError::InvalidInput(InvalidInputReason::ZeroAttractions {
                total_pop,
                total_emp,
                total_hh,
            })
            .into(),
        );
    }

    // Case 5: zone centroids form multiple strongly-connected components.
    // Furness cannot distribute trips across component boundaries because the
    // off-diagonal friction values are exactly zero (no path exists).
    let scc_start = std::time::Instant::now();
    let components = zone_scc(network, zones);
    log_additional!(
        EVENT_PREFLIGHT,
        "Connectivity check complete",
        zones = zones.len(),
        components = components.len(),
        elapsed_ms = format!("{:.3}", scc_start.elapsed().as_secs_f64() * 1000.0)
    );
    if components.len() > 1 {
        return Err(
            PipelineError::InvalidInput(InvalidInputReason::DisconnectedComponents { components })
                .into(),
        );
    }

    Ok(())
}

/// Run traffic assignment with the configured method.
///
/// Dispatches to the appropriate algorithm based on
/// [`AssignmentMethodType`](crate::config::AssignmentMethodType)
/// in the model config.
///
/// When `initial_volumes` is `Some`, the algorithm skips the cold-start
/// AON initialization and uses the given link volumes as starting point
/// (warm start). Pass `None` for the first feedback iteration.
fn run_assignment(
    network: &Network,
    graph: &IndexedGraph,
    od_matrix: &dyn OdMatrix,
    config: &ModelConfig,
    initial_volumes: Option<&HashMap<LinkID, f64>>,
) -> Result<AssignmentResult, crate::assignment::error::AssignmentError> {
    match config.assignment_method {
        AssignmentMethodType::FrankWolfe => {
            let method = FrankWolfe::new();
            method.assign(
                network,
                graph,
                od_matrix,
                &config.bpr,
                &config.assignment_config,
                initial_volumes,
            )
        }
        AssignmentMethodType::Msa => {
            let method = Msa::new();
            method.assign(
                network,
                graph,
                od_matrix,
                &config.bpr,
                &config.assignment_config,
                initial_volumes,
            )
        }
        AssignmentMethodType::GradientProjection => {
            let method = GradientProjection::with_step_scale(config.gp_step_scale);
            method.assign(
                network,
                graph,
                od_matrix,
                &config.bpr,
                &config.assignment_config,
                initial_volumes,
            )
        }
    }
}

/// Scale an OD matrix by a fraction. Used to split total AUTO demand
/// into per-class OD matrices for multi-class assignment.
fn scale_od(od: &DenseOdMatrix, fraction: f64) -> DenseOdMatrix {
    let zone_ids = od.zone_ids().to_vec();
    let data: Vec<f64> = od.data().iter().map(|&v| v * fraction).collect();
    DenseOdMatrix::from_data(zone_ids, data)
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

            result.set_by_index(
                i,
                j,
                haversine_km(o.latitude, o.longitude, d.latitude, d.longitude),
            );
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
    use crate::gmns::meso::link::Link;
    use crate::gmns::meso::node::Node;
    use crate::transit::TransitRoute;
    use crate::trip_distribution::ExponentialImpedance;
    use crate::trip_generation::RegressionGenerator;

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

    // A minimal 2-zone network. pop/emp are balanced so Furness converges:
    // with P = 0.5 pop + 0.1 emp and A = 0.1 pop + 0.8 emp, pop = 4 * emp
    // gives sum(P) = sum(A).
    fn two_zone_setup() -> (Network, Vec<Zone>) {
        let mut net = Network::new();
        net.add_node(
            Node::new(1)
                .with_zone_id(1)
                .with_coordinates(55.75, 37.62)
                .build(),
        )
        .unwrap();
        net.add_node(
            Node::new(2)
                .with_zone_id(2)
                .with_coordinates(55.76, 37.62)
                .build(),
        )
        .unwrap();
        for (id, a, b) in [(100, 1, 2), (101, 2, 1)] {
            net.add_link(
                Link::new(id, a, b)
                    .with_length_meters(1000.0)
                    .with_free_speed(60.0)
                    .with_capacity(1800.0)
                    .with_lanes_num(2)
                    .build(),
            )
            .unwrap();
        }
        let zones = vec![
            Zone::new(1)
                .with_population(1000.0)
                .with_employment(250.0)
                .build(),
            Zone::new(2)
                .with_population(1000.0)
                .with_employment(250.0)
                .build(),
        ];
        (net, zones)
    }

    fn base_config() -> ModelConfig {
        ModelConfig::new()
            .with_max_iterations(20)
            .with_feedback_iterations(1)
            .build()
    }

    #[test]
    fn pipeline_without_transit_has_no_transit_result() {
        let (net, zones) = two_zone_setup();
        let result = run_four_step_model(
            &net,
            &zones,
            &RegressionGenerator::new(),
            &ExponentialImpedance::new(0.1),
            &MultinomialLogit::default_auto_bike_walk(),
            &base_config(),
            None,
            None,
        )
        .unwrap();
        assert!(result.transit.is_none());
        assert!(!result.mode_od.contains_key(&AgentType::Transit));
    }

    #[test]
    fn pipeline_with_transit_assigns_transit_demand() {
        let (net, zones) = two_zone_setup();
        // Transit centroids = zone IDs: a line directly over stops 1 and 2.
        let mut transit_net = TransitNetwork::new();
        transit_net.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));

        let result = run_four_step_model(
            &net,
            &zones,
            &RegressionGenerator::new(),
            &ExponentialImpedance::new(0.1),
            &MultinomialLogit::default_auto_bike_walk_transit(),
            &base_config(),
            Some(TransitInput {
                network: &transit_net,
                options: TransitAssignmentOptions::default(),
                fixed_od: None,
                analysis_period: None,
            }),
            None,
        )
        .unwrap();

        // Mode choice produced a transit share, and it was assigned.
        let transit_od_total = result.mode_od[&AgentType::Transit].total();
        assert!(
            transit_od_total > 0.0,
            "transit demand = {}",
            transit_od_total
        );
        let transit = result.transit.expect("transit result present");
        // The line carries the 1 -> 2 transit demand (2 -> 1 is unavailable,
        // so all transit demand is on 1 -> 2).
        assert!(transit.total_boardings > 0.0);
        assert!((transit.od_costs[&(1, 2)] - 16.0).abs() < 1e-9);
    }

    #[test]
    fn pipeline_fixed_transit_od_adds_to_mode_choice_share() {
        let (net, zones) = two_zone_setup();
        let mut transit_net = TransitNetwork::new();
        transit_net.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));

        // Endogenous demand only, to read the baseline transit total.
        let base = run_four_step_model(
            &net,
            &zones,
            &RegressionGenerator::new(),
            &ExponentialImpedance::new(0.1),
            &MultinomialLogit::default_auto_bike_walk_transit(),
            &base_config(),
            Some(TransitInput {
                network: &transit_net,
                options: TransitAssignmentOptions::default(),
                fixed_od: None,
                analysis_period: None,
            }),
            None,
        )
        .unwrap();
        let base_demand = base.transit.unwrap().total_demand;

        // 500 captive riders on 1 -> 2, added on top of the mode-choice share.
        let mut fixed = DenseOdMatrix::new(vec![1, 2]);
        fixed.set(1, 2, 500.0);
        let with_fixed = run_four_step_model(
            &net,
            &zones,
            &RegressionGenerator::new(),
            &ExponentialImpedance::new(0.1),
            &MultinomialLogit::default_auto_bike_walk_transit(),
            &base_config(),
            Some(TransitInput {
                network: &transit_net,
                options: TransitAssignmentOptions::default(),
                fixed_od: Some(&fixed),
                analysis_period: None,
            }),
            None,
        )
        .unwrap();
        let with_demand = with_fixed.transit.unwrap().total_demand;

        // The fixed 500 are added on top of the endogenous share.
        assert!((with_demand - base_demand - 500.0).abs() < 1e-6);
    }

    #[test]
    fn pipeline_fixed_transit_od_without_transit_mode() {
        // A logit with no transit alternative, but a fixed transit matrix:
        // the fixed demand alone is assigned, mode choice adds nothing.
        let (net, zones) = two_zone_setup();
        let mut transit_net = TransitNetwork::new();
        transit_net.add_route(TransitRoute::new("L1", vec![1, 2], vec![10.0], 6.0));

        let mut fixed = DenseOdMatrix::new(vec![1, 2]);
        fixed.set(1, 2, 300.0);
        let result = run_four_step_model(
            &net,
            &zones,
            &RegressionGenerator::new(),
            &ExponentialImpedance::new(0.1),
            &MultinomialLogit::default_auto_bike_walk(),
            &base_config(),
            Some(TransitInput {
                network: &transit_net,
                options: TransitAssignmentOptions::default(),
                fixed_od: Some(&fixed),
                analysis_period: None,
            }),
            None,
        )
        .unwrap();
        // No transit mode in the logit -> mode_od has no transit share.
        assert!(!result.mode_od.contains_key(&AgentType::Transit));
        // The fixed 300 are still assigned.
        let transit = result.transit.expect("fixed transit assigned");
        assert!((transit.total_demand - 300.0).abs() < 1e-6);
    }

    #[test]
    fn pipeline_transit_preload_raises_road_cost() {
        // A frequent bus running on link 100 loads that link even with no
        // passengers, so the road cost on 100 rises; link 101 (no bus) is
        // untouched. Logit has no transit mode, isolating the preload
        // effect from any mode shift.
        let (net, zones) = two_zone_setup();
        let mut transit_net = TransitNetwork::new();
        transit_net.add_route(
            TransitRoute::new("B1", vec![1, 2], vec![10.0], 6.0)
                .with_segment_links(vec![vec![100]]),
        );

        let run = |analysis_period: Option<f64>| {
            run_four_step_model(
                &net,
                &zones,
                &RegressionGenerator::new(),
                &ExponentialImpedance::new(0.1),
                &MultinomialLogit::default_auto_bike_walk(),
                &base_config(),
                Some(TransitInput {
                    network: &transit_net,
                    options: TransitAssignmentOptions::default(),
                    fixed_od: None,
                    analysis_period,
                }),
                None,
            )
            .unwrap()
        };

        let base = run(None);
        // 60 min period, headway 6 -> 10 buses * 2.0 pce = 20 background PCU
        // on link 100.
        let loaded = run(Some(60.0));

        let base_100 = base.assignment.link_costs[&100];
        let loaded_100 = loaded.assignment.link_costs[&100];
        assert!(
            loaded_100 > base_100,
            "cost on 100 should rise with the bus preload: {} -> {}",
            base_100,
            loaded_100
        );
        // Link 101 carries no bus and the same auto flow, so it is unchanged.
        let base_101 = base.assignment.link_costs[&101];
        let loaded_101 = loaded.assignment.link_costs[&101];
        assert!((base_101 - loaded_101).abs() < 1e-9);
    }

    #[test]
    fn pipeline_road_congestion_slows_transit() {
        // Link 100 has a low capacity, so the auto demand congests it. A bus
        // that runs on link 100 (segment_links) is slowed by that congestion:
        // its in-vehicle time, and hence the transit skim and od cost, exceed
        // the free-flow value (6 wait + 10 ride = 16).
        let mut net = Network::new();
        net.add_node(
            Node::new(1)
                .with_zone_id(1)
                .with_coordinates(55.75, 37.62)
                .build(),
        )
        .unwrap();
        net.add_node(
            Node::new(2)
                .with_zone_id(2)
                .with_coordinates(55.76, 37.62)
                .build(),
        )
        .unwrap();
        // link 100 (1 -> 2): low capacity -> congests. link 101 (2 -> 1): normal.
        net.add_link(
            Link::new(100, 1, 2)
                .with_length_meters(1000.0)
                .with_free_speed(60.0)
                .with_capacity(150.0)
                .with_lanes_num(1)
                .build(),
        )
        .unwrap();
        net.add_link(
            Link::new(101, 2, 1)
                .with_length_meters(1000.0)
                .with_free_speed(60.0)
                .with_capacity(1800.0)
                .with_lanes_num(2)
                .build(),
        )
        .unwrap();
        let zones = vec![
            Zone::new(1)
                .with_population(1000.0)
                .with_employment(250.0)
                .build(),
            Zone::new(2)
                .with_population(1000.0)
                .with_employment(250.0)
                .build(),
        ];

        let mut transit_net = TransitNetwork::new();
        transit_net.add_route(
            TransitRoute::new("B1", vec![1, 2], vec![10.0], 6.0)
                .with_segment_links(vec![vec![100]]),
        );

        let result = run_four_step_model(
            &net,
            &zones,
            &RegressionGenerator::new(),
            &ExponentialImpedance::new(0.1),
            &MultinomialLogit::default_auto_bike_walk_transit(),
            &ModelConfig::new()
                .with_max_iterations(30)
                .with_feedback_iterations(2)
                .build(),
            Some(TransitInput {
                network: &transit_net,
                options: TransitAssignmentOptions::default(),
                fixed_od: None,
                analysis_period: None,
            }),
            None,
        )
        .unwrap();

        let transit = result.transit.expect("transit assigned");
        // Congested in-vehicle time pushes the 1 -> 2 cost above free-flow 16.
        assert!(
            transit.od_costs[&(1, 2)] > 16.0,
            "congested transit cost {} should exceed free-flow 16",
            transit.od_costs[&(1, 2)]
        );
    }
}
