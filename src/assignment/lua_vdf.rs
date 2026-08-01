//! Lua-scripted volume-delay functions.
//!
//! Allows users to define custom VDFs as Lua scripts instead of
//! implementing the [`VolumeDelayFunction`] trait in Rust.
//! The script must define two global functions: `travel_time` and
//! `integral`, each taking `(free_flow_time, volume, capacity)`
//! and returning a number.
//!
//! # Sandboxing
//!
//! Scripts run in a restricted Lua 5.4 environment:
//!
//! * Only the `math`, `string` and `table` standard libraries are
//!   loaded. There is no `os`, `io`, `debug` or `package`, so scripts
//!   cannot touch the filesystem, spawn processes or load C code.
//! * `pcall`, `xpcall`, `load`, `loadstring`, `dofile`, `loadfile`
//!   and `collectgarbage` are removed from the globals. Without
//!   `pcall` a script cannot swallow the watchdog errors below.
//! * An instruction-count watchdog aborts any single VDF call that
//!   executes more than [`MAX_INSTRUCTIONS_PER_CALL`] VM instructions
//!   (an honest formula needs well under a thousand). This turns
//!   infinite loops into `AssignmentError::LuaError` instead of a
//!   hung process.
//! * The Lua allocator is capped at [`MEMORY_LIMIT_BYTES`], so a
//!   runaway string/table cannot exhaust host memory.
//! * Unbounded recursion is stopped by Lua's own stack check and
//!   surfaces as a regular error.
//!
//! [`LuaVdf::new`] also runs both functions on a set of probe inputs
//! (including zero capacity), so most broken scripts fail at
//! construction time rather than in the middle of an assignment.
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
//! assert!((vdf.travel_time(10.0, 0.0, 1000.0).unwrap() - 10.0).abs() < 1e-10);
//! assert!((vdf.travel_time(10.0, 1000.0, 1000.0).unwrap() - 11.5).abs() < 1e-10);
//! ```

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use mlua::{HookTriggers, Lua, LuaOptions, StdLib, VmState};

use super::assignment::VolumeDelayFunction;
use super::error::AssignmentError;

/// The watchdog hook fires every this many VM instructions.
const HOOK_INTERVAL: u32 = 10_000;

/// Maximum VM instructions a single `travel_time`/`integral` call may
/// execute before it is aborted. A typical formula needs under a
/// thousand, so this is a ~1000x safety margin, while an infinite
/// loop is cut off in well under a millisecond.
pub const MAX_INSTRUCTIONS_PER_CALL: u64 = 1_000_000;

/// Memory limit for the Lua state (64 MiB).
pub const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

// Probe inputs used by `LuaVdf::new` to validate both functions:
// free flow, mid load, at capacity, over capacity, and zero capacity.
const PROBE_INPUTS: [(f64, f64, f64); 5] = [
    (10.0, 0.0, 1000.0),
    (10.0, 500.0, 1000.0),
    (10.0, 1000.0, 1000.0),
    (10.0, 2000.0, 1000.0),
    (10.0, 100.0, 0.0),
];

// Base-library globals that are removed from the sandbox. Formulas do
// not need them, and `pcall`/`xpcall` in particular would let a script
// catch the watchdog error and keep spinning.
const REMOVED_GLOBALS: [&str; 7] = [
    "pcall",
    "xpcall",
    "load",
    "loadstring",
    "dofile",
    "loadfile",
    "collectgarbage",
];

/// A volume-delay function defined by a Lua script.
///
/// The Lua state is created once and reused for all calls.
/// Built-in VDFs (BPR, Conical, Akcelik) are ~35x faster per call;
/// use `LuaVdf` only when no built-in matches your formula.
///
/// The script runs sandboxed: see the [module docs](self) for the
/// exact restrictions (stdlib subset, instruction budget, memory
/// limit).
///
/// # Arguments (constructor)
///
/// * `script` - Lua source defining `travel_time(ff, vol, cap)` and
///   `integral(ff, vol, cap)`. See `SCRIPTING.md` for the full contract.
pub struct LuaVdf {
    lua: Lua,
    travel_time_fn: mlua::Function,
    integral_fn: mlua::Function,
    // Instructions executed by the current call, counted in
    // HOOK_INTERVAL steps by the watchdog hook. Reset before each call.
    instructions: Rc<Cell<u64>>,
}

impl std::fmt::Debug for LuaVdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaVdf").finish_non_exhaustive()
    }
}

impl LuaVdf {
    /// Create a Lua VDF from a script string.
    ///
    /// Builds a sandboxed Lua 5.4 state (math/string/table only,
    /// instruction watchdog, memory limit), loads and executes the
    /// script, validates that both `travel_time` and `integral` are
    /// defined as global functions, and runs both on a set of probe
    /// inputs (free flow, mid load, at/over capacity, zero capacity).
    ///
    /// # Arguments
    ///
    /// * `script` - Lua source code. Must define `travel_time(ff, vol, cap)`
    ///   and `integral(ff, vol, cap)` as global functions.
    ///
    /// # Errors
    ///
    /// Returns `AssignmentError::LuaError` if the script has syntax
    /// errors, does not define the required functions, exceeds the
    /// instruction or memory budget, or fails on any probe input.
    pub fn new(script: &str) -> Result<Self, AssignmentError> {
        let lua = Lua::new_with(
            StdLib::MATH | StdLib::STRING | StdLib::TABLE,
            LuaOptions::default(),
        )
        .map_err(|e| AssignmentError::LuaError(format!("sandbox init: {}", e)))?;

        lua.set_memory_limit(MEMORY_LIMIT_BYTES)
            .map_err(|e| AssignmentError::LuaError(format!("memory limit: {}", e)))?;

        for name in REMOVED_GLOBALS {
            lua.globals()
                .set(name, mlua::Nil)
                .map_err(|e| AssignmentError::LuaError(format!("sandbox setup: {}", e)))?;
        }

        let instructions = Rc::new(Cell::new(0_u64));
        let counter = Rc::clone(&instructions);
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(HOOK_INTERVAL),
            move |_lua, _debug| {
                let executed = counter.get() + HOOK_INTERVAL as u64;
                counter.set(executed);
                if executed >= MAX_INSTRUCTIONS_PER_CALL {
                    Err(mlua::Error::RuntimeError(format!(
                        "script exceeded the budget of {} VM instructions per call \
                         (infinite loop?)",
                        MAX_INSTRUCTIONS_PER_CALL
                    )))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )
        .map_err(|e| AssignmentError::LuaError(format!("watchdog setup: {}", e)))?;

        instructions.set(0);
        lua.load(script)
            .exec()
            .map_err(|e| AssignmentError::LuaError(format!("script load: {}", e)))?;

        let travel_time_fn: mlua::Function = lua.globals().get("travel_time").map_err(|_| {
            AssignmentError::LuaError("script must define a global 'travel_time' function".into())
        })?;
        let integral_fn: mlua::Function = lua.globals().get("integral").map_err(|_| {
            AssignmentError::LuaError("script must define a global 'integral' function".into())
        })?;

        let vdf = LuaVdf {
            lua,
            travel_time_fn,
            integral_fn,
            instructions,
        };

        for (ff, vol, cap) in PROBE_INPUTS {
            vdf.call("travel_time", &vdf.travel_time_fn, ff, vol, cap)?;
            vdf.call("integral", &vdf.integral_fn, ff, vol, cap)?;
        }

        Ok(vdf)
    }

    // Invoke a script function under the instruction watchdog.
    fn call(
        &self,
        name: &str,
        function: &mlua::Function,
        free_flow_time: f64,
        volume: f64,
        capacity: f64,
    ) -> Result<f64, AssignmentError> {
        self.instructions.set(0);
        function
            .call::<f64>((free_flow_time, volume, capacity))
            .map_err(|e| {
                AssignmentError::LuaError(format!(
                    "{}(ff={}, vol={}, cap={}) failed: {}",
                    name, free_flow_time, volume, capacity, e
                ))
            })
    }

    /// Memory currently used by the Lua state, in bytes.
    pub fn used_memory(&self) -> usize {
        self.lua.used_memory()
    }
}

impl VolumeDelayFunction for LuaVdf {
    fn travel_time(
        &self,
        free_flow_time: f64,
        volume: f64,
        capacity: f64,
    ) -> Result<f64, AssignmentError> {
        self.call(
            "travel_time",
            &self.travel_time_fn,
            free_flow_time,
            volume,
            capacity,
        )
    }

    fn integral(
        &self,
        free_flow_time: f64,
        volume: f64,
        capacity: f64,
    ) -> Result<f64, AssignmentError> {
        self.call(
            "integral",
            &self.integral_fn,
            free_flow_time,
            volume,
            capacity,
        )
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
            let lt = lua_vdf.travel_time(10.0, vol, 1000.0).unwrap();
            let nt = native.travel_time(10.0, vol, 1000.0).unwrap();
            assert!(
                (lt - nt).abs() < 1e-10,
                "travel_time mismatch at vol={}: lua={}, native={}",
                vol,
                lt,
                nt
            );

            let li = lua_vdf.integral(10.0, vol, 1000.0).unwrap();
            let ni = native.integral(10.0, vol, 1000.0).unwrap();
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
        assert_eq!(vdf.travel_time(10.0, 100.0, 0.0).unwrap(), f64::INFINITY);
        assert_eq!(vdf.integral(10.0, 100.0, 0.0).unwrap(), f64::INFINITY);
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

    #[test]
    fn infinite_loop_at_load_time_errors() {
        let script = "while true do end";
        let err = LuaVdf::new(script).unwrap_err();
        match err {
            AssignmentError::LuaError(msg) => {
                assert!(msg.contains("budget"), "unexpected: {}", msg);
            }
            other => panic!("expected LuaError, got: {:?}", other),
        }
    }

    #[test]
    fn infinite_loop_in_function_errors() {
        // The loop only triggers over capacity, so the free-flow and
        // mid-load probes pass and the over-capacity probe catches it.
        let script = r#"
            function travel_time(ff, vol, cap)
                if cap > 0 and vol > cap then
                    while true do end
                end
                return ff
            end
            function integral(ff, vol, cap) return ff * vol end
        "#;
        let err = LuaVdf::new(script).unwrap_err();
        assert!(matches!(err, AssignmentError::LuaError(_)));
    }

    #[test]
    fn infinite_recursion_errors_instead_of_panicking() {
        let script = r#"
            function travel_time(ff, vol, cap)
                return travel_time(ff, vol, cap)
            end
            function integral(ff, vol, cap) return 0 end
        "#;
        let err = LuaVdf::new(script).unwrap_err();
        assert!(matches!(err, AssignmentError::LuaError(_)));
    }

    #[test]
    fn memory_bomb_errors() {
        let script = r#"
            function travel_time(ff, vol, cap)
                local s = "x"
                while true do s = s .. s end
            end
            function integral(ff, vol, cap) return 0 end
        "#;
        // Either the memory limit or the instruction budget stops it;
        // both surface as LuaError.
        let err = LuaVdf::new(script).unwrap_err();
        assert!(matches!(err, AssignmentError::LuaError(_)));
    }

    #[test]
    fn os_and_io_are_unavailable() {
        let script = r#"
            assert(os == nil, "os must not be available")
            assert(io == nil, "io must not be available")
            assert(pcall == nil, "pcall must not be available")
            function travel_time(ff, vol, cap) return ff end
            function integral(ff, vol, cap) return ff * vol end
        "#;
        LuaVdf::new(script).unwrap();
    }

    #[test]
    fn pcall_cannot_swallow_the_watchdog() {
        // With pcall removed, a script cannot catch the budget error
        // and keep looping; referencing pcall is itself an error.
        let script = r#"
            function travel_time(ff, vol, cap)
                while true do pcall(function() end) end
            end
            function integral(ff, vol, cap) return 0 end
        "#;
        let err = LuaVdf::new(script).unwrap_err();
        assert!(matches!(err, AssignmentError::LuaError(_)));
    }

    #[test]
    fn runtime_error_in_script_is_reported_with_inputs() {
        let script = r#"
            function travel_time(ff, vol, cap)
                if cap <= 0 then error("bad capacity") end
                return ff
            end
            function integral(ff, vol, cap) return ff * vol end
        "#;
        // The zero-capacity probe fails during construction.
        let err = LuaVdf::new(script).unwrap_err();
        match err {
            AssignmentError::LuaError(msg) => {
                assert!(msg.contains("cap=0"), "unexpected: {}", msg);
                assert!(msg.contains("bad capacity"), "unexpected: {}", msg);
            }
            other => panic!("expected LuaError, got: {:?}", other),
        }
    }

    #[test]
    fn long_but_finite_computation_succeeds() {
        // A numerically integrated VDF stays well within the budget.
        let script = r#"
            function travel_time(ff, vol, cap)
                if cap <= 0 then return math.huge end
                return ff * (1.0 + 0.15 * (vol / cap) ^ 4)
            end
            function integral(ff, vol, cap)
                if cap <= 0 then return math.huge end
                if vol <= 0 then return 0.0 end
                local n = 1000
                local h = vol / n
                local sum = travel_time(ff, 0, cap) + travel_time(ff, vol, cap)
                for i = 1, n - 1 do
                    local w = (i % 2 == 1) and 4 or 2
                    sum = sum + w * travel_time(ff, i * h, cap)
                end
                return sum * h / 3.0
            end
        "#;
        let vdf = LuaVdf::new(script).unwrap();
        let exact = 10.0 * (1000.0 + 0.15 * 1000.0 / 5.0);
        let approx = vdf.integral(10.0, 1000.0, 1000.0).unwrap();
        assert!(
            (approx - exact).abs() / exact < 1e-6,
            "simpson vs exact: {} vs {}",
            approx,
            exact
        );
    }
}
