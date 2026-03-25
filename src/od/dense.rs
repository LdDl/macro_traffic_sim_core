//! # Dense OD Matrix
//!
//! Row-major flat vector implementation of the OD matrix.
//! Suitable for typical zone counts (hundreds to low thousands).
//!
//! The matrix stores all `n x n` cells, including zeros, which gives
//! O(1) access and cache-friendly iteration at the expense of memory
//! for very large zone systems.
//!
//! ## Storage layout
//!
//! Internally the data is kept in a `Vec<f64>` of length `n * n` where
//! element `[i * n + j]` holds the demand from `zone_ids[i]` to
//! `zone_ids[j]`.  A `HashMap<ZoneID, usize>` maps external zone IDs
//! to their position indices.
//!
//! ## Examples
//!
//! ### Creating and populating
//!
//! ```
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//!
//! let mut od = DenseOdMatrix::new(vec![10, 20, 30]);
//!
//! od.set(10, 20, 500.0);
//! od.set(20, 30, 300.0);
//!
//! assert_eq!(od.get(10, 20), 500.0);
//! assert_eq!(od.get(30, 10), 0.0);
//! assert_eq!(od.total(), 800.0);
//! ```
//!
//! ### Building from pre-computed data
//!
//! ```
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//!
//! // 2x2 matrix: zones [1, 2], row-major
//! let data = vec![0.0, 100.0, 200.0, 0.0];
//! let od = DenseOdMatrix::from_data(vec![1, 2], data);
//!
//! assert_eq!(od.get(1, 2), 100.0);
//! assert_eq!(od.get(2, 1), 200.0);
//! assert_eq!(od.total(), 300.0);
//! ```
//!
//! ### Direct access to raw storage
//!
//! ```
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//!
//! let mut od = DenseOdMatrix::new(vec![1, 2]);
//! od.set(1, 2, 42.0);
//!
//! // Read raw slice
//! assert_eq!(od.data(), &[0.0, 42.0, 0.0, 0.0]);
//!
//! // Mutate raw slice
//! od.data_mut()[0] = 10.0;
//! assert_eq!(od.get(1, 1), 10.0);
//! ```

use std::collections::HashMap;

use super::OdMatrix;
use crate::gmns::types::ZoneID;

/// Dense origin-destination matrix stored as a flat `Vec<f64>` in row-major order.
///
/// All `n x n` cells are allocated up-front, making read/write O(1) and
/// iteration cache-friendly.  Best suited for zone counts up to a few
/// thousand; for very sparse demand patterns consider [`super::SparseOdMatrix`].
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
///
/// let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
/// od.set(1, 3, 250.0);
/// assert_eq!(od.get(1, 3), 250.0);
/// assert_eq!(od.zone_count(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct DenseOdMatrix {
    /// Ordered list of zone IDs.
    zone_ids: Vec<ZoneID>,
    /// Mapping from zone ID to index position.
    zone_index: HashMap<ZoneID, usize>,
    /// Flat storage: data[i * n + j] = demand from zone_ids[i] to zone_ids[j].
    data: Vec<f64>,
}

impl DenseOdMatrix {
    /// Create a new dense OD matrix for the given zone IDs.
    ///
    /// All demand values are initialised to zero.
    ///
    /// # Arguments
    /// * `zone_ids` - Ordered list of zone IDs.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let od = DenseOdMatrix::new(vec![100, 200, 300]);
    /// assert_eq!(od.zone_count(), 3);
    /// assert_eq!(od.zone_ids(), &[100, 200, 300]);
    /// assert_eq!(od.total(), 0.0);
    /// ```
    pub fn new(zone_ids: Vec<ZoneID>) -> Self {
        let n = zone_ids.len();
        let zone_index: HashMap<ZoneID, usize> =
            zone_ids.iter().enumerate().map(|(i, &z)| (z, i)).collect();
        DenseOdMatrix {
            zone_ids,
            zone_index,
            data: vec![0.0; n * n],
        }
    }

    /// Create a dense OD matrix from existing row-major data.
    ///
    /// # Arguments
    /// * `zone_ids` - Ordered list of zone IDs.
    /// * `data` - Flat row-major data. Must have length `zone_ids.len() ^ 2`.
    ///
    /// # Panics
    /// Panics if `data.len() != zone_ids.len() ^ 2`.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// // 2x2 matrix for zones [1, 2]
    /// //   to:  1      2
    /// //  1: [ 0.0, 150.0 ]
    /// //  2: [ 80.0,  0.0 ]
    /// let od = DenseOdMatrix::from_data(vec![1, 2], vec![0.0, 150.0, 80.0, 0.0]);
    ///
    /// assert_eq!(od.get(1, 2), 150.0);
    /// assert_eq!(od.get(2, 1), 80.0);
    /// assert_eq!(od.get(1, 1), 0.0);
    /// assert_eq!(od.total(), 230.0);
    /// ```
    ///
    /// ```should_panic
    /// use macro_traffic_sim_core::od::DenseOdMatrix;
    ///
    /// // Wrong length - panics
    /// let _od = DenseOdMatrix::from_data(vec![1, 2], vec![1.0, 2.0]);
    /// ```
    pub fn from_data(zone_ids: Vec<ZoneID>, data: Vec<f64>) -> Self {
        let n = zone_ids.len();
        assert_eq!(data.len(), n * n, "data length must be n*n");
        let zone_index: HashMap<ZoneID, usize> =
            zone_ids.iter().enumerate().map(|(i, &z)| (z, i)).collect();
        DenseOdMatrix {
            zone_ids,
            zone_index,
            data,
        }
    }

    /// Get mutable access to the raw data slice.
    ///
    /// This is useful for bulk operations like Furness balancing that
    /// operate directly on the flat array.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let mut od = DenseOdMatrix::new(vec![1, 2]);
    /// // cell (1, 2)
    /// od.data_mut()[1] = 99.0;
    /// assert_eq!(od.get(1, 2), 99.0);
    /// ```
    pub fn data_mut(&mut self) -> &mut [f64] {
        &mut self.data
    }

    /// Get read access to the raw data slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let mut od = DenseOdMatrix::new(vec![1, 2]);
    /// od.set(1, 2, 42.0);
    /// assert_eq!(od.data(), &[0.0, 42.0, 0.0, 0.0]);
    /// ```
    pub fn data(&self) -> &[f64] {
        &self.data
    }

    /// Get the index for a zone ID, or None if not found.
    fn index_of(&self, zone_id: ZoneID) -> Option<usize> {
        self.zone_index.get(&zone_id).copied()
    }

    /// Number of zones.
    fn n(&self) -> usize {
        self.zone_ids.len()
    }
}

impl OdMatrix for DenseOdMatrix {
    fn get(&self, origin: ZoneID, destination: ZoneID) -> f64 {
        let Some(i) = self.index_of(origin) else {
            return 0.0;
        };
        let Some(j) = self.index_of(destination) else {
            return 0.0;
        };
        self.data[i * self.n() + j]
    }

    fn set(&mut self, origin: ZoneID, destination: ZoneID, value: f64) {
        let Some(i) = self.index_of(origin) else {
            return;
        };
        let Some(j) = self.index_of(destination) else {
            return;
        };
        let n = self.n();
        self.data[i * n + j] = value;
    }

    fn add(&mut self, origin: ZoneID, destination: ZoneID, delta: f64) {
        let Some(i) = self.index_of(origin) else {
            return;
        };
        let Some(j) = self.index_of(destination) else {
            return;
        };
        let n = self.n();
        self.data[i * n + j] += delta;
    }

    fn zone_ids(&self) -> &[ZoneID] {
        &self.zone_ids
    }

    fn row_sum(&self, origin: ZoneID) -> f64 {
        let Some(i) = self.index_of(origin) else {
            return 0.0;
        };
        let n = self.n();
        self.data[i * n..(i + 1) * n].iter().sum()
    }

    fn col_sum(&self, destination: ZoneID) -> f64 {
        let Some(j) = self.index_of(destination) else {
            return 0.0;
        };
        let n = self.n();
        (0..n).map(|i| self.data[i * n + j]).sum()
    }

    fn total(&self) -> f64 {
        self.data.iter().sum()
    }

    fn iter(&self) -> Vec<(ZoneID, ZoneID, f64)> {
        let n = self.n();
        let mut result = Vec::with_capacity(n * n);
        for i in 0..n {
            for j in 0..n {
                let val = self.data[i * n + j];
                result.push((self.zone_ids[i], self.zone_ids[j], val));
            }
        }
        result
    }
}
