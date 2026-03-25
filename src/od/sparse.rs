//! # Sparse OD Matrix
//!
//! HashMap-based implementation for large or fragmented zone systems
//! where most OD pairs have zero demand.
//!
//! Only non-zero entries are stored, so memory usage scales with the
//! number of active OD pairs rather than `n^2`.  Zero entries are
//! automatically removed on [`OdMatrix::set`] and [`OdMatrix::add`].
//!
//! ## Trade-offs vs [`super::DenseOdMatrix`]
//!
//! - Memory: O(nnz) (nnz = number of non-zero entries) instead of O(n^2) - significant when demand is
//!   concentrated on a small fraction of all possible pairs.
//! - Access: HashMap lookup (amortised O(1), but slower constant than
//!   a flat array index).
//! - Iteration: [`OdMatrix::iter`] returns only non-zero entries.
//!
//! ## Examples
//!
//! ### Basic usage
//!
//! ```
//! use macro_traffic_sim_core::od::{SparseOdMatrix, OdMatrix};
//!
//! let mut od = SparseOdMatrix::new(vec![1, 2, 3]);
//!
//! od.set(1, 2, 500.0);
//! od.set(3, 1, 200.0);
//!
//! assert_eq!(od.get(1, 2), 500.0);
//! // unset pair
//! assert_eq!(od.get(2, 3), 0.0);
//! assert_eq!(od.nnz(), 2);
//! assert_eq!(od.total(), 700.0);
//! ```
//!
//! ### Automatic zero removal
//!
//! ```
//! use macro_traffic_sim_core::od::{SparseOdMatrix, OdMatrix};
//!
//! let mut od = SparseOdMatrix::new(vec![1, 2]);
//! od.set(1, 2, 100.0);
//! assert_eq!(od.nnz(), 1);
//!
//! // setting to zero removes the entry
//! od.set(1, 2, 0.0);
//! assert_eq!(od.nnz(), 0);
//! assert_eq!(od.get(1, 2), 0.0);
//! ```
//!
//! ### Incremental updates with `add`
//!
//! ```
//! use macro_traffic_sim_core::od::{SparseOdMatrix, OdMatrix};
//!
//! let mut od = SparseOdMatrix::new(vec![1, 2]);
//! od.add(1, 2, 60.0);
//! od.add(1, 2, 40.0);
//! assert_eq!(od.get(1, 2), 100.0);
//!
//! // back to zero - entry is removed
//! od.add(1, 2, -100.0);
//! assert_eq!(od.nnz(), 0);
//! ```

use std::collections::HashMap;

use super::OdMatrix;
use crate::gmns::types::ZoneID;

/// Sparse origin-destination matrix using a `HashMap<(ZoneID, ZoneID), f64>`.
///
/// Stores only non-zero demand values.  Setting a cell to zero or
/// adding a delta that brings the value to zero automatically removes
/// the entry from the map.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::od::{SparseOdMatrix, OdMatrix};
///
/// let mut od = SparseOdMatrix::new(vec![10, 20]);
/// od.set(10, 20, 75.0);
///
/// assert_eq!(od.get(10, 20), 75.0);
/// assert_eq!(od.nnz(), 1);
/// assert_eq!(od.zone_count(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct SparseOdMatrix {
    /// Ordered list of zone IDs.
    zone_ids: Vec<ZoneID>,
    /// Sparse storage of non-zero demand values.
    data: HashMap<(ZoneID, ZoneID), f64>,
}

impl SparseOdMatrix {
    /// Create a new sparse OD matrix for the given zone IDs.
    ///
    /// All demand values start at zero (i.e. the internal map is empty).
    ///
    /// # Arguments
    /// * `zone_ids` - Ordered list of zone IDs.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{SparseOdMatrix, OdMatrix};
    ///
    /// let od = SparseOdMatrix::new(vec![1, 2, 3, 4]);
    /// assert_eq!(od.zone_count(), 4);
    /// assert_eq!(od.nnz(), 0);
    /// assert_eq!(od.total(), 0.0);
    /// ```
    pub fn new(zone_ids: Vec<ZoneID>) -> Self {
        SparseOdMatrix {
            zone_ids,
            data: HashMap::new(),
        }
    }

    /// Returns the number of non-zero entries.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{SparseOdMatrix, OdMatrix};
    ///
    /// let mut od = SparseOdMatrix::new(vec![1, 2, 3]);
    /// assert_eq!(od.nnz(), 0);
    ///
    /// od.set(1, 2, 10.0);
    /// od.set(2, 3, 20.0);
    /// assert_eq!(od.nnz(), 2);
    ///
    /// od.set(1, 2, 0.0);
    /// assert_eq!(od.nnz(), 1);
    /// ```
    pub fn nnz(&self) -> usize {
        self.data.len()
    }
}

impl OdMatrix for SparseOdMatrix {
    fn get(&self, origin: ZoneID, destination: ZoneID) -> f64 {
        self.data
            .get(&(origin, destination))
            .copied()
            .unwrap_or(0.0)
    }

    fn set(&mut self, origin: ZoneID, destination: ZoneID, value: f64) {
        if value == 0.0 {
            self.data.remove(&(origin, destination));
        } else {
            self.data.insert((origin, destination), value);
        }
    }

    fn add(&mut self, origin: ZoneID, destination: ZoneID, delta: f64) {
        let entry = self.data.entry((origin, destination)).or_insert(0.0);
        *entry += delta;
        if *entry == 0.0 {
            self.data.remove(&(origin, destination));
        }
    }

    fn zone_ids(&self) -> &[ZoneID] {
        &self.zone_ids
    }

    fn row_sum(&self, origin: ZoneID) -> f64 {
        self.data
            .iter()
            .filter(|&(&(o, _), _)| o == origin)
            .map(|(_, &v)| v)
            .sum()
    }

    fn col_sum(&self, destination: ZoneID) -> f64 {
        self.data
            .iter()
            .filter(|&(&(_, d), _)| d == destination)
            .map(|(_, &v)| v)
            .sum()
    }

    fn total(&self) -> f64 {
        self.data.values().sum()
    }

    fn iter(&self) -> Vec<(ZoneID, ZoneID, f64)> {
        let mut result: Vec<(ZoneID, ZoneID, f64)> =
            self.data.iter().map(|(&(o, d), &v)| (o, d, v)).collect();
        result.sort_by_key(|&(o, d, _)| (o, d));
        result
    }
}
