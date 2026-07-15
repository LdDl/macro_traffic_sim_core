# multiclass_network

In-memory 4-zone diamond network with multi-class PCU-based assignment.
No external files needed.

Identical to [`simple_network`](../simple_network/README.md) except the config includes two user classes
(car + truck). The only code difference is `.with_user_classes(...)` in the
config builder.

## Run

```sh
cargo run -example multiclass_network
```

## Network topology

4 intersection nodes arranged as a diamond, each serving as a zone centroid:

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

Detailed view with link IDs and zone attributes
(if your Markdown viewer does not render mermaid format, just use https://mermaid.live live viewer/editor):

```mermaid
graph TD
    N1["<b>Node 1</b><br/>Zone 1: Residential<br/>pop=6000, emp=500"]
    N2["<b>Node 2</b><br/>Zone 2: Mixed<br/>pop=4000, emp=2500"]
    N3["<b>Node 3</b><br/>Zone 3: Commercial<br/>pop=2000, emp=3000"]
    N4["<b>Node 4</b><br/>Zone 4: Industrial<br/>pop=2000, emp=2000"]

    N1 - "link 100<br/>~840 m" -> N2
    N2 - "link 101" -> N1
    N1 - "link 102<br/>~840 m" -> N3
    N3 - "link 103" -> N1
    N2 - "link 104<br/>~840 m" -> N4
    N4 - "link 105" -> N2
    N3 - "link 106<br/>~840 m" -> N4
    N4 - "link 107" -> N3
```

Each diamond edge is a pair of one-way road segment links (forward + reverse),
giving 8 road links total. At each intersection, connection links allow all
turns except U-turns (8 connection links total). Total: 16 links.

All road segments: 2 lanes, 60 km/h free speed, 1800 veh/h capacity.
Connection links: 10 m length, 30 km/h, 1800 veh/h.

## Zones

| Zone | Name               | Population | Employment |
|------|--------------------|------------|------------|
| 1    | Residential North  | 6000       | 500        |
| 2    | Mixed East         | 4000       | 2500       |
| 3    | Commercial West    | 2000       | 3000       |
| 4    | Industrial South   | 2000       | 2000       |

**Total:** pop=14000, emp=8000.

## Model parameters

**Trip generation** - regression with default coefficients:
- Production: $P_i = 0.5 \cdot pop_i + 0.1 \cdot emp_i$
- Attraction: $A_i = 0.1 \cdot pop_i + 0.8 \cdot emp_i$

The ratio $P_{total} / E_{total} = 1.75$ ensures balanced totals
(required for Furness/IPF convergence):

$$\sum P_i = \sum A_i = 7800$$

**Trip distribution** - gravity model with exponential impedance:

$$f(t) = e^{-0.1 \cdot t}$$

Furness (IPF) balancing with default tolerance ($10^{-6}$, max 100 iterations).

**Mode choice** - multinomial logit with 3 modes:

| Mode | ASC  | Time coeff |
|------|------|------------|
| Auto | 0.0  | -0.03      |
| Bike | -1.0 | -0.05      |
| Walk | -2.0 | -0.08      |

**Assignment** - multi-class Frank-Wolfe, max 50 iterations, convergence
gap $10^{-4}$, BPR function with default $\alpha=0.15$, $\beta=4.0$.

**Feedback** - 3 iterations (assignment costs update skim for next distribution/mode choice pass).

### Multi-class configuration

The AUTO OD matrix from mode choice is split into two user classes:

| Class | PCU factor | ff_time_multiplier | Demand fraction |
|-------|------------|-------------------|-----------------|
| car   | 1.0        | 1.0               | 0.9 (90%)       |
| truck | 2.5        | 2.5               | 0.1 (10%)       |

All classes share the same VDF (BPR). Formulation:

- Link flow is aggregated in PCU: $V_a = \sum_m x_a^m \cdot pcu_m$
- Per-class cost: $c_a^m = ff\_mult_m \cdot t_a(V_a)$
- Beckmann objective: $Z = \sum_a \int_0^{V_a} t_a(w) \, dw$

The Beckmann convex program structure is preserved when the symmetry
condition holds: $ff\_mult_m / pcu_m = const$ for all classes m.
This is validated at runtime. When the condition holds, Frank-Wolfe
converges to the multi-class User Equilibrium.

The simplest valid configuration is $ff\_mult_m = pcu_m$ (used here).
In this case the per-class cost multiplier is a constant applied equally
to all links, so it does not change shortest paths - classes differ only
in PCU weight and OD patterns, not routing. On larger networks with
heterogeneous links this still holds: a constant multiplier preserves
relative link costs.

**PCU factor**: one truck = 2.5 passenger car equivalents. A link carrying
100 cars + 40 trucks has PCU-total = 100 + 40*2.5 = 200 PCU.

**demand_fraction**: fractions must sum to 1.0. The pipeline validates this.

An alternative formulation uses class-specific VDFs (each class has its own
volume-delay function). That turns the problem into a variational inequality -
the Beckmann objective no longer applies, and solvers like diagonalization
(Dafermos, 1982) are needed. Convergence is not guaranteed for
general cost structures and computational cost is substantially higher.
We do not implement this.

Ref: Dafermos, S.C. (1972) "The Traffic Assignment Problem for
Multiclass-User Transportation Networks",
Transportation Science, 6(1), 73-87.
DOI: 10.1287/trsc.6.1.73

## Results

All output is structured JSON via `tracing`.

### Steps 1-3: Trip generation, distribution, mode choice

Identical to `simple_network` - see that README for the full step-by-step
derivation. The multi-class split happens after mode choice.

| Step | Result |
|------|--------|
| Trip generation | P=[3050, 2250, 1300, 1200], A=[1000, 2400, 2600, 1800] |
| Trip distribution | 7800 total trips, Furness converges in 5 iterations |
| Mode choice | Auto: 5692 (73%), Bike: 1787 (23%), Walk: 320 (4%) |

### Step 4: Multi-class traffic assignment

The AUTO OD (5692 trips) is split:
- Car OD = 0.9 * AUTO OD = 5123 trips
- Truck OD = 0.1 * AUTO OD = 569 trips

#### Multi-class Frank-Wolfe algorithm

```text
1. Initialize per-class volumes (AON at free-flow per class)
2. Loop:
   a. PCU total: V_a = sum_m(x_a^m * pcu_m)
   b. Shared cost: t_a = BPR(V_a)
   c. Per-class cost: c_a^m = ff_mult_m * t_a
   d. Per-class AON: y_a^m = shortest_path(OD_m, c^m)
   e. Auxiliary PCU total: W_a = sum_m(y_a^m * pcu_m)
   f. Relative gap on PCU totals
   g. Line search lambda on Beckmann (PCU totals)
   h. Update: x_a^m += lambda * (y_a^m - x_a^m)
```

Key insight: line search and convergence check operate on PCU-total volumes,
not per-class volumes. Per-class volumes are updated with the same lambda.

#### Feedback iteration 1 (cold start)

FW converges in **4 iterations** with gap $8.0 \times 10^{-5}$.

#### Feedback iteration 2 (warm start)

Per-class volumes from iteration 1 are reused as initial solution.
FW converges in **2 iterations** with gap $4.0 \times 10^{-5}$.

#### Feedback iteration 3 (warm start)

FW converges in **1 iteration** with gap $4.1 \times 10^{-5}$.

Warm start is effective: 4 -> 2 -> 1 iterations across feedback loops.

#### Per-class results

Per-class total vehicle-trips on the network (sum across all links):

| Class | Total veh-trips | PCU contribution |
|-------|-----------------|------------------|
| car   | 6883            | 6883 * 1.0 = 6883 |
| truck | 765             | 765 * 2.5 = 1912 |
| **PCU total** | | **8795** |

Note: the sum of per-class volumes exceeds the original demand (5692 auto
trips) because each trip traverses multiple links. "Total veh-trips on the
network" counts link traversals, not OD trips.

#### Final link volumes (PCU)

| Link | Direction | PCU volume | Cost (hours) | V/C ratio |
|------|-----------|------------|--------------|-----------|
| 102  | 1 -> 3    | 1555       | 0.0140       | 0.43      |
| 104  | 2 -> 4    | 1444       | 0.0140       | 0.40      |
| 100  | 1 -> 2    | 1353       | 0.0140       | 0.38      |
| 106  | 3 -> 4    | 1156       | 0.0140       | 0.32      |
| 107  | 4 -> 3    | 1141       | 0.0140       | 0.32      |
| 105  | 4 -> 2    | 953        | 0.0140       | 0.26      |
| 101  | 2 -> 1    | 746        | 0.0140       | 0.21      |
| 103  | 3 -> 1    | 448        | 0.0140       | 0.12      |

Compare to `simple_network` (single-class, same demand):

| Link | Single-class volume | Multi-class PCU volume | Difference |
|------|--------------------|-----------------------|------------|
| 102  | 1379               | 1555                  | +13%       |
| 104  | 1229               | 1444                  | +17%       |
| 100  | 1150               | 1353                  | +18%       |
| 106  | 1031               | 1156                  | +12%       |
| 107  | 984                | 1141                  | +16%       |
| 105  | 836                | 953                   | +14%       |
| 101  | 657                | 746                   | +14%       |
| 103  | 382                | 448                   | +17%       |

PCU volumes are ~15% higher across the board. This is expected: the same
5692 auto trips now include 10% trucks at PCU=2.5, adding
$569 \times (2.5 - 1.0) = 854$ extra PCU units to the network.

On this small network the routing is the same because there are no
meaningful alternative paths. On larger networks with route choice,
the higher PCU-total would cause more congestion on loaded links,
potentially shifting truck/car routes to different corridors.

### Convergence summary

| Feedback iter | FW iters | FW gap  | Warm start |
|---------------|----------|---------|------------|
| 1             | 4        | 8.0e-5  | no (cold)  |
| 2             | 2        | 4.0e-5  | yes        |
| 3             | 1        | 4.1e-5  | yes        |

### Timings

| Step           | Time (ms) |
|----------------|-----------|
| Generation     | 0.000     |
| Distribution   | 1.244     |
| Mode choice    | 0.108     |
| Assignment     | 0.949     |
| **Total**      | **3.229** |

---

## Difference from simple_network

The **only code difference** is in the config builder:

```rust
// simple_network
let config = ModelConfig::new()
    .with_assignment_method(AssignmentMethodType::FrankWolfe)
    .with_max_iterations(50)
    .with_convergence_gap(1e-4)
    .with_feedback_iterations(3)
    .with_verbose_level(VerboseLevel::Main)
    .build();

// multiclass_network - one extra line
let config = ModelConfig::new()
    .with_assignment_method(AssignmentMethodType::FrankWolfe)
    .with_max_iterations(50)
    .with_convergence_gap(1e-4)
    .with_feedback_iterations(3)
    .with_verbose_level(VerboseLevel::Main)
    .with_user_classes(vec![
        UserClassConfig::new("car", 1.0, 1.0, 0.9),
        UserClassConfig::new("truck", 2.5, 2.5, 0.1),
    ])
    .build();
```

Network, zones, trip generation, distribution, mode choice - all identical.
The pipeline detects `user_classes` in the config and automatically switches
to multi-class assignment for Step 4.
