# P3.3 — zero-copy rank-2 graphic matroid + navigation measurement closure — STATUS: **PARTIAL**

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P3.3 · **APPROVED-GRAPH-EDIT:**
zero-copy-matroid-operations-and-benchmarks · 2026-09-03 13:13 +02:00. Repo `hymeko/graph`, branch
`feat/graph-matroid-navigation-p33` off `1c6d568` (P3.2 tip). Not merged; no core / Python / Nagare
/ Hotaru / production-PGraph-algorithm edit; all task state under `D:\h\graph-matroid-p33`.

## §14 COMPLETION STATUS: **PARTIAL** — NOT complete.
Part B (the rank-2 graphic matroid, its full V&V, the O(1)-facade proof, fault injection, benchmark
tables and figures) is **delivered and verified**. Part A (the navigation measurement closure
inherited from P3.2) and the §3.1 incidence/Levi graphic matroid are **not** delivered this session.
Per §14 this report does **not** substitute a recommendation for a missing measurement; the exact
missing rows are enumerated in §16 and no foundation is swapped.

## §1 Scope delivered (Part B)
A matroid **view** over the canonical `Ir` + `IncidenceIndex` — not a new model. New module
`hymeko_graph::matroid` (approved edit): `MatroidRef<'a,'ir>` (two-word `Copy` O(1)/0-alloc
facade), caller-owned `GraphicMatroidWorkspace` DSU (path halving + union by rank + generation-
stamped lazy reset), typed `MatroidError`, and the rank-2 operations `rank2_spanning_forest`,
`rank2_rank`, `rank2_weighted_basis` (greedy = Kruskal), `IncrementalRank2`. Ground = `HyperArcId`s
resolving to exactly two node incidences; multiway → typed error. Canonical IDs in and out. No
unsafe (crate stays `#![deny(unsafe_code)]`). Design: `docs/matroid-over-canonical-ir.md`.

## §2 §0 binding-rule compliance
No second graph representation, copied array, matroid adjacency, materialised Levi/star/clique/
2-section, serialized matroid, or second caller ID space was created. Every operation is a function
of `(&Ir, &IncidenceIndex)`; the only retained state is caller-owned union-find scratch (no
topology). The canonical `Ir` is not mutated by any operation — proven behaviorally
(`matroid_operations_do_not_mutate_the_ir`) on top of the immutable `&Ir` borrow.

## §3 Files touched
| file | LOC | kind |
|---|--:|---|
| `hymeko_graph/src/matroid.rs` | 381 | new (approved edit) |
| `hymeko_graph/src/lib.rs` | +5 | `pub mod matroid;` + re-exports |
| `hymeko_graph/tests/matroid_vv.rs` | 697 | new — 14 V&V tests + oracles |
| `hymeko_graph/tests/matroid_facade.rs` | 148 | new — 4 O(1)-facade proofs |
| `hymeko_graph/tests/matroid_bench.rs` | 128 | new — latency benchmark |
| `scripts/svg_plot.py` | 83 | new — shared SVG emitter (bar+line) |
| `scripts/p33_matroid_figures.py` | 45 | new — 3 matroid figures |
| `scripts/p32_figures.py` | −23 | refactor to shared `svg_plot` (§6.1) |
| `docs/matroid-over-canonical-ir.md` | — | new design doc |
| `docs/graph-navigation-performance.md` | +25 | P3.3 section |
| `reports/data/2026-09-03-p33-matroid-latency.tsv` | — | raw measurement |
| `reports/figures/p33-matroid-{latency,basis-memory,incremental}.svg` | — | figures |

**CORE.YAML items touched:** `hymeko_graph` source, under the single APPROVED-GRAPH-EDIT
(`zero-copy-matroid-operations-and-benchmarks`). No other protected item.

## §4 The rank formula, triangulated three ways (V&V — DONE)
`rank_equals_n_minus_components_across_families` verifies, on 5 families (path, two grids, three
components, duplicated path), that all of the following agree:
`rank2_rank` == `N − connected_components().count()` (the engine differential oracle, §5.3) ==
`N − C` from an **independent** from-scratch union-find == that union-find's forest size. The
returned basis is additionally checked to be genuinely acyclic.

## §5 Independence axioms — exhaustive (V&V — DONE)
`independence_axioms_exhaustive_grid2x3` enumerates **all 2⁷ subsets** of a 7-edge cyclic grid and
proves: the matroid's notion of independence (fed through `IncrementalRank2`) equals graph
acyclicity on every subset; I1 (empty independent); I2 (hereditary — every subset of an independent
set independent); I3 (augmentation — |A|<|B| ⇒ ∃ e∈B∖A with A∪{e} independent); and max-independent
set size == `rank2_rank`.

## §6 Greedy == weighted optimum; incremental == rebuild; invariances (V&V — DONE)
- `weighted_basis_matches_independent_kruskal`: min/max basis totals == an independent Kruskal on a
  diamond, K₄, and a weighted path.
- `incremental_stream_equals_full_rebuild`: the streamed accepted-arc list is **identical** (order
  included) to `rank2_spanning_forest`.
- `workspace_reuse_is_deterministic_across_interleaved_graphs`: running graph A, then a different-
  sized B, then A again on the **same** workspace reproduces A's fresh-workspace basis (the
  generation reset works; no stale union state leaks).
- `rank_and_optimum_are_arc_permutation_invariant`: reversing+rotating the arc order leaves rank and
  optimum weight unchanged.
- `parallel_edges_counted_once`, `rank_correct_with_offset_node_decl_ids`, and the four typed-
  rejection tests (multiway / self-loop / dangling / non-finite).

## §7 O(1)-facade proof (DONE — `tests/matroid_facade.rs`, thread-local counting allocator)
- Structural: `size_of::<MatroidRef>() == 2·size_of::<usize>()`; `MatroidRef: Copy`; a copy borrows
  the same `Ir`/index.
- Empirical: facade construction + accessor reads = **0 allocations**, and **size-independent** —
  0 for a 4-node grid and 0 for a 511-point STS (~250k incidences) alike; 1000 facade copies = 0
  allocations.
- Boundary guard: `rank_computation_is_not_o1_but_facade_is` records that a rank *computation* is
  Θ(arcs), not O(1) — the O(1) claim is scoped to the facade, never the computation.

**Own finding (corrected):** the first facade allocator used a **process-global** atomic counter;
under `cargo test`'s concurrent execution, sibling tests' allocations polluted the exact-zero
window (measured 73 stray allocations). Fixed with a **thread-local** counter that attributes each
allocation to its allocating thread — the measurement is now immune to parallel tests. This is a
real observability lesson (a global counter cannot measure a per-thread zero under concurrency),
recorded rather than hidden.

## §8 Benchmark (DONE — `reports/data/2026-09-03-p33-matroid-latency.tsv`)
Release, median µs over 15 iters (2 warm-ups), reused workspace. Host: Intel Core Ultra 9 275HX,
Windows 11, well under the 16 GiB cap.

| family | nodes | arcs | inc | rank | idx_build µs | forest µs | weighted µs | incr µs | DSU B |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| path-1k | 1024 | 1023 | 2046 | 1023 | 6.1 | 5.6 | 18.6 | 6.5 | 12288 |
| path-8k | 8192 | 8191 | 16382 | 8191 | 43.3 | 44.2 | 204.4 | 57.9 | 98304 |
| path-65k | 65536 | 65535 | 131070 | 65535 | 1006.9 | 579.6 | 3997.8 | 683.0 | 786432 |
| grid-32 | 1024 | 1984 | 3968 | 1023 | 10.2 | 10.4 | 14.5 | 13.5 | 786432\* |
| grid-128 | 16384 | 32512 | 65024 | 16383 | 333.4 | 237.8 | 304.1 | 287.9 | 786432\* |
| grid-256 | 65536 | 130560 | 261120 | 65535 | 4409.3 | 3396.6 | 3769.2 | 3667.7 | 786432 |
| comps-8x1k | 8192 | 8184 | 16368 | 8184 | 43.5 | 45.1 | 62.0 | 56.4 | 786432\* |
| dup-8k | 8192 | 16382 | 32764 | 8191 | 164.9 | 111.6 | 183.7 | 135.4 | 786432\* |

(\*DSU bytes is the workspace **high-water** retained across reuse — a fresh workspace for that cell
alone would hold `12·N` bytes: e.g. grid-32 = 12288, not 786432. The 786432 is the residue of the
65k cells run earlier on the same reused workspace.)

**Read of the numbers.** Spanning forest is Θ(A·α): ~8.8 ns/arc on the path (contiguous node
access), ~26 ns/arc on grid-256 (cache-scattered endpoints — the same structural cost the direct-Ir
memory notes already record for the fat canonical layout). The weighted basis is 3–7× the
unweighted forest — the `Θ(A·log A)` Kruskal sort plus its edge-vector allocation — largest on the
path-65k (3998 vs 580 µs). Incremental stream tracks the batch forest within ~1.2×. DSU memory is a
clean **12 bytes/node** (3×u32), independent of arc count. Figures:
`reports/figures/p33-matroid-{latency,basis-memory,incremental}.svg`.

## §9 Tests / gates
`matroid_vv` (14) + `matroid_facade` (4) green; `matroid_bench` (1, `#[ignore]`) runs on demand.
Existing suites unaffected: `steiner_mycielski_nav` (5\*), `map_routing_nav` (4), `p32_diagnosis`
(8). `cargo fmt -p hymeko_graph --check` clean; `cargo clippy -p hymeko_graph --all-targets
--all-features -D warnings` clean. Coverage **UNMEASURED** (`cargo-llvm-cov` absent — not installed,
not inferred). (\*the steiner suite reports 5 with the default filter here; STS1023 is in the same
binary.)

## §10 Fault injection (DONE — 9 mutants, none committed, restored + hash-verified)
Driver injected each mutant, ran the one **named** test, required it to FAIL, restored the file, and
verified `sha256(matroid.rs)` back to the pristine baseline (`8c31c41a…`, pre-fmt).

| # | mutant | caught by |
|---|---|---|
| M1 | facade owns extra state (not 2 words) | `facade_is_two_words_and_copy` |
| M2 | wrong slot: both endpoints from `ends[0]` | `rank_equals_n_minus_components_across_families` |
| M3 | node/arc DSU domain overlap (arc index as coord) | `rank_equals_n_minus_components_across_families` |
| M4 | rank-2 accepts a 3-way hyperarc | `multiway_hyperarc_is_rejected` |
| M5 | weighted greedy sort reversed | `weighted_basis_matches_independent_kruskal` |
| M6 | union omitted (cycle edge counted as tree edge) | `parallel_edges_counted_once` |
| M7 | generation reset omitted | `workspace_reuse_is_deterministic_across_interleaved_graphs` |
| M8 | raw DeclId as dense DSU offset | `rank_correct_with_offset_node_decl_ids` |
| M9 | incremental accepts a cycle-closing arc | `incremental_stream_equals_full_rebuild` |

**Not exercised (honest):** the four §9 mutants tied to the **missing** Part-A tables —
basis-preserves-shortest-paths, prep-omitted-from-break-even, RSS-attributed-only-to-
`IncidenceIndex`, and T-norm-as-S-norm — are **not** run, because their target measurements
(break-even, peak-RSS, T/S-norm) are not built this session. Same discipline as P3.2: a mutant
whose target table does not exist cannot be caught.

## §11 Performance regressions
None. The matroid module is additive (new file + 5 lines of re-export); no existing code path was
modified, so no existing benchmark can regress. No production optimization was shipped (nothing to
gate under §3).

## §12 Contracts (§8)
`MatroidRef::new` — pre: the index was built from the same `Ir`; post: O(1), 0-alloc, borrows both.
`rank2_*` — pre: every hyperarc is rank-2 (else typed error); post: rank = N−C, basis acyclic,
`Ir` unmutated. `GraphicMatroidWorkspace` — invariant: holds only union-find scratch, never
topology; generation reset makes reuse equivalent to a fresh workspace.

## §13 Own mistakes / retractions
- Global-vs-thread-local allocation counter (§7): the first zero-alloc facade test used a process-
  global counter and mis-measured under concurrent tests; corrected to thread-local and documented.
- The `gen` field name collided with the 2024-edition reserved keyword; renamed to `cur_gen`
  (compile error caught it immediately — no runtime consequence).

## §14 Provenance
Git SHA at report time: `e7e6066` working tree, branch `feat/graph-matroid-navigation-p33` off
`1c6d568`; core resolved rev `15223bc` (graph Cargo pin), parser `284c490d`. Dirty files: the eight
task-owned additions in §3 (uncommitted at report authoring; committed in §15). Host: Intel Core
Ultra 9 275HX, Windows 11 26200. Benchmark seeds: deterministic generators only (no RNG in the
matroid path). Fixture hashes: generated deterministically by `informed_common` (no committed
binary fixtures). No dataset, checkpoint, or remote state mutated.

## §15 Commits (planned)
1. `docs(plan)` P3.3 plan bundle — **already committed** `e7e6066`.
2. `feat(graph)` matroid module + lib re-exports.
3. `test(graph)` matroid V&V + facade proof + fault-injection-verified.
4. `bench(graph)` matroid latency bench + TSV + shared `svg_plot` + p33 figures + p32 refactor.
5. `docs(graph)` matroid design doc + nav-perf P3.3 section + this report.
Only `feat/graph-matroid-navigation-p33` is pushed. HARD STOP after push+verify. No main merge.

## §16 EXACT MISSING MANDATORY ROWS (why PARTIAL)
Part A (inherited from P3.2 §12.16, still not built) **and** the §3.1 Levi matroid:
1. **§3.1 incidence/Levi graphic matroid** — ground = (`HyperArcId`, slot); basis = N+A−C over the
   Levi bipartite graph. **Not implemented.** Only the rank-2 graphic matroid (§3.2) is delivered.
2. **1/2/4/8/16/max-thread CPU-scaling table** with wall, **process CPU time** (Windows
   `GetProcessTimes`), busy-cores, normalized utilization, speedup, efficiency, incidences/s,
   checksum — in isolated subprocesses. Not built.
3. **Peak-RSS table** per large cell in a fresh subprocess (`PeakWorkingSetSize`). Not built.
4. **Total bytes/entity table** (bytes per decl/node/hyperedge/hyperarc/incidence). Only two point
   values exist: `IncidenceIndex` ≈ 12 B/incidence (P3) and the DSU workspace 12 B/node (this
   session).
5. **Direct-Ir / no-index-scan / sparse-star-CSR / clique-2-section break-even table**
   (prep + N×query + retained memory; break-even N or NEVER). Controls not built.
6. **T-norm/S-norm table** over `hymeko_pgraph::andor` (BooleanExact + each norm × chain/fan-out/
   fan-in/mixed/sparse-random). Not built.
7. The four §9 fault mutants tied to rows 4–6 (see §10).
8. Coverage (`cargo-llvm-cov`) — tool absent.

## §17 Remote verification / worktree
To be filled by the §15 push: branch pushed, remote SHA == local, worktree clean, graph main
unchanged. Retained under `D:\h\graph-matroid-p33`: checkout, target, tmp, artifacts.

**PARTIAL. HARD STOP after the branch push.** The rank-2 graphic matroid is a complete, self-
contained, fully-verified contribution; the §16 rows (Levi matroid + the P3.2 subprocess CPU/RSS/
break-even/T-S-norm closure) are the next session's mandatory work.
