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
   and WALK do not contribute to road congestion. Three algorithms are available:
   - **Frank-Wolfe** - convex combinations with bisection line search
   - **MSA** - method of successive averages (step = 1/n)
   - **Gradient Projection** - path-based with explicit path management

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
macro_traffic_sim_core = "0.1.0"
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

### `simple_network` - in-memory 4-zone diamond

Builds a small network entirely in code, runs the pipeline, and prints
results to stdout. No external files needed.

```sh
cargo run --example simple_network
```

This example demonstrates:
- Building nodes with zone centroids and coordinates
- Creating bidirectional road segments
- Adding connection links (turn permissions) at intersections
- Balancing trip generation totals for Furness convergence
- Running the full pipeline and inspecting results

## Configuration

All pipeline parameters are controlled via `ModelConfig`:

```rust
use macro_traffic_sim_core::config::{ModelConfig, AssignmentMethodType};
use macro_traffic_sim_core::verbose::VerboseLevel;

let config = ModelConfig::new()
    .with_assignment_method(AssignmentMethodType::FrankWolfe)
    // BPR alpha, beta
    .with_bpr(0.15, 4.0)
    // assignment iterations
    .with_max_iterations(100)
    // relative gap threshold
    .with_convergence_gap(1e-4)
    // outer loop iterations
    .with_feedback_iterations(3)
    // IPF balancing iterations
    .with_furness_max_iterations(200)
    // IPF convergence tolerance
    .with_furness_tolerance(1e-6)
    .with_verbose_level(VerboseLevel::Main)
    .build();
```

## Module structure

```text
macro_traffic_sim_core
  assignment/           - traffic assignment algorithms
    frank_wolfe         - Frank-Wolfe with bisection line search
    msa                 - method of successive averages
    gradient_projection - gradient projection (path-based)
    shortest_path       - Dijkstra, all-or-nothing, skim matrices
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

- **BPR volume-delay function.** Link travel times follow the standard BPR formula:

$$t(x) = t_0 \cdot \left(1 + \alpha \cdot \left(\frac{x}{c}\right)^\beta\right)$$

  where $t_0$ is free-flow time, $x$ is volume, and $c$ is capacity.

## References

1. Ortuzar J. de D., Willumsen L.G. *Modelling Transport*. 4th ed. Wiley, 2011.
   The standard textbook on the 4-step model, gravity distribution, logit mode choice, and equilibrium assignment.

2. Sheffi Y. *Urban Transportation Networks: Equilibrium Analysis with Mathematical Programming Methods*. Prentice-Hall, 1985.
   Frank-Wolfe algorithm, user equilibrium, BPR function, convergence theory.
   Freely available at https://sheffi.mit.edu/sites/sheffi.mit.edu/files/sheffi_urban_trans_networks_0.pdf

3. Bureau of Public Roads. *Traffic Assignment Manual*. U.S. Dept. of Commerce, 1964.
   Origin of the BPR volume-delay function.

4. TransitWiki. *Four-step travel model*.
   https://www.transitwiki.org/TransitWiki/index.php/Four-step_travel_model

5. GMNS (General Modeling Network Specification).
   https://github.com/zephyr-data-specs/GMNS

6. go-gmns - Go implementation of basic data in GMNS. https://github.com/LdDl/go-gmns

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.