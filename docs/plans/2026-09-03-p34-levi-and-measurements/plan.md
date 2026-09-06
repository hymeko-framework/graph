# P3.4 plan — provenance-safe facade, implicit-Levi matroid, final measurement closure

**Created-at:** 2026-09-03 13:20 (local, CEST). **STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 ·
**ROADMAP-TASK:** P3.4 · **APPROVED-GRAPH-EDIT:** implicit-levi-matroid-and-final-measurements.
Branch `feat/graph-matroid-navigation-p34` off `cead2c8`. Target `hymeko/graph` only. No main merge;
no core/Python/Nagare/Hotaru/production-PGraph edit. All task state under `D:\h\graph-matroid-p34`.

**ETA basis:** ~ large multi-hour closure. Dominant costs: ~6 release builds (~15–30 s each) + the
subprocess CPU/RSS sweep (6 threads × ~5 workloads × ~6 ops ≈ 180 child runs, most < 1 s; the 1e6
cells a few s) + the Levi/representation/T-S/12A benches. No single run approaches the 16 GiB cap
(largest DSU ~16 MB at 1.3e6 vertices) or a 2× wall surprise. Checkpoint = commit per phase.

## Scope (mission sections → deliverables)

1. **Provenance-safe facade (§1).** Make the invalid `(ir, foreign-index)` pair *unrepresentable*:
   `MatroidRef<'a,'ir>{ incidence: &'a IncidenceIndex<'ir> }` (one word); `new(incidence)`; private
   `ir(&self) -> &'ir Ir { self.incidence.ir() }`; every op reads the Ir through the index. Remove
   the stored `&Ir` and the `ir` ctor arg. No pointer check, no `debug_assert`, no `SnapshotMismatch`
   (never existed — nothing to remove). Add a `compile_fail` doctest (old 2-arg ctor) + a compiling
   twin + size(==1 word)/0-alloc assertions. **Module split** (same edit): `matroid.rs` →
   `matroid/{mod,error,workspace,facade,rank2}.rs` so the DSU is a shared sibling (§6.5#4; file was
   381 LOC / 4 concerns).
2. **Implicit incidence/Levi graphic matroid (§2).** New `matroid/levi.rs`. Ground = exact incidence
   coordinates `(HyperArcId, slot)`. Implicit bipartite graph: vertex per node + vertex per hyperarc;
   one edge per incidence coordinate. **Reuse** `GraphicMatroidWorkspace` (no 2nd DSU) over the
   combined coordinate space (node `n` → `n`; arc `a` → `N + a`). Ops: rank, component count,
   spanning basis, independence test, incremental insertion, caller-owned out buffer + workspace.
   Vertex universe = **all** N nodes + **all** A hyperarcs (isolated node / zero-incidence hyperarc =
   singleton component). Non-node/dangling target → typed `DanglingTarget` (never silently skipped);
   overflow → `Overflow`. Formula: `basis_size = (N+A) − C`. Connectivity certificate only.
3. **Levi V&V (§3).** Independent oracles: rank, component count, basis cardinality, direct-Ir
   connectivity parity (map arc→node components to Levi components), incremental==rebuild,
   deterministic canonical-coordinate output, workspace-reuse parity, duplicate-incidence
   preservation, canonical hash unchanged. Exhaustive I1/I2/I3 on a tiny instance. Workloads:
   STS(31/127/511/1023), two-Steiner, random rank-3 ~1e5 & ~1e6, multiway transport map.
4. **Levi performance (§4)** — the full per-workload column set; facade flat across 0/1e3/1e5/1e6.
5. **Connectivity break-even (§5)** — `basis_total(Q)=prep+Q·query` vs `direct_total(Q)=Q·query`;
   Q∈{1,16,256}; report break-even or NEVER.
6. **CPU closure (§6)** — one committed PowerShell subprocess harness using
   `Process.TotalProcessorTime` + `PeakWorkingSet64`; a built example binary `p34_cell` runs one
   (workload,op,threads) cell (RAYON_NUM_THREADS set by parent). Threads 1/2/4/8/16/host-max; ops
   reachable/batch/components/warm-Dijkstra/rank2-basis/Levi-basis; workloads STS(511/1023),
   Mycielski-large, random~1e6, largest-safe-map. Per cell: wall, process CPU, busy cores, norm.
   utilization, speedup, efficiency, incidences/s, checksum.
7. **Memory closure (§7)** — per large workload in a fresh subprocess: retained bytes per component
   (Ir est / IncidenceIndex / SearchWorkspace / rank2 ws / Levi ws / heuristic / basis out /
   controls), peak RSS, alloc count, allocated bytes; normalized bytes/{decl,node,edge,hyperarc,
   incidence,rank2-ground,Levi-ground}. Never present ~12 B/inc as total model memory.
8. **Representation break-even (§8)** — controls (benchmark-only, never production): direct;
   no-index scan; genuine sparse star CSR; clique/2-section; Levi basis certificate. Star and clique
   named separately. Per control: prep, retained, peak RSS, first/warm latency, 1/16/256 totals,
   break-even/NEVER, exact semantic info lost.
9. **T/S-norm matrix (§9)** — reuse `hymeko_pgraph::andor` (`propagate` + `BooleanExact/Godel/
   Product/Lukasiewicz`) over chain/OR-fan-out/AND-fan-in/mixed/sparse-random; prep, warm, iters,
   incidences, allocs, retained, peak RSS, checksum. New TEST file only (no andor.rs edit).
10. **Preserve findings (§10)** — restate rank-2 O(1)/one-word, ~8.8/26 ns/arc, weighted 3–7×,
    grid128 A*==Dijkstra 16283 states (no A* victory manufactured).
11. **Fault injection (§11)** — 12 mutants, each caught by a named test, restored + hash-verified.
12. **Coverage (§12)** — `UNMEASURED — tool unavailable; not inferred` (do not install).
13. **12A operation matrix.** Unified public Ir construction/manipulation bench across sizes
    {0,1,2,63,64,65,1e3,1e4,1e5,1e6} × topologies {Steiner,Mycielski,uniform,degree-skewed,
    disconnected,map,multiway}. Per op: latency, throughput, allocs/op, bytes/op, retained delta,
    peak-RSS delta where resolvable, observed scaling. Only public sanctioned APIs. Missing ops
    (canonical deletion / transactional update) → `NOT IMPLEMENTED` line, not faked.

## Affected files (planned)
- `hymeko_graph/src/matroid.rs` → split to `hymeko_graph/src/matroid/{mod,error,workspace,facade,rank2,levi}.rs`
- `hymeko_graph/src/lib.rs` (re-export list unchanged in surface; add Levi symbols)
- `hymeko_graph/tests/matroid_vv.rs`, `matroid_facade.rs`, `matroid_bench.rs` (facade call-site + new twin/compile_fail; Levi V&V in a new `matroid_levi_vv.rs`)
- `hymeko_graph/examples/p34_cell.rs` (subprocess cell entry, measurement)
- `hymeko_graph/tests/{levi_bench,representation_breakeven,ir_ops_bench}.rs`
- `hymeko_pgraph/tests/andor_ts_matrix.rs` (new test; reuses `propagate`, no production edit)
- `scripts/p34_cpu_rss.ps1` (PowerShell subprocess telemetry harness)
- `scripts/svg_plot.py` (reuse; add grouped/scaling helpers if needed), `scripts/p34_figures.py`
- `reports/2026-09-03-matroid-navigation-p34.md` + `reports/data/*.tsv` + `reports/figures/*.svg`

**CORE.YAML items touched:** `hymeko_graph` source, under the single APPROVED-GRAPH-EDIT. `andor`
production code is **not** modified (new test file only). No other protected item.

## Interface changes
- `MatroidRef::new(&IncidenceIndex)` (was `new(&Ir, &IncidenceIndex)`) — breaking; all call sites
  updated in the same commit. `MatroidRef` is now one word; `ir()` becomes private (was public).
- New Levi API: `levi_rank`, `levi_components`, `levi_spanning_basis`, `levi_independent`,
  `IncrementalLevi`, `LeviCoord = (HyperArcId, u32)` output.

## Test strategy
Unit + V&V per §3/§11; independent oracles in-test; exhaustive axioms on tiny instances; production-
scale smoke (STS(1023), random 1e6) before the subprocess sweep; 5+-iter median for every bench
number; alloc via thread-local counting allocator; subprocess CPU/RSS via the PowerShell harness.

## Performance budget
Peak RSS < 16 GiB (largest structure ~16 MB). No production optimization shipped (nothing to gate).
Facade must stay O(1)/0-alloc/one-word. No >10% regression on existing paths (module is additive to
call sites; only the facade shape changes — re-measured by the size/alloc asserts).

## Rollback path
Each phase is a separate commit; revert the offending commit. The module split is mechanical (git
tracks the rename). No main merge, so `origin/main` is never at risk.

## Risk anticipation
- **Worst-case input:** random rank-3 at 1e6 incidences (~333k arcs); DSU over N+A ~1.3e6 u32×3
  ≈ 16 MB; basis build Θ(incidences). Smoke this one cell before the full sweep.
- **Contract preservation:** the one-word facade must keep O(1)/0-alloc (assert), and every op must
  read `ir` via `incidence.ir()` (no separate `&Ir`). Levi must reject non-node targets (typed), not
  skip. The subprocess harness must read **process** CPU time (not host-wide %).
- **Blocker policy:** only a genuine external blocker (e.g. a norm family andor does not expose, a
  missing public API) yields a PARTIAL row with the exact blocker — not session length.
