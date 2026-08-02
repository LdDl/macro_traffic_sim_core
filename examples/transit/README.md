# transit

Frequency-based public transit assignment with the optimal strategies algorithm (Spiess & Florian, 1989). No external files needed.

Reproduces the example network from the paper (pages 96-97): four stops A, X, Y, B and four transit lines. One unit of demand travels from A to B and the optimal strategy splits it across lines according to their frequencies.

## Run

```sh
cargo run --example transit
```

## Network

| Route | Stops | Segment times (min) | Headway (min) |
|-------|-------|---------------------|---------------|
| Line 1 | A -> B | 25 | 6 |
| Line 2 | A -> X -> Y | 7, 6 | 6 |
| Line 3 | X -> Y -> B | 4, 4 | 15 |
| Line 4 | Y -> B | 10 | 3 |

The expanded route graph, redrawn after Fig. 7 of the paper (labels are
`(travel time, frequency)`; `[..]` = stop, `(..)` = route node, i.e. a
line platform; `inf` frequency = no waiting):

```text
   +--------------------- (25, 1/6) ---------------------+
   |                                                     v
 [A] --(7, 1/6)--> (X2) ------(6, inf)------> [Y] --(10, 1/3)--> [B]
                    | ^                        | ^                 ^
           (0, inf) | | (0, 1/6)    (0, 1/15)  | | (0, inf)        |
                    v |                        v |                 |
                   [X] -------(4, 1/15)------> (Y3) ---(4, inf)----+
```

X2 is the Line 2 platform at X and Y3 the Line 3 platform at Y: the paper built these route nodes by hand, while `assign_transit` derives them automatically from the route definitions above.

## The model

Passengers do not pick a single path. At each stop they choose a set of attractive lines (a strategy) and board whichever attractive vehicle arrives first. Expected waiting time at a stop is `1 / (combined frequency of attractive lines)`; the flow splits between attractive lines proportionally to their frequencies.

Each route is expanded into a route graph following the paper: boarding links (stop -> route node, waiting = headway), alighting links (route node -> stop, instant), riding links (route node -> route node, segment travel time). The optimal strategy is computed per destination by the [hyperpaths-rs](https://crates.io/crates/hyperpaths-rs) crate implementing the Spiess-Florian algorithm; volumes are aggregated over destinations.

## The algorithm, step by step

1) Phase 1 - find the optimal strategy (label-setting, backward from the destination B).

Initialize `u_B = 0`, all other labels to infinity, all node frequencies `f_i = 0`; queue every link with key `u_j + c_a`. Then repeatedly pop the link with the smallest key and accept it if `u_i >= u_j + c_a`:

    - a boarding link (finite frequency `f_a`) joins the basket: `u_i = (f_i * u_i + f_a * (u_j + c_a)) / (f_i + f_a)`, where the numerator starts from 1 when the basket is empty, then `f_i += f_a`;
    - a no-wait link (`f_a = inf`) replaces the whole basket: `u_i = u_j + c_a`, `f_i = inf` (the paper's modified step, p. 96).

The trace on this network - Table 2 of the paper, reproduced by the implementation exactly:

| # | Link | `f_a` | `u_j + c_a` | Accepted? | Update |
|---|------|-------|-------------|-----------|--------|
| 1 | Y3 -> B | inf | 4.0 | yes | `u_Y3 = 4`, `f_Y3 = inf` |
| 2 | Y -> Y3 | 1/15 | 4.0 | yes | `u_Y = (1 + 4/15) / (1/15) = 19` |
| 3 | X -> Y3 | 1/15 | 8.0 | yes | `u_X = (1 + 8/15) / (1/15) = 23` |
| 4 | Y -> B | 1/3 | 10.0 | yes | `u_Y = (19/15 + 10/3) / (1/15 + 1/3) = 11.5` |
| 5 | Y3 -> Y | inf | 11.5 | no | `u_Y3 = 4 < 11.5` |
| 6 | X2 -> Y | inf | 17.5 | yes | `u_X2 = 17.5`, `f_X2 = inf` |
| 7 | X -> X2 | 1/6 | 17.5 | yes | `u_X = (23/15 + 17.5/6) / (7/30) = 19.07` |
| 8 | X2 -> X | inf | 19.07 | no | `u_X2 = 17.5 < 19.07` |
| 9 | A -> X2 | 1/6 | 24.5 | yes | `u_A = (1 + 24.5/6) / (1/6) = 30.5` |
| 10 | A -> B | 1/6 | 25.0 | yes | `u_A = (30.5/6 + 25/6) / (1/3) = 27.75` |

The attractive set is the eight "yes" rows; links 5 and 8 are examined but rejected. Note how `u_A` first becomes 30.5 (Line 2 alone) and then improves to 27.75 when Line 1 joins the basket: waiting for whichever of the two comes first beats committing to either line.

2) Phase 2 - load the demand (Table 3 of the paper).

Initialize node volumes from the OD column: `V_A = 1`, and seed the destination with minus the total demand (`V_B = -1`) so that arrivals cancel it to zero. Process the links in the inverse of the examination order - no sorting needed, as the paper notes on p. 97 - assigning `v_a = (f_a / f_i) * V_i` to attractive links and `v_a = 0` to rejected ones, then pushing the volume downstream (`V_j += v_a`). A link out of an infinite-frequency basket takes the whole node volume.

The worked splits: `A -> B` gets `(1/6)/(1/3) * 1 = 0.5`; at Y the remaining 0.5 splits `(1/3)/0.4 * 0.5 = 5/12 = 0.42` onto Line 4 and `(1/15)/0.4 * 0.5 = 1/12 = 0.08` onto Line 3; `X2 -> Y` and `Y3 -> B` (infinite baskets) carry their node volumes whole.

The full Table 3 of the paper (`#` is the link's examination number from Table 2, processed in reverse; right half is the node volume state after each row):

| # | Link | `v_a` | A | X2 | X | Y3 | Y | B | Effect |
|---|------|-------|------|------|------|------|------|-------|--------|
| - | (init) | | 1.00 | 0.00 | 0.00 | 0.00 | 0.00 | -1.00 | demand at A, destination seeded negative |
| 10 | A -> B | 0.50 | 1.00 | 0.00 | 0.00 | 0.00 | 0.00 | -0.50 | half rides Line 1 |
| 9 | A -> X2 | 0.50 | 1.00 | 0.50 | 0.00 | 0.00 | 0.00 | -0.50 | half boards Line 2 |
| 8 | X2 -> X | 0.00 | 1.00 | 0.50 | 0.00 | 0.00 | 0.00 | -0.50 | rejected in phase 1 |
| 7 | X -> X2 | 0.00 | 1.00 | 0.50 | 0.00 | 0.00 | 0.00 | -0.50 | attractive, but `V_X = 0` |
| 6 | X2 -> Y | 0.50 | 1.00 | 0.50 | 0.00 | 0.00 | 0.50 | -0.50 | whole basket rides to Y |
| 5 | Y3 -> Y | 0.00 | 1.00 | 0.50 | 0.00 | 0.00 | 0.50 | -0.50 | rejected in phase 1 |
| 4 | Y -> B | 0.42 | 1.00 | 0.50 | 0.00 | 0.00 | 0.50 | -0.08 | Line 4 share (5/12) |
| 3 | X -> Y3 | 0.00 | 1.00 | 0.50 | 0.00 | 0.00 | 0.50 | -0.08 | attractive, but `V_X = 0` |
| 2 | Y -> Y3 | 0.08 | 1.00 | 0.50 | 0.00 | 0.08 | 0.50 | -0.08 | Line 3 share (1/12) |
| 1 | Y3 -> B | 0.08 | 1.00 | 0.50 | 0.00 | 0.08 | 0.50 | 0.00 | arrives: `V_B` cancels to 0 |

Rows 5 and 8 are the links rejected in phase 1: the paper visits them in order and assigns zero (the implementation equivalently skips them - their volumes stay zero-initialized). The final `V_B = 0` confirms conservation: everything that left A arrived at B.

## Results

Expected travel time A -> B: **27.75 min** (matches the paper).

| Route | Segment | Volume |
|-------|---------|--------|
| Line 1 | A -> B | 0.5 |
| Line 2 | A -> X | 0.5 |
| Line 2 | X -> Y | 0.5 |
| Line 4 | Y -> B | 0.4167 (= 5/12) |
| Line 3 | Y -> B | 0.0833 (= 1/12) |

Half the passengers take Line 1 directly; the other half rides Line 2 to Y and there splits between Line 4 and Line 3 proportionally to their frequencies (1/3 vs 1/15).

## Waiting time factor

The expected wait at a stop is `wait_factor / combined_frequency`. The
paper (p. 91) calls this the `alpha` parameter:

- `alpha = 1` (the default, used above and in the paper's worked example):
  exponentially distributed vehicle arrivals with a uniform passenger
  arrival rate, expected wait = full headway;
- `alpha = 0.5`: an approximation of constant vehicle interarrival times,
  the passenger waits on average half the headway. The paper notes this is
  "the most widely used approach in practice", despite being a rough
  approximation.

`assign_transit_with_options(&network, &od, &TransitAssignmentOptions { wait_factor: 0.5 })`
runs the same assignment with half-headway waiting. On this network the
A -> B expected travel time drops from 27.75 to **25.25 min**. Note this
is not a simple constant subtraction: cheaper waiting can change the
optimal strategy itself. Here it opens a transfer at X - with waiting
halved it becomes worthwhile for the Line 2 riders to alight at X and
board Line 3, which was not attractive at `alpha = 1`.

The factor scales only the waiting term, applied uniformly to every
boarding link, so the line-choice proportions among the lines that stay
attractive are unchanged (this is the paper's frequency-scaling remark,
p. 91). Implemented core-side: the solver is untouched, boarding links
are simply built with an effective headway of `wait_factor * headway`.

## Reference

Spiess, H. and Florian, M. (1989) "Optimal strategies: A new assignment model for transit networks". Transportation Research Part B 23(2), 83-102. DOI: [10.1016/0191-2615(89)90034-9](https://doi.org/10.1016/0191-2615(89)90034-9)
