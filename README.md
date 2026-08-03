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

3. **Mode Choice** - splits the total OD matrix into per-mode matrices with a
   multinomial logit model and configurable utilities (time, distance, cost
   per mode). Modes are AUTO, BIKE, WALK, and optionally **TRANSIT** (public
   transport). The transit alternative is fed by a transit level-of-service
   skim, so how much demand rides transit is decided here, not fixed by hand.

4. **Traffic Assignment** - loads the AUTO OD onto the road network for User
   Equilibrium link flows (BIKE and WALK do not congest roads). Four
   algorithms: **Frank-Wolfe**, **MSA**, **Gradient Projection**, and
   **Diagonalization** (per-class VDFs, Dafermos, 1982). The TRANSIT OD is
   assigned separately with the **optimal strategies** algorithm (Spiess &
   Florian, 1989) on the transit network.

Steps 2-4 run inside a **feedback loop**: after each assignment the congested
travel times update the skim, which feeds back into distribution and mode
choice, capturing the interaction between congestion and
route/destination/mode decisions.

Road and transit are **two-way coupled** inside this loop: transit vehicles
that run in mixed traffic add a background load to the roads they share with
cars (so buses help congest the road), and that road congestion in turn
raises the in-vehicle time of those transit segments (so traffic slows the
buses). A slower, pricier transit then shifts mode choice back toward cars on
the next iteration, and vice versa - the two modes reach a joint equilibrium.

```text
Trip Generation
      |
      v
+---> Trip Distribution  <-- skim (road + transit travel time)
|           |
|           v
|     Mode Choice  (AUTO / BIKE / WALK / TRANSIT)
|         |     \
|         v      v
|   Road Assign   Transit Assign (optimal strategies)
|    ^   |              |
|    |   +-- buses preload the road (PCU)
|    +------ road congestion slows transit segments
+---- update skims from congested costs
      (repeat N times)
```

## Public transit

Alongside the road model the library assigns frequency-based public transit with the optimal strategies algorithm (Spiess & Florian, 1989).

Passengers do not pick a single line: at each stop they choose a set of attractive lines and board whichever vehicle comes first, so flow splits between competing lines in proportion to their frequencies.

- **Lines over stops.** - a `TransitRoute` is an ordered list of stops with per-segment travel times and a headway. Stops are GMNS `location` records - points pinned to road links (`link_id` + offset). The road graph is never split or modified; the `location` is the single bridge between the two layers, and no map matching is performed.
- **Zone access.** - zone centroids join as pseudo-stops via walk links to several candidate stops; the algorithm itself picks the access stop  per destination (no nearest-stop heuristic).
- **GTFS input.** - a frequency-based GTFS Schedule dataset is converted into routes: trips are grouped into patterns by stop sequence headways come from `frequencies.txt`, `stop_times` provide relative travel profiles.

The GTFS data model lives in the [gtfs-rs](https://crates.io/crates/gtfs-rs) crate; the assignment is solved by [hyperpaths-rs](https://crates.io/crates/hyperpaths-rs).

Transit is fully wired into the 4-step pipeline: mode choice derives the transit demand from a transit skim (or you can supply a fixed exogenous transit OD, e.g. captive riders), and inside the feedback loop transit and road congest each other - a route can declare the road links it runs on, so its vehicles preload those links and its in-vehicle time follows their congestion (a route on its own right-of-way, like a metro, does neither). This frequency-based coupling follows De Cea & Fernandez (1993) - road congestion is an exogenous input to the in-vehicle time - and the two-mode equilibrium of Florian & Spiess (1983). Transit can also be assigned standalone with a manually supplied OD.

**Crowding (soft capacity).** Give a `TransitRoute` a per-vehicle `capacity` and the assignment turns on passenger crowding: as the flow a line attracts approaches its capacity, its effective frequency drops and its waiting time rises, so it sheds riders onto less crowded alternatives. This is De Cea & Fernandez's (1993) congested-transit model, where a stop is a queue and "as the number of passengers trying to use a given service approaches its capacity, waiting times increase". Rather than a hard cap they use a BPR-like convex volume-delay term, so the line's effective frequency becomes `f_eff = f / (1 + alpha * (load/capacity)^beta)` (their effective frequency, Eq. 16) - a soft cap that a line can overrun under very heavy demand. Cominetti & Correa (2001) put this on a rigorous footing: waiting times "obey an inverse additive law of the form `1/W_s(v) = sum 1/W_i(v)`", i.e. the Spiess-Florian combined frequency with a flow-dependent `f_i(v)`. So crowding is an outer method-of-successive-averages loop that rescales each line's frequency by its load and re-runs the unchanged optimal-strategies solver - the hyperpaths solver never sees the flow dependence. Uncapacitated routes are unaffected. See `assign_transit_crowded` / `CrowdingParams`, or the pipeline's `TransitInput.crowding`.

**Strict capacity (Cepeda-Cominetti-Florian).** `assign_transit_congested` / `CongestedParams` is the rigorous congested equilibrium of Cepeda, Cominetti & Florian (2006). It uses the strict effective frequency `f_a = mu * (1 - (v_a / (mu*c - v'_a + v_a))^beta)`, which vanishes as the on-board flow reaches the line capacity, so a line cannot be overloaded: excess demand is forced onto other lines or onto walking, and the method reveals corridors that lack capacity. Convergence is measured by their computable gap function `G(v)` (Theorem 3.2), zero exactly at equilibrium, so the outer method-of-successive-averages loop has a rigorous stopping rule. Like crowding it wraps the unchanged optimal-strategies solver, generalizing the plain Spiess-Florian model (which is its uncongested special case). Available standalone or in the pipeline via `TransitInput.congested` (which takes precedence over `TransitInput.crowding`). The `transit_congested` example reproduces the paper's own worked example.

See the `transit`, `transit_gtfs`, `gtfs_patterns`, `multimodal`, `transit_crowding` and `transit_congested` examples.

## Network format

The library works on a **mesoscopic** (meso) network based on the
[GMNS](https://github.com/zephyr-data-specs/GMNS) specification:

- **Road segment links** - pieces of road between nodes, carrying capacity,
  speed, and lane count.
- **Connection links** - turn maneuvers at intersections. A connection link
  exists only if the turn is allowed. This encodes turn restrictions directly
  in the graph topology - routing algorithms respect them automatically
  without any extra logic.
- **Locations** - GMNS `location` records: points along a link (`link_id` +
  offset), optionally carrying a `gtfs_stop_id`. Used as transit stops; they annotate the road graph without splitting it.

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
| [`transit`](examples/transit/) | Transit assignment with optimal strategies (Spiess & Florian, 1989) |
| [`transit_gtfs`](examples/transit_gtfs/) | Transit assignment from a GTFS feed linked via GMNS locations |
| [`gtfs_patterns`](examples/gtfs_patterns/) | How GTFS trips are grouped into patterns (template trips, directions, short-turns, interpolation) |
| [`multimodal`](examples/multimodal/) | Cars and public transit on one network: 4-step road pipeline + buses/tram over GMNS locations |
| [`transit_crowding`](examples/transit_crowding/) | Crowding: two parallel lines, demand sweep to the tipping point where the small line runs out of seats |
| [`transit_congested`](examples/transit_congested/) | Strict-capacity congested equilibrium (Cepeda-Cominetti-Florian 2006), reproducing the paper's worked example |

```sh
cargo run --example simple_network
cargo run --example diagonalization
cargo run --example lua_vdf --features lua
cargo run --example transit
cargo run --example transit_gtfs
cargo run --example gtfs_patterns
cargo run --example multimodal
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
    location/           - GMNS location: point on a link (link_id + offset), the road-transit bridge
    meso/               - mesoscopic network
      node              - Node (intersection/mid-link point)
      link              - Link (road segment or connection/turn)
      network           - Network container with adjacency
  mode_choice/          - multinomial logit mode split
  od/                   - OD matrices (dense and sparse)
  pipeline/             - 4-step model orchestrator
  transit/              - frequency-based public transit (optimal strategies)
    route               - TransitRoute, WalkLink, TransitNetwork data model
    assignment          - route graph expansion, assign_transit, skims
      congested         - strict-capacity congested equilibrium (Cepeda-Cominetti-Florian)
    crowding            - soft-capacity crowding (De Cea-Fernandez / Cominetti-Correa)
    connectors          - zone access connector generation from coordinates
    road_interaction    - transit <-> road coupling (vehicle preload + congested times)
    from_gtfs           - GTFS pattern reconstruction (frequencies + stop_times)
    error               - TransitError
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

12. Spiess, H. and Florian, M. (1989) "Optimal strategies: A new assignment
    model for transit networks",
    Transportation Research Part B, 23(2), 83-102.
    DOI: 10.1016/0191-2615(89)90034-9
    Frequency-based transit assignment (the `transit` module).

13. Dial, R.B. (1967) "Transit pathfinder algorithm",
    Highway Research Record, 205, 67-85.

14. Le Clercq, F. (1972) "A public transport assignment method",
    Traffic Engineering and Control, 91-96.

15. Chapleau, R. (1974) "Reseaux de transport en commun: Structure
    informatique et affectation", PhD thesis, Departement d'informatique et
    de recherche operationnelle, Universite de Montreal, Quebec.

16. Rapp, M.H., Mattenberger, P., Piguet, S. and Robert-Grandpierre, A.
    (1976) "Interactive graphic system for transit route optimization",
    Transportation Research Record, 619.

17. UMTA/FHWA (1977) "UTPS Reference Manual",
    U.S. Department of Transportation.

18. GTFS (General Transit Feed Specification), static reference.
    https://gtfs.org/documentation/schedule/reference/
    Source format for transit routes/frequencies (the `gtfs-rs` crate and
    the `transit::from_gtfs` converter).

19. go-gmns - Go implementation of basic data in GMNS. https://github.com/LdDl/go-gmns

20. Florian, M. and Spiess, H. (1983) "On Binary Mode Choice/Assignment
    Models",
    Transportation Science, 17(1), 32-47.
    DOI: 10.1287/trsc.17.1.32
    Two-mode road+transit equilibrium (costs depend on both modes' flows,
    solved by diagonalization) - basis of the road<->transit coupling.

21. De Cea, J. and Fernandez, E. (1993) "Transit Assignment for Congested
    Public Transport Systems: An Equilibrium Model",
    Transportation Science, 27(2), 133-147.
    DOI: 10.1287/trsc.27.2.133
    Congested-transit model: route sections, road congestion as an exogenous
    parameter for the in-vehicle time, and the effective-frequency crowding
    (Eq. 16) - basis of the congested transit segment times and of the
    crowding assignment.

22. Cominetti, R. and Correa, J. (2001) "Common-Lines and Passenger
    Assignment in Congested Transit Networks",
    Transportation Science, 35(3), 250-267.
    DOI: 10.1287/trsc.35.3.250.10154
    Congested transit: crowding raises waiting via an inverse-additive law
    on effective frequencies - basis of the crowding assignment
    (`assign_transit_crowded`).

23. Cepeda, M., Cominetti, R. and Florian, M. (2006) "A frequency-based
    assignment model for congested transit networks with strict capacity
    constraints: characterization and computation of equilibria",
    Transportation Research Part B, 40(6), 437-459.
    DOI: 10.1016/j.trb.2005.05.006
    Strict-capacity congested equilibrium with a computable gap function,
    solved by MSA over the Spiess-Florian solver - basis of the congested
    assignment (`assign_transit_congested`).

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.