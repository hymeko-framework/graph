# P3.2 — measurement closure — STATUS: **PARTIAL**

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P3.2 ·
**APPROVED-GRAPH-EDIT:** search-work-counters-and-measurement · 2026-09-03. Repo
`hymeko/graph`, branch `feat/graph-navigation-performance-p32` off `3bcab17`. Not merged; no
core/Python/Nagare/production-PGraph-algorithm edit; all task state under `D:\h\graph-search-p32`.

## §12.2 COMPLETION STATUS: **PARTIAL** — NOT complete.
Per §11, the phase is not marked complete: several mandatory tables (CPU scaling, peak-RSS,
bytes/entity, direct-Ir/star/clique break-even, T/S-norm, 2 of 4 SVGs) are **not measured** this
session (budget). This report does not substitute a recommendation for a missing measurement;
the exact missing rows are enumerated in §12.16.

## §1 P3.1 finding preserved (first-class)
Uniform-grid Dijkstra is faster than every tested A\* variant; all A\* variants remain optimal
and correct; **no A\* win established**; discrete preparation is a separate O(graph) cost. This
phase's counters *explain* it (below); the benchmark was not tuned to manufacture an A\* win.

## §12.3 Search work-counter table (DONE — `reports/data/2026-09-03-p32-work-counters.tsv`)
Engine `SearchStats` (existing: expanded_nodes/arcs, resolved_incidences, pushes, pops,
stale_pops) + added `heuristic_evals` + `reopened`. Overhead: plain non-atomic usize increments
on the already-mutable workspace stats (no alloc/atomic/dispatch); Dijkstra grid128 1641 µs vs
P3.1 1806 µs → **no >3% regression**. Instrumentation proven not to change route/cost/ordering
(`instrumented_search_matches_oracle`); `reopened == 0` (consistent-heuristic guarantee, now
observed, not assumed).

| workload | algo | µs | exp_nodes | exp_arcs | res_inc | push | pop | stale | h_eval | reopened |
|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| grid128 | dijkstra | 1641 | 16283 | 32108 | 32108 | 16308 | 16308 | 25 | 16307 | 0 |
| grid128 | euclidean | 2000 | 16283 | 32108 | 32108 | 19303 | 19303 | 3020 | 19302 | 0 |
| grid128 | manhattan | 2478 | 16283 | 32108 | 32108 | 19328 | 19328 | 3045 | 19327 | 0 |
| grid128 | discrete | 2479 | 16283 | 32108 | 32108 | 19328 | 19328 | 3045 | 19327 | 0 |
| corridor64 | (all 4) | ~99 | 4033 | 4032 | 4032 | 4033 | 4033 | 0 | 4032 | 0 |

## §12.4 Heuristic cost decomposition (DONE via counters)
**The counter-backed answer to §3's question:** on grid128 **every A\* variant expands the same
16283 states as Dijkstra** — the heuristic prunes **nothing** (identical `exp_nodes`). It is
therefore **structurally weak** (not merely overhead-bound): euclidean/manhattan/hop bounds
underestimate grid path length badly, so A\* explores the whole reachable set and additionally
pays ~18% more pushes (19303 vs 16308) and **120× more stale pops** (3020 vs 25) from the
reordered frontier → strictly slower. The corridor is a single forced path (no branching → no
pruning possible; all algorithms identical). Manhattan is proven admissible + consistent on the
16×16 fixture. **Overhead threshold:** with zero pruning, the added per-expansion cost is the
heuristic eval + the extra stale-pop churn — measured as the 1641→2478 µs gap.

## §12.5 Workspace-reuse + batch + allocation (DONE — `2026-09-03-p32-workspace-batch-alloc.tsv`)
Bench-local counting global allocator. **Honest measured result:** reuse cuts allocations ~4×
(fresh 21 → reused ~5 allocs/query on grid128) but is **NOT zero-alloc** — the returned path Vec,
the reconstruction, and heap growth for a larger frontier on a varied goal still allocate. A true
zero-alloc warm path needs engine-side path-buffer reuse (a candidate optimization; not done).
| mode | queries | µs/query | allocs/query |
|---|--:|--:|--:|
| fresh-workspace/query | 1 | 1685 | 21.0 |
| reused/same-goal | 1 | 1701 | 7.0 |
| reused/varied batch=1 | 1 | 1.0* | 2.0 |
| reused/varied batch=16 | 16 | 262 | 4.9 |
| reused/varied batch=256 | 256 | 516 | 5.8 |
(*batch=1 short-goal cell; the per-query µs varies with the goal distance.)

## §12.6–§12.10 CPU scaling / peak-RSS / bytes-per-entity / break-even / T-S-norm — **MISSING**
Not measured this session (see §12.16). The retained **~12 B/incidence** (P3) is the
`IncidenceIndex` only, **not** total model memory.

## §12.11 Large-case results
STS(1023) **DONE** (`sts_1023_formula_validation`): v=1023, blocks=174251, incidences=522753,
degree=511, connected — validated in 0.05 s (well under 16 GiB). Large Mycielski / 1e6-random /
512×512-map CPU+RSS cells are part of the missing §12.16 rows.

## §12.12 Tests / coverage
`p32_diagnosis` (4: manhattan admissibility+consistency, instrumented==oracle, counter-reset,
diagnosis) + `p32_workspace_alloc` (2) + `steiner_mycielski_nav` (8, incl. STS1023) +
`map_routing_nav` (5) + `informed_search_vv` (14) + `informed_oracles` (3) all green. `cargo fmt
--check` + `clippy -p hymeko_graph --all-targets --all-features -D warnings` clean. Coverage
**UNMEASURED** (`cargo-llvm-cov` absent; not installed; not inferred).

## §12.13 Fault mutants (DONE for the delivered surface; none committed; restored + hash-verified)
| mutant | caught by |
|---|---|
| overestimating Manhattan (×2) | `manhattan_admissible_and_consistent` |
| omit engine counter reset | `counters_reset_under_reuse` |
(+ P3/P3.1 mutants: u32-truncation, token-drop, order-reversal, drop-slot, overestimate-h,
combined-sum-not-max, ignored-obstacle.) The §9 mutants tied to the MISSING tables
(skip-pop-count, stale-visited, clique-equivalence, memory-misattribution, prep-asymmetry,
AND-as-OR) are **not** exercised — their target measurements are not built.

## §12.14 Performance regressions
None introduced: counters ≤3% (measured); `instrumented_search_matches_oracle` proves identical
results. No production optimization shipped (§13 gate not cleared).

## §12.15 Own mistakes / retractions
- I first asserted a **false zero-alloc** warm path; the allocation counter showed ~5
  allocs/query on a reused workspace. Corrected the test + docstring to the true ~4× reduction
  (not zero), and identified path-buffer reuse as the would-be optimization.
- My first "omit counter reset" mutant **escaped** (the invariant held cumulatively); I added
  `counters_reset_under_reuse` (two identical queries must have identical stats), which catches
  it. Reported rather than hidden.
- The serpentine corridor I built is a single forced path — it does **not** discriminate A\*
  (no branching). A branching open-area-with-offset-goal workload is still needed to look for a
  positive A\* case; not built.

## §12.16 EXACT MISSING MANDATORY ROWS (why PARTIAL)
1. **1/2/4/8/16/max-thread CPU table** with wall, **process CPU time**, busy-cores, normalized
   utilization, speedup, efficiency, incidences/s, checksum — on STS(511/1023), large Mycielski,
   1e6-random, 512×512 map. (Needs an isolated-subprocess harness reading Windows `GetProcessTimes`.)
2. **Peak-RSS table** per large cell in a fresh subprocess (generator/parse/index/workspace/heuristic/
   result RSS). (Needs subprocess `PeakWorkingSetSize` probes.)
3. **Total bytes/entity table** (bytes per declaration/node/hyperedge/hyperarc/incidence) — only
   the `IncidenceIndex` 12 B/inc exists.
4. **Direct-Ir / no-index-scan / sparse-star-CSR / clique-2-section break-even table**
   (`prepare + N×query + retained memory`, break-even N or `NEVER`). Controls not built.
5. **T-norm/S-norm table** over `hymeko_pgraph::andor` (BooleanExact + each norm × chain/fan-out/
   fan-in/mixed/sparse-random; prep/warm/iterations/incidences/alloc/mem/checksum).
6. **SVG figures 3 & 4** (CPU-scaling, break-even, memory) — only work-counter + allocation SVGs
   exist (2 of 4).
7. The §9 fault mutants tied to the above tables.

## §12.17 Remote verification / worktree
Branch `feat/graph-navigation-performance-p32` pushed; remote SHA == local (return message).
Worktree clean. Retained under `D:\h\graph-search-p32`: checkout, target, tmp, artifacts.
Graph main unchanged `b234c487`.

## §12.1 Commits
`89e810d` (counters + diagnosis), `42a79ba` (workspace/batch/alloc), `57e8cf5` (STS1023 +
reset test + SVG), report (this commit).

**PARTIAL. HARD STOP.** The missing §12.16 rows are the entirety of the next session's mandatory
work — no new capability, only the subprocess CPU/RSS harness, the materialised controls, and the
T/S-norm bench over existing code.
