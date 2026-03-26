# grid_city

6-zone city on a 3x3 grid with a proper mesoscopic graph.

Unlike `simple_network` where each intersection is a single node,
here each intersection is **split** into approach/departure sub-nodes.
Connection links between sub-nodes encode allowed turning movements.
One-way streets and turn restrictions are enforced by topology.

Zone centroids are separate nodes connected to the road graph via
bidirectional connector links (two directed links per connector).

No external files needed.

## Running

```sh
cargo run --example grid_city
```

## Network topology

9 macro-intersections [1]-[9] form a 3x3 grid (~1 km spacing, Moscow area).
6 zone centroid nodes (101)-(106) sit outside the grid.

```text
           (101)---[1]====[2]====[3]---(102)
                    |      ||      |
                   [4]-->[5]--->[6]
                    |      ||      |
           (104)---[7]<--[8]<---[9]---(105)
                          ||
                        (103)

  (106) connects to [2] and [5]

  ====  bidirectional, 2 lanes, 60 km/h
  ---   bidirectional, 1 lane,  40 km/h
  -->   one-way eastbound,  2 lanes, 50 km/h
  <--   one-way westbound,  1 lane,  40 km/h
  ||    bidirectional, 2 lanes, 60 km/h  (central avenue)
```

### Mesoscopic representation

Each macro-intersection N is split into sub-nodes:

| Sub-node type | ID scheme       | Example (int. 5, from 2) |
|---------------|-----------------|--------------------------|
| Incoming      | N * 1000 + A    | 5002                     |
| Outgoing      | N * 1000 + 500 + B | 5508                  |

A **connection link** from incoming sub-node to outgoing sub-node
encodes one allowed turn. If there is no connection link, the turn
is forbidden. The node-based Dijkstra naturally enforces this because
traffic must traverse through sub-nodes.

Example path from zone 1 (centroid 101) to zone 3 (centroid 103):

```text
101 --connector--> 1_in_101 --turn--> 1_out_2 --road--> 2_in_1
    --turn--> 2_out_5 --road--> 5_in_2 --turn--> 5_out_8
    --road--> 8_in_5 --turn--> 8_out_103 --connector--> 103
```

### One-way streets

- Middle row (4 -> 5 -> 6): eastbound only, 2 lanes, 50 km/h
- Bottom row (9 -> 8 -> 7): westbound only, 1 lane, 40 km/h

### Turn restrictions

- Intersection 5: no left from south (8) to east (6)
- Intersection 2: no left from east (3) to south (5)

### Road links

| Segment     | Lanes | Speed (km/h) | Capacity (veh/h/lane) | Direction |
|-------------|-------|--------------|-----------------------|-----------|
| 1-2, 2-3    | 2     | 60           | 1800                  | bidir     |
| 1-4, 4-7    | 1     | 40           | 1200                  | bidir     |
| 2-5, 5-8    | 2     | 60           | 1800                  | bidir     |
| 3-6, 6-9    | 1     | 40           | 1200                  | bidir     |
| 4-5, 5-6    | 2     | 50           | 1500                  | east only |
| 9-8, 8-7    | 1     | 40           | 1200                  | west only |

### Centroid connectors

| Centroid | Zone             | Connects to |
|----------|------------------|-------------|
| 101      | Residential NW   | nodes 1, 4  |
| 102      | Residential NE   | nodes 3, 6  |
| 103      | CBD South        | node 8      |
| 104      | Residential SW   | node 7      |
| 105      | Industrial SE    | node 9      |
| 106      | University N     | nodes 2, 5  |

Each connector: 1 lane, 40 km/h, 9999 veh/h (unconstrained).
9 connectors x 2 directions = 18 connector links.

### Network totals

- 62 nodes (56 intersection sub-nodes + 6 centroid nodes)
- ~106 links (connection + road + connector)

## Zones

| Zone | Name             | Population | Employment |
|------|------------------|------------|------------|
| 1    | Residential NW   | 15000      | 1500       |
| 2    | Residential NE   | 12000      | 1500       |
| 3    | CBD South        | 3500       | 12000      |
| 4    | Residential SW   | 8000       | 2000       |
| 5    | Industrial SE    | 7000       | 8000       |
| 6    | University N     | 7000       | 5000       |

**Totals:** pop=52500, emp=30000 (ratio 1.75, balanced for Furness).

## Model parameters

**Trip generation** -- regression with default coefficients:
- Production: P_i = 0.5 * pop_i + 0.1 * emp_i
- Attraction: A_i = 0.1 * pop_i + 0.8 * emp_i
- sum(P) = sum(A) = 29250

**Trip distribution** -- gravity model, exponential impedance f(t) = exp(-0.15 * t).

**Mode choice** -- multinomial logit (auto / bike / walk).

**Assignment** -- Frank-Wolfe, 200 max iterations, gap threshold 1e-3,
BPR function (alpha=0.15, beta=4.0).

**Feedback** -- 3 iterations.

## Results

### Trip generation

| Zone | Name             | Production | Attraction | P - A  |
|------|------------------|------------|------------|--------|
| 1    | Residential NW   | 7650       | 2700       | +4950  |
| 2    | Residential NE   | 6150       | 2400       | +3750  |
| 3    | CBD South        | 2950       | 9950       | -7000  |
| 4    | Residential SW   | 4200       | 2400       | +1800  |
| 5    | Industrial SE    | 4300       | 7100       | -2800  |
| 6    | University N     | 4000       | 4700       | -700   |
| **Total** |             | **29250**  | **29250**  |        |

The P - A column reveals the city's commuting structure:

- **Zones 1, 2, 4** are net producers (P > A): residential areas that
  export trips. Zone 1 (Residential NW, pop=15000) is the biggest trip
  source -- it generates 7650 trips but attracts only 2700, so nearly 5000
  trips per period flow outward toward jobs.

- **Zone 3** (CBD South, emp=12000) is the dominant attractor: it pulls
  in 7000 more trips than it produces. This creates the classic radial
  commute pattern -- traffic flows from the periphery toward the CBD.

- **Zone 5** (Industrial SE) is the second attractor (deficit -2800).
  Combined with zone 3, the southern half of the city attracts 9800
  more trips than it produces, so the network must carry massive
  north-to-south and west-to-south flows.

- **Zone 6** (University) is nearly balanced (P ~ A), acting as both
  a residential and an employment area.

### Mode split

| Mode  | Trips | Share |
|-------|-------|-------|
| Auto  | 22657 | 77.5% |
| Bike  | 6163  | 21.1% |
| Walk  | 430   | 1.5%  |

Auto share is higher here (77.5%) than in the simple_network example (73.0%)
because average trip distances are larger (~1 km grid spacing with multi-hop
routes), and longer distances penalize slow modes harder through the logit
model's time coefficient.

Walk share is negligible (1.5%) -- at 5 km/h, a 2 km trip takes 24 minutes,
giving a utility of $-2.0 + (-0.08) \times 24 = -3.92$ versus auto's
$0.0 + (-0.03) \times 2 = -0.06$. The utility gap makes walking uncompetitive
for all but the shortest OD pairs.

### Assignment

Frank-Wolfe converges in 12-15 iterations (gap < 1e-3).
The mesograph structure provides enough route alternatives for fast convergence.

#### Road segments

All 20 road segments sorted by volume:

| Segment | Lanes | Volume | Capacity | V/C  | Cost (h) | Free-flow (h) | Delay factor |
|---------|-------|--------|----------|------|----------|---------------|--------------|
| 5 -> 8  | 2    | 6395   | 3600     | 1.78 | 0.042    | 0.015         | 2.8x         |
| 6 -> 9  | 1    | 5472   | 1200     | 4.56 | 1.647    | 0.015         | 110x         |
| 2 -> 5  | 2    | 4738   | 3600     | 1.32 | 0.024    | 0.015         | 1.6x         |
| 3 -> 2  | 2    | 4450   | 3600     | 1.24 | 0.018    | 0.013         | 1.4x         |
| 1 -> 2  | 2    | 4373   | 3600     | 1.21 | 0.018    | 0.013         | 1.4x         |
| 4 -> 5  | 2    | 3927   | 3000     | 1.31 | 0.023    | 0.015         | 1.5x         |
| 7 -> 4  | 1    | 3141   | 1200     | 2.62 | 0.201    | 0.025         | 8.0x         |
| 5 -> 2  | 2    | 2998   | 3600     | 0.83 | 0.018    | 0.015         | 1.2x         |
| 5 -> 6  | 2    | 2901   | 3000     | 0.97 | 0.018    | 0.015         | 1.2x         |
| 2 -> 3  | 2    | 2818   | 3600     | 0.78 | 0.014    | 0.013         | 1.1x         |
| 2 -> 1  | 2    | 2482   | 3600     | 0.69 | 0.014    | 0.013         | 1.1x         |
| 8 -> 5  | 2    | 2301   | 3600     | 0.64 | 0.017    | 0.015         | 1.1x         |
| 9 -> 8  | 1    | 1998   | 1200     | 1.67 | 0.044    | 0.015         | 2.9x         |
| 3 -> 6  | 1    | 1307   | 1200     | 1.09 | 0.030    | 0.025         | 1.2x         |
| 9 -> 6  | 1    | 1288   | 1200     | 1.07 | 0.030    | 0.025         | 1.2x         |
| 4 -> 7  | 1    | 1248   | 1200     | 1.04 | 0.029    | 0.025         | 1.2x         |
| 6 -> 3  | 1    | 925    | 1200     | 0.77 | 0.026    | 0.025         | 1.0x         |
| 1 -> 4  | 1    | 728    | 1200     | 0.61 | 0.026    | 0.025         | 1.0x         |
| 8 -> 7  | 1    | 614    | 1200     | 0.51 | 0.021    | 0.015         | 1.4x         |
| 4 -> 1  | 1    | 211    | 1200     | 0.18 | 0.025    | 0.025         | 1.0x         |

The "delay factor" column shows how much slower each link is compared to
free-flow: 1.0x means no congestion, 2x means double the travel time.

#### Congestion levels

The V/C ratio classifies links into congestion categories:

**Severe congestion (V/C > 2.0):**

- **6 -> 9** (V/C = 4.56, delay 110x). This is the worst bottleneck in the
  network. A single-lane road carries 5472 veh against 1200 capacity. BPR
  yields $t = 0.015 \times (1 + 0.15 \times 4.56^4) = 0.015 \times 110 = 1.65$ hours
  for a segment that should take under a minute. This is physically unrealistic --
  in the real world the queue would spill back across the network. Within the
  BPR model though, this signals a critical capacity deficit.

  Why so much traffic? Zone 2 (Residential NE, 6150 productions) sends trips
  to zone 5 (Industrial SE, 7100 attractions). The only path from zone 2
  to zone 5 through the east side goes 3 -> 6 -> 9 -> 105. The one-way
  constraint on the middle row (eastbound only: 4 -> 5 -> 6) means traffic
  cannot go 6 -> 5 directly. The turn restriction at intersection 2
  (no left from 3 to 5) further limits alternatives, forcing even more
  traffic through the 6 -> 9 segment.

- **7 -> 4** (V/C = 2.62, delay 8x). Zone 4 (Residential SW) traffic
  returning northward. 3141 vehicles on a single-lane road (capacity 1200).
  Most of this is zone 4 residents (connector 104 -> 7: 3141 veh) entering
  the network via node 7 and heading north to intersections 4 and 5.

**Moderate congestion (V/C 1.0-2.0):**

- **5 -> 8** (V/C = 1.78, delay 2.8x). Central avenue southbound -- the
  main pipeline to CBD South (zone 3). 6395 vehicles, the highest volume
  of any road segment. This is the combined flow from zones 1, 2, 6
  heading south to the CBD.

- **9 -> 8** (V/C = 1.67, delay 2.9x). Bottom row westbound (one-way).
  Traffic from Industrial SE heading toward CBD (zone 3 via node 8).

- **2 -> 5, 4 -> 5** (V/C ~1.3). Central avenue southbound and eastbound
  one-way -- continuation of the north-to-south corridor.

- **1 -> 2, 3 -> 2** (V/C ~1.2). Top arterial -- traffic converges on
  node 2 from both sides to enter the central avenue southbound.

- **3 -> 6, 9 -> 6, 4 -> 7** (V/C ~1.05-1.09). Minor links just at capacity.

**Free flow (V/C < 1.0):**

- **5 -> 2** (V/C = 0.83). Central avenue northbound -- reverse commute
  carries only 2998 veh vs 4738 southbound. This asymmetry reflects the
  CBD gravity: more people go south to work than return north.

- **5 -> 6** (V/C = 0.97). One-way eastbound -- nearly at capacity but not
  overloaded.

- **4 -> 1** (V/C = 0.18). The lightest road segment. Very few people
  travel from the middle of the city northwest toward residential zone 1.

#### Flow asymmetry

Comparing directional pairs reveals the commuting pattern:

| Corridor      | Southbound | Northbound | Ratio | Interpretation |
|---------------|------------|------------|-------|----------------|
| 2 -> 5 / 5 -> 2 | 4738    | 2998       | 1.6x  | To CBD dominates |
| 5 -> 8 / 8 -> 5 | 6395    | 2301       | 2.8x  | Heavy CBD pull |
| 1 -> 2 / 2 -> 1 | 4373    | 2482       | 1.8x  | Residential outflow |
| 1 -> 4 / 4 -> 1 | 728     | 211        | 3.5x  | Minor but one-sided |
| 4 -> 7 / 7 -> 4 | 1248    | 3141       | 0.4x  | Zone 4 entering network |

The 5 -> 8 / 8 -> 5 pair (ratio 2.8x) is the starkest: nearly three times
more traffic flows toward the CBD than away from it. This is the signature
of a monocentric city where employment is concentrated in the south.

The 7 -> 4 direction (3141) is much heavier than 4 -> 7 (1248) because
zone 4 residents enter the network at node 7 and head north through node 4
to reach the central avenue. The return flow (4 -> 7) is light because
few trips end in zone 4 relative to what zone 4 produces.

#### Connectors: zone-level in/out balance

Connector volumes show the total auto traffic entering and leaving each zone:

| Zone | Name           | Into zone | Out of zone | Net    |
|------|----------------|-----------|-------------|--------|
| 1    | Residential NW | 2108      | 6032        | -3924  |
| 2    | Residential NE | 1874      | 4789        | -2915  |
| 3    | CBD South      | 7771      | 2293        | +5478  |
| 4    | Residential SW | 1862      | 3141        | -1279  |
| 5    | Industrial SE  | 5472      | 3286        | +2186  |
| 6    | University N   | 5727      | 4909        | +818   |

Notes:
- Zone 1 inflow (2108) = connector 1 -> 101 (1754) + connector 4 -> 101 (354).
  Zone 1 outflow (6032) = connector 101 -> 1 (4161) + connector 101 -> 4 (1871).
- Zone 3 (CBD) receives 7771 vehicles but sends out only 2293 -- the net
  imbalance (+5478) confirms this is the city's job center.
- Zone 6 (University) is the most balanced zone: 5727 in vs 4909 out.
  Its dual connectors (to nodes 2 and 5) give it good access to the
  central avenue in both directions.
- Zone 1 has a dual connector too (nodes 1 and 4), with most outflow
  via node 1 (4161) rather than node 4 (1871) -- the top arterial (1 -> 2)
  is faster than the minor road (1 -> 4).

#### Turn volumes at intersections

Top 10 turning movements:

| Intersection | Movement    | Volume | Interpretation |
|--------------|-------------|--------|----------------|
| 8            | 5 -> 103    | 6395   | Central ave traffic exiting to CBD |
| 9            | 6 -> 105    | 5472   | East corridor traffic to Industrial SE |
| 1            | 101 -> 2    | 4161   | Zone 1 residents heading east to arterial |
| 3            | 102 -> 2    | 3525   | Zone 2 residents heading west to arterial |
| 5            | 2 -> 8      | 3180   | Straight through to CBD |
| 7            | 104 -> 4    | 3141   | Zone 4 residents entering northbound |
| 2            | 3 -> 106    | 3104   | Eastbound traffic exiting to University |
| 6            | 5 -> 9      | 2901   | Eastbound one-way continuing south |
| 4            | 7 -> 5      | 2576   | Southbound traffic turning east |
| 2            | 1 -> 5      | 2400   | Top arterial turning south to central ave |

The turn "2: 1 -> 5" (2400 veh) is critical: this is the left turn at
intersection 2 where traffic from the top arterial (node 1) enters the
central avenue southbound (node 5). If this turn were restricted, the
entire north-to-south corridor would be disrupted.

Turn "5: 2 -> 8" (3180 veh) is the straight-through at intersection 5.
The banned turn "5: 8 -> 6" (south to east) does not appear in the table --
the restriction is working. Traffic from node 8 wanting to go east must
detour through node 2 first.

#### Effect of turn restrictions

Two turns are banned:

1. **Intersection 5: no left from 8 to 6.** Without this ban, traffic
   from CBD (zone 3) heading to Industrial SE (zone 5) could go
   103 -> 8 -> 5 -> 6 -> 9 -> 105. Instead it must detour, e.g.,
   103 -> 8 -> 5 -> 2 -> 3 -> 6 -> 9 -> 105 or use other paths.
   This shifts load onto the central avenue northbound (8 -> 5 -> 2)
   and the top arterial (2 -> 3).

2. **Intersection 2: no left from 3 to 5.** Without this ban, traffic
   from zone 2 could go 102 -> 3 -> 2 -> 5 -> 8 -> 103 directly.
   Instead it must find alternatives: either go 102 -> 6 -> 9 -> 8 -> 103
   (which overloads 6 -> 9) or 102 -> 3 -> 6 -> 9 -> 8 -> 103.
   This is a major contributor to the 6 -> 9 bottleneck.

#### Effect of one-way streets

The middle row (4 -> 5 -> 6) is eastbound only and the bottom row
(9 -> 8 -> 7) is westbound only. This creates a one-way loop:

```text
[4] --> [5] --> [6]
                 |
                 v
[7] <-- [8] <-- [9]
```

Traffic circulates clockwise in the lower half of the grid. This means:

- Eastbound demand (zone 4 to zone 5) must go 4 -> 5 -> 6 -> 9 -> 105
  (3 hops instead of the direct 4 -> 5 -> 8 -> 9 -> 105 which would
  require westbound on the middle row).

- Westbound demand (zone 5 to zone 4) must go 105 -> 9 -> 8 -> 7 -> 104
  (the long way around via the bottom row).

- The one-way loop concentrates traffic on specific segments, increasing
  V/C ratios. Compare: segment 4 -> 5 (one-way, V/C=1.31) carries nearly
  all zone 4 eastbound flow, whereas the bidirectional top arterial splits
  flow between two directions (1 -> 2 at V/C=1.21, 2 -> 1 at V/C=0.69).

### Feedback convergence

| Feedback iter | FW iterations | FW gap   | Furness iters |
|---------------|---------------|----------|---------------|
| 1             | 12            | 9.75e-4  | 3             |
| 2             | 15            | 7.57e-4  | 4             |
| 3             | 12            | 9.37e-4  | 3             |

Unlike simple_network (where all 3 feedback iterations were identical),
here the feedback loop produces slightly different results each time.
The congested skim from iteration 1 (especially the huge delay on 6 -> 9)
shifts some OD flows in iteration 2, which changes the distribution
enough to require 4 Furness iterations (vs 3 in iterations 1 and 3).

The FW gap fluctuates (9.75e-4, 7.57e-4, 9.37e-4) rather than
monotonically decreasing -- this is normal for the feedback loop, which
is not guaranteed to converge monotonically. All gaps remain below the
1e-3 threshold.

### Execution time

| Step           | Time (ms) |
|----------------|-----------|
| Generation     | 0.002     |
| Distribution   | 0.189     |
| Mode choice    | 1.151     |
| Assignment     | 204.5     |
| **Total**      | **210.0** |

Assignment dominates (97% of total time). The mesograph has 62 nodes and
106 links -- each FW iteration runs Dijkstra from 6 zone centroids,
and 12-15 iterations over 3 feedback loops means ~120 Dijkstra runs.
Distribution and mode choice are fast because the OD matrix is only 6x6.
