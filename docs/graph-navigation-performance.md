# Direct-Ir graph navigation & search performance

The canonical HyMeKo graph search path is: **HyMeKo source → parser/lowering →
`hymeko_core::ir::Ir` → `build_incidence_index` (sparse `IncidenceIndex`) → direct Berge
navigation/search** (`hymeko_graph::direct_ir`). No dense graph, clique expansion, pairwise
adjacency, sparse-star CSR, or second semantic model is built on the production path; such
forms may exist only as benchmark controls/oracles.

## Complexity
- Creating a borrowed `HyperTraversal` / operator view: **O(1)**.
- Applying B / Bᵀ / BᵀB (`node_to_arc` / `arc_to_node` / `star_operator`), and BFS/Dijkstra:
  **Θ(nnz)** (a projection application is never O(1)).
- Retained index memory: **~12 bytes per incidence** (an `(arc, slot)` pair), measured stable
  across Steiner, Mycielski and random families.

## Workloads (reusable test/bench generators, `tests/informed_common/mod.rs`)
| family | generator | formula (verified independently) |
|---|---|---|
| Steiner STS(2^m−1) = PG(m−1,2) | `steiner_sts_ir(m)` | v=2^m−1; blocks v(v−1)/6; incidences v(v−1)/2; block size 3; every pair in exactly one block; degree (v−1)/2 |
| Iterated Mycielski (from C5) | `mycielski_ir(5, C5, iters)` | n′=2n+1; e′=3e+n; rank-2; triangle-free; no self-loop/dup |
| Seeded uniform rank-k | `uniform_rank_k_ir(seed, n, m, k)` | incidences = m·k; deterministic per seed |
| Disconnected control | `two_disjoint_sts_ir(m)` | cross-component pairs unreachable |

## Manifest (seeds / representative sizes)
- STS: m ∈ {3,5,7} → v ∈ {7,31,127}. (STS(511)/STS(1023) fit the budget for a later cell.)
- Mycielski: iters ∈ {2,4,6} → V ∈ {23,95,383}. Base C5 (triangle-free, χ=3).
- Random: `uniform_rank_k_ir(seed=0x51EED, n, 3n, 3)` at n ∈ {1000, 30000}; determinism test
  uses seed `0xC0FFEE`, n=500, m=800, k=3.

## Running
```
cargo test -p hymeko_graph --test steiner_mycielski_nav                       # correctness (7)
cargo test -p hymeko_graph --test steiner_mycielski_nav --release \
    measure_navigation_matrix -- --nocapture                                  # measurement table
cargo test -p hymeko_graph --test informed_search_vv --test informed_oracles  # engine V&V + oracles
```
Raw data: `reports/data/2026-09-03-navigation-measurement.tsv`. Full analysis + scope:
`reports/2026-09-03-graph-navigation-performance.md`.

## P3.1 — map routing + admissible heuristics

Map workloads carry **coordinates and weights in the canonical `Ir`** (`DeclNode.anno.value =
[x,y]`; incidence weights) — no parallel map object. Generators (`tests/informed_common`):
`map_grid_ir` (weighted grid, arterial 1.0 / local 1.5 ≥ euclidean, deterministic obstacles),
`map_grid_with_transport_ir` (one multiway metro hyperedge), `hop_distances_from` (discrete-
heuristic prep). Heuristics are **test-side** `BergeHeuristic` adapters (no core edit):
`IrPositions` (metric), `DiscreteHeuristic` (min remaining Berge hops × global min cost),
`Combined = max(metric, discrete)`.

Proven (`tests/map_routing_nav.rs`): non-negativity, `h(goal)=0`, admissibility `h ≤ oracle
dist`, consistency `h(u) ≤ cost(u,v)+h(v)` (exhaustive on 16×16), A\* optimal-cost parity with
Dijkstra/oracle for all three heuristics, blocked→typed `None`, multiway-transport discrete
optimality. **Measured:** on uniform grids with far-corner queries, Dijkstra beats all three
A\* heuristics (weak euclidean grid lower bound + prep/overhead; discrete prep never breaks
even) — all return the correct optimal cost. Data:
`reports/data/2026-09-03-map-routing-measurement.tsv`; full analysis + deferred items:
`reports/2026-09-03-graph-navigation-p31.md`.

## P3.3 — zero-copy rank-2 graphic matroid

A **matroid view over the same canonical objects**, not a new model. `hymeko_graph::matroid`
adds a borrowed `MatroidRef<'a,'ir>{ ir, incidence }` (two words, `Copy`, O(1)/0-alloc — proven
in `tests/matroid_facade.rs`) and a caller-owned `GraphicMatroidWorkspace` union-find (path
halving + union by rank + generation-stamped lazy reset). Ground elements are canonical
`HyperArcId`s that resolve to **exactly two** node incidences; a multiway hyperarc is a typed
`MatroidError::WrongCardinality`, never silently reinterpreted as a pairwise edge. No second ID
space, no materialised Levi/star, no serialized matroid. Full design:
`docs/matroid-over-canonical-ir.md`.

Operations (all Θ(ground set), never O(1) — the *facade* is O(1), the *computation* is not):
`rank2_spanning_forest` / `rank2_rank` (rank = **N − C**), `rank2_weighted_basis`
(min/max = the greedy = Kruskal), and an append-only `IncrementalRank2` cycle stream.

Verified (`tests/matroid_vv.rs`, 14 tests): the rank formula triangulated three ways
(matroid == `N − connected_components().count()` == an independent from-scratch union-find);
the independence axioms (I1/I2/I3) **exhaustively** on a 7-edge cyclic grid, with the matroid's
notion of independence proven equal to acyclicity on every subset; greedy == an independent
Kruskal; incremental == full rebuild; workspace-reuse determinism across interleaved graphs;
arc-permutation invariance of rank and optimum weight; correctness under node-DeclId ≠ NodeId;
and the four typed rejections. 9 source mutants injected and each caught by a named test
(restored bit-for-bit). **Measured** (`reports/data/2026-09-03-p33-matroid-latency.tsv`):
spanning forest Θ(A·α) (~9 ns/arc on a path, ~26 ns/arc on a grid — cache-scattered node
access); weighted basis 3–7× that (Kruskal sort); DSU workspace **12 bytes/node** (3×u32),
retained at high-water across reuse. Full analysis + PARTIAL scope:
`reports/2026-09-03-zero-copy-matroid-performance.md`.
