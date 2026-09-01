# P1.4 — graph/pgraph query performance matrix (+ Phase-3 audit)

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P1.4 · **APPROVED-GRAPH-EDIT:**
graph-query-performance-matrix. 2026-09-02. Branch `feat/graph-query-performance-matrix` @ base P1.3
`b234c48`; **not merged**. Single host (Win11, RTX 5080 laptop, rustc 1.97.1 release), `median_us`
(median-of-9), ambient not pinned ⇒ wall PROVISIONAL; exact output and allocation results are reliable.

## Scope (honest)
P1.3 changed **only** the axiom-check adjacency path. The graph/search operations (`reachable_nodes`,
`connected_components`, `s_connected_components`, Berge BFS/shortest-path, Dijkstra, A* metric, A*+discrete)
live in `hymeko_graph`, are **P1.1-measured**, and are **P1.3-invariant** (untouched) — they are not re-run
here. This matrix measures the two pgraph operations that could move: `AxiomBundle::validate` (the path
whose `build_forward_adj`/`build_producers` rebuilds P1.3 removed) and the BooleanExact closure, across
three workload families at small/medium/large. Families are **not averaged together**.

## A/B — A = P1.2 tip (own adjacency rebuilds), B = P1.3 (delegated). Same source, exact output identical.
`AxiomBundle::validate` µs (validity + producible count identical on both — anchored inline):

| family | size | units | A µs | B µs | B/A |
|---|---|--:|--:|--:|--:|
| chain | small | 99 | 558 | 474 | 0.85 |
| chain | medium | 999 | 69 504 | 64 385 | 0.93 |
| chain | large | 3999 | 1 205 849 | 1 144 059 | 0.95 |
| OR fan-out | small | 50 | 8.0 | 3.2 | 0.40 |
| OR fan-out | medium | 500 | 105 | 36 | 0.34 |
| OR fan-out | large | 3000 | 705 | 240 | 0.34 |
| AND fan-in | small | 1 | 7.6 | 9.0 | 1.18 |
| AND fan-in | medium | 1 | 421 | 738 | 1.75 |
| AND fan-in | large | 1 | 16 326 | 25 692 | 1.57 |

BooleanExact closure (for scale, unchanged by P1.3): chain large 302 µs vs the axiom `validate`'s 1.14 s —
the axiom A4 does a per-O-node reachability BFS (`O(units·edges)`, quadratic on a chain), while the AND/OR
worklist closure is `O(n)`; the closure is ~3800× cheaper on the deep chain (different operations, not a
substitution — A* connectivity is not producibility).

## Verdict (§2.3 acceptance)
- **Exact output identical:** ✓ (valid + producible counts match A and B on every cell).
- **Timing:** B is faster on chain (5–15%) and OR fan-out (60–66%) — the removed `build_forward_adj`/
  `build_producers` rebuild wins where there are many O-nodes and few materials. On AND fan-in (1 unit, many
  materials) B is **provisionally slower** (18–75%): `check_a2`'s per-material `schema.predecessors(m)`
  lookup costs slightly more than the pre-built `producers.get(m)` on a map with one entry. This is
  single-host, no A/A-null control ⇒ **PROVISIONAL, not a proven >3% regression**; it is a real fixed-cost
  shift worth noting, and the follow-up (`LoweredPGraph`-borrows-Ir, §P1.3 report) would replace the
  `schema.predecessors` lookup with an `IncidenceIndex::degree`-style O(1) query.
- **Retained topological memory:** **unchanged** — P1.3 kept `schema.incoming/outgoing` (its removal is
  deferred). What P1.3 removed is the **per-`validate` rebuild allocation** (two `BTreeMap`s +
  `O(edges)` nodes), provable from the diff; the retained-bytes reduction is the deferred follow-up.
- **Hot search/lookup zero-alloc:** the AND/OR warm loop is allocation-free (P1.2 alloc test); the axiom
  `reaches_any` still allocates a per-call `visited`/queue (unchanged from A, minus the two removed
  rebuild maps).

## Phase-3 read-only architecture / duplication audit
- **Traversal/fixpoint/queue/visited duplication:** after P1.3, `reaches_any` exists **once**
  (`axioms::reaches_any`, shared). The AND/OR worklist (`andor::propagate`) and the direct_ir traversals are
  **distinct constructions** (producibility AND-closure vs Berge connectivity) — not duplicates. The
  `visited`-generation pattern appears in `direct_ir::TraversalWorkspace`, `andor::AndOrWorkspace`, and the
  ad-hoc `reaches_any` `BTreeSet` — a *generalizable* dense-generation-stamp mechanism, but generalizing it
  across the graph/pgraph boundary is a **next-phase** candidate (not an opportunistic refactor: it would
  cross crate ownership and needs its own parity/bench proof).
- **Functions > 80 LOC:** none introduced by P1.2/P1.3 (`andor::propagate` ≈ 55 lines; `view` methods small).
- **Per-call `Vec`/`HashMap`:** the AND/OR loop reuses the workspace (no per-call alloc). `reaches_any`
  allocates a per-call `visited`+queue — acceptable (axiom checks are not a warm hot path); folding it into a
  reusable workspace is a minor next-phase win.
- **Domain vs topology mixing:** P1.3 improved the separation (axioms now query the single schema adjacency;
  the AND-closure/classification/cost stay domain). No new mixing.
- **Benchmark generator duplication:** the `<isa>` chain/source helpers recur across `andor_bench`,
  `andor_alloc`, and `perf_matrix` — a **real** small duplication; consolidating them into one shared
  test-support module is a clean next-phase simplification (deferred here to avoid churn mid-overnight).

**Audit conclusion:** no opportunistic refactor is implemented this phase; the two genuine candidates
(shared visited-generation workspace across the boundary; shared `<isa>` bench source helper) are recorded
as next-phase proposals, each requiring its own parity + perf proof.

## Provenance / HARD STOP
rustc 1.97.1; pulls `hymeko_core` == `5fe2c88`. Branch pushed + verified, not merged. The A/B scratch bench
copy in the P1.2 worktree was removed after measurement (the worktree itself preserved). HARD STOP on this
phase; P2 proceeds on the core branch only after P1.3 is green + pushed (it is).
