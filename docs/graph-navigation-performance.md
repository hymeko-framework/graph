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
