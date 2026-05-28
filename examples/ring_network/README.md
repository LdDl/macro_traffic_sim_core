# ring_network

4-node one-way ring demonstrating automatic trip-end normalization.

Reproduces a real request that fails with "Furness did not converge after 100 iterations" when attractions are not normalized. The root cause was not a broken network but **unbalanced trip ends**: the regression model produced `sum(P) != sum(A)`, which makes the doubly-constrained problem infeasible.

The pipeline detects this and normalizes attractions to match the production total before distribution. This step is visible in the pipeline log between the Generation and Distribution phases.

No external files needed.

## Running

```sh
cargo run --example ring_network
```

## Network topology

4 nodes, each acting directly as a zone centroid.
Links form a single one-way ring (one strongly-connected component).

```text
  (1) --link 1--> (2) --link 7--> (3) --link 2--> (4) --link 12--> (1)
 zone 1          zone 2          zone 4           zone 3
```

| Link | From | To | Length (m) | Speed (km/h) | Capacity | Lanes | Connection |
|------|------|----|------------|--------------|----------|-------|------------|
| 1    | 1    | 2  | 205.8      | 60           | 5400     | 3     | no         |
| 7    | 2    | 3  | 42.5       | 60           | 5400     | 3     | yes        |
| 2    | 3    | 4  | 197.6      | 60           | 7200     | 4     | no         |
| 12   | 4    | 1  | 35.8       | 60           | 7200     | 4     | yes        |

All paths between any two zones follow the ring in one direction. There is exactly one route per OD pair, so Frank-Wolfe converges in a single iteration (no route choice to balance).

## Zones

| Zone | Node | Population | Employment |
|------|------|------------|------------|
| 1    | 1    | 100        | 15         |
| 2    | 2    | 10         | 99         |
| 3    | 4    | 100        | 200        |
| 4    | 3    | 190        | 22         |

## Trip generation

Default regression coefficients:

```
P_i = 0.5 * pop_i + 0.1 * emp_i
A_i = 0.1 * pop_i + 0.8 * emp_i
```

| Zone | Production | Attraction (raw) |
|------|------------|------------------|
| 1    | 51.5       | 22.0             |
| 2    | 14.9       | 80.2             |
| 3    | 70.0       | 170.0            |
| 4    | 97.2       | 36.6             |
| **Total** | **233.6** | **308.8**   |

`sum(P) = 233.6`, `sum(A) = 308.8` -imbalance 24.4%.

Doubly-constrained Furness requires `sum(P) == sum(A)` as a necessary condition. If they differ, no matrix exists that satisfies both row and column constraints simultaneously. Furness oscillates between row-scaling (forcing total to 233.6) and column-scaling (forcing it to 308.8) and never converges.

Production and attraction coefficients are calibrated independently in practice, so imbalance is the normal case, not an error.

## Normalization

The pipeline automatically normalizes attractions before distribution:

```
factor = sum(P) / sum(A) = 233.6 / 308.8 = 0.756477

A'_i = A_i * factor
```

Scaled attractions:

| Zone | A (raw) | A (normalized) |
|------|---------|----------------|
| 1    | 22.0    | 16.6           |
| 2    | 80.2    | 60.6           |
| 3    | 170.0   | 128.5          |
| 4    | 36.6    | 27.7           |
| **Total** | **308.8** | **233.6** |

This step appears in the pipeline log between Generation and Distribution:

```
[progress] phase=generation
{"message":"Trip generation complete","total_productions":"234","total_attractions":"309"}
{"message":"Normalizing attractions to match production total",
 "sum_productions":"233.600","sum_attractions":"308.800","normalization_factor":"0.756477"}
[progress] phase=distribution iter=1/3
```

## Model parameters

**Impedance** -exponential, beta=0.1: `f(t) = exp(-0.1 * t_minutes)`.

**Assignment** -Frank-Wolfe, 100 max iterations, gap 1e-3.

**Feedback** -3 iterations.

## Results

### OD matrix (all modes, 233.6 trips total)

|        | -> z1 | -> z2 | -> z3 | -> z4 | row sum |
|--------|-------|-------|-------|-------|---------|
| z1     | 0.0   | 7.9   | 39.0  | 4.6   | 51.5    |
| z2     | 0.8   | 0.0   | 12.6  | 1.5   | 14.9    |
| z3     | 11.2  | 37.2  | 0.0   | 21.6  | 69.9    |
| z4     | 4.7   | 15.6  | 77.0  | 0.0   | 97.2    |
| col    | 16.6  | 60.7  | 128.6 | 27.7  |         |

Row sums match productions. Column sums match normalized attractions.

Zone 3 (pop=100, emp=200) is the dominant attractor (128.6): high
employment pulls most trips from zones 1, 2, 4.

### Mode split

| Mode | Trips | Share |
|------|-------|-------|
| Auto | 158.2 | 67.7% |
| Bike | 56.8  | 24.3% |
| Walk | 18.6  | 8.0%  |

The network covers only ~500m total circumference. At such short distances the logit model finds bike (15 km/h) and walk (5 km/h) competitive with auto. Compare with grid_city (~1 km spacing) where auto share is 77.5%.

### Link volumes (auto)

| Link | Route segment | Volume | Cost (h) | V/C  |
|------|---------------|--------|----------|------|
| 1    | 1 -> 2        | 84.9   | 0.0034   | 0.016 |
| 7    | 2 -> 3        | 54.0   | 0.0007   | 0.010 |
| 2    | 3 -> 4        | 101.1  | 0.0033   | 0.014 |
| 12   | 4 -> 1        | 61.6   | 0.0006   | 0.009 |

All V/C ratios well below 1.0 -the network is uncongested. Costs are essentially free-flow (BPR adds < 0.01% delay).

Link 2 (3->4) carries the highest volume (101.1) because zone 3 (the dominant attractor, node 4) receives traffic from all three other zones going the long way around the ring.

### Assignment convergence

Frank-Wolfe converges in **1 iteration** across all 3 feedback loops. With a single route per OD pair, the all-or-nothing assignment is already optimal -no line search needed, gap is exactly 0.
