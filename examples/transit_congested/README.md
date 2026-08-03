# transit_congested

Strict-capacity congested transit assignment (Cepeda, Cominetti and Florian,
2006), reproducing the paper's own worked example (Section 4.1.1). This is the
rigorous congested equilibrium behind EMME's capacitated transit assignment: a
line cannot be loaded beyond the capacity of its vehicles, and a computable gap
function certifies the distance to equilibrium.

```
cargo run --example transit_congested
```

## The network

An express and a local line between A and C, plus short local demand:

```text
                  express (A-C, 16 bus/h, 320 pax/h)
         +----------------------------------------------+
         |                                              v
       [A] --- local-AB ---> [B] --- local-BC ---> [C]
                   (local: A-B-C, 6 bus/h, 120 pax/h)
```

| Line | Itinerary | In-vehicle | Headway | Capacity |
|------|-----------|------------|---------|----------|
| Express | A -> C | 24 min | 3.75 min (16 bus/h) | 20 pax/bus -> 320 pax/h |
| Local | A -> B -> C | 20 + 20 min | 10 min (6 bus/h) | 20 pax/bus -> 120 pax/h |

Demand: 10 A -> B, 10 B -> C, and a variable A -> C load. The A -> C trip can
take the express (E), the local (L), or board the first of the two (EL).

## What strict capacity does

Each boarding line has a flow-dependent effective frequency

```text
f_a = mu * ( 1 - ( v_a / (mu*c - v'_a + v_a) )^beta )
```

where `mu` is the nominal frequency, `c` the per-vehicle capacity (`mu*c` the
line capacity per hour), `v_a` the boarding flow and `v'_a` the on-board flow
right after the stop. As the on-board flow approaches the line capacity the
residual `mu*c - v'_a` vanishes, the effective frequency drops to zero, and the
waiting time explodes - so the line **cannot** be overloaded. Excess demand is
pushed onto other lines (here the slower local) or, in a saturated network,
onto walking, revealing the corridors that need more capacity.

This wraps the unchanged Spiess-Florian optimal-strategies solver in the
Cepeda-Cominetti-Florian method of successive averages: each iteration freezes
the effective frequencies at the current flow, solves the ordinary shortest
hyperpath, and averages the result. Progress is measured by their gap function
(Theorem 3.2)

```text
G(v) = sum_d [ sum_a t_a(v) v^d_a + sum_i max_a (v^d_a / f_a(v)) - sum_i g^d_i tau^d_i(v) ]
```

which is `>= 0` always and `= 0` exactly at equilibrium; the run stops on the
relative gap `G(v) / sum g^d_i tau^d_i`.

## What comes out

### Low demand (100 A -> C)

The express has spare seats, but its rising wait makes the local competitive,
so the A -> C demand splits across the two lines:

| | Paper | This run |
|-|-------|----------|
| Express boardings | 84.3 | 84.26 |
| A -> C time (min) | 40.02 | 40.02 |

### High demand (350 A -> C)

An all-or-nothing load would put 350 on the express, over its 320 capacity. The
express saturates and the local carries the overflow:

| | Paper | This run |
|-|-------|----------|
| Express boardings | 260.5 | 260.55 |
| Local segment (riding) | 99.5 | 99.45 |
| A -> C time (min) | 97.36 | 97.42 |

The express settles within its 320 pax/hour capacity in both cases.

## On the small differences

The boardings match the paper to its reported precision (84.26 rounds to 84.3,
260.55 to 260.5), and the low-demand time is exact (40.02). The high-demand time
differs by 0.06 min (97.42 vs 97.36). This is not a model error: our value is
stable as the gap tolerance is tightened to `1e-10`, so it is the true
equilibrium, while the paper's numbers are themselves method-of-successive-
averages approximations reported to two decimals and stopped at a looser gap
(their Figure 5 shows the MSA halted around a 0.25% relative gap). Two
approximations of the same equilibrium agreeing to about 0.06% is a strong
validation; the pure-express strategy time matches the paper's 117.04 as well.

## References

- Cepeda, M., Cominetti, R. and Florian, M. (2006) "A frequency-based assignment
  model for congested transit networks with strict capacity constraints:
  characterization and computation of equilibria". Transportation Research Part
  B 40(6), 437-459. DOI: [10.1016/j.trb.2005.05.006](https://doi.org/10.1016/j.trb.2005.05.006)
- Cominetti, R. and Correa, J. (2001) "Common-Lines and Passenger Assignment in
  Congested Transit Networks". Transportation Science 35(3), 250-267.
  DOI: [10.1287/trsc.35.3.250.10154](https://doi.org/10.1287/trsc.35.3.250.10154)
- Spiess, H. and Florian, M. (1989) "Optimal strategies: A new assignment model
  for transit networks". Transportation Research Part B 23(2), 83-102.
  DOI: [10.1016/0191-2615(89)90034-9](https://doi.org/10.1016/0191-2615(89)90034-9)
