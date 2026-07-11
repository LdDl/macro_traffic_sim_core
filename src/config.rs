//! # Configuration
//!
//! Model configuration structures for the 4-step traffic demand model.
//!
//! ## Components
//!
//! - [`AssignmentMethodType`] - selects the assignment algorithm
//! - [`ModelConfig`] - all parameters for the pipeline
//! - [`ModelConfigBuilder`] - builder for `ModelConfig`
//!
//! ## Examples
//!
//! ### Default configuration
//!
//! ```
//! use macro_traffic_sim_core::config::ModelConfig;
//!
//! let config = ModelConfig::default();
//! assert_eq!(config.feedback_iterations, 3);
//! assert_eq!(config.assignment_config.max_iterations, 100);
//! ```
//!
//! ### Custom configuration via builder
//!
//! ```
//! use macro_traffic_sim_core::config::{ModelConfig, AssignmentMethodType};
//! use macro_traffic_sim_core::verbose::VerboseLevel;
//!
//! let config = ModelConfig::new()
//!     .with_assignment_method(AssignmentMethodType::Msa)
//!     .with_max_iterations(200)
//!     .with_convergence_gap(1e-5)
//!     .with_bpr(0.15, 4.0)
//!     .with_feedback_iterations(5)
//!     .with_verbose_level(VerboseLevel::Main)
//!     .build();
//!
//! assert_eq!(config.assignment_config.max_iterations, 200);
//! assert_eq!(config.feedback_iterations, 5);
//! ```

use crate::assignment::multiclass::UserClass;
use crate::assignment::{AssignmentConfig, BprFunction};
use crate::trip_distribution::furness::FurnessConfig;
use crate::verbose::VerboseLevel;

/// Multi-class user definition for the pipeline.
///
/// When provided in [`ModelConfig::user_classes`], the AUTO OD matrix
/// is split by `demand_fraction` and multi-class assignment is used.
///
/// # Beckmann symmetry condition
///
/// For the Frank-Wolfe / MSA algorithms to converge to the exact
/// multi-class User Equilibrium, all classes must satisfy:
///
///   `ff_time_multiplier / pcu = const` across all classes
///
/// This is the Dafermos (1972) symmetry condition. The assignment
/// will return an error if it is violated.
///
/// Any constant ratio works. The simplest is `ff_time_multiplier = pcu`
/// (ratio = 1.0), but any `k` is valid as long as it is the same for
/// all classes:
///
/// ```
/// use macro_traffic_sim_core::config::UserClassConfig;
///
/// // OK: ratio = 1.0 for both (mu = pcu)
/// let car   = UserClassConfig::new("car",   1.0, 1.0, 0.9);
/// let truck = UserClassConfig::new("truck", 2.5, 2.5, 0.1);
///
/// // OK: ratio = 0.5 for both
/// let car   = UserClassConfig::new("car",   1.0, 0.5, 0.8);
/// let truck = UserClassConfig::new("truck", 3.0, 1.5, 0.2);
///
/// // OK: ratio = 1.3 for all three classes
/// let car   = UserClassConfig::new("car",   1.0, 1.3, 0.7);
/// let truck = UserClassConfig::new("truck", 2.5, 3.25, 0.2);
/// let bus   = UserClassConfig::new("bus",   3.0, 3.9, 0.1);
/// ```
///
/// This does NOT work - `assign_multiclass_fw` returns
/// `Err(InvalidConfig("Beckmann symmetry condition violated: ..."))`:
///
/// ```
/// use macro_traffic_sim_core::config::UserClassConfig;
/// use macro_traffic_sim_core::assignment::multiclass::UserClass;
///
/// // BAD: 1.0/1.0 = 1.0, but 1.2/2.5 = 0.48 - ratios differ
/// let _car   = UserClassConfig::new("car",   1.0, 1.0, 0.9);
/// let _truck = UserClassConfig::new("truck", 2.5, 1.2, 0.1);
/// // These configs will be rejected by the assignment at runtime.
/// ```
///
/// # demand_fraction
///
/// Fractions across all classes must sum to 1.0. The pipeline
/// validates this and returns an error if the sum deviates
/// by more than 1e-6.
#[derive(Debug, Clone)]
pub struct UserClassConfig {
    pub name: String,
    /// Passenger Car Units (PCE) per vehicle. Cars = 1.0,
    /// trucks typically 2.0-3.0.
    pub pcu: f64,
    /// Multiplier applied to link travel time for this class.
    /// Must satisfy ff_time_multiplier / pcu = const for all classes
    /// (Beckmann symmetry condition).
    pub ff_time_multiplier: f64,
    /// Fraction of total AUTO demand assigned to this class.
    /// All fractions must sum to 1.0.
    pub demand_fraction: f64,
}

impl UserClassConfig {
    pub fn new(name: &str, pcu: f64, ff_time_multiplier: f64, demand_fraction: f64) -> Self {
        UserClassConfig {
            name: name.to_string(),
            pcu,
            ff_time_multiplier,
            demand_fraction,
        }
    }

    pub fn to_user_class(&self) -> UserClass {
        UserClass::new(&self.name, self.pcu, self.ff_time_multiplier)
    }
}

/// Assignment method selection.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::config::AssignmentMethodType;
///
/// let method = AssignmentMethodType::FrankWolfe;
/// assert_eq!(method.to_string(), "frank_wolfe");
///
/// let method = AssignmentMethodType::GradientProjection;
/// assert_eq!(method.to_string(), "gradient_projection");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentMethodType {
    FrankWolfe,
    Msa,
    GradientProjection,
}

impl std::fmt::Display for AssignmentMethodType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AssignmentMethodType::FrankWolfe => "frank_wolfe",
            AssignmentMethodType::Msa => "msa",
            AssignmentMethodType::GradientProjection => "gradient_projection",
        };
        write!(f, "{}", s)
    }
}

/// Complete model configuration.
///
/// Controls all parameters for the 4-step pipeline:
/// assignment method, BPR function, convergence thresholds,
/// Furness balancing, feedback loop count, and logging level.
///
/// # Examples
///
/// ```
/// use macro_traffic_sim_core::config::ModelConfig;
///
/// // Default: Frank-Wolfe, 100 iterations, gap 1e-4, 3 feedback loops
/// let config = ModelConfig::default();
/// assert_eq!(config.gp_step_scale, 0.1);
/// ```
#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// Assignment algorithm to use.
    pub assignment_method: AssignmentMethodType,
    /// BPR function parameters (alpha, beta).
    pub bpr: BprFunction,
    /// Assignment convergence config (max iterations, gap threshold).
    pub assignment_config: AssignmentConfig,
    /// Furness balancing config (max iterations, tolerance).
    pub furness_config: FurnessConfig,
    /// Number of outer feedback loop iterations (steps 2-4).
    /// Minimum 1 (no feedback, single pass).
    pub feedback_iterations: usize,
    /// Verbose level for logging.
    pub verbose_level: VerboseLevel,
    /// Gradient projection step scale (only used with GP method).
    pub gp_step_scale: f64,
    /// Use warm start for assignment from the 2nd feedback iteration
    /// onwards. When true, link volumes from the previous feedback
    /// iteration are passed as the initial solution, skipping the
    /// cold-start AON initialization. Default: true.
    pub warm_start: bool,
    /// Multi-class user definitions. When `Some`, the pipeline splits
    /// the AUTO OD by each class's `demand_fraction` and runs
    /// multi-class PCU-based assignment (Dafermos 1972).
    /// When `None`, single-class assignment is used.
    pub user_classes: Option<Vec<UserClassConfig>>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        ModelConfig {
            assignment_method: AssignmentMethodType::FrankWolfe,
            bpr: BprFunction::default(),
            assignment_config: AssignmentConfig::default(),
            furness_config: FurnessConfig::default(),
            feedback_iterations: 3,
            verbose_level: VerboseLevel::None,
            gp_step_scale: 0.1,
            warm_start: true,
            user_classes: None,
        }
    }
}

impl ModelConfig {
    /// Create a new config builder starting from defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use macro_traffic_sim_core::config::ModelConfig;
    ///
    /// let config = ModelConfig::new()
    ///     .with_feedback_iterations(1)
    ///     .build();
    /// assert_eq!(config.feedback_iterations, 1);
    /// ```
    pub fn new() -> ModelConfigBuilder {
        ModelConfigBuilder {
            instance: ModelConfig::default(),
        }
    }
}

/// Builder for `ModelConfig`.
pub struct ModelConfigBuilder {
    instance: ModelConfig,
}

impl ModelConfigBuilder {
    /// Set the assignment algorithm.
    pub fn with_assignment_method(mut self, method: AssignmentMethodType) -> Self {
        self.instance.assignment_method = method;
        self
    }

    /// Set BPR function parameters.
    pub fn with_bpr(mut self, alpha: f64, beta: f64) -> Self {
        self.instance.bpr = BprFunction::new(alpha, beta);
        self
    }

    /// Set maximum assignment iterations.
    pub fn with_max_iterations(mut self, max_iter: usize) -> Self {
        self.instance.assignment_config.max_iterations = max_iter;
        self
    }

    /// Set the assignment convergence gap threshold.
    pub fn with_convergence_gap(mut self, gap: f64) -> Self {
        self.instance.assignment_config.convergence_gap = gap;
        self
    }

    /// Set maximum Furness balancing iterations.
    pub fn with_furness_max_iterations(mut self, max_iter: usize) -> Self {
        self.instance.furness_config.max_iterations = max_iter;
        self
    }

    /// Set Furness balancing tolerance.
    pub fn with_furness_tolerance(mut self, tol: f64) -> Self {
        self.instance.furness_config.tolerance = tol;
        self
    }

    /// Set the number of outer feedback loop iterations.
    pub fn with_feedback_iterations(mut self, n: usize) -> Self {
        self.instance.feedback_iterations = n;
        self
    }

    /// Set the verbose logging level.
    pub fn with_verbose_level(mut self, level: VerboseLevel) -> Self {
        self.instance.verbose_level = level;
        self
    }

    /// Set the gradient projection step scale.
    pub fn with_gp_step_scale(mut self, scale: f64) -> Self {
        self.instance.gp_step_scale = scale;
        self
    }

    /// Enable or disable warm start for the 2nd feedback iteration onwards.
    pub fn with_warm_start(mut self, enabled: bool) -> Self {
        self.instance.warm_start = enabled;
        self
    }

    /// Set multi-class user definitions for PCU-based assignment.
    pub fn with_user_classes(mut self, classes: Vec<UserClassConfig>) -> Self {
        self.instance.user_classes = Some(classes);
        self
    }

    /// Construct the final `ModelConfig`.
    pub fn build(self) -> ModelConfig {
        self.instance
    }
}
