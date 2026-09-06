# Plan — P3.1 complete navigation/routing/CPU/memory measurements

**Created-at:** 2026-09-03 02:12 +02:00 · **ROADMAP-TASK:** P3.1 · **APPROVED-GRAPH-EDIT:**
direct-ir-map-routing-and-admissible-heuristics · Repo `hymeko/graph`, branch
`feat/graph-navigation-performance-p31` off `cc5b895`.

**Process note (honest):** for this focused increment the plan was written alongside the first
implementation rather than strictly before it — a deviation from the plan-first rule. The scope
decision below was made explicitly and up front.

## Reuse-first
The direct-Ir engine (reachability, components, Dijkstra, A* with consistent-heuristic
optimality, reusable workspaces, Rayon batch) and the P3 workloads/oracles are on graph main +
the P3 branch. P3.1 adds ONLY: a map/routing workload emitting canonical Ir (coords in
`DeclNode.anno.value`, weights on incidences — no parallel DTO), and **test-side** admissible
heuristics implementing the public `BergeHeuristic` trait (no engine/core edit). No second
engine, no CLI, no dense/pairwise projection on the production path.

## Delivered (verified)
- Map workload: weighted grid (arterial 1.0 / local 1.5, all ≥ euclidean → metric admissible),
  deterministic obstacles (isolated regions), one multiway transport hyperedge; coords read
  back from the Ir.
- Discrete heuristic (min remaining Berge hops × global min transition cost; prep = one BFS
  from the goal, measured separately) + Combined = max(metric, discrete).
- Proofs (tests): non-negativity, h(goal)=0, admissibility (h ≤ oracle dist) and consistency
  (h(u) ≤ cost(u,v)+h(v)) exhaustive on a 16×16 map; A*(metric/discrete/combined) optimal-cost
  parity with Dijkstra and the oracle; valid alternating steps; blocked→typed None;
  multiway-transport discrete optimality.
- Warm A* measurement with a reused workspace + discrete-prep timing.
- Fault injection (none committed; each caught by a named test; restored + hash-verified):
  overestimating heuristic, combined-by-sum-not-max, ignored obstacle.

## Deferred — reported as NOT done, with reasons (§1/§17)
Full 1/2/4/8/16-thread isolated-subprocess CPU scaling with process-CPU-time; T-norm/S-norm
AND/OR sweep (reuse `hymeko_pgraph::andor`; deferred); the materialised-star/clique control
break-even table (§10); SVG figures (§14); per-cell peak-RSS in fresh subprocesses (§9);
STS(1023) (predict-and-skip). Rationale: session budget; each is additive measurement over the
existing engine and is the recommended next phase. No optimization shipped (§13 gate not
cleared; the honest measured finding is that on uniform grids with far-corner queries plain
Dijkstra beats all three A* heuristics — euclidean is a weak grid lower bound and prep+overhead
exceed pruning).

## Gates / commits / stop
fmt · clippy `-D warnings` · build · tests. Commits: plan; map+correctness; (measurement is in
the test file); report+data. Push only the feature branch; no main merge; no core/PGraph edit.
All task state under `D:\h\graph-search-p31` (never C:). plan.pdf unavailable (no TeX).
