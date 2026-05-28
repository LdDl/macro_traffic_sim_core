# Disconnected network - why it cannot work

This example demonstrates that feeding a disconnected network into the
4-step model is not a "configuration issue" but a **mathematical
impossibility**. No amount of tuning (more Furness iterations, looser
tolerance) will fix it.

## What "disconnected" means here

A network is disconnected in the macro-model sense when there exist at
least two zones `i` and `j` with no directed path between them. This
makes the shortest-path cost `c(i,j) = +inf` and therefore the
impedance `f(c(i,j)) = 0` (for any reasonable decay function).

The example network:

```
Node 1 (zone 1) ---link 1---> Node 2 (zone 2)

Node 3 (zone 4) ---link 2---> Node 4 (zone 3)
```

Nodes 2 and 4 have no outgoing links, so zones 2 and 3 cannot reach
anyone. The friction matrix looks like:

```
         Z1    Z2    Z3    Z4
Z1  [  f11   f12    0     0  ]
Z2  [   0     0     0     0  ]   <-- entire row is zero
Z3  [   0     0     0    f34 ]
Z4  [   0     0     0     0  ]   <-- entire row is zero
```

## The Furness (IPF) feasibility condition

Furness is an instance of the **Sinkhorn-Knopp algorithm**. Its
theoretical guarantee is:

> A non-negative seed matrix `S` can be scaled to satisfy given row
> targets `P` and column targets `A` **if and only if** `S` has
> *total support*: the non-zero entries of `S` cover a set that admits
> a feasible doubly-stochastic structure for those marginals.

For a block-diagonal friction matrix (off-diagonal blocks all zero),
this condition reduces to:

```
sum_{i in component k} P[i]  =  sum_{j in component k} A[j]
                                 for every connected component k
```

That is, the **within-component productions must equal within-component
attractions**. The global balance `sum P = sum A` is necessary but not
sufficient.

### Why global balance is not enough

Suppose zones split into two components `{1,2}` and `{3,4}`.
Trips from zone 1 can only ever reach zones 1 or 2 -- there is no
physical path to zones 3 or 4. Therefore the row sum constraint
`T[1,1] + T[1,2] = P[1]` is the only achievable target for zone 1.
Furness cannot redistribute the "missing" trips across the block
boundary because `S[1,3] = S[1,4] = 0` -- scaling a zero by any
finite factor leaves it zero.

As long as `P[1] + P[2] != A[1] + A[2]`, the system has no solution.
Furness will oscillate forever, alternately satisfying the row target
for zone 1 while violating the column target for zone 2 and vice versa.

### Formal statement

For a matrix `T` to exist satisfying:

```
T[i,j]  >= 0                  for all i, j
sum_j T[i,j]  = P[i]          for all i
sum_i T[i,j]  = A[j]          for all j
T[i,j]  = 0  whenever  f(c(i,j)) = 0   (gravity structure)
```

it is necessary and sufficient (Gale-Ryser for the continuous case)
that for every subset `R` of rows and every subset `C` of columns such
that `S[i,j] = 0` for all `i in R, j not in C`:

```
sum_{i in R} P[i]  <=  sum_{j in C} A[j]
```

A disconnected network creates exactly such a forbidden configuration:
take `R = {zone 2}` and `C = {}` (zone 2 can reach no zone). Then
`sum P[R] = P[2] > 0` but `sum A[C] = 0`. The inequality is violated.

## The "two sub-cities" idea does not help

One might think: if I model two isolated districts in one run, and I
carefully balance productions and attractions *within each district*,
Furness should converge for each block independently.

This is **true but useless in practice**:

1. **You must guarantee per-component balance manually.** The model
   gives no error or warning if `sum(P_k) != sum(A_k)` for a
   component. It just does not converge.

2. **The gravity model assumes universal interaction.** The deterrence
   function `f(c(i,j))` encodes *how much* people avoid long trips,
   not *whether* a trip is possible. Setting `f = 0` for unreachable
   pairs is a degenerate case outside the model's design intent.

3. **Cross-component mode choice is undefined.** After distribution,
   the pipeline builds skim matrices for auto/bike/walk. For
   unreachable pairs the skim is `+inf`; the logit utility is `-inf`;
   the mode probability is undefined (0/0 under softmax).

4. **Two sub-models are cheaper and correct.** Running two separate
   pipeline calls with two separate connected networks produces valid,
   independent results with no coupling artefacts.

## What a correct network looks like

Every zone centroid must have at least one outgoing path to every other
zone centroid. For the four nodes above, the minimal fix is to add
cross-chain links so the graph is strongly connected:

```
Node 1 (zone 1) <---> Node 2 (zone 2)
     |                      |
     v                      v
Node 4 (zone 3) <---> Node 3 (zone 4)
```

See the `simple_network` example for a properly built 4-zone network.
