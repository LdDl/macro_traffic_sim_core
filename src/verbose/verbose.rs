use std::fmt;
use std::sync::OnceLock;
use tracing::{Level, debug, info, trace};
use tracing_subscriber::{
    EnvFilter, fmt as tracing_fmt, layer::SubscriberExt, reload, util::SubscriberInitExt,
};

/// Hierarchical logging levels for simulation debugging.
///
/// Each level includes all lower levels, providing increasingly detailed output.
/// Uses JSON structured logging via the `tracing` crate.
///
/// # Examples
///
/// ```rust
/// use macro_traffic_sim_core::verbose::{VerboseLevel, set_verbose_level};
///
/// set_verbose_level(VerboseLevel::Main);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum VerboseLevel {
    /// No logging at all.
    None = 0,
    /// Major simulation phases - `info` level.
    Main = 1,
    /// Function-level details - `debug` level.
    Additional = 2,
    /// Everything including traces - `trace` level.
    All = 3,
}

impl fmt::Display for VerboseLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            VerboseLevel::None => "none",
            VerboseLevel::Main => "main",
            VerboseLevel::Additional => "additional",
            VerboseLevel::All => "all",
        };
        write!(f, "{}", s)
    }
}

impl From<VerboseLevel> for Level {
    fn from(level: VerboseLevel) -> Self {
        match level {
            VerboseLevel::None => Level::ERROR,
            VerboseLevel::Main => Level::INFO,
            VerboseLevel::Additional => Level::DEBUG,
            VerboseLevel::All => Level::TRACE,
        }
    }
}

impl From<VerboseLevel> for String {
    fn from(level: VerboseLevel) -> Self {
        match level {
            VerboseLevel::None => "error".to_string(),
            VerboseLevel::Main => "info".to_string(),
            VerboseLevel::Additional => "debug".to_string(),
            VerboseLevel::All => "trace".to_string(),
        }
    }
}

pub const EVENT_PIPELINE: &str = "pipeline";
pub const EVENT_NETWORK_LOAD: &str = "network_load";
pub const EVENT_TRIP_GENERATION: &str = "trip_generation";
pub const EVENT_TRIP_DISTRIBUTION: &str = "trip_distribution";
pub const EVENT_MODE_CHOICE: &str = "mode_choice";
pub const EVENT_ASSIGNMENT: &str = "assignment";
pub const EVENT_ASSIGNMENT_ITERATION: &str = "assignment_iteration";
pub const EVENT_SHORTEST_PATH: &str = "shortest_path";
pub const EVENT_CONVERGENCE: &str = "convergence";
pub const EVENT_FEEDBACK_LOOP: &str = "feedback_loop";
pub const EVENT_FURNESS_ITERATION: &str = "furness_iteration";
pub const EVENT_GRAVITY_MODEL: &str = "gravity_model";

static VERBOSE_LEVEL: OnceLock<VerboseLevel> = OnceLock::new();
static LOGGER_INITIALIZED: OnceLock<bool> = OnceLock::new();
static RELOAD_HANDLE: OnceLock<reload::Handle<EnvFilter, tracing_subscriber::Registry>> =
    OnceLock::new();

/// Initialize the tracing logger once.
pub fn init_logger() {
    if LOGGER_INITIALIZED.set(true).is_ok() {
        let default_level = String::from(*VERBOSE_LEVEL.get().unwrap_or(&VerboseLevel::Main));
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
        let (filter_layer, handle) = reload::Layer::new(env_filter);
        let _ = RELOAD_HANDLE.set(handle);
        tracing_subscriber::registry()
            .with(filter_layer)
            .with(
                tracing_fmt::layer()
                    .json()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_thread_names(false)
                    .with_file(false)
                    .with_line_number(false),
            )
            .init();
    }
}

/// Sets the global verbose level and updates tracing filter.
pub fn set_verbose_level(level: VerboseLevel) {
    let _ = VERBOSE_LEVEL.set(level);
    init_logger();
    if let Some(handle) = RELOAD_HANDLE.get() {
        let _ = handle.modify(|f| {
            *f = EnvFilter::new(String::from(level));
        });
    }
}

/// Gets the current global verbose level.
pub fn get_verbose_level() -> VerboseLevel {
    *VERBOSE_LEVEL.get().unwrap_or(&VerboseLevel::None)
}

/// Checks if current global verbose level is at least the specified level.
pub fn is_verbose_level(level: VerboseLevel) -> bool {
    get_verbose_level() >= level
}

/// Logs a message if the global verbose level allows it.
pub fn verbose_log(level: VerboseLevel, event: &str, message: &str) {
    if !is_verbose_level(level) {
        return;
    }
    match level {
        VerboseLevel::None => {}
        VerboseLevel::Main => {
            info!(event = event, message);
        }
        VerboseLevel::Additional => {
            debug!(event = event, message);
        }
        VerboseLevel::All => {
            trace!(event = event, message);
        }
    }
}

/// Logs a message with additional fields using global verbose level.
pub fn verbose_log_with_fields(
    level: VerboseLevel,
    event: &str,
    message: &str,
    fields: &[(&str, &dyn fmt::Display)],
) {
    if !is_verbose_level(level) {
        return;
    }
    let mut field_map = std::collections::HashMap::new();
    for (key, value) in fields {
        field_map.insert(*key, format!("{}", value));
    }
    match level {
        VerboseLevel::None => {}
        VerboseLevel::Main => {
            info!(event = event, ?field_map, message);
        }
        VerboseLevel::Additional => {
            debug!(event = event, ?field_map, message);
        }
        VerboseLevel::All => {
            trace!(event = event, ?field_map, message);
        }
    }
}

impl VerboseLevel {
    /// Logs a message if this verbose level allows it.
    pub fn log(self, event: &str, message: &str) {
        if self == VerboseLevel::None {
            return;
        }
        match self {
            VerboseLevel::None => {}
            VerboseLevel::Main => info!(event = event, message),
            VerboseLevel::Additional => debug!(event = event, message),
            VerboseLevel::All => trace!(event = event, message),
        }
    }

    /// Logs a message with fields if this verbose level allows it.
    pub fn log_with_fields(self, event: &str, message: &str, fields: &[(&str, &dyn fmt::Display)]) {
        if self == VerboseLevel::None {
            return;
        }
        let mut field_map = std::collections::HashMap::new();
        for (key, value) in fields {
            field_map.insert(*key, format!("{}", value));
        }
        match self {
            VerboseLevel::None => {}
            VerboseLevel::Main => info!(event = event, ?field_map, message),
            VerboseLevel::Additional => debug!(event = event, ?field_map, message),
            VerboseLevel::All => trace!(event = event, ?field_map, message),
        }
    }

    /// Checks if this level is at least the minimum level.
    pub fn is_at_least(self, min_level: VerboseLevel) -> bool {
        self >= min_level
    }
}

/// Convenience macro for global verbose logging.
#[macro_export]
macro_rules! verbose_log {
    ($level:expr, $event:expr, $msg:literal) => {
        $crate::verbose::verbose_log($level, $event, $msg)
    };
    ($level:expr, $event:expr, $msg:literal, $($key:literal => $value:expr),+) => {
        $crate::verbose::verbose_log_with_fields(
            $level,
            $event,
            $msg,
            &[$(($key, &$value)),+]
        )
    };
}

/// Logs an info-level message if the global verbose level is [`VerboseLevel::Main`] or higher.
#[macro_export]
macro_rules! log_main {
    ($event:expr, $msg:literal, $($key:ident = $value:expr),*) => {
        if $crate::verbose::is_verbose_level($crate::verbose::VerboseLevel::Main) {
            tracing::info!(
                event = $event,
                $($key = $value,)*
                $msg
            );
        }
    };
}

/// Logs a debug-level message if the global verbose level is [`VerboseLevel::Additional`] or higher.
#[macro_export]
macro_rules! log_additional {
    ($event:expr, $msg:literal, $($key:ident = $value:expr),*) => {
        if $crate::verbose::is_verbose_level($crate::verbose::VerboseLevel::Additional) {
            tracing::debug!(
                event = $event,
                $($key = $value,)*
                $msg
            );
        }
    };
}

/// Logs a trace-level message if the global verbose level is [`VerboseLevel::All`].
#[macro_export]
macro_rules! log_all {
    ($event:expr, $msg:literal, $($key:ident = $value:expr),*) => {
        if $crate::verbose::is_verbose_level($crate::verbose::VerboseLevel::All) {
            tracing::trace!(
                event = $event,
                $($key = $value,)*
                $msg
            );
        }
    };
}
