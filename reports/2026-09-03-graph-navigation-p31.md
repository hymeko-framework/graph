# P3.1 — map routing + admissible heuristics on the direct-Ir path

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P3.1 ·
**APPROVED-GRAPH-EDIT:** direct-ir-map-routing-and-admissible-heuristics · 2026-09-03. Repo
`hymeko/graph`, branch `feat/graph-navigation-performance-p31` off `cc5b895` (the P3 branch).
Not merged; no core/PGraph/Python/Nagare edit; all task state under `D:\h\graph-search-p31`.

## Scope statement (honest)
P3.1 asked to close ~11 measurement gaps. This session delivers the **verified core**: the
map/routing workload, the discrete + combined admissible heuristics with full
admissibility/consistency/optimality proofs, warm-query measurement with a reused workspace,
and fault injection. Explicitly **NOT done this session** (deferred to the next phase, with
reasons — session budget; each is additive measurement over the existing engine, not new
capability): full 1/2/4/8/16-thread isolated-subprocess CPU scaling with process-CPU-time; the
T-norm/S-norm AND/OR sweep (reuse `hymeko_pgraph::andor`); the materialised-star/clique control
break-even table (§10); SVG figures (§14); per-cell peak-RSS in fresh subprocesses (§9);
STS(1023). **Process deviation:** the plan bundle was written alongside the first
implementation rather than strictly before it — flagged, not hidden.

## §17.1 Branch / SHAs / verification
Branch `feat/graph-navigation-performance-p31` @ **`797d769`** off `cc5b895`; commits:
`d7f0362` (map workload + heuristics + tests), `797d769` (plan + data), report (this commit).
Graph main unchanged `b234c487`. Remote-verified in the return message.

## §17.2 Workloads (added; formula/structure verified)
Map coordinates + traversal weights live **in the canonical `Ir`**: node `(r,c)` carries its
`[x,y]` in `DeclNode.anno.value`; street segments are arity-2 arcs with an incidence weight;
one **multiway transport hyperarc** (arity-4 metro) carries a flat fare. No parallel map
object. `map_grid_ir` (arterial 1.0 / local 1.5, **every weight ≥ euclidean** so the metric
heuristic is admissible), `map_obstacles` (deterministic, leaves a connected detour),
`map_grid_with_transport_ir`. (P3's Steiner/Mycielski/random are retained and still green.)

## §17.3 / §17.4 Warm-query table (release medians; reused workspace, post-index)
| map | V | E | nnz | Dijkstra µs | A\*(metric) | A\*(discrete) | A\*(combined) | discrete prep µs |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| grid32×32 | 1024 | 1888 | 3776 | 65.5 | 82.5 | 98.7 | 100.6 | 29.7 |
| grid128×128 | 16384 | 32108 | 64216 | 1806 | 2108 | 2705 | 2656 | 849.5 |

**Measured finding (honest, discriminating):** on a **uniform** weighted grid with a
**far-corner** query, plain **Dijkstra is fastest**; A\*(metric) is ~1.2× slower and
A\*(discrete/combined) ~1.5× slower. Euclidean is a *weak* lower bound on a grid metric (it
underestimates the actual grid path badly), so A\* prunes little while paying per-expansion
heuristic overhead; and the discrete heuristic adds a per-goal BFS prep (30 µs / 850 µs) that,
combined with a slower query, **never breaks even** for this query pattern. A\* wins where the
heuristic is a *tight* lower bound and the goal sits behind a focused corridor — not a uniform
grid corner query. All three heuristics return the **correct optimal cost**; the difference is
purely time.

## §17.5 / §17.6 Map Dijkstra/A\* + admissibility & optimality proofs
`map_routing_nav.rs` (5 tests, all green):
- **Admissibility** — `h_metric(n) ≤ oracle_dist(n,goal)` and `h_discrete(n) ≤ oracle_dist(n,goal)`
  for **every** node of a 16×16 map (exhaustive vs the frozen linear-scan Dijkstra oracle);
  `h(goal)=0`; `h ≥ 0`.
- **Consistency** — `h(u) ≤ cost(u,v) + h(v)` for **every** legal Berge step of the map, for
  both metric and discrete.
- **Optimality** — `A*(metric)`, `A*(discrete)`, `A*(combined=max)` each return cost **==
  Dijkstra == oracle** across a grid of (start,goal) pairs; returned path has valid alternating
  steps (`nodes = hyperedges + 1`).
- **Unreachable** — a walled-off corner returns typed `None`.
- **Multiway transport** — with a metro hyperedge whose fare is below the station-pair euclidean
  distance (so metric is *inadmissible* there), the **discrete** heuristic stays
  admissible+optimal (== oracle). Combined uses `max`, never a sum (proven by fault injection).

## §17.7 Work counters
Not yet surfaced per-row (the engine returns `total_cost` / `hyperedge_len`; expanded-state /
pq-push counters are an engine-instrumentation follow-up). Reported as a gap, not claimed.

## §17.8 T-norm/S-norm — deferred
`hymeko_pgraph::andor` (P1.2) is on main and is the reuse target; the AND/OR / T/S-norm
benchmark (BooleanExact + each norm, chain/fan-out/fan-in/mixed, cold/warm/convergence/parity)
was **not** run this session. It is a separate operation from A\* and must not be mixed into
`g+h` (respected — no fuzzy score enters the heuristic).

## §17.9 Rayon CPU scaling — deferred
`par_batch_reachable_counts` exists; the isolated-subprocess 1/2/4/8/16-thread sweep with
process-CPU-time, active-cores, efficiency and incidences/s was **not** run. Reported as a gap.

## §17.10 / §17.11 Memory / bytes-per-entity
P3 established retained **~12 bytes/incidence** (structural, `IncidenceIndex::retained_bytes`).
Per-cell process peak-RSS in fresh subprocesses (§9) and bytes-per-canonical-entity were **not**
re-measured this session — deferred with the CPU-scaling phase.

## §17.12 Direct-Ir vs materialised controls — deferred
The star/clique break-even table (`prepare + N×query + retained memory`) was **not** built this
session. Direct-Ir stays production (CORE.YAML); controls may exist only in bench support. Top
follow-up.

## §17.13 Tests / coverage
`map_routing_nav` (5) + `steiner_mycielski_nav` (7) + `informed_search_vv` (14) +
`informed_oracles` (3) all green. `cargo fmt --check` + `clippy -p hymeko_graph --all-targets
--all-features -D warnings` clean (fixed a new `manual_is_multiple_of` lint in the map
generator). Coverage **UNMEASURED** (`cargo-llvm-cov` absent; not installed; not inferred).

## §17.14 Fault injection (this phase; none committed; restored + hash-verified clean)
| mutant | caught by |
|---|---|
| discrete heuristic overestimates (×2) | `metric_and_discrete_admissible_and_consistent` |
| combined heuristic by **sum** instead of **max** | `astar_metric_discrete_combined_all_optimal` |
| generator ignores obstacles | `blocked_region_is_unreachable` |

(Plus P3's 3 generator/engine mutants; and the engine's own `informed_search_vv` fault
injection.)

## §17.15 Optimization — none
No production optimization shipped. The §13 gate was not cleared: the measured finding is that
the heuristics do not accelerate the tested grid queries (weak lower bound + prep/overhead), so
the useful next step is a *tighter* heuristic or a *focused-corridor* workload, not an engine
change. A no-optimization result is acceptable per §13.

## §17.16 Own errors / retractions / unresolved
- A new clippy lint (`manual_is_multiple_of`) failed the gate on my `% n == 0` map code; fixed
  to `.is_multiple_of()` (it also affects any pre-existing `% == 0`, but only my new code
  tripped it here).
- **Unresolved (honest):** whether A\* ever beats Dijkstra on *this* map family is unproven —
  the corner-query result is negative; a focused-corridor / tight-heuristic workload is needed
  to show a positive A\* case, and the work-counter instrumentation to attribute it. Deferred.

## §17.17 Retained under `D:\h\graph-search-p31`
`checkout`, `target` (build cache), `tmp`, `artifacts`. Not deleted (awaiting authorization).

## §17.18 Files changed
`hymeko_graph/tests/informed_common/mod.rs` (+map generators, coord/hop/weight helpers),
`hymeko_graph/tests/map_routing_nav.rs` (new; 5 tests + measurement),
`docs/plans/2026-09-03-graph-navigation-p31/{plan.md,plan.tex,plan.mmd}`,
`reports/2026-09-03-graph-navigation-p31.md`, `reports/data/2026-09-03-map-routing-measurement.tsv`,
`docs/graph-navigation-performance.md` (appended). **No `hymeko_graph`/`hymeko_pgraph` source
(CORE.YAML) changed** — all edits are tests/docs/data.

## Recommended next minimal phase
One measurement phase, no new algorithm: (a) engine work-counters (expanded states, pq
pushes/pops, heuristic evals) surfaced on the search result; (b) the direct-Ir-vs-star/clique
break-even table; (c) Rayon 1/2/4/8/16 scaling + per-cell peak-RSS in isolated subprocesses;
(d) the T/S-norm AND/OR sweep over `hymeko_pgraph::andor`; (e) a focused-corridor map to show a
positive A\* case; (f) SVG figures from a committed script.
