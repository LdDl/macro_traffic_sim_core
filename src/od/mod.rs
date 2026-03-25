//! # OD Matrix Module
//!
//! Origin-destination matrix representations for storing trip tables
//! in the 4-step traffic demand model.
//!
//! An OD matrix stores the number of trips between each pair of transport
//! analysis zones (TAZ). It is the primary data structure flowing between
//! the trip distribution (Step 2), mode choice (Step 3), and traffic
//! assignment (Step 4) stages.
//!
//! ## Choosing an implementation
//!
//! | Implementation | Storage | Best for |
//! |---|---|---|
//! | [`DenseOdMatrix`] | `Vec<f64>` (n x n) | Typical zone counts (up to a few thousand). Cache-friendly, O(1) access. |
//! | [`SparseOdMatrix`] | `HashMap<(ZoneID, ZoneID), f64>` | Very large zone systems with mostly zero demand. Saves memory but slower iteration. |
//!
//! Both implement the [`OdMatrix`] trait, so any algorithm that accepts
//! `&dyn OdMatrix` works with either backend transparently.
//!
//! ## Examples
//!
//! ### Creating and populating a dense matrix
//!
//! ```
//! use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
//!
//! let zones = vec![100, 200, 300];
//! let mut od = DenseOdMatrix::new(zones);
//!
//! od.set(100, 200, 500.0);
//! od.set(100, 300, 150.0);
//! od.set(200, 100, 420.0);
//!
//! assert_eq!(od.get(100, 200), 500.0);
//! // unset pairs are 0
//! assert_eq!(od.get(300, 100), 0.0);
//! // 500 + 150
//! assert_eq!(od.row_sum(100), 650.0);
//! assert_eq!(od.col_sum(100), 420.0);
//! assert_eq!(od.total(), 1070.0);
//! ```
//!
//! ### Using the sparse backend
//!
//! ```
//! use macro_traffic_sim_core::od::{SparseOdMatrix, OdMatrix};
//!
//! let mut od = SparseOdMatrix::new(vec![1, 2, 3]);
//! od.set(1, 2, 100.0);
//! od.add(1, 2, 50.0);
//!
//! assert_eq!(od.get(1, 2), 150.0);
//! // only one non-zero entry
//! assert_eq!(od.nnz(), 1);
//! ```
//!
//! ### Passing either implementation to an algorithm
//!
//! ```
//! use macro_traffic_sim_core::od::{DenseOdMatrix, SparseOdMatrix, OdMatrix};
//!
//! fn total_demand(od: &dyn OdMatrix) -> f64 {
//!     od.total()
//! }
//!
//! let mut dense = DenseOdMatrix::new(vec![1, 2]);
//! dense.set(1, 2, 300.0);
//!
//! let mut sparse = SparseOdMatrix::new(vec![1, 2]);
//! sparse.set(1, 2, 300.0);
//!
//! assert_eq!(total_demand(&dense), total_demand(&sparse));
//! ```
pub mod dense;
pub mod sparse;

pub use self::{dense::*, sparse::*};

use crate::gmns::types::ZoneID;

/// Trait for origin-destination matrix operations.
///
/// Provides a uniform interface over different storage backends
/// ([`DenseOdMatrix`], [`SparseOdMatrix`]) so that model algorithms
/// can work with any representation.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
///
/// let mut od = DenseOdMatrix::new(vec![10, 20]);
/// od.set(10, 20, 42.0);
/// assert_eq!(od.get(10, 20), 42.0);
/// assert_eq!(od.zone_count(), 2);
/// ```
pub trait OdMatrix {
    /// Returns the demand for a given OD pair.
    ///
    /// Returns `0.0` if the zone IDs are not part of the matrix.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let od = DenseOdMatrix::new(vec![1, 2]);
    /// assert_eq!(od.get(1, 2), 0.0);
    /// // unknown zone returns 0
    /// assert_eq!(od.get(999, 1), 0.0);
    /// ```
    fn get(&self, origin: ZoneID, destination: ZoneID) -> f64;

    /// Sets the demand for a given OD pair.
    ///
    /// Silently ignores unknown zone IDs.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let mut od = DenseOdMatrix::new(vec![1, 2]);
    /// od.set(1, 2, 100.0);
    /// assert_eq!(od.get(1, 2), 100.0);
    ///
    /// // overwrites
    /// od.set(1, 2, 200.0);
    /// assert_eq!(od.get(1, 2), 200.0);
    /// ```
    fn set(&mut self, origin: ZoneID, destination: ZoneID, value: f64);

    /// Adds a delta to the demand for a given OD pair.
    ///
    /// Equivalent to `set(o, d, get(o, d) + delta)` but may be more
    /// efficient for some backends.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let mut od = DenseOdMatrix::new(vec![1, 2]);
    /// od.set(1, 2, 100.0);
    /// od.add(1, 2, 50.0);
    /// od.add(1, 2, -30.0);
    /// assert_eq!(od.get(1, 2), 120.0);
    /// ```
    fn add(&mut self, origin: ZoneID, destination: ZoneID, delta: f64);

    /// Returns the ordered list of zone IDs that define this matrix.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let od = DenseOdMatrix::new(vec![10, 20, 30]);
    /// assert_eq!(od.zone_ids(), &[10, 20, 30]);
    /// ```
    fn zone_ids(&self) -> &[ZoneID];

    /// Returns the sum of demand originating from a zone (row sum).
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
    /// od.set(1, 2, 100.0);
    /// od.set(1, 3, 200.0);
    /// assert_eq!(od.row_sum(1), 300.0);
    /// assert_eq!(od.row_sum(2), 0.0);
    /// ```
    fn row_sum(&self, origin: ZoneID) -> f64;

    /// Returns the sum of demand attracted to a zone (column sum).
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let mut od = DenseOdMatrix::new(vec![1, 2, 3]);
    /// od.set(1, 2, 100.0);
    /// od.set(3, 2, 200.0);
    /// assert_eq!(od.col_sum(2), 300.0);
    /// assert_eq!(od.col_sum(1), 0.0);
    /// ```
    fn col_sum(&self, destination: ZoneID) -> f64;

    /// Returns the total demand across all OD pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let mut od = DenseOdMatrix::new(vec![1, 2]);
    /// od.set(1, 2, 100.0);
    /// od.set(2, 1, 80.0);
    /// assert_eq!(od.total(), 180.0);
    /// ```
    fn total(&self) -> f64;

    /// Returns all (origin, destination, demand) triples.
    ///
    /// For [`DenseOdMatrix`] this includes zero entries.
    /// For [`SparseOdMatrix`] only non-zero entries are returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{SparseOdMatrix, OdMatrix};
    ///
    /// let mut od = SparseOdMatrix::new(vec![1, 2]);
    /// od.set(1, 2, 50.0);
    /// let triples = od.iter();
    /// assert_eq!(triples.len(), 1);
    /// assert_eq!(triples[0], (1, 2, 50.0));
    /// ```
    fn iter(&self) -> Vec<(ZoneID, ZoneID, f64)>;

    /// Returns the number of zones.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::od::{DenseOdMatrix, OdMatrix};
    ///
    /// let od = DenseOdMatrix::new(vec![1, 2, 3, 4]);
    /// assert_eq!(od.zone_count(), 4);
    /// ```
    fn zone_count(&self) -> usize {
        self.zone_ids().len()
    }
}
