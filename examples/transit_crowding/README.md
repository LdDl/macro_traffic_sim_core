# transit_crowding

Soft-capacity passenger crowding (De Cea & Fernandez 1993; Cominetti & Correa
2001): two parallel lines with the same nominal service, one with small
vehicles. As the small line fills, its effective frequency drops and its
waiting time rises, so it sheds riders onto the roomy line. Sweeping the demand
upward shows the tipping point where the small line runs out of seats.

```
cargo run --example transit_crowding
```

## The network

Two lines A -> B, identical 10-minute headway, differing only in vehicle size:

```text
           Big  (headway 10 min, 100 seats/veh -> 600 seats/hour)
         +----------------------------------------------------+
         |                                                    v
       [A]                                                  [B]
         |                                                    ^
         +----------------------------------------------------+
           Small (headway 10 min, 30 seats/veh -> 180 seats/hour)
```

The line capacity is `(analysis_period / headway) * seats`, so over a one-hour
period the Big line holds 600 passengers and the Small line 180.

## What soft crowding does

Uncrowded, two equal frequencies split the demand 50/50 (the ordinary
optimal-strategies result). With crowding on, each line's effective frequency
falls with its load:

```text
f_eff = f / (1 + alpha * (load / capacity)^beta)
```

This is De Cea & Fernandez's effective frequency (their Eq. 16), a BPR-like
convex penalty. As the Small line's load approaches its capacity its wait rises,
so the line-choice split tips toward the Big line. Crowding is an outer
method-of-successive-averages loop that rescales each line's frequency by its
load and re-runs the unchanged Spiess-Florian solver; uncapacitated routes are
unaffected. See `assign_transit_crowded` / `CrowdingParams`.

## What comes out

Demand swept from 100 to 900 passengers/hour (Small line capacity 180):

| demand | uncrowded (Big / Small) | crowded (Big / Small) | Small load |
|-------:|------------------------:|----------------------:|-----------:|
| 100 | 50 / 50 | 50 / 50 | 28% |
| 200 | 100 / 100 | 104 / 96 | 53% |
| 300 | 150 / 150 | 168 / 132 | 73% |
| 360 | 180 / 180 | 212 / 148 | 82% |
| 500 | 250 / 250 | 322 / 178 | 99% |
| 700 | 350 / 350 | 482 / 218 | 121% |
| 900 | 450 / 450 | 637 / 263 | 146% |

Below about 360 passengers/hour (twice the Small capacity) both lines have spare
seats and the split stays even. Above it the Small line's growth flattens out
near its 180-passenger capacity and the Big line takes most of the extra demand.

## Soft vs strict capacity

The effective-frequency law here is a **soft** BPR-like penalty, not a hard
cutoff: under very heavy demand the Small line still creeps over its capacity
(121%, 146% in the table) rather than refusing passengers outright. That matches
De Cea & Fernandez's own modeling choice ("same links can be overloaded when
demands are too high"). When a line must **never** exceed its capacity, use the
strict-capacity model of Cepeda-Cominetti-Florian (2006) instead - see the
[`transit_congested`](../transit_congested/) example, where the effective
frequency vanishes at capacity and a gap function certifies the equilibrium.

## References

- De Cea, J. and Fernandez, E. (1993) "Transit Assignment for Congested Public
  Transport Systems: An Equilibrium Model". Transportation Science 27(2),
  133-147. DOI: [10.1287/trsc.27.2.133](https://doi.org/10.1287/trsc.27.2.133)
- Cominetti, R. and Correa, J. (2001) "Common-Lines and Passenger Assignment in
  Congested Transit Networks". Transportation Science 35(3), 250-267.
  DOI: [10.1287/trsc.35.3.250.10154](https://doi.org/10.1287/trsc.35.3.250.10154)
- Spiess, H. and Florian, M. (1989) "Optimal strategies: A new assignment model
  for transit networks". Transportation Research Part B 23(2), 83-102.
  DOI: [10.1016/0191-2615(89)90034-9](https://doi.org/10.1016/0191-2615(89)90034-9)
