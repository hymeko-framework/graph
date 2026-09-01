# Direct canonical-Ir hypergraph algorithms — report

**APPROVED-CORE-EDIT: graph-direct-ir-algorithms** · 2026-09-01 02:50 CEDT
Branch `feat/graph-direct-ir-algorithms` (graph) @ base `5078980`; core promoted to `70b4f36`.

## Summary

The standard HyMeKo graph path now traverses the canonical `hymeko_core::ir::Ir` **in place**, through
the borrowed `IncidenceIndex<'ir>`, with **no** dense view, `Ir→SignedGraph` adapter, pairwise
flattening, materialised star/clique, second hypergraph model, or new adapter crate. New module
`hymeko_graph::direct_ir` provides a caller-owned workspace, one alternating-walk engine, the four
required algorithms, an optional layer (bounded Berge enumeration, s-connected components, batch
reachability), a benchmark, and source-first V&V.

## 1. Core promotion — SHA and remote match

Core `main` fast-forwarded `3190097 → 70b4f36` (the incidence-navigation corrective). Gates re-run on
the feature worktree before promotion: fmt, clippy `-D warnings`, build, 120 lib + 210 integration +
52 wgpu + subsuites (0 failed), 27 doctests, incidence golden/oracle/operator/fault isolated — all
green. `git ls-remote` proof: `refs/heads/main = 70b4f364…`. No documented fixture regression appeared.

## 2. Graph commit SHAs and remote feature tip

Nine commits on `feat/graph-direct-ir-algorithms` (newest first):

| SHA | commit |
|---|---|
| 038c5af | test(direct_ir): coverage-closing — strict non-node error, Display, budgets, accessors |
| 7d2f620 | test(direct_ir): metamorphic + generated families, large-scale + isolated zero-alloc |
| efde071 | bench(direct_ir): direct vs sparse-star vs clique, k² falsifier, zero-alloc proof |
| 9f5274b | fix(pgraph-tests): migrate meta_resolve_ir_faults to the sealed Ir builder API |
| 9fdf6a2 | feat(direct_ir): optional layer — bounded Berge enum, s-components, batch |
| a2cba26 | feat(direct_ir): zero-copy canonical-Ir traversal + required algorithms |
| c3ebd68 | chore(deps): hymeko_graph → hymeko_core@70b4f36; correct CORE.YAML depends_on |
| 59b3f21 | docs(plan): direct canonical-Ir hypergraph traversal — plan + ownership audit |

Remote feature tip after push: see §18. Graph `main` **not** merged this session.

## 3. Dependency DAG and lockfile diff

`hymeko_graph → hymeko_core` (new); `hymeko_pgraph → hymeko_graph + hymeko_core`; `hymeko_core ↛
hymeko_graph` (acyclic, verified — core has no graph dep). Lockfile: `hymeko_core 15223bc → 70b4f36`;
`parser` pinned back to `e28a2051` (a `cargo update -p hymeko_core` drifted it to master-tip `284c490d`;
reverted — unrelated drift is not accepted, and core@70b4f36 was verified green against exactly
parser@e28a2051). Exactly one `hymeko_core` version in the lock. `CORE.YAML`'s untrue `depends_on: []`
corrected to record `../core` and the acyclic edges.

## 4. The direct-Ir public API (canonical IDs only)

- `TraversalWorkspace` — caller-owned scratch: generation-marked visited-node **and** visited-arc sets
  (`DeclId` / `HyperArcId` space), node & arc frontiers, lazy predecessor arrays, discovery buffers.
  `new` / `ensure_capacity` / `begin`. Warm queries allocate nothing; generation wrap handled + tested.
- `HyperTraversal<'a,'ir>` — one structured facade over `&IncidenceIndex<'ir>` (no `&Ir + &IncidenceIndex`
  pairing). `reachable_nodes → NodeSet`, `berge_shortest_path → Option<BergePath>`, `ir()`, `index()`.
- `connected_components → ComponentLabeling`; `dual_reachability → DualReach`.
- Optional: `enumerate_berge_paths` / `enumerate_berge_cycles` (`EnumBudget`, `EnumOutcome`, `BergeCycle`);
  `s_connected_components → SComponentLabeling`; `batch_reachable_counts` / `par_batch_reachable_counts`.
- `BergeStep { arc, slot, sign, node }` — a step resolves to a full `IncidenceRef` via `(arc, slot)`;
  nothing is dropped. Errors: `DirectIrError::{InvalidStart, InvalidStartArc, TargetNotANode,
  PredecessorInconsistency, BudgetExceeded}` — ids-only, no attacker text.

## 5. Visited-node / visited-arc complexity proof

`HyperTraversal::expand` is the single alternating step: for each *unseen* incident arc it marks the arc
(`mark_arc`, `debug_assert!(!arc_seen)`), then iterates the arc's members once. A `k`-member hyperarc is
therefore expanded **at most once per search** — enforced by the debug-assert, and demonstrated by
control **B** (below), which drops the guard and re-expands. Cost is `O(nodes + arcs + incidences)` over
the visited component. Marking each node once on first discovery gives the symmetric guarantee in the
dual (`arc → node → arc`): a node shared by `m` arcs is scanned once.

## 6. Algorithms — implemented and explicitly out

Implemented: `reachable_nodes`, `berge_shortest_path`, `connected_components`, `dual_reachability`
(required); `enumerate_berge_paths`, `enumerate_berge_cycles`, `s_connected_components`,
`batch_reachable_counts` + `par_batch_reachable_counts` (optional). **Deliberately not built** (owned by
Hotaru, not duplicated — §15): transversals/matchings/edge-covers, the acyclicity hierarchy
(α/conformal/Helly), Levi/2-section/line-graph constructions, s-betweenness/closeness.

## 7. Source-first workload matrix

All V&V is `*.hymeko → RealParser → ModuleStore::compile → Ir → IncidenceIndex → algorithm → canonical
result` (in-memory `MemProvider`; no new dev-dependency). Golden shapes: multiway `+/−/~`, weighted
incidence, duplicate incidence, self-incidence, singleton arc, multi-component, shared node.
Metamorphic: declaration-order permutation, disjoint union. Generated families: sunflower (shared
core), hub-skew, many-small-components grid. Scale: `large_scale_all_approaches_agree` at ~1e6
incidences (direct == star == clique). Independent `O(V·E)` fixpoint oracle (does **not** use the
`IncidenceIndex`, not the same generator) cross-checks reachability.

## 8. Test count, doctest, coverage

**Full workspace: 434 passed / 0 failed / 5 ignored** (baseline 406/0/4; the pgraph fix restored all
406, +28 net new). Doctests green. direct_ir suite: 32 tests across 7 files + 3 in-lib workspace-wrap
units. Line coverage (cargo-llvm-cov 0.9.0): mod 100 %, components 100 %, batch 100 %, traversal 99.3 %,
workspace 99.2 %, s_connect 98.9 %, berge_enum 93.5 %, dual 93.8 %. Residual uncovered lines are
provably-defensive tripwires (`PredecessorInconsistency`; the post-`is_node` `None` guard).

## 9. Fault-mutant coverage (which test catches which mutation)

- **drop visited-arc / double-expand an arc** → `mark_arc`'s `debug_assert!(!arc_seen)` panics; any
  shared-arc test trips it. Control **B** in the bench demonstrates the re-expansion behaviour.
- **double-discover a node / mis-dedup a duplicate** → `discover_node`'s `debug_assert!(!node_seen)`;
  `duplicate_incidence_is_a_single_visit…` pins the semantics.
- **generation-wrap / queue-head reset** → `generation_wrap_resets_stale_stamps`,
  `warm_workspace_is_reused_across_queries`.
- **wrong predecessor arc / sign/slot shift** → `berge_shortest_path_length_nodes_and_lossless_steps`
  (`resolve(arc,slot).sign == step.sign`, `.target == step node decl`).
- **invalid target flattened as a node** → `non_node_target_is_a_typed_error_in_every_traversal`.
No mutant is left in a committed source file.

## 10. Direct vs sparse-star vs clique benchmark (µs, honest)

`tests/direct_ir_bench.rs` (release, `--ignored`), one typed unit (µs, median-of-9), nnz~1e6:

| workload | index build | bytes/inc | A=direct | B=no-arc | C=star | D=clique | clique build |
|---|--:|--:|--:|--:|--:|--:|--:|
| arity 5 (200k arcs) | 14.1 ms | 12.8 | 14.1 ms | 13.8 ms | 1.0 ms | 0.66 ms | 60.7 ms (4.3×) |
| arity 20 (50k arcs) | 13.1 ms | 12.2 | 10.1 ms | 12.6 ms | 0.65 ms | 0.51 ms | 118.6 ms (9.0×) |

**Honest, isolated interpretation.** The direct warm query (A) is **~13–15× slower than the flat-u32
sparse-star CSR** (C). The discriminator: **A ≈ B** even though B does k-fold more member-visits — so
A's cost is dominated by iterating the **fat canonical `Vec<SignedRefR>`** and the per-incidence
`is_node` kind lookup, **not** the arc-expansion count. The flat CSR streams packed integers; the
canonical structure chases fat enums. This is the price of zero-copy + losslessness (sign/weight/anno
inline via `(arc,slot)`) + canonical IDs. **Direct beats clique decisively on build (4.3×–9×).** The
mission's 3 %-vs-sparse-star gate is taken via the **profiled-explanation branch**, not met on raw
warm-query speed — a structural tradeoff, not a bug. The k²-avoidance is correct and enforced; its
wall-clock benefit over B is modest only because A's constant factor is high. Raw numbers +
interpretation: `docs/plans/2026-09-01-graph-direct-ir-algorithms/bench-raw.txt`.

## 11. Allocation, bytes/incidence, peak RSS

Warm query: **0 allocations**, proven by a counting global allocator over 64 warm `reachable_nodes`
queries in an isolated single-test binary (`direct_ir_alloc.rs`). Retained: 12.2–12.8 B/incidence
(index; real 8-byte `HyperArcId`). Peak RSS well under 16 GiB (1e6-incidence Ir + star + clique in one
process; no cap approached).

## 12. Serial / Rayon scaling

`batch_reachable_counts` (serial, one warm workspace) and `par_batch_reachable_counts` (Rayon,
per-worker thread-local workspaces via `map_init`) return **identical** results (same order, same
values — `batch_reachable_serial_equals_parallel_and_singletons`). Parallelism changes speed, never the
answer. Per the mission, parallelism is offered, not forced; the Rayon floor vs the serial warm path is
a per-workload choice.

## 13. P-graph regression oracle

`hymeko_pgraph` `workloads_suite` (FNV output-hash oracle) and `book_validation` (Pimentel examples)
pass unchanged in the 434 — the P-graph output hash is stable across the core bump.

## 14. Duplication removed / long functions

The shared source-first harness was lifted to `tests/direct_ir_common/mod.rs`; `direct_ir_vv.rs` was
refactored to reuse it (duplicate harness removed). One alternating `expand` step is the sole
node→arc→node body; `reachable_nodes`, `berge_shortest_path`, `connected_components` and the enumerator
all drive it (no duplicated BFS). No function exceeds the §6.2 ceilings. No §6.5 anti-patterns
introduced (no Cartesian API, no string-typed config, no algorithm behind a binding boundary, no
`_v2` files, no global mutable state).

## 15. Hotaru / Nagare ownership audit (the load-bearing finding)

**§0.1 doc-vs-code finding:** the brief assumed *general hypergraph traversal → hymeko_graph,
domain-specific → Hotaru*, but the code disagrees. **Hotaru already owns a mature GENERAL hypergraph
suite** (Berge paths/cycles/components, s-connectivity, dual, Levi/2-section, transversals,
α-acyclicity, signed/directed traversal — 151 tests) on its **own** signed-CSR `Hypergraph`, reached
from the Ir only through an optional feature-gated **copy**. **Nagare** carries a second copy-based
model (`HyperGraphView`, the live HNN numeric path) plus a **dormant** Berge/Levi topological traversal
(0 production call-sites, 1 test) that sits **downstream** of hymeko_graph. Neither traverses the
canonical Ir **zero-copy**. The non-duplicated primitive we built is exactly that — zero-copy on the
canonical Ir — at the correct layer (upstream of Nagare, parallel to Hotaru). We did **not** clone
Hotaru's suite. Hotaru and Nagare were **not written to** this session. Migration/reuse items recorded:
R1 Nagare's dormant traversal can be subsumed by this upstream one; R2 Hotaru could gain a zero-copy
`IncidenceIndex` backend; R3 `hymeko_pgraph::lowering` could re-express its producers/consumers over
`IncidenceIndex`. Full inventory: `docs/plans/2026-09-01-graph-direct-ir-algorithms/ownership-inventory.md`.

## 16. Own mistakes / retracted hypotheses / measurement errors

- `gen` is a reserved keyword in edition 2024 — the workspace's generation field was renamed `stamp`.
- The zero-alloc test first lived beside an allocating large-scale test in one binary; Rust runs a
  binary's tests in parallel and the global allocator counts every thread, so the measurement was
  corrupted. **Retracted** and isolated into its own single-test binary — the honest fix, not a
  loosened assertion.
- I did **not** assume the direct path would match the sparse-star speed; the benchmark showed a ~13×
  gap, and I isolated the cause (A ≈ B ⇒ fat-struct traversal, not arc-expansion count) rather than
  asserting a convenient explanation. No reflexive precompute was added (the mission and §6.5 forbid
  optimizing before profiling / adding topology state).
- Bumping core to 70b4f36 broke one pgraph white-box test (`meta_resolve_ir_faults`) that poked the
  now-sealed `decl_to_*` fields; this was a real, reported fallout, fixed by migrating the test to the
  public `push_decl` + `bind_*_decl` builder (26 tests, same intent, nothing weakened).

## 17. Next step — dynamic Ir transaction index update

When the mutable/transactional IR lands, promote `IncidenceIndex` to a stored index rebuilt/patched at
transaction commit (coherence-validated) so repeated node-major queries need no rebuild — the core
corrective's deferred item, now with a live consumer (`direct_ir`). A companion, explicitly **not**
done here (it would betray zero-copy if made the default): an operator-side flat `Vec<DeclId>` target
cache to close the ~13× warm-query gap to the flat CSR, trading losslessness/coherence for speed.

## 18. Clean worktree and residual processes

Worktree `/d/hnnwork/graph-direct` clean at HEAD `038c5af`; the fresh worktree was created from graph
remote `main` (5078980), no foreign dirty/untracked files touched. No stale torch/long-running
processes (this is a Rust build; nothing backgrounded). Amaterasu second-host run: **not performed**;
a quiet-host re-measurement is advisable before publishing the absolute benchmark numbers.

## Provenance

rustc 1.97.1, x86_64-pc-windows-msvc; cargo-llvm-cov 0.9.0. Core `main` = 70b4f36 (fast-forward,
remote-verified). Graph `main` = 5078980 (untouched, not merged). Random inputs: deterministic
generators (sliding-window; string builders); no system entropy for seeds.
