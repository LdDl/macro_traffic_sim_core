# Lua Scripting for Volume-Delay Functions

This document describes how to write custom volume-delay functions (VDFs) using [Lua](https://www.lua.org/). The engine embeds [LuaJIT](https://luajit.org/) (Lua 5.1 compatible, with select 5.2 extensions), so Lua scripts run inside the same process with no IPC (inter-process communication) overhead.

## Why Lua

The library ships three built-in VDFs: BPR, Conical (Spiess 1990), and Akcelik (1991). They cover most practical cases. When they do not - for example, a network with unusual intersection delay models, or a research formula not yet added to the core - you can supply a VDF as a Lua script instead of modifying the Rust source.

E.g. the architecture is: UI client -> Go HTTP API -> Intermediate gRPC ([here](https://github.com/LdDl/macro_traffic_sim_grpc)) -> Rust core. A custom expression parser would need to agree on syntax across all three layers. Lua avoids this: the formula arrives as a plain string and gets executed in the Rust core via the [mlua crate](https://crates.io/crates/mlua).

## Function contract

Every Lua VDF script **MUST** define exactly two global functions with
fixed signatures:

```lua
function travel_time(free_flow_time, volume, capacity)
    -- must return a number (travel time)
end

function integral(free_flow_time, volume, capacity)
    -- must return a number (integral of the VDF from 0 to volume)
end
```

### Rules

1. Both functions are required.

    The engine calls `travel_time` during cost updates and `integral` during the Frank-Wolfe line search (Beckmann objective). A missing function is a fatal error.

2. The signature is fixed: three arguments, one return value.

    All three parameters are always passed. Even if your formula does not depend on `capacity`, the function must still accept three arguments. Do not add, remove, or reorder parameters.

3. Return a single number.

    Returning nil, a string, or multiple values is an error.

4. The function names are case-sensitive.

    `travel_time` and `integral`, lowercase, with an underscore.

### Parameters

| Parameter | Type | Unit | Description |
|-----------|------|------|-------------|
| `free_flow_time` | number | hours | Link travel time at zero volume. Computed from link length and free-flow speed. Always > 0. |
| `volume` | number | PCU | Total PCU (passenger car units) on the link. Includes background traffic from other classes. Can be 0 or greater than capacity. |
| `capacity` | number | PCU/hour | Link capacity. Always > 0 in a well-formed network, but the function should handle <= 0 defensively. |

### Return value semantics

`travel_time(ff, v, c)` must return the travel time for a link with free-flow time `ff`, current volume `v`, and capacity `c`. The result must be `>= free_flow_time` for `v >= 0` (monotonically non-decreasing in volume). Return `math.huge` (Lua's infinity) when capacity is zero or negative.

`integral(ff, v, c)` must return the definite integral of the VDF from
0 to `v`:

```
            v
integral = int  travel_time(ff, w, c) dw
            0
```

This integral is needed by the Beckmann objective function used in Frank-Wolfe line search. If you cannot compute the closed-form integral for your formula, see "Numerical integration fallback" below.

## Examples

### BPR (Bureau of Public Roads)

```lua
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
```

### Linear delay

A simple linear VDF: `t(v) = ff * (1 + a * v / c)`.

```lua
local a = 0.5

function travel_time(ff, vol, cap)
    if cap <= 0 then return math.huge end
    return ff * (1.0 + a * vol / cap)
end

function integral(ff, vol, cap)
    if cap <= 0 then return math.huge end
    if vol <= 0 then return 0.0 end
    return ff * (vol + a * vol * vol / (2.0 * cap))
end
```

### Capacity-independent formula

Even if your model ignores capacity, accept all three parameters:

```lua
local k = 0.001

function travel_time(ff, vol, _cap)
    return ff + k * vol
end

function integral(ff, vol, _cap)
    if vol <= 0 then return 0.0 end
    return ff * vol + k * vol * vol / 2.0
end
```

Prefix unused parameters with underscore as a convention, but do not
remove them from the signature.

## Available Lua environment

The script runs in a standard LuaJIT environment. The following
globals are available:

- `math` - full math library (`math.sqrt`, `math.exp`, `math.log`,
  `math.pow`, `math.huge`, `math.pi`, etc.)
- `string` - string library (rarely needed for VDFs)
- `table` - table library
- `print` - for debugging only; output goes to stderr
- `tonumber`, `tostring`, `type`, `error`, `pcall`
- `^` operator - exponentiation (`x^4` is `math.pow(x, 4)`)

Not available (sandboxed out): `io`, `os`, `require`, `loadfile`,
`dofile`. VDF scripts cannot read files or execute system commands.

## Script lifecycle

1. The script is loaded and executed once at initialization.
   Top-level code runs at this point (e.g., `local alpha = 0.15`).
2. `travel_time` and `integral` are called many times during
   assignment - potentially millions of times on large networks.
3. The Lua state persists for the lifetime of the assignment run.
   Global variables set at the top level remain available across calls.

Top-level computation (e.g., precomputing derived constants) runs once
and costs nothing at call time:

```lua
local alpha = 0.15
local beta = 4.0
local beta_plus_1 = beta + 1.0
local inv_beta_plus_1 = 1.0 / beta_plus_1
```

## Numerical integration fallback

If no closed-form integral exists for your VDF, you can approximate it
with numerical quadrature in Lua. This is slower than a closed-form
solution but works for any formula.

Simpson's rule example:

```lua
local N = 50

local function simpson(ff, v, cap)
    if v <= 0 then return 0.0 end
    local h = v / N
    local s = travel_time(ff, 0, cap) + travel_time(ff, v, cap)
    for i = 1, N - 1, 2 do
        s = s + 4.0 * travel_time(ff, i * h, cap)
    end
    for i = 2, N - 2, 2 do
        s = s + 2.0 * travel_time(ff, i * h, cap)
    end
    return s * h / 3.0
end

function integral(ff, vol, cap)
    if cap <= 0 then return math.huge end
    return simpson(ff, vol, cap)
end
```

This calls `travel_time` ~50 times per `integral` call. On large
networks the overhead is significant. Prefer a closed-form integral
when possible.

## Performance

Lua VDFs are slower than native Rust VDFs. Benchmark results on a
100-zone grid with 2 classes (diagonalization, 10 outer iterations):

| Scenario | Time | vs native |
|----------|------|-----------|
| Both classes native BPR | 10.5 ms | baseline |
| One class Lua, one native | 29.9 ms | x2.9 |
| Both classes Lua | 89.2 ms | x8.5 |

Per-call overhead: ~120 ns (Lua/LuaJIT) vs ~2.6 ns (native Rust),
roughly 46x per individual call. The full-algorithm slowdown is
smaller because Dijkstra shortest-path (the dominant cost) does not
call the VDF.

### When the overhead matters

- **Small networks (< 1000 links):** Lua overhead is negligible
  in absolute terms (milliseconds).
- **Large networks (> 50k links), many iterations:** Lua VDF adds
  measurable time. Consider whether a built-in VDF can approximate
  your formula.
- **Mixed setup (recommended):** Use Lua only for the class that
  needs a custom formula. Other classes use native VDFs and pay
  no overhead. The `VdfDispatch` enum handles this automatically -
  built-in VDFs get inlined via match, Lua falls back to vtable
  dispatch.

### Optimization tips

- Precompute constants at the top level, not inside the function.
- Avoid creating tables or strings inside `travel_time`/`integral`.
- Use local variables (`local x = ...`) instead of globals inside
  hot functions - LuaJIT optimizes locals better.
- `x^4` is faster than `math.pow(x, 4)` in LuaJIT.

## Validation

Before using a Lua VDF in production, verify:

1. **Correctness.** Compare `travel_time` output against a known
   implementation for several volume levels (0, 0.5*cap, cap, 2*cap).

2. **Integral consistency.** The derivative of `integral` must equal
   `travel_time`. Check numerically:
   ```
   h = 0.001
   numerical = (integral(ff, v+h, c) - integral(ff, v, c)) / h
   analytical = travel_time(ff, v, c)
   assert |numerical - analytical| < 1e-4
   ```

3. **Edge cases.** Test with `volume = 0`, `capacity = 0`,
   `volume >> capacity`.

4. **Monotonicity.** `travel_time` should be non-decreasing in volume.
   Non-monotone VDFs can cause assignment oscillation.

## Error handling

| Error | When | Effect |
|-------|------|--------|
| Script syntax error | Load time | Assignment fails immediately |
| `travel_time` not defined | First call | Assignment fails immediately |
| `integral` not defined | First line search | Assignment fails immediately |
| Function returns nil | Call time | Assignment fails with Lua error |
| Function errors (division by zero, etc.) | Call time | Assignment fails with Lua error |

All errors propagate as `AssignmentError::LuaError` with the Lua
error message included.
