# P3 — direct-Ir graph navigation & search performance

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P3 ·
**APPROVED-GRAPH-EDIT:** direct-ir-navigation-benchmark · 2026-09-03. Repo `hymeko/graph`,
branch `feat/graph-steiner-mycielski-navigation-bench` off main `b234c487`. Not merged; no
core/graph main change; no Python touched; PGraph untouched. All task state under
`D:\h\graph-search-p3` (never C:).

## Scope statement (honest, up front)
This is a large research study. This phase delivers a **verified foundation**: the missing
workloads (Steiner / Mycielski / seeded-random) driving the *existing* direct-Ir engine,
formula- and oracle-verified correctness, a focused cold/warm measurement matrix, and
fault-injection at the workload↔oracle boundary. Explicitly **NOT done** (reported as such,
not claimed): the full 1/2/4/8/16-thread Rayon scaling sweep; the map/routing DTO-free
coordinate workload family (§4.3); T-norm/S-norm AND/OR benchmarking (§5.2); a
discrete/combined A* heuristic (§5.1 — would edit CORE.YAML-protected `hymeko_graph`, gated);
SVG figures; and any production optimization (§10 — none cleared the evidence gate; the
engine is already at the Θ(nnz) floor for the measured operations). `cargo llvm-cov` is not
installed → coverage **UNMEASURED**.

## §1 Entry audit
- graph main `b234c487`; core pinned `branch=main` → **`70b4f364`** (Cargo.lock). `cargo tree
  -d`: one `hymeko_core` source (the repeated lines are the same rev reached via workspace
  members, not a version split), one `parser`.
- Feature branches present, **not merged/promoted**: `feat/graph-direct-ir-algorithms`
  (98c6a93), `feat/direct-ir-informed-berge-search` (d4086a0),
  `feat/graph-query-performance-matrix` (e710e84), `feat/pgraph-direct-ir-and-or-algebra`
  (9655f07), `feat/pgraph-adjacency-delegation`.
- **On main already** (reused, not re-implemented — this is the key finding): the whole
  direct-Ir engine — `reachable_nodes` (BFS), `berge_shortest_path`, `connected_components`,
  `s_connected_components`, `dual_reachability`, `batch_reachable_counts` +
  `par_batch_reachable_counts` (Rayon), `dijkstra_berge_path`, `astar_berge_path` (requires a
  **consistent** heuristic in optimal mode; refuses otherwise); costs `UnitCost` /
  `IncidenceWeightCost` / `MetricStepCost`; heuristics `ZeroHeuristic` (=Dijkstra) /
  `MetricHeuristic`; policies `Undirected` / `Directed*` / `SignCompatible`; reusable
  `TraversalWorkspace` / `InformedSearchWorkspace`. Test-support `informed_common`:
  `median_us`, `oracle_dijkstra_cost` (linear-scan), `exhaustive_min_cost`, Ir builders.
  Frozen oracles in `informed_oracles.rs`; engine-internal fault injection in
  `informed_search_vv.rs` (14 tests). So P3's genuine gap was **workloads + measurement**, not
  algorithms.

## §3 Reuse — no parallel framework
Generators were added to the existing `informed_common` test-support (one module, no second
harness), emitting canonical `hymeko_core::ir::Ir` via the same `push_decl` / `arcs.push`
pattern as `window_ir`/`grid_ir`. No second graph engine, no map DTO, no CLI, no dense/pairwise
projection on the navigation path.

## §4 Workloads (formula-verified)
- **Steiner STS(2^m−1) = PG(m−1,2)** (`steiner_sts_ir`): points are nonzero m-bit vectors,
  blocks `{a,b,c}` with `a⊕b⊕c=0` emitted once (via `c=a⊕b>b`). Rank-3 hyperarcs.
- **Iterated Mycielski** (`mycielski_ir`) from **C5** (triangle-free, χ=3): rank-2 hyperarcs,
  `μ(G)` on `V∪U∪{w}`; `n′=2n+1`, `e′=3e+n`. `μ(C5)` = Grötzsch graph.
- **Seeded random** (`uniform_rank_k_ir`, local LCG): uniform rank-k, deterministic per seed.
- **Disconnected control** (`two_disjoint_sts_ir`): two disjoint STS components.

Verified **independently of the emitted edge list** (`steiner_mycielski_nav.rs`, 7 tests):
STS — `v=2^m−1`, blocks `v(v−1)/6`, incidences `v(v−1)/2`, every block size 3, **every
unordered pair in exactly one block**, uniform degree `(v−1)/2` (m=3..7). Mycielski —
recurrence, no self-loop, no duplicate edge, **triangle-free preserved** (iters 0..3). Random
— same seed ⇒ byte-identical arc membership; incidences `= m·k`.

## §6 Correctness vs independent oracles
- STS(31): engine `reachable_nodes` == the O(V·E) `oracle_reachable` scan (and reaches all V,
  STS is connected); `dijkstra_berge_path` cost == `oracle_dijkstra_cost` for several pairs.
- Mycielski μ(C5)²: `astar_berge_path(ZeroHeuristic)` cost == `dijkstra_berge_path` ==
  oracle Dijkstra (optimal-cost parity, structurally valid path).
- Disconnected: cross-component target → typed `None`; within-component → `Some`.
- Engine-internal invariants (reverse incidence, drop slot, visited-clear, A* overestimate,
  serial/Rayon parity, sign/weight/owner preservation) are covered by the pre-existing
  `informed_search_vv.rs` (14 tests, all green here).

### Fault injection (this phase; none committed; restored + hash-verified clean)
| mutant | caught by |
|---|---|
| STS block drops a slot (size-2 block) | `sts_counts_match_formula_independent_of_edge_list` |
| Mycielski recurrence `n′=2n+1 → 2n` (apex dropped) | `mycielski_recurrence_and_properties` |
| engine `UnitCost` step 1.0 → 2.0 (overcount) | `steiner_navigation_matches_oracles` |

(Engine-internal mutants beyond the workload/oracle boundary are the domain of
`informed_search_vv.rs`, which already performs that fault injection per its module docs.)

## §7 Measurement (release; medians of 9; `reports/data/2026-09-03-navigation-measurement.tsv`)
Cold `idx_us` = `build_incidence_index` (the Θ(nnz) sparse-index preparation). Warm
`reach_us`/`dijkstra_us` = a query on the prepared index, **including** per-query workspace
allocation (the engine supports reuse via `TraversalWorkspace`/`InformedSearchWorkspace`; the
amortised repeated-query cost is lower and is a follow-up cell). `B/inc` = `retained_bytes() /
nnz`.

| workload | V | E | nnz | idx µs | reach µs | dijkstra µs | bytes/inc |
|---|--:|--:|--:|--:|--:|--:|--:|
| STS(2³−1) | 7 | 7 | 21 | 0.20 | 0.40 | 0.50 | 13.5 |
| STS(2⁵−1) | 31 | 155 | 465 | 0.90 | 1.50 | 3.30 | 12.3 |
| STS(2⁷−1) | 127 | 2667 | 8001 | 59.0 | 13.6 | 61.9 | 12.1 |
| Mycielski² | 23 | 71 | 142 | 0.40 | 0.80 | 1.60 | 12.7 |
| Mycielski⁴ | 95 | 755 | 1510 | 3.50 | 3.80 | 12.2 | 12.3 |
| Mycielski⁶ | 383 | 7271 | 14542 | 38.3 | 32.7 | 171.0 | 12.1 |
| rand3(1000) | 1000 | 2000 | 6000 | 19.4 | 31.0 | 111.0 | 12.7 |
| rand3(30000) | 30000 | 90000 | 270000 | 3803 | 6560 | 17720 | 12.4 |

**Measured facts:** retained memory is **~12 bytes/incidence**, stable across all three
topology families (an `(arc, slot)` pair per incidence in the CSR-like `IncidenceIndex`).
Index build and BFS reachability are **Θ(nnz)** (index build ≈ 14 ns/incidence at the
270k-incidence cell). Dijkstra grows nnz·log(frontier). Families are **not averaged** into a
headline — STS, Mycielski and random are reported separately.
**Complexity honesty:** creating a borrowed `HyperTraversal` view is O(1); applying B / Bᵀ /
BᵀB (`node_to_arc` / `arc_to_node` / `star_operator`) is Θ(nnz) — none is called O(1).
**Inference (not yet measured):** the per-query numbers include workspace allocation; a
reuse-amortised warm cell and the materialised-control break-even (§9) are the next
measurements.

## §9 Controls (direct-Ir vs materialised) — NOT measured this phase
The break-even (`prepare + repeated queries + retained memory`) vs a materialised
pairwise/clique or sparse-star CSR control was **not** measured here; it is the top follow-up.
The direct-Ir path is retained as production regardless (CORE.YAML): materialised forms may
exist only as controls. No claim is made that direct-Ir search is faster than a materialised
inner loop — only that its preparation and memory (~12 B/inc, Θ(nnz) build) are measured.

## §10 Optimization — none (evidence gate not cleared)
No production optimization shipped. The measured operations sit at the Θ(nnz) floor with a
compact 12-B/inc index and reusable scratch already present; nothing showed a measured
bottleneck clearing the §10 gate (differential test + memory accounting + material gain + no
>3% regression). A negative result is acceptable per §10.

## §11 Gates
`cargo fmt --check` clean; `cargo clippy -p hymeko_graph --all-targets --all-features -D
warnings` clean; build ok; `informed_oracles` (3) + `informed_search_vv` (14) +
`steiner_mycielski_nav` (7) all pass. Coverage **UNMEASURED** (`cargo-llvm-cov` absent; not
installed; not inferred from test count).

## §16 Files changed
- `hymeko_graph/tests/informed_common/mod.rs` (+generators: `steiner_sts_ir`,
  `sts_expected_counts`, `mycielski_ir`, `mycielski_expected_counts`, `base_c5`,
  `uniform_rank_k_ir`, `two_disjoint_sts_ir`, `Lcg`).
- `hymeko_graph/tests/steiner_mycielski_nav.rs` (new; 7 tests + measurement).
- `docs/plans/2026-09-03-graph-navigation-performance/{plan.md,plan.tex,plan.mmd}`.
- `reports/2026-09-03-graph-navigation-performance.md`, `docs/graph-navigation-performance.md`,
  `reports/data/2026-09-03-navigation-measurement.tsv`.
No `hymeko_graph`/`hymeko_pgraph` **source** (CORE.YAML) changed; edits are tests/docs/data.

## §18 Recommended next minimal phase
One focused measurement phase: (a) reuse-amortised warm queries (thread the reusable
workspace through a repeated-query loop) and batch 1/16/256; (b) the direct-Ir-vs-materialised
break-even (a pairwise/clique control built once, its full prepare + retained memory reported);
(c) Rayon 1/2/4/8/16 scaling on `par_batch_reachable_counts` over STS(511)/Mycielski⁸ in
isolated subprocesses. No new algorithm needed; all reuse the existing engine.
