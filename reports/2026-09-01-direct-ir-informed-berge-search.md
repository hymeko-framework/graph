# P1.1 — direct-Ir informed Berge search + P-graph extension audit

**GOVERNING-STRATEGY:** ../core/docs/hymeko-platform-strategy.md · **STRATEGY-ID:**
HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P1.1. 2026-09-01 23:47 CEST.
**APPROVED-GRAPH-EDIT:** direct-ir-informed-berge-search. Additive graph feature + read-only pgraph audit;
branch `feat/direct-ir-informed-berge-search`, **not promoted** to main. Core production source untouched;
`hymeko_pgraph` not modified.

## 1. P1 promotion gate (§0)
Verified 4 refs by `ls-remote`; ancestry `2ebb833 ≺ 5fe2c88`, `cbaadd4 ≺ c35b9f2`; re-ran the P1 targeted
gates (core fmt/clippy/KAT, graph fmt/clippy/pgip) green; **fast-forward-only** promotion **core main
`2ebb833..5fe2c88`**, **graph main `cbaadd4..c35b9f2`**. New branch off the promoted graph main.

## 2. New graph commits (feature branch; remote verify in the return summary)
`c75b601` contracts (plan bundle) · `12147ce` freeze oracles · `85a5b18` A* engine (token) · `4c60511`
rustfmt oracle harness · `b9aedc0` benchmark · pgraph audit doc · this report.

## 3. A* API + state / cost semantics (§3)
```rust
pub fn astar_berge_path<C: BergeCost, H: BergeHeuristic, P: TransitionPolicy>(
    traversal: &HyperTraversal<'_,'_>, start: NodeId, goal: NodeId,
    cost: &C, heuristic: &H, policy: &P, ws: &mut InformedSearchWorkspace,
) -> Result<Option<WeightedBergePath>, InformedSearchError>;
pub fn dijkstra_berge_path<C,P>(…) // = astar with ZeroHeuristic
```
- **State space:** the implicit incidence space `node → incident arcs → (arc,slot) member → node`,
  reusing the same primitives as the BFS engine (`IncidenceIndex::{incident_arcs,incident_slots,resolve}`
  + `ir.arcs[a].refs`). **No** Levi/star/clique materialised; nothing dropped — each `BergeStep{arc,slot,
  sign,node}` resolves back to a full `IncidenceRef`.
- **API boundary is canonical `NodeId`** in/out (matching `berge_shortest_path`; the mission's proposed
  `DeclId` signature was refined to `NodeId` for consistency with the existing engine — the internal walk
  runs in `DeclId` space). `WeightedBergePath{start, steps, total_cost}` + a dependency-free FNV `path_hash`.
- **Cost** (`BergeCost`, monomorphised, no runtime mode boolean / no `Box<dyn>` hot dispatch): finite,
  non-negative; `NaN`/`±∞`/negative ⇒ typed `BadCost`. Primary model **exit-independent separable**
  (`UnitCost`, `IncidenceWeightCost`): `EXIT_INDEPENDENT = true` ⇒ an arc is activated once at its
  first-settled member (`O(nodes+arcs+incidences)`). `MetricStepCost` is exit-dependent
  (`EXIT_INDEPENDENT = false`) ⇒ per-member relaxation.
- **Policy** (`TransitionPolicy`, filter only): `Undirected` / `SignCompatible` /
  `DirectedProducerConsumer` / `DirectedConsumerProducer`.
- **Determinism:** key `(f = g+h via f64::total_cmp, discrete_tiebreak, DeclId, HyperArcId, slot)`;
  `total_cmp` everywhere (never `partial_cmp`). Lazy-deletion binary heap; **warm query allocation-free**
  (proved by `informed_search_alloc.rs`: 0 allocations across 64 warm unreachable queries); dense
  generation-stamped `g/pred/settled/arc-active` arrays, **no per-node/per-incidence HashMap**.

## 4. Heuristic classification + proofs (§6; full table in `docs/plans/…/heuristic-inventory.md`)
Implemented **C1 (admissible + consistent, proven)**: **Zero** (`0 ≤ h*`, `0 ≤ c+0`; = Dijkstra) and
**Metric/Euclidean** (triangle inequality; admissible+consistent with a metric cost, and — as a bonus,
proven here — also with unit cost, since `‖·‖₂ ≤` Manhattan `= h*` on a grid and `|Δ‖·‖| ≤ 1 = c`). The
benchmark's **Manhattan** heuristic is C1 on a 4-connected unit grid (tight, `h=h*`). **Component-membership
pre-check** is an admissible early reject. The **hypergraph-discrete** quantities (degree, arity, cut,
s-connectivity, sign distribution, dual distance) are classified **C3 tie-break-only** — the mission's §6.2
warning that they are *not automatically admissible* is honored; none enters `f`. Combination rule:
`max(h₁,…)` (addition only with a disjoint-additivity proof — none claimed). The engine enforces the
boundary mechanically: `astar_berge_path` **refuses** any heuristic with `CONSISTENT = false`
(`InconsistentHeuristicInOptimalMode`) — fail-closed, never a silent sub-optimal path.

## 5. Dijkstra / BFS / A* correctness (§7; `informed_search_vv.rs` 14 tests + `informed_oracles.rs`)
Against **independent** oracles (a separate linear-scan Dijkstra + an exhaustive Berge-path enumerator, no
shared code with the engine): `A*(Zero).cost == oracle Dijkstra == exhaustive` (unit + weighted);
`A*(unit).length == BFS berge_shortest_path length`; `A*(metric).cost == Dijkstra.cost` and expands a
**subset**; `Zero == Dijkstra` path-hash + determinism; boundary (start==goal, unreachable, disconnected,
invalid start, singleton/self-incidence); directed-policy restriction; declaration-order + disjoint-union
metamorphic; duplicate-incidence cost-preserving + observable; typed `BadCost` and inconsistent-heuristic
rejection. **Fault-injection** (mutate engine, named test catches, no mutant committed, git-clean restore):
M1 consistency-gate removed → caught; M2 inverted relaxation → caught; M3 dropped step cost → caught; M4
directed-policy always-true → caught; **M5′** generation-stamp not advanced → caught. **Honest finding:**
removing `heap.clear()` (M5) is **not** a correctness fault — the engine's correctness rests on the
generation-stamped `g`/`settled` arrays (expansion always reads current `g`), so a stale heap entry only
causes a redundant pop, never a wrong result. That is a robustness property, not a gap.

## 6. D* Lite / LPA* decision (§8) — **DEFERRED**, reasoned
Decision matrix: static graph + single query → **A*** (this phase); same goals, changing cost/topology →
LPA* candidate; moving start + changing cost → D* Lite candidate; AND/OR producibility → a **separate
domain model**, not simple Berge D*. **Production incremental state is out of scope and forbidden this
phase** (§14): there is no canonical transaction/snapshot layer, and a fresh `IRDelta`/mutable-graph system
must not be created. A benchmark-only prototype (two immutable before/after `Ir` snapshots + an explicit
`ChangedIncidence` list — *not* a canonical delta; query-workspace state `g/rhs/queue/version` only, no
second adjacency) was **designed but not built**, because its numbers are not actionable without the
production incremental path, and building it risks the very snapshot-framework §14 forbids. **Analytical
break-even** from the measured A* data: a fresh A* to a `d`-cell goal expands `≈ d` states (grid: 21/41/81
for d=20/40/80); an LPA*/D* Lite replan after a **localized** change re-expands only the affected cone,
`≈ (changed radius)²/…` bounded by the same `d`; so incremental replanning pays only when (a) many queries
(`≳ 10¹`) share a goal region **and** (b) each change is local **and** (c) the snapshot/transaction layer
exists to feed `ChangedIncidence` soundly. None holds today. **Verdict: DEFERRED**; revisit when the
canonical snapshot layer lands, then build the benchmark prototype first and gate on a measured break-even
query count.

## 7. Query latency + time-to-first (§9, §11) — measured, `informed_search_bench.rs`
Single host (Win11, RTX 5080 laptop, rustc 1.97.1 release), `median_us` (median of 9, warm); ambient not
pinned ⇒ wall PROVISIONAL, **expanded-state counts are exact/host-independent**. For A*,
`time_to_first == total` (the optimal goal leaves the queue at completion).

**Grid 200×200, unit cost, start `(100,50)`, goal `d` cells right (large grid ⇒ Dijkstra explores a
Manhattan disk, A* a cone):**

| d | Dijkstra µs | Dijkstra expanded | A*(L1) µs | A*(L1) expanded | L1 speedup(exp) | A*(L2) expanded | L2 speedup |
|--:|--:|--:|--:|--:|--:|--:|--:|
| 20 | 52.9 | 802 | 1.5 | 21 | **38.2×** | 21 | 38.2× |
| 40 | 235.9 | 3202 | 3.0 | 41 | **78.1×** | 41 | 78.1× |
| 80 | 1207.8 | 11931 | 6.4 | 81 | **147.3×** | 81 | 147.3× |

A* cost == Dijkstra cost (asserted). L1 and L2 coincide here (the optimal path is a straight segment, so
both heuristics are tight along it). Wall speedup tracks the expanded-state ratio (35–190×).

**Window (arity 5, unit) — the uninformative-heuristic / nothing-to-prune case:**

| m | nnz | BFS µs | Dijkstra µs | cost |
|--:|--:|--:|--:|--:|
| 1 000 | 4 980 | 13.4 | 34.2 | 250 |
| 10 000 | 49 980 | 200.7 | 401.0 | 2 500 |
| 100 000 | 499 980 | 8 217 | 11 183 | 25 000 |

## 8. Expanded state / queue / memory / allocation (§11, §12)
- **Expanded state:** the headline metric (table above); A* is a strict subset of Dijkstra's expansions
  under a consistent heuristic (proved by construction: only nodes with `g+h ≤ C*` settle).
- **Allocation:** warm query **0 allocations** (alloc test). A *successful* query's only allocation is the
  returned `Vec<BergeStep>` (the owned result, `O(path length)`), unavoidable and documented.
- **Memory:** `O(nodes + arcs)` dense arrays + a transient heap `O(open set) ≤ O(incidences)` — **not**
  `O(V×E)`. No materialised adjacency.
- **Untouched-op regression (§12 ≤3%):** warm `reachable_nodes` at 1e6 nnz measured **≈14.8 ns/incidence**,
  tracking the graph performance-report baseline (13–14 ns/inc); the informed module is additive and
  changes no existing op — **no regression**.
- **§12 A*-retention gate:** ✓ optimal cost identical on every oracle cell; ✓ warm allocation-free; ✓ no
  materialised adjacency; ✓ ≥2× fewer expanded (38–147×) on the metric-friendly grid; ✓ ~0 overhead vs the
  Dijkstra engine for an uninformative heuristic (`h≡0` is a no-op — A*(Zero) *is* Dijkstra); ✓ memory
  `O(nodes+arcs)`; ✓ no untouched-op regression.

## 9. Break-even query count (§11)
The metric heuristic's precompute is `O(1)` (goal-position lookup at `MetricHeuristic::new`); the
`PositionProvider` is caller-owned (its construction is amortized across all queries and reported
separately, not folded into a warm query). So per-query the heuristic pays for itself immediately on any
metric-friendly workload (break-even = **1 query**). For the incremental (LPA*/D*) line, the break-even is
**deferred** (§6): it depends on the unbuilt snapshot layer.

## 10. Winning heuristic per workload
- **Metric-friendly, branching (grid / spatial), near start-goal in a large space:** A*(metric) — 38–147×
  fewer expanded, 35–190× faster.
- **Uniform / non-metric chain or window, single optimal path, nothing to prune:** **BFS** for unit cost
  (its FIFO beats the heap); A*/Dijkstra pay for the priority queue with no pruning to recover it. The
  benchmark *selects* this correctly — the goal is not that every heuristic always wins.
- **Metric-misleading positions:** A*(metric) stays correct (admissible) but loses its pruning edge → falls
  back toward Dijkstra's expansion count (no worse than Dijkstra by the subset property).

## 11–13. PGRAPH audit (full narrative: `docs/pgraph-as-graph-extension.md`)
**Duplicated topology inventory:** `schema.incoming/outgoing/edges` (`schema.rs:84,92,95`),
`in/out_degree` O(E) scans (`:210,215`), adjacency accessors (`schema.rs:172,184`; `lowering.rs:122–160`),
`build_forward_adj`/`build_producers` (`axioms.rs:340,353`; the `axiom_extensions.rs:221` copy is
**verbatim duplicate**), and `reaches_any` OR-reachability BFS **defined twice verbatim**
(`axioms.rs:363`; `axiom_extensions.rs:229`) — the clean cat-6 target for `direct_ir::reachable_nodes`
(needs a **directed** variant, which the P1.1 policy layer models). **Target architecture:**
`PGraphView<'ir,'idx>` borrowing `HyperTraversal` for all adjacency/degree/reachability, with `friedler.rs`
(DFS in graph via `CyclePruner`, axiom predicate in pgraph) as the exemplar. **Stays PGRAPH-domain:** the
AND-closure feasibility kernel (`close_producible` `.all()` `msg.rs:198`, `is_feasible_base`,
`ssg_dm::recurse`, ABB reachability bound), M/O/raw/product classification (incl. the `<isa>` `IsaWalk`),
and cost/multi-objective/regime policy. **A Berge path is connectivity, not producibility** — A* may bound/
order/prune ABB but never replaces the AND-closure; a tighter bound would be a pgraph-side AND/OR (Knuth)
hyperpath borrowing `direct_ir` topology, not a second engine.

## 14. Own mistakes / retracted hypotheses
- **Grid corner-to-corner showed no pruning (exp x = 1.00).** First benchmark placed start/goal at opposite
  corners; on a uniform grid *every* node lies on some shortest corner-corner path, so `f = C*` everywhere
  and A* expands all. Retracted that setup; the correct demonstration is **near start/goal in a large
  grid** (Dijkstra a disk, A* a cone) — 38–147×.
- **Euclidean heuristic ≠ pruning on a Manhattan grid.** Realized (and proved) Euclidean underestimates
  Manhattan remaining cost, so it is admissible but loose; used the tight L1 heuristic for the headline and
  reported both.
- **Mutation-restore footgun (recurred from P1, then avoided):** the fault sweep used `git checkout` to
  restore, but the engine was already committed, so no uncommitted work was lost this time.
- No architecture claim changed; the strategy §11 retractions stand.

## 15. Production PGRAPH-migration — approval scope + entry conditions
**Scope of a future migration approval:** the adjacency/degree/reachability **delegation only** — introduce
`PGraphView` (borrowing `IncidenceIndex`), remove the duplicated adjacency + the two `reaches_any` copies
in the same change, **never** touching the AND-closure feasibility kernel, classification, or cost/regime
policy. **Entry conditions:** (1) a V&V'd `direct_ir` **directed reachability** primitive exists; (2)
`PGraphView` lands additively with a **byte-identical parity gate** (six P-graph dumps, `.pgip` round-trip,
MSG/SSG/ABB results identical per fixture before/after); (3) an explicit `APPROVED-GRAPH-EDIT` **and** the
pgraph-core approval (pgraph is core in this repo's CORE.YAML); (4) no long-lived double ownership.

## Provenance / HARD STOP
rustc 1.97.1, x86_64-pc-windows-msvc; pulls `hymeko_core` == core `5fe2c88`. Graph V&V: fmt clean; clippy
`--workspace --all-features --all-targets -D warnings` green; workspace tests **466/0/6** (+22 vs the
promoted main's 444); informed suite 14 vv + 3 oracle + 1 alloc + 3 unit; pgip round-trip 6/6 (pgraph
untouched). Branch **not promoted**; core/pgraph/Nagare/Hotaru production untouched. **HARD STOP** — no
production PGRAPH migration, no P2 operator-precompute.
