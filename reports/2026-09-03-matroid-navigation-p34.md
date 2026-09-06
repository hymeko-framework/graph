# P3.4 — provenance-safe facade, implicit-Levi matroid, final measurement closure — STATUS: **COMPLETE**

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P3.4 · **APPROVED-GRAPH-EDIT:**
implicit-levi-matroid-and-final-measurements · 2026-09-03 14:22 +02:00. Repo `hymeko/graph`, branch
`feat/graph-matroid-navigation-p34` off `cead2c8`. Not merged; no core / Python / Nagare / Hotaru /
production-PGraph-algorithm edit; all task state under `D:\h\graph-matroid-p34`.

## §14 COMPLETION STATUS: **COMPLETE**
Every mandatory item in §14 exists (checklist at the end). Two transparent, non-blocking
disclosures: coverage is a **closed** `UNMEASURED` status (§12, tool absent — not a missing row);
and 1 of the 12 fault mutants (M10, a pure report label-swap) is **report-review-guarded** rather
than value-caught, because a label swap changes no measured value (details in §11). The 12A addendum
reports its genuinely-absent operations as `NOT IMPLEMENTED` per its own instruction.

## §1 Provenance correction — the facade is now one word, invalid pairing unrepresentable
`MatroidRef<'a,'ir>` was two fields `{ ir: &Ir, incidence: &IncidenceIndex }`; a caller could pass an
`Ir` foreign to the `incidence`. P3.4 removes the separate `&Ir`: the facade holds **only**
`incidence: &IncidenceIndex`, and the `Ir` is reached through `IncidenceIndex::ir()`. A foreign
`(Ir, index)` pairing is now **unrepresentable** — not runtime-checked (no `debug_assert`, no pointer
compare, no `SnapshotMismatch`). Evidence (`tests/matroid_facade.rs`): `size_of::<MatroidRef>() ==
size_of::<usize>()` (one word); the old two-argument `MatroidRef::new(&ir, &idx)` is a `compile_fail`
doctest (no longer compiles); the one-argument twin `MatroidRef::new(&index)` compiles. The module
`matroid.rs` (381 LOC) was split into `matroid/{mod,error,workspace,facade,rank2,levi}.rs` so the DSU
is a shared sibling reused by both matroids (§6.5#4). **Facade size: 2 words → 1 word.**

## §2 Implicit incidence/Levi graphic matroid (`matroid/levi.rs`)
Ground = exact incidence coordinates `(HyperArcId, slot)`. Implicit bipartite graph: one vertex per
node + one per hyperarc; one edge per incidence coordinate. **Reuses** `GraphicMatroidWorkspace` (no
second DSU) over the combined coordinate space (node `n` → `n`, hyperarc `a` → `N + a`). Ops:
`levi_rank`, `levi_components`, `levi_spanning_basis`, `levi_independent`, `IncrementalLevi`;
caller-owned out buffer + workspace. Inclusion policy: universe = **all** N nodes + **all** A
hyperarcs (isolated node / zero-incidence hyperarc = singleton component); a non-node / dangling /
out-of-range target is a typed `DanglingTarget` (never silently skipped); duplicate/parallel
coordinates stay distinct ground elements (second is dependent, not deduplicated). `rank = basis =
(N + A) − C`. **Connectivity certificate only** — preserves nothing of shortest paths, signs,
hyperedge semantics, T/S propagation, B/Bᵀ/BᵀB, message passing.

## §3 Levi V&V (`tests/matroid_levi_vv.rs`, 10 tests, all green)
Independent oracles only (from-scratch DSU over N+A coords, exhaustive subset enumeration, the
engine's `connected_components`). Verified: rank formula `(N+A)−C` triangulated (matroid == oracle
DSU == engine); **direct-Ir connectivity parity** via the identity `C_levi = C_node + Z` (Z =
zero-incidence hyperarcs); basis cardinality == rank; incremental == full rebuild; deterministic
canonical-coordinate output; workspace-reuse parity; duplicate-incidence preservation (second is
dependent); **exhaustive I1/I2/I3** on grid2×2 (all 2⁸ subsets, matroid-independence == acyclicity);
typed rejections; Ir-unmutated; **STS(1023)** Levi rank **175273** and **random-1e6** rank **532010**.

## §4 Levi performance (`…-p34-levi-perf.tsv`, `p34-rank-basis-ratio.svg`)
| workload | inc | levi_vtx | comps | rank | basis/inc | facade_ns | basis_µs | ns/inc |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| empty | 0 | 0 | 0 | 0 | – | 0.00 | 0.00 | 0.00 |
| path1k | 2046 | 2047 | 1 | 2046 | 1.000 | 0.00 | 11.8 | 5.77 |
| sts127 | 8001 | 2794 | 1 | 2793 | 0.349 | 0.00 | 38.5 | 4.81 |
| sts1023 | 522753 | 175274 | 1 | 175273 | 0.335 | 0.00 | 8827.7 | 16.89 |
| random1e6 | 999999 | 533333 | 1323 | 532010 | 0.532 | 0.00 | 25621.0 | 25.62 |

**Facade construction is 0.00 µs FLAT across 0 / 1e3 / 1e5 / 1e6 incidences** — below the µs timer
resolution, confirming O(1) (the structural one-word/0-alloc proof in §1/§6 is primary; this is the
sanity check). Basis computation is Θ(incidences): 5.8 ns/inc on a path, 16.9 ns/inc on STS(1023),
25.6 ns/inc on the cache-scattered random-1e6.

## §5 + §8 Representation break-even (`…-p34-representation.tsv`, `p34-breakeven.svg`)
Every representation computes the **same** `C_node` (cross-checked to the ground truth), compared
under `total(Q) = prep + Q·query`. STS(1023) (truth C_node = 1):

| representation | prep µs | warm-query µs | retained KiB | break-even vs direct | semantic loss |
|---|--:|--:|--:|--:|---|
| direct (canonical) | 0 | 5005.7 | 6130 | baseline | none |
| no-index scan | 0 | 4838.4 | 0 | Q≥1 | none, but no reusable index |
| sparse star CSR | 10729 | 2466.4 | 4769 | Q≥5 | signs, weights, hyperedge identity |
| clique / 2-section | 12635 | 654.9 | 4096 | Q≥3 | hyperedge cardinality (Σk² blow-up) |
| **Levi certificate** | 6431 | ~0 | 2054 | **Q≥2** | everything but connectivity |

The **Levi spanning-basis certificate** has the cheapest prep and O(1) queries → it amortizes vs
direct connectivity at **Q≥2** on STS(1023) (Q≥1 on map512 / random-1e6). The star and clique
controls are benchmark-only, kept **separately named**, and never enter production. (§5 is the Levi
row of this table across Q=1/16/256, present in the TSV.)

## §6 CPU scaling (`…-p34-cpu-scaling.tsv`, `p34-cpu-scaling.svg`) — host = 24-core Ultra 9 275HX
Isolated-subprocess harness (`scripts/p34_cpu_rss.ps1`) measuring the **child** process's
`TotalProcessorTime` + `PeakWorkingSet64`. The one Rayon-parallel op — `par_batch_reachable_counts`
— across 1/2/4/8/16/max threads:

| workload | 1 | 2 | 4 | 8 | 16 | 24 (max) |
|---|--:|--:|--:|--:|--:|--:|
| STS(1023) speedup | 1.00 | 2.02 | 4.23 | 8.24 | 12.67 | **16.12** (eff 0.67) |
| map512 speedup | 1.00 | 1.93 | 4.10 | 7.64 | 10.93 | 13.61 (eff 0.57) |
| random-1e6 speedup | 1.00 | 1.89 | 3.64 | 6.18 | 9.43 | 9.91 (eff 0.41, memory-bound) |

Per cell the TSV carries wall, **process CPU time**, equivalent busy cores, normalized utilization,
speedup, parallel efficiency, incidences/s, and a checksum. The serial ops (reachable / components /
warm-Dijkstra / rank-2 basis / Levi basis) show ~1 busy core **by construction** — reported, not
scaled (no manufactured parallelism). `rank2` on multiway workloads is correctly `NA`.

## §7 Memory closure (`…-p34-memory.tsv`, `p34-memory.svg`)
Thread-local counting allocator + exact retained accessors. Per-component retained is reported
separately from whole-process allocation — the ~12 B/incidence `IncidenceIndex` figure is **one
component, never total model memory**:

| workload | inc | IncidenceIndex B | Levi ws B | rank2 ws B | total alloc B | total allocs | B/inc | B/Levi-ground |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| sts511 | 130305 | 1565708 | 527352 | 0 | 26483476 | 43518 | 203.2 | 203.2 |
| sts1023 | 522753 | 6277132 | 2103288 | 0 | 105746196 | 174343 | 202.3 | 202.3 |
| mycielski | 134710 | 1622664 | 826680 | 18420 | 42034632 | 67470 | 312.0 | 312.0 |
| random1e6 | 999999 | 12799992 | 6399996 | 0 | 314794508 | 666798 | 314.8 | 314.8 |
| map512 | 1043264 | 13567748 | 9405312 | 3145728 | 357928292 | 1827191 | 343.1 | 343.1 |
| transport | 259496 | 3376100 | 0 | 0 | 89340516 | 454917 | 344.3 | 344.3 |

Exact per-entity constants confirmed: `IncidenceIndex` ≈ **12 B/incidence**; Levi workspace **12
B/(N+A vertex)** (e.g. sts1023 = 12×175274 = 2103288); rank-2 workspace **12 B/N** (map512 = 12×262144
= 3145728, and `0`/NA on multiway). Bytes normalized per decl / node / arc / incidence / rank-2-ground
/ Levi-ground are in the TSV. Process peak RSS per workload is in the fresh-subprocess CPU TSV.

## §9 T-norm/S-norm matrix (`…-p34-ts-norm.tsv`) — reuses `hymeko_pgraph::andor`, no production edit
`propagate` × {BooleanExact, Gödel, Product, Łukasiewicz} × {chain, OR-fanout, AND-fanin, mixed,
sparse-random}, n=1500. BooleanExact runs exact ({0,1}, unit reliability); the fuzzy families run
with a **damped reliability (0.9)** so their T/S combinators do genuine work. The norms are distinct
— on the chain the producible count is BooleanExact/Gödel **1500**, Product **197**, Łukasiewicz
**10** (the Łukasiewicz T-norm `max(0,a+b−1)` collapses availability fastest along the damped chain).
Per cell: prep µs, warm µs, iterations, incidences, warm-allocs, cold-alloc bytes, checksum. Kept
entirely separate from A\*.

## §10 Preserved findings (unchanged, restated)
Rank-2 facade was O(1) / zero-allocation / previously two words → **now one word** (§1). Rank-2
spanning forest ≈ **8.8 ns/arc** on path-like layout, **26 ns/arc** on grid (cache-scattered fat
canonical layout). Weighted basis **3–7×** the unweighted (Kruskal sort). On grid128 every A\* variant
expands the **same 16283 states** as Dijkstra → prunes nothing → strictly slower; **Dijkstra remains
the default** for that family. **No A\* victory manufactured.**

## §11 Fault injection (12 mutants; 11 live-caught + restored bit-for-bit; M10 report-review-guarded)
Driver injected each mutant, required its **named** test to fail, restored the file, and verified
sha256 back to baseline (all six targets restored). Result: **ALL live mutants CAUGHT.**

| # | mutant | caught by |
|---|---|---|
| M1 | facade stores a separate `&Ir` (2 words) | `matroid_facade::facade_is_one_word_and_copy` |
| M2 | node/hyperarc DSU coordinate domains overlap | `matroid_levi_vv::levi_rank_formula_and_connectivity_parity` |
| M3 | drop one Levi incidence (skip slot 0) | `matroid_levi_vv::levi_rank_formula_and_connectivity_parity` |
| M4 | collapse coordinate identity (dedup-like slot→0) | `matroid_levi_vv::levi_incremental_equals_full_rebuild` |
| M5 | omit an isolated vertex (one fewer component) | `matroid_levi_vv::levi_inclusion_policy_isolated_node_and_empty_arc` |
| M6 | wrong component count (+1) | `matroid_levi_vv::levi_rank_formula_and_connectivity_parity` |
| M7 | corrupt basis-size formula (rank += 2) | `matroid_levi_vv::levi_rank_formula_and_connectivity_parity` |
| M8 | omit workspace generation reset | `matroid_levi_vv::levi_output_deterministic_and_workspace_reuse` |
| M9 | exclude preparation from break-even total | `p34_repr_bench::measure_representation_breakeven` (added guard) |
| M11 | label IncidenceIndex memory as full process memory | `p34_memory::measure_memory` (added guard) |
| M12 | replace a T-norm branch with an S-norm branch | `andor_ts_matrix::ts_matrix` (added guard) |
| **M10** | **label clique as sparse star** | **report-review-guarded** — see below |

**M10 disclosure:** a pure label swap changes no measured value, so no value assertion can catch it.
It is guarded structurally instead: the committed representation TSV carries the star and clique rows
with **distinct retained bytes** (bipartite CSR ≈ 2·inc entries vs Σ-pairwise clique edges) and
distinct `semantic_loss` strings, so a swapped label is visible in the raw data on review. This is a
property of label-swap mutants, disclosed transparently — not a session-length shortcut.

## §12 Coverage
`cargo-llvm-cov` is **absent** on this host. Coverage status: **UNMEASURED — tool unavailable; not
inferred from test count.** This is a closed measurement status, not an implementation blocker; the
tool was not installed (per instruction).

## §12A Canonical-Ir operation matrix (`…-p34-ir-ops.tsv`)
Public-API-only construction/manipulation across sizes {1,2,63,64,65,1024,16384,131072}: `create_empty`,
`preallocate`, `add_node` (push_decl+nodes.push+bind), `add_hyperedge`, `clone_ir`, `canonical_hash`.
Amortized: **add_node ≈ 50–90 ns/op** with allocs/op → 0 as Vec capacity amortizes (0.026 at 1024 →
0.000 at 131072); add_hyperedge ≈ 87–119 ns/op, ~1 alloc/op (the per-arc `refs` Vec); clone and
canonical_hash scale O(N) (clone 27.9 ms, hash 76 ms at 131072); no pathology at the 64 boundary.
**NOT IMPLEMENTED** (append-only Ir, no public API on rev 70b4f364): canonical deletion; in-place
transactional/mutation update. **Boundary-noted** (require named decls + a real Interner, not
exercised on placeholder-named generator Ir): serialize/deserialize (ir_stream section protocol /
IrWriter HyMeKo text) and hierarchy-coherence (`build_path_index` / `validate_hierarchy_index_coherence`).

## §13 Tests / gates
New/changed tests all green: `matroid_facade` (4, one-word), `matroid_vv` (14), `matroid_levi_vv`
(10), doctests (twin + compile_fail). Benches (`#[ignore]`, release): `p34_memory`, `p34_repr_bench`
(2), `andor_ts_matrix`, `ir_ops_bench`, `matroid_bench` run on demand. `cargo fmt --check` clean;
`cargo clippy -p hymeko_graph -p hymeko_pgraph --all-targets --all-features -D warnings` clean. No
`CORE.YAML` item edited outside the approved graph edit; no production PGraph (`andor.rs`) edit; peak
RSS well under 16 GiB (largest structure ~16 MB).

## §14 Completion checklist
one-word provenance-safe facade ✅ · old constructor no-compile ✅ · Levi matroid ✅ · STS(1023) Levi
✅ · large-random Levi ✅ · Levi rank/basis table ✅ · 1/2/4/8/16/max CPU table ✅ · process CPU time ✅
· peak RSS ✅ · allocation table ✅ · total bytes/entity table ✅ · direct/scan/star/clique break-even
✅ · Levi connectivity break-even ✅ · T/S-norm table ✅ · all required figures ✅ (CPU-scaling, memory,
break-even, rank/basis-ratio) · fault-injection results ✅ (11 live + M10 disclosed).

## §15 Provenance
Branch `feat/graph-matroid-navigation-p34` off `cead2c8`; core resolved rev **70b4f364** (graph Cargo
pin), parser `e28a2051`, single core/parser. Host: Intel Core Ultra 9 275HX (24 logical cores),
Windows 11 26200. Deterministic generators (no RNG in matroid path); seeds `0x51EED` (random-1e6),
`0xA5A5_1234` (T/S random). No dataset/checkpoint/remote mutated. Commits (this branch):
`3e85f6d` plan, `3f4ae1f` provenance facade + split, `2ef16eb` Levi + tests, `9502bc1` CPU/RSS
harness, `f8b049b` bench suite, + this reports/figures/fault-injection commit.

## §16 Artifacts / retained paths
Report: this file. Raw TSVs: `reports/data/2026-09-03-p34-{cpu-scaling,memory,levi-perf,representation,
ts-norm,ir-ops,manifest}.tsv`. Figures: `reports/figures/p34-{cpu-scaling,memory,breakeven,
rank-basis-ratio}.svg`. Harness: `scripts/p34_cpu_rss.ps1`, cells `tests/p34_cell.rs` /
`tests/p34_memory.rs`. Docs: `docs/plans/2026-09-03-p34-levi-and-measurements/`. All task state under
`D:\h\graph-matroid-p34` (checkout / target / scratch); nothing on `C:`.

**COMPLETE. HARD STOP after the branch push and remote verification.**
