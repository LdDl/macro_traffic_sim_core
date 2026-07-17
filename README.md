# macro_traffic_sim_core

Rust implementation of the classical 4-step macroscopic traffic demand model.

## What it does

The library implements the four sequential steps of aggregate travel demand
forecasting:

1. **Trip Generation** - estimates how many trips each zone produces and
   attracts, using regression or cross-classification on socioeconomic data
   (population, employment, households, income).

2. **Trip Distribution** - distributes trips between origin-destination zone
   pairs via a gravity model. An impedance function (exponential, power, or
   combined) controls distance sensitivity. Furness (IPF) balancing ensures
   row/column totals match productions and attractions.

3. **Mode Choice** - splits the total OD matrix into per-mode matrices
   (AUTO, BIKE, WALK) using a multinomial logit model with configurable
   utility functions (time, distance, cost coefficients per mode).

4. **Traffic Assignment** - loads the AUTO OD matrix onto the network to
   find User Equilibrium link flows. Only AUTO is assigned because BIKE
   and WALK do not contribute to road congestion. Four algorithms are available:
   - **Frank-Wolfe** - convex combinations with golden section line search
   - **MSA** - method of successive averages (step = 1/n)
   - **Gradient Projection** - path-based with explicit path management
   - **Diagonalization** - Gauss-Seidel relaxation for per-class VDFs (Dafermos, 1982)

Steps 2-4 run inside a **feedback loop**: after each assignment the congested
travel times update the skim matrix, which feeds back into distribution and
mode choice. This captures the interaction between congestion and
route/destination/mode decisions.

```text
Trip Generation
      |
      v
+---> Trip Distribution  <-- skim (travel time)
|           |
|           v
|     Mode Choice
|           |
|           v
+---- Assignment -----> update skim from congested costs
      (repeat N times)
```

## Network format

The library works on a **mesoscopic** (meso) network based on the
[GMNS](https://github.com/zephyr-data-specs/GMNS) specification:

- **Road segment links** - pieces of road between nodes, carrying capacity,
  speed, and lane count.
- **Connection links** - turn maneuvers at intersections. A connection link
  exists only if the turn is allowed. This encodes turn restrictions directly
  in the graph topology - routing algorithms respect them automatically
  without any extra logic.

The library does not include I/O or CSV parsing. You build the `Network`
in code or write your own loader (see the examples).

## Quick start

Add the dependency:

```toml
[dependencies]
macro_traffic_sim_core = "0.1.1"
```

Minimal usage:

```rust
use macro_traffic_sim_core::config::ModelConfig;
use macro_traffic_sim_core::gmns::meso::network::Network;
use macro_traffic_sim_core::gmns::meso::node::Node;
use macro_traffic_sim_core::gmns::meso::link::Link;
use macro_traffic_sim_core::mode_choice::MultinomialLogit;
use macro_traffic_sim_core::pipeline::run_four_step_model;
use macro_traffic_sim_core::trip_distribution::ExponentialImpedance;
use macro_traffic_sim_core::trip_generation::RegressionGenerator;
use macro_traffic_sim_core::zone::Zone;

// 1. Build network
let mut network = Network::new();
network.add_node(Node::new(1).with_zone_id(1).with_coordinates(55.76, 37.62).build()).unwrap();
// ... add more nodes and links

// 2. Define zones
let zones = vec![
    Zone::new(1).with_population(6000.0).with_employment(500.0).build(),
];

// 3. Configure and run
let config = ModelConfig::new()
    .with_feedback_iterations(3)
    .with_max_iterations(100)
    .build();

let result = run_four_step_model(
    &network,
    &zones,
    &RegressionGenerator::new(),
    &ExponentialImpedance::new(0.1),
    &MultinomialLogit::default_auto_bike_walk(),
    &config,
).unwrap();

println!("Total auto trips: {:.0}", result.mode_od[&macro_traffic_sim_core::gmns::types::AgentType::Auto].total());
println!("Converged: {}", result.assignment.converged);
```

## Examples

All examples build an in-memory network and run without external files.

| Example | What it shows |
|---------|---------------|
| [`simple_network`](examples/simple_network/) | Full 4-step pipeline on a 4-zone diamond network |
| [`four_step`](examples/four_step/) | Same pipeline with detailed step-by-step output |
| [`grid_city`](examples/grid_city/) | Larger grid network (scalable) |
| [`ring_network`](examples/ring_network/) | Ring topology with unidirectional links |
| [`disconnected_network`](examples/disconnected_network/) | Handling unreachable zones gracefully |
| [`multiclass_network`](examples/multiclass_network/) | Multi-class (car + truck) with shared BPR, Beckmann FW |
| [`path_analysis_single_class`](examples/path_analysis_single_class/) | `store_paths = true`, OD pair query, select link analysis |
| [`path_analysis_multi_class`](examples/path_analysis_multi_class/) | Multi-class path extraction, per-class OD/select-link |
| [`diagonalization`](examples/diagonalization/) | Per-class VDFs (asymmetric costs), direct assignment call |
| [`warm_start_test`](examples/warm_start_test/) | Warm start: reuse previous iteration flows |
| [`lua_vdf`](examples/lua_vdf/) | Lua-scripted VDF with diagonalization (requires `lua` feature) |

```sh
cargo run --example simple_network
cargo run --example diagonalization
cargo run --example lua_vdf --features lua
```

## Configuration

### Pipeline (4-step model)

All pipeline parameters are controlled via `ModelConfig`:

```rust
use macro_traffic_sim_core::config::{ModelConfig, AssignmentMethodType, UserClassConfig};
use macro_traffic_sim_core::verbose::VerboseLevel;

let config = ModelConfig::new()
    .with_assignment_method(AssignmentMethodType::FrankWolfe)
    .with_bpr(0.15, 4.0)
    .with_max_iterations(100)
    .with_convergence_gap(1e-4)
    .with_feedback_iterations(3)
    .with_furness_max_iterations(200)
    .with_furness_tolerance(1e-6)
    .with_verbose_level(VerboseLevel::Main)
    .build();
```

### Multi-class assignment

Add `with_user_classes` to split the AUTO OD matrix into per-class
matrices. Each class has a PCU factor, free-flow time multiplier,
and demand fraction:

```rust
let config = ModelConfig::new()
    .with_assignment_method(AssignmentMethodType::FrankWolfe)
    .with_store_paths(true)
    .with_user_classes(vec![
        UserClassConfig::new("car", 1.0, 1.0, 0.9),
        UserClassConfig::new("truck", 2.5, 2.5, 0.1),
    ])
    .build();
```

The Beckmann symmetry condition requires `ff_time_multiplier / pcu = const`
for all classes. This is validated at runtime. When it holds, all classes
share the same shortest path tree - Dijkstra runs once per origin, not
once per class.

### Per-class VDFs (diagonalization)

When classes need different volume-delay functions (e.g. trucks perceive
congestion differently), the Beckmann symmetry condition does not apply.
Use `assign_diagonalization` directly instead of the pipeline:

```rust
use macro_traffic_sim_core::assignment::diagonalization::assign_diagonalization;

let result = assign_diagonalization(
    &graph, &classes, &od_matrices, &class_vdfs, &config,
    20,    // max outer iterations
    1e-4,  // outer convergence gap
)?;
```

See the [`diagonalization`](examples/diagonalization/) example for a
complete working example with per-class BPR parameters.

### Assignment method trade-offs

| Method | Paths per OD | Multi-class | Per-class VDF |
|--------|-------------|-------------|---------------|
| Frank-Wolfe | 1 (shortest) | yes (Beckmann) | no |
| MSA | 1 (shortest) | yes (Beckmann) | no |
| Gradient Projection | multiple | no | no |
| Diagonalization | 1 (shortest, per class) | yes | yes |

Frank-Wolfe produces one path per OD pair - the equilibrium shortest
path. For multiple paths with flow distribution, use Gradient Projection.
For per-class VDFs (asymmetric costs), use diagonalization.

### Path extraction

Set `store_paths = true` to extract per-OD shortest paths after
assignment converges:

```rust
let config = ModelConfig::new()
    .with_store_paths(true)
    // ...
    .build();
```

Paths are stored in `result.assignment.path_flows`. Each `OdPath` carries
origin/dest zones, flow, cost, link sequence, and `class_index`
(multi-class only). Path extraction adds one Dijkstra per origin after
convergence (~40% overhead on a 625-zone grid).

## Module structure

```text
macro_traffic_sim_core
  assignment/           - traffic assignment algorithms
    frank_wolfe         - Frank-Wolfe with golden section line search
    msa                 - method of successive averages
    gradient_projection - gradient projection (path-based)
    multiclass          - multi-class PCU-based assignment (Beckmann FW/MSA)
    diagonalization     - Gauss-Seidel relaxation with per-class VDFs
    indexed_graph       - CSR graph, Dijkstra, all-or-nothing
    od_path             - OdPath struct for path extraction
  config                - ModelConfig and builder
  error                 - top-level SimError enum
  gmns/                 - network data model (GMNS-based)
    types               - NodeID, LinkID, ZoneID, AgentType, LinkType and so on.
    defaults            - default speed/capacity/lanes by link type
    error               - GraphError
    meso/               - mesoscopic network
      node              - Node (intersection/mid-link point)
      link              - Link (road segment or connection/turn)
      network           - Network container with adjacency
  mode_choice/          - multinomial logit mode split
  od/                   - OD matrices (dense and sparse)
  pipeline/             - 4-step model orchestrator
  trip_distribution/    - gravity model + Furness balancing + impedance
  trip_generation/      - regression and cross-classification generators
  verbose/              - structured logging (tracing-based)
  zone                  - transport analysis zones
```

## Parallel execution

The `parallel` feature is ENABLED by default. It uses [rayon](https://docs.rs/rayon) to parallelize the most expensive steps: all-or-nothing assignment (Dijkstra per origin zone) and skim matrix computation.

To disable and use single-threaded execution:

```toml
[dependencies]
macro_traffic_sim_core = { version = "...", default-features = false }
```

## Key design decisions

- **Meso graph only.** The library operates on the mesoscopic graph where
  turn restrictions are encoded in the topology. There is no macro-level
  graph - the meso graph is the single source of truth for all computations. 
  It simplifies the code and avoids synchronization issues between macro and meso representations.
  In future I may add pipeline steps to generate the meso graph from a macro graph.

- **No I/O in the core.** CSV/JSON parsing, file loading, and serialization
  are the caller's responsibility. The library accepts in-memory data
  structures. This keeps the core dependency-free (no serde, csv, etc.) and
  lets users bring their own formats. May be in future I will rethink this and add some basic CSV loaders.

- **Volume-delay functions.** Three VDFs are available, all implementing the
  `VolumeDelayFunction` trait (with `travel_time` and `integral`):
  - **BPR** (Bureau of Public Roads): $t(x) = t_0 \cdot (1 + \alpha \cdot (x/c)^\beta)$
  - **Conical** (Spiess, 1990): smooth, no flat region near free-flow
  - **Akcelik** (Akcelik, 1991): delay-based, better for signalized intersections

  The pipeline uses a single shared VDF. For per-class VDFs (e.g. trucks
  perceive congestion differently), use `assign_diagonalization` directly.

## References

1. Ortuzar J. de D., Willumsen L.G. *Modelling Transport*. 4th ed. Wiley, 2011.
   The standard textbook on the 4-step model, gravity distribution, logit mode choice, and equilibrium assignment.

2. Sheffi Y. *Urban Transportation Networks: Equilibrium Analysis with Mathematical Programming Methods*. Prentice-Hall, 1985.
   Frank-Wolfe algorithm, user equilibrium, BPR function, convergence theory.
   Freely available at https://sheffi.mit.edu/sites/sheffi.mit.edu/files/sheffi_urban_trans_networks_0.pdf

3. Dafermos, S.C. (1972) "The Traffic Assignment Problem for Multiclass-User
   Transportation Networks", Transportation Science, 6(1), 73-87.
   DOI: 10.1287/trsc.6.1.73
   Multi-class user equilibrium formulation.

4. Dafermos, S.C. (1982) "Relaxation Algorithms for the General Asymmetric
   Traffic Equilibrium Problem", Transportation Science, 16(2), 231-240.
   DOI: 10.1287/trsc.16.2.231
   Diagonalization (Gauss-Seidel relaxation) for multi-class assignment with per-class VDFs.
   See also: https://en.wikipedia.org/wiki/Gauss%E2%80%93Seidel_method

5. Bureau of Public Roads. *Traffic Assignment Manual*. U.S. Dept. of Commerce, 1964.
   Origin of the BPR volume-delay function.

6. TransitWiki. *Four-step travel model*.
   https://www.transitwiki.org/TransitWiki/index.php/Four-step_travel_model

7. GMNS (General Modeling Network Specification).
   https://github.com/zephyr-data-specs/GMNS

8. Dial, R.B. (2006) "A path-based user-equilibrium traffic assignment algorithm
   that obviates path storage and enumeration",
   Transportation Research Part B, 40(10), 917-936.
   DOI: 10.1016/j.trb.2006.02.008
   Warm start concept for traffic assignment.

9. Levin, M.W. and Boyles, S.D. (2015) "A cell transmission model for
   dynamic lane reversal with autonomous vehicles",
   Transportmetrica B, 3(2), 126-143.
   DOI: 10.1080/21680566.2014.937788
   DTA warm start reference (for future works).

10. Spiess, H. (1990) "Conical Volume-Delay Functions",
    Transportation Science, 24(2), 153-158.
    DOI: 10.1287/trsc.24.2.153
    Conical VDF.

11. Akcelik, R. (1991) "Travel time functions for transport planning
    purposes: Davidson's function, its time-dependent form and an
    alternative travel time function",
    Australian Road Research, 21(3), 49-59.
    Akcelik VDF for signalized intersections.

12. go-gmns - Go implementation of basic data in GMNS. https://github.com/LdDl/go-gmns

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.