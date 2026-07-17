//! Lua-scripted volume-delay functions.
//!
//! Allows users to define custom VDFs as Lua scripts instead of
//! implementing the [`VolumeDelayFunction`] trait in Rust.
//! The script must define two global functions: `travel_time` and
//! `integral`, each taking `(free_flow_time, volume, capacity)`
//! and returning a number.
//!
//! See `SCRIPTING.md` for the full contract, examples, and
//! performance notes.
//!
//! # Examples
//!
//! ```
//! use macro_traffic_sim_core::assignment::lua_vdf::LuaVdf;
//! use macro_traffic_sim_core::assignment::VolumeDelayFunction;
//!
//! let script = r#"
//!     local alpha = 0.15
//!     local beta = 4.0
//!     function travel_time(ff, vol, cap)
//!         if cap <= 0 then return math.huge end
//!         return ff * (1.0 + alpha * (vol / cap) ^ beta)
//!     end
//!     function integral(ff, vol, cap)
//!         if cap <= 0 then return math.huge end
//!         if vol <= 0 then return 0.0 end
//!         local ratio = vol / cap
//!         return ff * (vol + alpha * cap * ratio ^ (beta + 1.0) / (beta + 1.0))
//!     end
//! "#;
//!
//! let vdf = LuaVdf::new(script).unwrap();
//!
//! // Same result as BprFunction::default()
//! assert!((vdf.travel_time(10.0, 0.0, 1000.0) - 10.0).abs() < 1e-10);
//! assert!((vdf.travel_time(10.0, 1000.0, 1000.0) - 11.5).abs() < 1e-10);
//! ```

use std::any::Any;

use mlua::Lua;

use super::assignment::VolumeDelayFunction;
use super::error::AssignmentError;

/// A volume-delay function defined by a Lua script.
///
/// The Lua state is created once and reused for all calls.
/// Built-in VDFs (BPR, Conical, Akcelik) are ~46x faster per call;
/// use `LuaVdf` only when no built-in matches your formula.
///
/// # Arguments (constructor)
///
/// * `script` - Lua source defining `travel_time(ff, vol, cap)` and
///   `integral(ff, vol, cap)`. See `SCRIPTING.md` for the full contract.
pub struct LuaVdf {
    lua: Lua,
}

impl std::fmt::Debug for LuaVdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaVdf").finish_non_exhaustive()
    }
}

impl LuaVdf {
    /// Create a Lua VDF from a script string.
    ///
    /// Loads and executes the script, then validates that both
    /// `travel_time` and `integral` are defined as global functions.
    ///
    /// # Arguments
    ///
    /// * `script` - Lua source code. Must define `travel_time(ff, vol, cap)`
    ///   and `integral(ff, vol, cap)` as global functions.
    ///
    /// # Errors
    ///
    /// Returns `AssignmentError::LuaError` if the script has syntax errors
    /// or does not define the required functions.
    pub fn new(script: &str) -> Result<Self, AssignmentError> {
        let lua = Lua::new();
        lua.load(script)
            .exec()
            .map_err(|e| AssignmentError::LuaError(format!("script load: {}", e)))?;

        let _: mlua::Function = lua
            .globals()
            .get("travel_time")
            .map_err(|_| {
                AssignmentError::LuaError(
                    "script must define a global 'travel_time' function".into(),
                )
            })?;
        let _: mlua::Function = lua
            .globals()
            .get("integral")
            .map_err(|_| {
                AssignmentError::LuaError(
                    "script must define a global 'integral' function".into(),
                )
            })?;

        Ok(LuaVdf { lua })
    }
}

impl VolumeDelayFunction for LuaVdf {
    fn travel_time(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64 {
        let f: mlua::Function = self
            .lua
            .globals()
            .get("travel_time")
            .expect("travel_time disappeared from Lua globals");
        f.call::<f64>((free_flow_time, volume, capacity))
            .expect("Lua travel_time() error")
    }

    fn integral(&self, free_flow_time: f64, volume: f64, capacity: f64) -> f64 {
        let f: mlua::Function = self
            .lua
            .globals()
            .get("integral")
            .expect("integral disappeared from Lua globals");
        f.call::<f64>((free_flow_time, volume, capacity))
            .expect("Lua integral() error")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BPR_SCRIPT: &str = r#"
        local alpha = 0.15
        local beta = 4.0
        function travel_time(ff, vol, cap)
            if cap <= 0 then return math.huge end
            return ff * (1.0 + alpha * (vol / cap) ^ beta)
        end
        function integral(ff, vol, cap)
            if cap <= 0 then return math.huge end
            if vol <= 0 then return 0.0 end
            local ratio = vol / cap
            return ff * (vol + alpha * cap * ratio ^ (beta + 1.0) / (beta + 1.0))
        end
    "#;

    #[test]
    fn lua_bpr_matches_native() {
        use crate::assignment::BprFunction;

        let lua_vdf = LuaVdf::new(BPR_SCRIPT).unwrap();
        let native = BprFunction::default();

        for vol in [0.0, 500.0, 1000.0, 2000.0] {
            let lt = lua_vdf.travel_time(10.0, vol, 1000.0);
            let nt = native.travel_time(10.0, vol, 1000.0);
            assert!(
                (lt - nt).abs() < 1e-10,
                "travel_time mismatch at vol={}: lua={}, native={}",
                vol,
                lt,
                nt
            );

            let li = lua_vdf.integral(10.0, vol, 1000.0);
            let ni = native.integral(10.0, vol, 1000.0);
            assert!(
                (li - ni).abs() < 1e-6,
                "integral mismatch at vol={}: lua={}, native={}",
                vol,
                li,
                ni
            );
        }
    }

    #[test]
    fn lua_zero_capacity_returns_infinity() {
        let vdf = LuaVdf::new(BPR_SCRIPT).unwrap();
        assert_eq!(vdf.travel_time(10.0, 100.0, 0.0), f64::INFINITY);
        assert_eq!(vdf.integral(10.0, 100.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn missing_travel_time_errors() {
        let script = r#"
            function integral(ff, vol, cap) return 0 end
        "#;
        let err = LuaVdf::new(script).unwrap_err();
        match err {
            AssignmentError::LuaError(msg) => {
                assert!(msg.contains("travel_time"), "unexpected: {}", msg);
            }
            other => panic!("expected LuaError, got: {:?}", other),
        }
    }

    #[test]
    fn missing_integral_errors() {
        let script = r#"
            function travel_time(ff, vol, cap) return ff end
        "#;
        let err = LuaVdf::new(script).unwrap_err();
        match err {
            AssignmentError::LuaError(msg) => {
                assert!(msg.contains("integral"), "unexpected: {}", msg);
            }
            other => panic!("expected LuaError, got: {:?}", other),
        }
    }

    #[test]
    fn syntax_error_in_script() {
        let script = "function travel_time(ff, vol, cap";
        let err = LuaVdf::new(script).unwrap_err();
        assert!(matches!(err, AssignmentError::LuaError(_)));
    }
}
