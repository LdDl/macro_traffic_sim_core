# diagonalization

Multi-class assignment with per-class volume-delay functions using the
Gauss-Seidel diagonalization algorithm (Dafermos, 1982). No external files needed.

Unlike the Beckmann-based multiclass FW/MSA (which requires the symmetry condition
`ff_time_multiplier / pcu = const`), diagonalization supports per-class VDFs where
each class may perceive link costs differently.

Same 4-zone diamond network as [`multiclass_network`](../multiclass_network/README.md),
but cars and trucks have **different BPR parameters**:
- Car: BPR(alpha=0.15, beta=4.0) - standard
- Truck: BPR(alpha=0.30, beta=4.0) - steeper, more sensitive to congestion

## Run

```sh
cargo run --example diagonalization
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

Each diamond edge is a pair of one-way road segment links (forward + reverse),
giving 8 road links total. Connection links at each intersection allow all
turns except U-turns (8 connection links total). Total: 16 links.

All road segments: 2 lanes, 60 km/h free speed, 1800 veh/h capacity per lane.

## Multi-class configuration

| Class | PCU | ff_time_multiplier | VDF |
|-------|-----|--------------------|-----|
| car   | 1.0 | 1.0                | BPR(0.15, 4.0) |
| truck | 2.5 | 1.0                | BPR(0.30, 4.0) |

Note: `ff_time_multiplier / pcu` is NOT constant (1.0/1.0 != 1.0/2.5), so the
Beckmann symmetry condition is violated. This is intentional - we use
diagonalization specifically because Beckmann FW cannot handle this case.

## OD demand

OD matrices are provided directly (not through the 4-step pipeline):

| | Zone 1 | Zone 2 | Zone 3 | Zone 4 |
|---|--------|--------|--------|--------|
| **Car** | | | | |
| Zone 1 | - | 2000 | 3000 | 4000 |
| Zone 2 | 1500 | - | 1000 | 2500 |
| Zone 3 | 2000 | 1500 | - | 3500 |
| Zone 4 | 1000 | 2000 | 2500 | - |
| **Truck** | | | | |
| Zone 1 | - | 200 | 300 | 500 |
| Zone 2 | 150 | - | 100 | 300 |
| Zone 3 | 200 | 150 | - | 400 |
| Zone 4 | 100 | 250 | 300 | - |

Car total: 26,500 trips. Truck total: 2,950 trips.

## Results

### Assignment convergence

| Parameter | Value |
|-----------|-------|
| Total inner FW iterations | 881 |
| Outer gap | < 1e-6 |
| Converged | yes |

### Per-class volumes

| Class | Total veh-trips | PCU contribution |
|-------|-----------------|------------------|
| car   | 34,004          | 34,004 * 1.0 = 34,004 |
| truck | 3,800           | 3,800 * 2.5 = 9,500 |
| **PCU total** | | **43,504** |

### Link volumes (PCU) and costs

| Link | Direction | PCU volume | Cost (h) | V/C |
|------|-----------|------------|----------|-----|
| 106  | 3 -> 4    | 7064       | 0.0450   | 1.96 |
| 100  | 1 -> 2    | 7062       | 0.0449   | 1.96 |
| 102  | 1 -> 3    | 6751       | 0.0398   | 1.88 |
| 104  | 2 -> 4    | 6750       | 0.0398   | 1.88 |
| 103  | 3 -> 1    | 4127       | 0.0176   | 1.15 |
| 105  | 4 -> 2    | 4124       | 0.0176   | 1.15 |
| 107  | 4 -> 3    | 3815       | 0.0166   | 1.06 |
| 101  | 2 -> 1    | 3811       | 0.0166   | 1.06 |

### Per-class cost comparison

On the most loaded link (106, V/C = 1.96):

| Class | VDF | Cost (h) |
|-------|-----|----------|
| car   | BPR(0.15, 4.0) | 0.045124 |
| truck | BPR(0.30, 4.0) | 0.076248 |
| **Ratio** | | **1.69** |

The truck VDF (alpha=0.30) amplifies the congestion penalty: at V/C = 1.96
trucks perceive 69% higher cost than cars on the same link. At free flow
(V/C = 0) both VDFs return the same cost, so the ratio grows with congestion.

### Path analysis

24 paths total: 12 per class (one per OD pair).

**Zone 1 -> Zone 4:**

| Class | Flow | Cost (h) | Route |
|-------|------|----------|-------|
| car   | 4000 | 0.0848   | 100 -> 104 (via Zone 2) |
| truck | 500  | 0.1416   | 100 -> 104 (via Zone 2) |

Both classes take the same route because the diamond network is symmetric:
both alternative paths (via Zone 2 and via Zone 3) have equal V/C ratios,
so the steeper truck VDF scales costs equally on both routes without changing
their relative ordering.

Paths diverge when the network has **asymmetric** properties:
- Different capacities or speeds on parallel routes
- Class-specific link restrictions (truck bans modeled as high-cost VDF)
- Class-specific tolls

### Select link: link 102 (1 -> 3)

| Origin | Dest | Class | Flow |
|--------|------|-------|------|
| 1      | 3    | car   | 3000 |
| 2      | 3    | car   | 1000 |
| 1      | 3    | truck | 300  |
| 2      | 3    | truck | 100  |
| **Total** | | | **4400** |

## Difference from other multiclass examples

| | [`multiclass_network`](../multiclass_network/README.md) | [`path_analysis_multi_class`](../path_analysis_multi_class/README.md) | diagonalization |
|---|---|---|---|
| VDF | Shared BPR | Shared BPR | Per-class BPR |
| Symmetry condition | Required | Required | Not required |
| Algorithm | Beckmann FW | Beckmann FW | Gauss-Seidel diagonalization |
| SPT | Shared | Shared | Per-class (may differ) |
| store_paths | no | yes | yes |
| Path analysis | - | OD query, select link | OD query, select link |
| Pipeline | Full 4-step | Full 4-step | Direct assignment call |
| OD demand | From trip generation | From trip generation | Explicit matrices |

### Code difference

The other multiclass examples go through the 4-step pipeline:

```rust
// path_analysis_multi_class: pipeline with shared VDF
let config = ModelConfig::new()
    .with_assignment_method(AssignmentMethodType::FrankWolfe)
    .with_store_paths(true)
    .with_user_classes(vec![
        UserClassConfig::new("car", 1.0, 1.0, 0.9),
        UserClassConfig::new("truck", 2.5, 2.5, 0.1),
    ])
    .build();

let result = run_four_step_model(&network, &zones, &trip_gen, &impedance, &logit, &config, None)?;
```

This example calls the assignment function directly with per-class VDFs:

```rust
// diagonalization: direct call with per-class VDFs

// 1. Build graph from network
let graph = IndexedGraph::from_network(&network);

// 2. Per-class VDFs (the whole point of diagonalization)
let bpr_car = BprFunction::new(0.15, 4.0);
let bpr_truck = BprFunction::new(0.30, 4.0);
let class_vdfs: Vec<&dyn VolumeDelayFunction> = vec![&bpr_car, &bpr_truck];

// 3. Classes (no demand_fraction -- OD matrices are separate)
let classes = vec![
    UserClass::new("car", 1.0, 1.0),
    UserClass::new("truck", 2.5, 1.0),
];

// 4. OD matrices: one per class, provided by the caller.
//    Can come from any source: DenseOdMatrix, SparseOdMatrix,
//    gravity model output, external CSV, pipeline split, etc.
let od_car = DenseOdMatrix::from_data(
     zones.clone(),
     vec![
          0.0, 2000.0, 3000.0, 4000.0,
          1500.0, 0.0, 1000.0, 2500.0,
          2000.0, 1500.0, 0.0, 3500.0,
          1000.0, 2000.0, 2500.0, 0.0,
     ],
);
let od_truck = DenseOdMatrix::from_data(
     zones.clone(),
     vec![
          0.0, 200.0, 300.0, 500.0,
          150.0, 0.0, 100.0, 300.0,
          200.0, 150.0, 0.0, 400.0,
          100.0, 250.0, 300.0, 0.0,
     ],
);
let od_matrices: Vec<&dyn OdMatrix> = vec![&od_car, &od_truck];

// 5. Assign
let result = assign_diagonalization(
    &graph, &classes, &od_matrices, &class_vdfs, &config,
    20,   // max outer iterations
    1e-4, // outer convergence gap
)?;
```

Key differences:
- No `demand_fraction` - OD matrices are provided separately per class
- No symmetry condition - `ff_time_multiplier / pcu` can differ across classes
- Per-class VDF - each class has its own `BprFunction` (or any `VolumeDelayFunction`)
- Two extra parameters: `max_outer_iterations` and `outer_convergence_gap`

## Benchmark results

Inner FW: max 20 iterations, gap=1e-3. Outer: max 10 iterations, gap=1e-3.

### Small grid: 100 zones

Grid 20x20, 400 nodes, 1520 links, uniform demand (10 car + 2 truck per OD pair).

#### Time

| Variant | Without paths | With paths | Overhead |
|---------|---------------|------------|----------|
| Beckmann multiclass FW | 17.1 ms | - | - |
| Diagonalization (shared VDF) | 10.0 ms | - | - |
| Diagonalization (per-class VDF) | 10.7 ms | 19.1 ms | **+79%** |

On a small grid, diagonalization is faster than Beckmann FW because the inner FW
converges faster when it sees only one class at a time.

#### Memory

| Variant | Paths | sizeof(OdPath) | Structs | Heap (link_ids) | Total |
|---------|-------|----------------|---------|-----------------|-------|
| Diag (2 cls) | 19,800 | 64 B | 1.21 MB | 2.01 MB | **3.22 MB** |

Avg 13.3 links/path.

### Large grid: 625 zones

Grid 50x50, 2500 nodes, 9800 links, gravity-model demand
(exponential decay beta=0.15, base 50 car / 10 truck per pair).
Car total: 4.06M trips, truck total: 812k trips.

#### Time

| Variant | Without paths | With paths | Overhead |
|---------|---------------|------------|----------|
| Beckmann multiclass FW | 872 ms | - | - |
| Diagonalization (shared VDF) | 7.70 s | - | - |
| Diagonalization (per-class VDF) | 7.29 s | 7.88 s | **+8%** |

On a large grid, diagonalization is **x8.8 slower** than Beckmann FW. Each outer
iteration runs a full inner FW per class (2 x 20 inner iterations x 10 outer =
up to 400 total AoN + line search passes vs 20 for Beckmann). Path extraction
overhead is small (+8%) because assignment dominates wall time.

For comparison, the Beckmann multiclass store_paths overhead on the same grid is
+42% (see [`path_analysis_multi_class`](../path_analysis_multi_class/README.md)).

#### Memory

| Variant | Paths | sizeof(OdPath) | Structs | Heap (link_ids) | Total |
|---------|-------|----------------|---------|-----------------|-------|
| Diag (2 cls) | 780,000 | 64 B | 47.6 MB | 199.0 MB | **246.6 MB** |

Avg 33.4 links/path. Same path count and memory as Beckmann multiclass (2 classes,
same OD pairs). Heap dominates (81%): `Vec<LinkID>` contents.

## Reference

Dafermos, S.C. (1982) "Relaxation Algorithms for the General Asymmetric
Traffic Equilibrium Problem", Transportation Science, 16(2), 231-240.
DOI: 10.1287/trsc.16.2.231
