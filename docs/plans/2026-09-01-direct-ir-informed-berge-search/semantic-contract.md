# Semantic contract — direct-Ir informed Berge search

Created-at 2026-09-01 20:25 CEST · STRATEGY-ID HYMEKO-PLATFORM-2026-09-01-v1 · ROADMAP-TASK P1.1
APPROVED-GRAPH-EDIT: direct-ir-informed-berge-search

This is the binding contract the implementation (`hymeko_graph::direct_ir::informed`) and its tests
enforce. It fixes the search state space, the cost convention, the policy space, and the determinism
rules **before** any code.

## 1. Path-notion separation (mission §1 — do not conflate without proof)
- **Classical Berge path** — an alternating `node, hyperedge, node, …` walk; two nodes are adjacent iff
  they share a hyperedge. Sign/weight/direction-agnostic. This is what `berge_shortest_path` (BFS) and this
  layer's **undirected** policy compute.
- **Implicit bipartite incidence path** — the same walk viewed on the Levi (node/edge bipartite) graph.
  We traverse it *implicitly* (`node → (arc,slot) → node`) and **never materialise** the Levi graph.
- **Directed B/F-hyperpath** (Gallo–Longo–Pallottino–Nguyen 1993) — a directed hyperpath where a
  hyperarc's head is reachable only once its whole tail is. This layer's **directed** policies model the
  *edge-orientation* of a Berge step (producer→consumer), which is a per-incidence orientation filter,
  **not** the full B-hyperpath AND-of-tail semantics.
- **AND/OR requirement / P-graph producibility** — an operating unit is producible iff **all** its inputs
  are producible (an AND over the tail). This is **strictly stronger** than any single Berge path and is
  **out of scope** for this search layer: it stays in `hymeko_pgraph`'s `close_producible` fixpoint
  (audit: `msg.rs:198` `.all()`). A* may bound/order/prune P-graph search but **cannot** replace the
  AND-closure. This document's search computes shortest Berge paths under a per-incidence orientation
  policy; it does **not** claim producibility.

## 2. Berge state space (§3.1)
The search runs on the **implicit incidence state space** of the canonical `Ir`:
```
node u (DeclId)
  → incident hyperarc e            via IncidenceIndex::incident_arcs(u)
  → exact incidence (e, slot)      via ir.arcs[e].refs[slot]  (the Bᵀ coordinate)
  → target node v (DeclId)         via ref.target()
```
**Forbidden** (enforced by construction + `no explicit graph` review): building an explicit Levi graph;
materialising a sparse-star / clique / 2-section CSR for the standard search; dropping a sign, weight,
slot, or duplicate incidence; reinterpreting a canonical `DeclId`/`HyperArcId` as a dense local id.

Every emitted step is the existing `BergeStep { arc: HyperArcId, slot: u32, sign: i8, node: NodeId }`
(traversal.rs:235) and **resolves back** to a full `IncidenceRef` via `IncidenceIndex::resolve(arc, slot)`
— sign, weights and annotation recoverable, nothing dropped. Duplicate incidences and self-incidence
follow the same fixed topology semantics as the BFS engine (a duplicate does not create a second settled
node but stays observable via `degree`/`incident_slots`; a self-incidence terminates).

## 3. Cost convention (§3.2)
Cost is a **typed, total** function of a *transition into a target incidence*, chosen once before the loop
(a generic type parameter / sealed enum — **never** a runtime mode boolean, **never** `Box<dyn>` in the hot
loop). Contract:

- **Finite, non-negative.** `NaN`, `±∞`, and any value `< 0` are a typed `InformedSearchError::BadCost`
  — checked at the point the cost is produced, never swallowed by `partial_cmp`.
- **Exit-independent separable (primary model).** The incremental cost of arriving at `v` via `(e, slot_v)`
  depends only on `(e, slot_v)` — i.e. on the *target* incidence and the arc — not on which member `u` the
  search arrived from (beyond `g[u]`). Formally `c(u→e→v) = w_arc(e) + w_entry(e, slot_v) ≥ 0`.
  - `UnitCost`: `w_arc = 1`, `w_entry = 0` → every hyperedge hop costs 1 (Berge distance). A* reduces to
    the existing BFS length under a zero heuristic.
  - `IncidenceWeightCost`: `w_arc = base ≥ 0`, `w_entry = |weight(e, slot_v)|` from the resolved
    `IncidenceRef.weights` (first `Num`, else 0). Non-negative by `abs`.
  - `MetricStepCost` (coupled with the metric heuristic, §Heuristics): `c = ‖pos(u) − pos(v)‖`. This is
    **exit-dependent** (depends on `u`), so it sets the associated const `EXIT_INDEPENDENT = false` and the
    engine uses per-member relaxation (no arc-activate-once). Correct, but `O(arity)` per incident arc.
  - `DomainCost` (generic/callback): a caller closure `Fn(&IncidenceRef) -> Result<f64, ()>`; the caller
    asserts exit-independence via the const, or accepts the general path.
- **Why exit-independence matters.** When it holds, an arc can be *activated once* — at the first
  (cheapest-`g`) settled member — because `min_u (g[u]) + w_arc(e) + w_entry(e,slot_v)` is minimised by the
  first Dijkstra-settled member. This reuses the BFS `visited_arcs` discipline and keeps the walk
  `O(nodes + arcs + incidences)`, avoiding the `O(k²)` re-expansion the direct_ir report warns about. When
  it does not hold (metric/general), the engine relaxes each member (documented `O(arity)`), still correct.

## 4. Transition policy (§3.2) — which steps are legal (orthogonal to cost)
A typed policy (generic / sealed enum, chosen once) decides whether the step `u →(e,slot_v)→ v` is allowed,
from the two incidence signs (`sign_u`, `sign_v`) it can resolve:
- `Undirected` — always allowed (classical Berge). Default.
- `SignCompatible` — allowed unless the two incidences are sign-incompatible under the chosen rule
  (e.g. same-sign only, or opposite-sign only); `~` (neutral, sign 0) is permissive.
- `DirectedProducerConsumer` — `u` must be an output/producer incidence (sign `−` by the P-graph
  convention) and `v` an input/consumer incidence (sign `+`). Models edge orientation of one Berge step.
- `DirectedConsumerProducer` — the reverse orientation (for backward/reverse search from a goal).
- `DomainPolicy` — a caller closure over the two `IncidenceRef`s.

Policy is a **filter only**; it never changes a cost. `~`/neutral handling is explicit per policy.

## 5. Determinism (§3.2, §6 key)
Floating comparison is **`f64::total_cmp`** (never `partial_cmp`, which hides `NaN`). The priority-queue key
is the lexicographic tuple, all canonical:
```
( f = g + h_admissible   (total_cmp),
  discrete_tiebreak       (u32, smaller first; 0 when unused),
  canonical DeclId(node)  (raw usize),
  canonical HyperArcId(pred arc),
  slot (u32) )
```
Equal-cost paths are therefore resolved identically across runs, hosts, and declaration orders that
preserve canonical ids. A path hash (BLAKE3 over the ordered `(arc,slot,sign,node)` tuples + total cost
bits) pins the exact result in tests.

## 6. A* ↔ Dijkstra ↔ BFS equivalences (proved by test, §7)
- **Zero heuristic ⇒ Dijkstra.** `h ≡ 0` makes `f = g`; the engine is uniform-cost search. Test:
  `astar(Zero) == dijkstra` on every oracle cell (path cost and, up to the deterministic tie-break, the
  path).
- **Unit cost + zero heuristic ⇒ BFS Berge distance.** Every hop costs 1, so the optimal `g[goal]` equals
  the existing `berge_shortest_path` hyperedge length. Test: equal length on every reachable pair.
- **Admissible heuristic ⇒ same optimal cost as Dijkstra.** For any admissible (and, for the no-reopen
  contract, consistent) heuristic, `astar(h).cost == dijkstra.cost` on every cell.
- **Unreachable ⇒ `Ok(None)`.** Malformed index (non-node target) ⇒ `Err(TargetNotANode)`; never a panic.

## 7. Reopen contract (§7)
The engine is **Dijkstra with a consistent-heuristic fast path**: a node is settled once (closed), and a
consistent heuristic guarantees the first pop is optimal (no reopen needed). If a heuristic is only
*admissible but not consistent*, optimality under a no-reopen engine is not guaranteed; the contract is:
**the optimal-mode engine accepts only heuristics that declare `CONSISTENT = true`** (a compile-time
associated const). An inconsistent heuristic used in optimal mode is a typed
`InformedSearchError::InconsistentHeuristicInOptimalMode` (fail-closed) — we choose *typed rejection* over
silent reopen, so no run can quietly return a sub-optimal path. (A future bounded-suboptimal mode may relax
this; not in this phase.) Inadmissible heuristics are usable only through an explicit non-optimal entry
point (not built this phase; classified in the inventory).

## 8. Memory / complexity model
- Workspace state is **dense, DeclId/HyperArcId-indexed, generation-stamped** — `g[]` (f64), `pred_*[]`,
  a `settled` stamp, and a binary-heap `Vec` — all retained across queries; a warm query does **zero heap
  allocation** (heap `Vec::clear` keeps capacity; the generation bump clears marks in O(1) amortised). No
  per-node/per-incidence `HashMap`. Memory is `O(nodes + arcs)` (the dense arrays) plus the transient heap
  `O(open set) ≤ O(incidences)` — **not** `O(V×E)`.
- Time: exit-independent cost ⇒ `O((nodes + arcs) log nodes + incidences)` (arc-activate-once);
  general cost ⇒ additional `O(arity)` per incident arc. Lazy heap deletion (push-on-relax, skip stale
  pops) avoids a decrease-key index.

## 9. Rollback / HARD STOP
Additive on `feat/direct-ir-informed-berge-search`; not promoted. If any oracle disagrees, any warm query
allocates, any untouched direct_ir op regresses >3%, or any forbidden artefact (Levi/star/clique on the
standard path, second engine, `Box<dyn>` hot dispatch, `partial_cmp` on cost) appears → drop the offending
change, do not promote. Do **not** begin production PGRAPH migration or P2 operator-precompute.
