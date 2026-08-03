//! # Road interaction of the transit layer
//!
//! Frequency-based coupling between the transit lines and the road network:
//! transit vehicles occupy road space (a background load on the road
//! assignment), and road congestion determines the in-vehicle travel time
//! of the segments that run in mixed traffic.
//!
//! The vehicle load is flow-independent - headways are inputs, so the
//! number of vehicles on a link is fixed regardless of how many passengers
//! ride. This is the frequency-based special case of the two-mode
//! equilibrium of Florian and Spiess (1983), and matches the assumption of
//! De Cea and Fernandez (1993) that the in-vehicle time is determined by
//! road congestion as an exogenous parameter.
//!
//! - Florian, M. and Spiess, H. (1983) "On Binary Mode Choice/Assignment
//!   Models". Transportation Science 17(1), 32-47.
//!   DOI: <https://doi.org/10.1287/trsc.17.1.32>
//! - De Cea, J. and Fernandez, E. (1993) "Transit Assignment for Congested
//!   Public Transport Systems: An Equilibrium Model". Transportation
//!   Science 27(2), 133-147. DOI: <https://doi.org/10.1287/trsc.27.2.133>

use std::collections::HashMap;

use crate::gmns::types::LinkID;
use crate::transit::error::TransitError;
use crate::transit::route::TransitNetwork;

/// Computes the background road load (in passenger-car equivalents) from
/// the transit vehicles, keyed by road link ID.
///
/// For every route that declares its `segment_links` (i.e. runs in mixed
/// traffic), the number of vehicles passing over the analysis period is
/// `analysis_period / headway`, each contributing `pce` units of road
/// space. A link the route runs on receives `vehicles * pce`; contributions
/// from all routes are summed. Routes without road links (dedicated
/// right-of-way, e.g. a metro) add nothing.
///
/// The result is meant to be fed as a fixed background volume to the road
/// assignment: the buses are there regardless of the passenger flow, so the
/// load does not change across assignment iterations.
///
/// # Arguments
///
/// * `network` - The transit network
/// * `analysis_period` - Length of the analysis period, in the same time
///   unit as the route headways (e.g. `60.0` for a one-hour period on a
///   network built in minutes, `3600.0` on one built in seconds). The
///   vehicle count is `analysis_period / headway`.
///
/// # Errors
///
/// Returns [`TransitError::InvalidAnalysisPeriod`] when `analysis_period`
/// is not strictly positive (or NaN).
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::transit::{transit_road_preload, TransitNetwork, TransitRoute};
///
/// // a bus every 6 min over stops 1 -> 2 -> 3, its segments on links
/// // [100] and [104]; pce defaults to 2.0
/// let mut network = TransitNetwork::new();
/// network.add_route(
///     TransitRoute::new("B1", vec![1, 2, 3], vec![6.0, 7.0], 6.0)
///         .with_segment_links(vec![vec![100], vec![104]]),
/// );
///
/// // one hour = 60 min -> 60 / 6 = 10 vehicles, each 2.0 pce = 20 per link
/// let preload = transit_road_preload(&network, 60.0).unwrap();
/// assert_eq!(preload[&100], 20.0);
/// assert_eq!(preload[&104], 20.0);
/// ```
pub fn transit_road_preload(
    network: &TransitNetwork,
    analysis_period: f64,
) -> Result<HashMap<LinkID, f64>, TransitError> {
    if analysis_period.is_nan() || analysis_period <= 0.0 {
        return Err(TransitError::InvalidAnalysisPeriod {
            value: analysis_period,
        });
    }

    let mut preload: HashMap<LinkID, f64> = HashMap::new();
    for route in &network.routes {
        let Some(segment_links) = &route.segment_links else {
            continue;
        };
        if route.headway <= 0.0 {
            continue;
        }
        let vehicles = analysis_period / route.headway;
        let load = vehicles * route.pce;

        // A link the route runs on gets the vehicle load once, even if it
        // appears in more than one segment (a vehicle traverses it once).
        let mut seen: std::collections::HashSet<LinkID> = std::collections::HashSet::new();
        for segment in segment_links {
            for &link in segment {
                if seen.insert(link) {
                    *preload.entry(link).or_insert(0.0) += load;
                }
            }
        }
    }
    Ok(preload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transit::route::TransitRoute;

    const EPS: f64 = 1e-9;

    #[test]
    fn test_preload_counts_vehicles_times_pce() {
        let mut network = TransitNetwork::new();
        network.add_route(
            TransitRoute::new("B1", vec![1, 2, 3], vec![6.0, 7.0], 6.0)
                .with_segment_links(vec![vec![100], vec![104]]),
        );
        // 60 / 6 = 10 vehicles, pce 2.0 -> 20 per link
        let preload = transit_road_preload(&network, 60.0).unwrap();
        assert!((preload[&100] - 20.0).abs() < EPS);
        assert!((preload[&104] - 20.0).abs() < EPS);
    }

    #[test]
    fn test_preload_sums_across_routes_and_respects_pce() {
        let mut network = TransitNetwork::new();
        // Two lines share link 100. B1: 10 veh * 2.0 = 20. B2: 5 veh * 3.0 = 15.
        network.add_route(
            TransitRoute::new("B1", vec![1, 2], vec![6.0], 6.0).with_segment_links(vec![vec![100]]),
        );
        network.add_route(
            TransitRoute::new("B2", vec![1, 2], vec![6.0], 12.0)
                .with_segment_links(vec![vec![100]])
                .with_pce(3.0),
        );
        let preload = transit_road_preload(&network, 60.0).unwrap();
        assert!((preload[&100] - 35.0).abs() < EPS);
    }

    #[test]
    fn test_dedicated_route_adds_no_load() {
        let mut network = TransitNetwork::new();
        // No segment_links -> metro on its own right-of-way, no road load.
        network.add_route(TransitRoute::new("M1", vec![1, 2], vec![5.0], 4.0));
        let preload = transit_road_preload(&network, 60.0).unwrap();
        assert!(preload.is_empty());
    }

    #[test]
    fn test_empty_segment_is_dedicated() {
        let mut network = TransitNetwork::new();
        // Segment 1 on link 100 (mixed), segment 2 empty (dedicated).
        network.add_route(
            TransitRoute::new("T1", vec![1, 2, 3], vec![5.0, 5.0], 6.0)
                .with_segment_links(vec![vec![100], vec![]]),
        );
        let preload = transit_road_preload(&network, 60.0).unwrap();
        assert!((preload[&100] - 20.0).abs() < EPS);
        assert_eq!(preload.len(), 1);
    }

    #[test]
    fn test_link_shared_by_two_segments_counted_once() {
        let mut network = TransitNetwork::new();
        // A route that runs on link 100 in both of its segments: the vehicle
        // traverses it once, so it loads it once.
        network.add_route(
            TransitRoute::new("B1", vec![1, 2, 3], vec![6.0, 7.0], 6.0)
                .with_segment_links(vec![vec![100], vec![100]]),
        );
        let preload = transit_road_preload(&network, 60.0).unwrap();
        assert!((preload[&100] - 20.0).abs() < EPS);
    }

    #[test]
    fn test_invalid_analysis_period() {
        let mut network = TransitNetwork::new();
        network.add_route(
            TransitRoute::new("B1", vec![1, 2], vec![6.0], 6.0).with_segment_links(vec![vec![100]]),
        );
        for bad in [0.0, -1.0, f64::NAN] {
            assert!(matches!(
                transit_road_preload(&network, bad),
                Err(TransitError::InvalidAnalysisPeriod { .. })
            ));
        }
    }
}
