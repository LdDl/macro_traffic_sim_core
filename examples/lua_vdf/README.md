# lua_vdf

Multi-class assignment with a Lua-scripted volume-delay function using
diagonalization. No external files needed.

Same 4-zone diamond network and OD demand as
[`diagonalization`](../diagonalization/README.md), but the truck VDF
is defined as a Lua script instead of a native `BprFunction`.

- Car: native `BprFunction(0.15, 4.0)` - compiled Rust, inlined by `VdfDispatch`
- Truck: `LuaVdf` running BPR(0.30, 4.0) in LuaJIT - vtable dispatch

The Lua script implements the same formula as the native truck VDF in
the `diagonalization` example, so results are identical. This serves
as both a usage demo and a correctness check.

## Run

```sh
cargo run --example lua_vdf --features lua
```

The `lua` feature is required. Without it, the `LuaVdf` type is not available.

## Lua script

```lua
local alpha = 0.30
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

See [SCRIPTING.md](../../SCRIPTING.md) for the full Lua VDF contract,
available environment, performance notes, and more examples.

## Network topology

Same diamond as all other examples:

```text
       Zone 1 (residential)
         |
    [1]--+--[2]
     |         |
Zone 2         Zone 3
(mixed)        (commercial)
     |         |
    [3]--+--[4]
         |
       Zone 4 (industrial)
```

4 nodes, 8 road links (bidirectional), 8 connection links. Total: 16 links.

## Multi-class configuration

| Class | PCU | ff_time_multiplier | VDF |
|-------|-----|--------------------|-----|
| car   | 1.0 | 1.0                | native BPR(0.15, 4.0) |
| truck | 2.5 | 1.0                | **Lua** BPR(0.30, 4.0) |

## Results

Matching the [`diagonalization`](../diagonalization/README.md) example
within IEEE 754 double precision:

| Parameter | Native | Lua | Difference |
|-----------|--------|-----|------------|
| Inner FW iterations | 881 | 881 | 0 |
| Outer gap | < 1e-6 | < 1e-6 | - |
| Car veh-trips | 34,004 | 34,003.7 | < 1 (rounding) |
| Truck veh-trips | 3,800 | 3,800.0 | 0 |
| PCU total | 43,504 | 43,503.7 | < 1 (rounding) |

The sub-unit differences (34,004 vs 34,003.7) come from floating-point
non-associativity: Lua and Rust may evaluate `a * (b + c)` in different
order at the machine level, producing results that differ in the last
few bits of the mantissa. Over 881 inner FW iterations these ULP-level
differences accumulate to ~0.3 vehicles out of 34,000 - well below any
practical significance. The iteration count, convergence gap, paths,
and route choices are identical.

### Zone 1 -> Zone 4

| Class | Flow | Cost (h) | Route |
|-------|------|----------|-------|
| car   | 4000 | 0.0848   | 100 -> 104 (via Zone 2) |
| truck | 500  | 0.1416   | 100 -> 104 (via Zone 2) |

### Select link: link 102 (1 -> 3)

| Origin | Dest | Class | Flow |
|--------|------|-------|------|
| 1      | 3    | car   | 3000 |
| 2      | 3    | car   | 1000 |
| 1      | 3    | truck | 300  |
| 2      | 3    | truck | 100  |
| **Total** | | | **4400** |

## Performance comparison

The native `diagonalization` example and this Lua example produce
identical results, but Lua adds overhead from crossing the Rust-Lua
boundary on every VDF call.

Benchmark on a 100-zone grid (see `benches/lua_vdf.rs`):

| Scenario | Time | vs native |
|----------|------|-----------|
| Both classes native | 10.5 ms | baseline |
| Truck Lua, car native | 29.9 ms | x2.9 |
| Both classes Lua | 89.2 ms | x8.5 |

Per-call: ~120 ns (LuaJIT) vs ~2.6 ns (native), ~46x overhead.
The full-algorithm ratio is smaller because Dijkstra (the dominant
cost) does not call the VDF.

## Difference from diagonalization example

Two lines change:

```rust
// diagonalization: native truck VDF
let bpr_truck = BprFunction::new(0.30, 4.0);
let class_vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];

// lua_vdf: Lua truck VDF
let lua_truck = LuaVdf::new(TRUCK_LUA_BPR).expect("failed to load Lua VDF");
let class_vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &lua_truck];
```

Everything else (network, OD, classes, assignment call) is identical.
The `VdfDispatch` enum in diagonalization recognizes `BprFunction` and
inlines it; `LuaVdf` falls through to the `Custom` variant and uses
vtable dispatch. No API changes needed.

## Reference

- [SCRIPTING.md](../../SCRIPTING.md) - full Lua VDF documentation
- [`diagonalization`](../diagonalization/README.md) - same example with native VDFs
