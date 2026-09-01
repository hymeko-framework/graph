# Heuristic inventory — direct-Ir informed Berge search

Created-at 2026-09-01 20:25 CEST · P1.1. Every heuristic is classified per mission §6:
**C1** admissible AND consistent (with proof) · **C2** admissible, consistency not proven · **C3**
tie-break only · **C4** experimental/inadmissible (explicit non-optimal mode only). Cost model assumed:
non-negative separable (`c = w_arc + w_entry ≥ 0`) unless a heuristic names its coupled cost.

Notation: `h(n)` estimate at node `n`; `h*(n)` true optimal remaining cost `n→goal`; consistency =
`h(n) ≤ c(n→n') + h(n')` for every legal step, and `h(goal)=0`.

## Implemented this phase

| # | heuristic | class | proof / justification |
|---|---|:--:|---|
| H0 | **Zero** `h≡0` | **C1** | `0 ≤ h*` trivially (costs ≥ 0); `0 ≤ c + 0` since `c ≥ 0`; `h(goal)=0`. Reduces A* to Dijkstra. The reference optimal engine. |
| H1 | **Metric (Euclidean) lower bound**, coupled with `MetricStepCost` `c(u,v)=‖pos(u)−pos(v)‖` | **C1** | With metric step cost, `h(n)=‖pos(n)−pos(goal)‖`. Admissible: any path `n⇝goal` has total length `≥ ‖pos(n)−pos(goal)‖` (triangle inequality over the polyline). Consistent: `h(n)=‖pos(n)−pos(goal)‖ ≤ ‖pos(n)−pos(v)‖ + ‖pos(v)−pos(goal)‖ = c(n→v)+h(v)`. Positions come from a `PositionProvider` keyed by **canonical DeclId** (never a second IR state); a node without a position ⇒ `h=0` (still admissible). Under a **non-metric** cost (unit/weight), Euclidean distance is *not* a lower bound on hop/weight count ⇒ demote to **C3** tie-break (see H1'). |
| H1' | Metric distance as **tie-break** under non-metric cost | **C3** | Not a bound on remaining cost; used only as `discrete_tiebreak` to steer toward the goal geometrically. Cannot affect optimality. |
| H2 | **Component-membership prune** | (pre-check, C1-safe) | If `goal ∉ reachable_nodes(start)` (or a precomputed `connected_components` label differs) ⇒ return `Ok(None)` immediately. Not an `h` value; an admissible early-exit (never prunes a real path: unreachable stays unreachable under any policy at least as strict as undirected — for stricter directed policies it is a *sound* over-approximation only for the undirected pre-check, so used as a fast reject **only** in undirected mode; in directed mode it is advisory). |

## Designed + classified, NOT implemented this phase (reasons stated)

| # | heuristic | class | why / bound status |
|---|---|:--:|---|
| H3 | **Berge-hop lower bound** (unit cost): `h(n)=` reverse-BFS hop distance `n→goal` | **C1** for unit cost | Exact `h*` under unit cost (it *is* the remaining distance) ⇒ admissible+consistent, but it is a full reverse BFS per query = as expensive as the search; useful only as a **landmark precompute** (H5). Deferred: precompute cost accounting belongs to the benchmark's break-even analysis, not a hot heuristic. |
| H4 | **Goal reverse-hop bound** (weighted): reverse Dijkstra `min` incidence cost | **C1** | Exact reverse distance = perfect heuristic but equals solving the problem backwards; only sensible as a shared landmark (H5). |
| H5 | **Landmark / ALT** lower bound on the implicit incidence state space | **C1** (per landmark, by triangle ineq) | `h(n)=max_L |d(L,goal) − d(L,n)|` over landmarks `L`. Admissible+consistent by the metric-space triangle inequality *if* the cost is a metric (symmetric). For directed/asymmetric costs, ALT needs forward+backward landmark distances (Goldberg–Harrelson). Deferred: needs a landmark-selection + precompute pass and a symmetric-cost proof; classified, not built. |
| H6 | **Dual-hypergraph distance** (arc-space) | **C2/C3** | Distance in the dual (arc adjacency, via `dual_reachability`) lower-bounds the *number of shared-node transitions*, not directly the node cost; admissible only under a proven mapping. Tie-break by default (C3). |
| H7 | **Sign-compatible reachability** filter | (policy, not `h`) | A reachability restriction (belongs to the policy layer §4), not a cost bound. |
| H8 | **s-connectivity** (`s_connected_components`, s≥2) | **C3** | s-connectivity (sharing ≥ s nodes) is a robustness/clustering property; it does **not** lower-bound path cost. Tie-break / structural filter only. Mission §6.2 explicitly warns it is not automatically admissible. |
| H9 | **hyperedge cardinality / arity** | **C3** | Arity is a local structural quantity, not a remaining-cost bound. Tie-break only (§6.2 warning). |
| H10 | **node / hyperarc degree** | **C3** | Degree steers toward hubs but is not a bound. Tie-break only (§6.2 warning). |
| H11 | **cut / cut-cardinality** | **C3** | A cut is a global separator property; not a per-node remaining-cost bound without a max-flow/min-cut proof. Tie-break/analysis only (§6.2 warning). |
| H12 | **incidence sign distribution** | **C3** | Descriptive statistic; no cost bound. Tie-break. |
| H13 | **minimum admissible remaining edge/incidence cost** `h(n) = (#hops_lb) · min_arc_cost` | **C1** for unit-ish costs | If every remaining step costs ≥ `c_min` and at least `d_lb` hops remain, `h = d_lb·c_min ≤ h*`. Needs a cheap hop lower bound `d_lb` (e.g. 1 if `n≠goal`, else 0 — a weak but valid `h = c_min`). The weak form (`h=c_min` when `n≠goal`) is **C1** and cheap; implemented as an optional refinement if time permits, else classified. |
| H14 | **P-graph raw/product backward reachability relaxation** | **C2** (design) | A P-graph-specific lower bound: relax the AND-closure to OR-reachability from the product set backward; this bounds *producibility depth*, not Berge cost. Belongs to the pgraph domain layer as a bound feeding ABB (audit: could feed `abb.rs:295`), **not** a general Berge heuristic. Design-only; see `pgraph-as-graph-extension.md`. |

## Combination rule (§6.2)
Multiple admissible lower bounds combine by **`max(h1, h2, …)`** by default (the max of admissible bounds
is admissible and, if each is consistent, consistent). **Summation is only permitted with a proof of
disjoint additivity** (the bounds account for disjoint portions of the remaining path) — none is claimed
this phase. Learned/empirical scores are **C3 tie-break** or an explicit bounded-suboptimal mode only
(not built).

## What the implementation ships
- H0 (Zero, = Dijkstra) and H1 (metric, coupled cost) as the two **optimal-mode C1** heuristics, each with
  the proof above and a test that `astar(h).cost == dijkstra.cost`.
- H2 component pre-check (undirected fast reject).
- H1' metric tie-break demonstrated under non-metric cost (C3) — shows the C1/C3 boundary is enforced
  (a C3 quantity never enters `f`, only the tie-break slot).
- The remaining rows are **classified and justified here**; building H5/H13/H14 is scheduled work, not a
  silent omission. The engine's heuristic trait makes each a drop-in `impl` with its declared
  `ADMISSIBLE`/`CONSISTENT` consts, so optimal mode mechanically refuses a non-C1 heuristic (§7).
