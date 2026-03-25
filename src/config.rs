//! # Configuration
//!
//! Model configuration structures for the 4-step traffic demand model.
//!
//! ## Components
//!
//! - [`AssignmentMethodType`] -- selects the assignment algorithm
//! - [`ModelConfig`] -- all parameters for the pipeline
//! - [`ModelConfigBuilder`] -- builder for `ModelConfig`
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

use crate::assignment::{AssignmentConfig, BprFunction};
use crate::trip_distribution::furness::FurnessConfig;
use crate::verbose::VerboseLevel;

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

    /// Construct the final `ModelConfig`.
    pub fn build(self) -> ModelConfig {
        self.instance
    }
}
