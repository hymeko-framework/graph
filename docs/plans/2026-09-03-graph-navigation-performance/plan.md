# Plan — direct-Ir graph navigation & search performance (P3)

**Created-at:** 2026-09-03 01:37 +02:00 · **STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 ·
**ROADMAP-TASK:** P3 · **APPROVED-GRAPH-EDIT:** direct-ir-navigation-benchmark ·
**Repo:** `hymeko/graph`, branch `feat/graph-steiner-mycielski-navigation-bench` off main
`b234c487`. **ETA basis:** measurement study; the engine already exists, so the work is
workloads + verified measurement, not new algorithms.

## Entry audit (§1, live)
- graph main `b234c487`; core pinned `branch=main` resolved to **`70b4f364`** (Cargo.lock);
  single `hymeko_core`, single `parser`. `cargo tree -d`: to confirm exactly one core.
- Feature branches (NOT merged, NOT promoted): `feat/graph-direct-ir-algorithms` (98c6a93),
  `feat/direct-ir-informed-berge-search` (d4086a0), `feat/graph-query-performance-matrix`
  (e710e84), `feat/pgraph-direct-ir-and-or-algebra` (9655f07), `feat/pgraph-adjacency-delegation`.
- **On main already** (reuse, do not re-implement): `direct_ir::{reachable_nodes,
  berge_shortest_path, connected_components, s_connected_components, dual_reachability,
  batch_reachable_counts, par_batch_reachable_counts, astar_berge_path}`; costs `UnitCost` /
  `IncidenceWeightCost` / `MetricStepCost`; heuristics `ZeroHeuristic` (=Dijkstra) /
  `MetricHeuristic`; `TransitionPolicy` (Undirected / Directed* / SignCompatible);
  `TraversalWorkspace` (reusable scratch). `build_incidence_index(&Ir)` in core.
- Test-support already present (`hymeko_graph/tests/informed_common/mod.rs`): `median_us`,
  `oracle_dijkstra_cost`, `exhaustive_min_cost`, and Ir builders (`window_ir`, `grid_ir`,
  `multi_component_ir`, `dup_heavy_ir`, positions). Oracles frozen in `informed_oracles.rs`.

## Genuine gap this phase → deliver
1. **Workloads** (§4), net-new, non-core, reusable test-support: Steiner STS(2^m−1) via
   PG(m−1,2) (triples {a,b,c} with a⊕b⊕c=0); iterated Mycielski (rank-2 hypergraph,
   n′=2n+1, e′=3e+n); seeded random controls (uniform rank-k, hub-skewed, disconnected). A
   small committed `.hymeko` map fixture + a deterministic generated grid map (positions as
   Ir values). All emit **canonical `Ir`** (reusing `push_decl`/`arcs.push` — the `window_ir`
   pattern), no dense graph, no pairwise projection, no second model.
2. **Correctness** (§6): formula counts (v, blocks, incidences, degree, pair-coverage for
   STS; recurrences + triangle-free for Mycielski) verified independently of the emitted edge
   list; navigation (reachability, shortest path, Dijkstra, A*) checked vs the existing
   `oracle_dijkstra_cost` / exhaustive / a linear O(V·E) incidence scan. Metamorphic
   (permute/rename/scale/unreachable) + fault mutants (none committed), each caught by a named
   test, restored + hash-verified.
3. **Measurement** (§7/§8): cold (Ir build → `build_incidence_index` → first result) and warm
   (view creation O(1); reachability, shortest path, Dijkstra, each A* heuristic, components,
   s-connect, dual; batch 1/16/256) via `median_us`; bytes/incidence; peak RSS in isolated
   subprocesses for large cells; serial vs Rayon where meaningful. Complexity honesty: view =
   O(1); applying B/Bᵀ = Θ(nnz) — never call a projection O(1).

## Separation of measured axes (§2)
cold end-to-end construction · resident preparation (index) · time-to-first-result · warm
query · repeated-query amortisation · algorithm work vs formatting · measured / inferred /
hypothesis — each reported distinctly, never averaged across topology families.

## Controls vs production (§0/§9)
Clique / pairwise / materialised-star exist ONLY as benchmark controls + math oracles; the
production search path stays direct canonical-Ir. Decision criterion is end-to-end
`prepare + repeated queries + retained memory`, with a break-even query count per control.

## Optimization (§10)
At most one, and only if a measured bottleneck clears the evidence gate (differential test,
memory accounting, ≥ material gain, no > 3% unexplained regression, canonical-Ir ownership
preserved, no second model). A negative result (benchmark + report only) is acceptable and is
the expected default if the existing engine is already near the Θ(nnz) floor.

## Scope honesty (declared up front)
This is a large research study. This phase delivers verified workloads + oracle-checked
navigation + a focused measurement matrix + report. Explicitly **out of scope / reported as
NOT done** where not completed: full 1/2/4/8/16-thread scaling sweep, T-norm/S-norm AND/OR
sweep (reuse only, benchmarked separately if time permits), discrete/combined A* heuristic
(would touch CORE.YAML-protected `hymeko_graph` — gated, deferred), SVG figure generation,
`cargo llvm-cov` coverage (report UNMEASURED unless already installed), and any production
optimization not cleared by evidence. No new CLI; no parallel benchmark framework.

## Gates, commits, rollback (§11/§13)
fmt · clippy `-D warnings` · check · build · unit/integration/doctest · bench smoke. Commits:
(1) plan+inventory, (2) generators+correctness, (3) measurement harness, (4) optional
optimization, (5) report+figures. No binaries / Criterion target dirs / huge generated
`.hymeko` / credentials / absolute paths committed. Push only the feature branch; no merge to
graph/core main; no Python touched; no PGraph restructuring. Rollback: the branch is
additive; drop it and main is untouched. All task state under `D:\h\graph-search-p3` (never C:).
plan.pdf unavailable (no TeX engine; sources retained).
