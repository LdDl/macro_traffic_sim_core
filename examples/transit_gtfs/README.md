# transit_gtfs

Transit assignment from a GTFS Schedule dataset linked to a GMNS road network. No external files needed: the dataset is built in memory with the [gtfs-rs](https://crates.io/crates/gtfs-rs) crate; the assignment is solved by [hyperpaths-rs](https://crates.io/crates/hyperpaths-rs).

Demonstrates the full chain:

1. GMNS road network.

  Nodes are intersections, links are streets. The road graph is never modified by transit.

2. Bus stops as GMNS locations.

  Points along links (`link_id` + offset), each carrying its `gtfs_stop_id`. The linkage between GTFS stops and the network is provided by the user; no map matching is performed.

3. Frequency-based GTFS Schedule dataset.

  Stops, routes, trips, stop times (used as relative travel time profiles, not timetables) and frequencies (headways). Calendars, absolute schedules, and GTFS Realtime are out of scope. The network is the Spiess & Florian (1989) example expressed as GTFS, times in seconds.

4. Conversion and assignment.

  `Network::gtfs_stop_mapping()` links the dataset to locations, `transit_network_from_gtfs()` builds transit routes (one per route and direction, headway = time-weighted average over frequency windows), `assign_transit()` runs the optimal strategies assignment.

## Run

```sh
cargo run --example transit_gtfs
```

## Network

Two layers joined by the locations - the point of this example. The road layer (`[..]` = intersection nodes, `o` = stop locations pinned to links by offset):

```text
        link 100 (1200 m)       link 101 (1200 m)       link 102 (1200 m)
  [1] ---o-----------o---- [2] ----------o--------- [3] ------o--------- [4]
         |           |                   |                    |
     501/stop_A  502/stop_X          503/stop_Y           504/stop_B
     offset 150  offset 900          offset 600           offset 300
```

The transit layer is the same expanded route graph as in the [`transit`](../transit/) example (Fig. 7 of the paper), only over location IDs and in GTFS seconds - labels are `(travel time, frequency)`, `[..]` = stop location, `(..)` = route node:

```text
    +--------------------- (1500, 1/360) ----------------------------------+
    |                                                                       v
 [501] --(420, 1/360)--> (X2) ----(360, inf)----> [503] --(600, 1/180)--> [504]
                          | ^                      | ^                      ^
                (0, inf)  | | (0,1/360) (0,1/900)  | | (0, inf)             |
                          v |                      v |                      |
                        [502] ---(240, 1/900)-->  (Y3) ----(240, inf)-------+
```

Same shape, same solution - only the stop names went through the linkage: `A -> 501`, `X -> 502`, `Y -> 503`, `B -> 504`, and minutes became seconds. The road graph knows nothing about the lines; the lines know nothing about links - only about stops. The `location` records (and their `gtfs_stop_id`) are the single bridge between the two.

## Dataset

| GTFS route | Stops | Segments (s) | Headway (s) |
|------------|-------|--------------|-------------|
| L1 | stop_A -> stop_B | 1500 | 360 |
| L2 | stop_A -> stop_X -> stop_Y | 420, 360 | 360 |
| L3 | stop_X -> stop_Y -> stop_B | 240, 240 | 900 |
| L4 | stop_Y -> stop_B | 600 | 180 |

Stop linkage (user-provided, GMNS `location` records):

| GTFS stop | Location | On link | Offset |
|-----------|----------|---------|--------|
| stop_A | 501 | 100 | 150 m |
| stop_X | 502 | 100 | 900 m |
| stop_Y | 503 | 101 | 600 m |
| stop_B | 504 | 102 | 300 m |

## The chain, step by step

1. Build the GMNS road network.

  Four intersections (nodes 1-4) in a chain, three street links (100, 101, 102). Nothing transit-specific happens here; the graph stays a plain road network throughout.

2. Pin the stops onto the links.

  Each bus stop becomes a GMNS `location`: `Location::new(501, 100, 1, 150.0)` reads "location 501 sits on link 100, 150 meters from node 1", plus `.with_gtfs_stop_id("stop_A")`. The user supplies `link_id` and offset - no map matching. `add_location` only verifies the link exists.

3. Build the GTFS dataset in memory.

  Four routes, one trip per route, `stop_times` giving stop sequences and relative travel times (seconds), `frequencies` giving headways. This is the Spiess & Florian network in GTFS clothing: Line 1 = 1500 s ride every 360 s, etc.

4. Derive the linkage.

  `road_network.gtfs_stop_mapping()` scans the locations and returns `{"stop_A": 501, "stop_X": 502, "stop_Y": 503, "stop_B": 504}` - the bridge between the GTFS ID space and the network ID space.

5. Convert.

  `transit_network_from_gtfs(&gtfs, &mapping)` groups trips into patterns (trivial here - one trip per route), maps stops through the linkage, aggregates frequency windows into headways, and yields four `TransitRoute`s over location IDs: `L1:0` with stops `[501, 504]`, headway 360 s, and so on.

6. Assign.

  `assign_transit(&transit_network, &od)` with 1000 passengers from 501 to 504 expands the route graph, runs the Spiess-Florian phases per destination and returns typed volumes, OD costs and boardings.

7. Read the results.

  `od_costs[(501, 504)] = 1665 s = 27.75 min` - the published value; riding volumes split 500 / 500 at A and 416.7 / 83.3 at Y, the paper's proportions scaled by 1000.

Every step is verifiable against the publication: a mistake in the linkage, the conversion or the expansion would show up as a deviation from 27.75.

## Results

1000 passengers from stop_A to stop_B, expected travel time 27.75 min (the paper's value; GTFS seconds scale it exactly):

| Route | Segment | Passengers |
|-------|---------|------------|
| L1:0 | 501 -> 504 | 500.0 |
| L2:0 | 501 -> 502 -> 503 | 500.0 |
| L4:0 | 503 -> 504 | 416.7 |
| L3:0 | 503 -> 504 | 83.3 |

## Reference

- GMNS `location` table: https://github.com/zephyr-data-specs/GMNS/blob/develop/docs/spec/location.md
- GTFS static reference: https://gtfs.org/documentation/schedule/reference/
- Spiess, H. and Florian, M. (1989) "Optimal strategies: A new assignment model for transit networks". Transportation Research Part B 23(2), 83-102. DOI: [10.1016/0191-2615(89)90034-9](https://doi.org/10.1016/0191-2615(89)90034-9)
- [`transit`](../transit/) - the same network defined directly with `TransitRoute`, without GTFS
