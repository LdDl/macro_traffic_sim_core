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
mod matrix;
pub mod sparse;

pub use self::matrix::*;
pub use self::{dense::*, sparse::*};
