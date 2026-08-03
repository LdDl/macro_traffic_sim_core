# transit_decea

Crowding on de Cea and Fernandez's (1993) "Modified Network G" (their
Figure 1), used here as a paper-grounded debugging scaffold: it runs our
crowded transit assignment on the network the paper builds its worked example
on, so the behaviour can be sanity-checked against the article.

```
cargo run --example transit_decea
```

## The network

Nodes N1..N4 and six lines (de Cea Fig. 1), one O-D pair N1 -> N3:

```text
                 L1 (N1-N2-N3)
        +--------------------------------+
        |          L2          L5, L6     |
       N1 ----------------> N2 ---------> N3
        \                  ^
      L3 \                / L4
          \              /
           +---> N4 ----+
```

| Line | Itinerary | Role |
|------|-----------|------|
| L1 | N1 -> N2 -> N3 | direct, through both segments |
| L2 | N1 -> N2 | feeder to N2 |
| L5, L6 | N2 -> N3 | second leg from N2 |
| L3, L4 | N1 -> N4 -> N2 | detour via N4 (longer, unused here) |

The per-line in-vehicle times, headways and capacities are __ILLUSTRATIVE__
(see the caveats below); the topology is exactly the paper's.

## What we compute (and what we do NOT)

This is the single most important thing to understand about this example.

- We compute the Spiess-Florian optimal-strategies solution, plus the de Cea / Cominetti-Correa effective-frequency crowding. de Cea note that Spiess "spreads the flows over more paths, using line L1 [...] due to the concept of optimum strategy", so on this network we reproduce their **Figure 7 (Spiess)**, NOT their route-section **Figure 6**. The Fig. 6 numbers come from a different assignment rule and are not our target.
- The per-line inputs are illustrative. de Cea's Table 1 gives route-section aggregates (S1..S6), two of them frequency-weighted "expected" times, so the individual line frequencies and travel times cannot be recovered uniquely. We picked per-line values of the right magnitude; they are not the paper's exact inputs.
- Crowding uses each line's own load. The cross-section coupling `V_hat` of de Cea's Eq. 7 (a line shared between route sections eating its own residual capacity) is dropped.

So this example is a qualitative scaffold - it shows the two phenomena the paper describes, on the paper's network - not a numeric reproduction of any published table.

## What comes out

### 1. Uncongested: optimal strategies spread the demand

600 passengers, N1 -> N3, no crowding:

| Line | Boardings |
|------|-----------|
| L1 (direct N1-N2-N3) | 425 |
| L2 (N1-N2) | 300 |
| L5 (N2-N3) | 125 |
| L6 (N2-N3) | 50 |

Expected N1 -> N3 time: **23.83 min**.

Boardings sum to more than 600 because passengers transfer: 425 ride L1 straight through, while 175 board L2 to N2 and there re-board L5 (125) or L6 (50). This spread over several lines - including the direct L1 - is exactly the "optimum strategy" behaviour de Cea contrast with their Fig. 6.

### 2. Crowded: the direct line saturates and sheds riders

L1 seats 30 per vehicle at a 6-minute headway, so its line capacity is `(60/6) * 30 = 300` passengers/hour - well below the 425 it attracts. With crowding on (effective frequency `f_eff = f / (1 + alpha*(load/cap)^beta)`, de Cea Eq. 16):

| Line | Uncrowded | Crowded |
|------|-----------|---------|
| L1 (direct, cap 300/h) | 425 | 311 |
| L2 (N1-N2) | 300 | 388 |
| L5 (N2-N3) | 125 | 209 |
| L6 (N2-N3) | 50 | 80 |

Expected N1 -> N3 time: **26.02 min** (up from 23.83).

L1 drops toward its ~300 capacity; the freed demand moves onto the two-leg N1 -> N2 -> N3 path (L2, then L5/L6), and the average trip gets slower. That is the crowding redistribution, computed with the unchanged optimal-strategies solver wrapped in the outer averaging loop.

## How this compares to the paper's numbers

de Cea's own results are in Section 5 (pp. 145-146): Table II and Figures 6-9. They report **route-section** flow vectors `V* = (S1..S6)`, and line-section loads written `v_l^s` (flow of line `l` over route section `s`), for three cases:

| Case | Demand | Paper result |
|------|--------|--------------|
| Uncongested | T=1 | `V=(0,0,1,1,0,1)`; loads `v_2^5=1, v_3^4=0.17, v_4^4=0.83` (Fig. 6) |
| Mild (beta=10, n=1) | T=100 | `V*=(57,0,0,43,43,0)`; loads `v_1^1=57, v_2^5=43, v_3^4=7, v_4^4=36` (Fig. 8) |
| High (beta=20) | T=240 | diagonalization `V*=(140,6,0,94,94,6)` (Fig. 9); exact algorithm `V*=(137,5,0,98,98,5)`, loads `v_1^1=137, v_2^2=5, v_2^5=98, v_3^4=12, v_3^6=5, v_4^4=86` |

These are **not directly comparable** to our line boardings, for four reasons:

1. **Different variables** - the paper reports route-section flows `V^s` (S1..S6); we report line boardings (L1..L6). A section aggregates several lines.
2. **Different demand** - the paper uses 1 / 100 / 240; this example uses 600.
3. **Different inputs** - the paper uses the real Table 1 section data; our per-line inputs are illustrative.
4. **Different assignment** - the paper solves a route-section diagonalization; we solve Spiess optimal strategies with effective frequencies. The paper itself shows its Fig. 6 differs from the Spiess Fig. 7.

The closest point of contact: the paper's own two methods already differ - Frank-Wolfe diagonalization gives `(140,6,0,94,94,6)`, its "exact algorithm" gives `(137,5,0,98,98,5)`, a difference de Cea attribute to "the use of the __effective frequencies__ instead of the nominal frequencies". Our crowding uses effective frequencies (Eq. 16), so it sits with de Cea's exact algorithm, just solved via the Spiess solver and an outer averaging loop rather than their route-section diagonalization.

Qualitatively the behaviour matches: uncongested, the direct line dominates; congestion / crowding shifts flow onto the transfer path. The paper's mild case is 57 direct / 43 transfer (57/43 at demand 100); ours crowded is 311 / 289 (about 52/48 at demand 600) - same direction, different magnitudes for the four reasons above.

## Note on per-segment load

Because every trip here rides end to end (a single O-D pair, all demand born at N1), each line's peak segment load already equals its total boardings, so using boardings as the crowding load is exact on this network. On networks with mid-route boarding/alighting (e.g. the `multimodal` example, where line B1 has an intermediate stop) the peak segment load is lower than total boardings; that refinement matters there, not here.

## References

- de Cea, J. and Fernandez, E. (1993) "Transit Assignment for Congested Public Transport Systems: An Equilibrium Model". Transportation Science 27(2), 133-147. DOI: [10.1287/trsc.27.2.133](https://doi.org/10.1287/trsc.27.2.133)
- Cominetti, R. and Correa, J. (2001) "Common-Lines and Passenger Assignment in Congested Transit Networks". Transportation Science 35(3), 250-267. DOI: [10.1287/trsc.35.3.250.10154](https://doi.org/10.1287/trsc.35.3.250.10154)
- Spiess, H. and Florian, M. (1989) "Optimal strategies: A new assignment model for transit networks". Transportation Research Part B 23(2), 83-102. DOI: [10.1016/0191-2615(89)90034-9](https://doi.org/10.1016/0191-2615(89)90034-9)
