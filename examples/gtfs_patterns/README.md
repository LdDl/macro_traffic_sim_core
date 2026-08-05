# gtfs_patterns

How `transit_network_from_gtfs` groups GTFS trips into patterns. No external files needed: the dataset is built in memory with the [gtfs-rs](https://crates.io/crates/gtfs-rs) crate.

## Run

```sh
cargo run --example gtfs_patterns
```

## Why grouping is needed

GTFS has no entity that matches what an assignment model calls a "line":

- `routes.txt` is an administrative registry record (number, name, agency) - no stops, no times, no directions;
- a `trips.txt` record is a single one-way run on given service days (`block_id` chains the runs of one physical vehicle), or a repeating template when `frequencies txt` applies;
- the actual structure - the ordered stop sequence - lives only in `stop_times.txt`. Directions, short-turns and weekend variants are simply trips with different sequences under one route id.

The modeling entity - the *pattern* (NeTEx: ServiceJourneyPattern; GTFS deliberately has no such table) - must be reconstructed by grouping the trips of each route by their stop sequence.

## The algorithm, step by step

For each `routes.txt` record:

1. **Iterate its trips** in dataset order.
2. **Filter out unusable trips:**
   - no `frequencies.txt` windows - schedule-based service, out of scope
     for a frequency-based model;
   - fewer than two stop times;
   - any GTFS-Flex row (`location_id`/`location_group_id` instead of
     `stop_id`) - on-demand zones have no headway/stop semantics.
3. **Map the stop sequence** through the user-provided
   `stop_id -> node/location ID` mapping
   (`TransitError::UnmappedGtfsStop` if a stop is missing).
4. **Compute the trip's segment times** from `stop_times`:
   `arrival[i+1] - departure[i]`, in seconds. Missing intermediate times (non-timepoint rows, spec-legal) are linearly interpolated between the surrounding known times; missing times at the first or last stop are an error, matching the spec requirement.
5. **Compute the trip's window statistics** over its frequency windows:
   total covered time and total departures (`window / headway_secs`).
6. **Group by the mapped stop sequence**: the first trip with a new sequence opens a pattern (in first-seen order), subsequent trips with the identical sequence join it.
7. **Accumulate per pattern**: window time, departures, and
   departure-weighted segment time sums.
8. **Emit one `TransitRoute` per pattern:**
   - id `"{route_id}:{direction}"`, where direction is the
     `direction_id` code of the pattern's first trip (0 when absent);
     further patterns with the same label get a `#k` suffix;
   - headway = total window time / total departures (the time-weighted average over ALL trips of the pattern - per-period template trips produce the correct combined frequency);
   - segment times = departure-weighted means.
9. If no route produced a pattern, the dataset is unusable:
   `TransitError::InvalidGtfsData`.

## Scenarios in this example

| Route | Input trips | Result | Demonstrates |
|-------|-------------|--------|--------------|
| 10 | `10_am` (3 h @ 300 s, segments 120+180 s) and `10_pm` (3 h @ 600 s, segments 180+240 s), same stops A-B-C | one pattern `10:0`, headway 21600/54 = **400 s**, segments **[140, 200]** | template trips aggregate |
| 20 | `20_f` (A-D) and `20_r` (D-A), **no** `direction_id` | `20:0` and `20:0#1` | reverse direction survives |
| 30 | `30_full` (A-B-C-D) and `30_short` (A-B), both direction 0 | `30:0` and `30:0#1` | short-turn kept as its own line |
| 40 | `40_t` (A-B-C), stop B has no times | `40:0`, segments **[300, 300]** | interpolation |
| 50 | `50_flex` (flex row) and `50_sched` (no frequencies) | route absent | out-of-scope trips skipped |

## Expected output

```text
Reconstructed patterns (6 total):
  10:0: stops [1, 2, 3], headway 400 s, segments [140.0, 200.0]
  20:0: stops [1, 4], headway 360 s, segments [600.0]
  20:0#1: stops [4, 1], headway 360 s, segments [600.0]
  30:0: stops [1, 2, 3, 4], headway 480 s, segments [240.0, 300.0, 360.0]
  30:0#1: stops [1, 2], headway 480 s, segments [240.0]
  40:0: stops [1, 2, 3], headway 300 s, segments [300.0, 300.0]
```

## Reference

- GTFS Schedule reference: https://gtfs.org/documentation/schedule/reference/
- [`transit_gtfs`](../transit_gtfs/) - the full chain: GMNS locations -> GTFS -> conversion -> Spiess-Florian assignment
- Spiess, H. and Florian, M. (1989) "Optimal strategies: A new assignment model for transit networks". Transportation Research Part B 23(2), 83-102. DOI: [10.1016/0191-2615(89)90034-9](https://doi.org/10.1016/0191-2615(89)90034-9)
