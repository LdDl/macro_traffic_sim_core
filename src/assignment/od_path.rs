use crate::gmns::types::{LinkID, ZoneID};

/// A single path between an OD pair with its flow and cost.
///
/// Produced when [`super::AssignmentConfig::store_paths`] is true.
/// GP returns multiple paths per OD pair with flow distribution.
/// FW and MSA return one shortest path per OD pair (post-processing
/// on final equilibrium costs) with flow equal to OD demand.
#[derive(Debug, Clone)]
pub struct OdPath {
    pub origin_zone: ZoneID,
    pub dest_zone: ZoneID,
    pub path_index: u32,
    pub flow: f64,
    pub cost: f64,
    pub link_ids: Vec<LinkID>,
    /// User class index for multi-class assignment.
    /// Maps to the position in the `classes` slice passed to
    /// `assign_multiclass_fw` / `assign_multiclass_msa`.
    /// `None` for single-class.
    pub class_index: Option<u16>,
}
