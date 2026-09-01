# P-graph as a domain extension of `hymeko_graph` (read-only audit)

**GOVERNING-STRATEGY:** ../core/docs/hymeko-platform-strategy.md · **STRATEGY-ID:**
HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P1.1. 2026-09-01.

**This is a read-only architectural + performance audit. No `hymeko_pgraph` production code is changed in
P1.1.** It records the target ownership split and the exact migration surface; the migration itself is a
**separate, approval-gated phase** (entry conditions at the end).

Target dependency/ownership:
```
hymeko_core::Ir  ──►  hymeko_graph::direct_ir  ──►  hymeko_pgraph (domain extension)
 (canonical)          (all adjacency/reach/search)    (classification + AND-closure + cost/regime)
```

## 1. The load-bearing distinction (do not conflate)
- **Berge connectivity / shortest path** (this phase's `direct_ir::informed`): two nodes are adjacent iff
  they share a hyperedge; a path is connectivity evidence. **Sign/weight/direction-agnostic** by default.
- **P-graph producibility**: an `OperatingUnit` is producible iff **all** its inputs are producible — an
  **AND over the tail** (a directed B-hyperpath / AND-OR requirement, Gallo–Longo–Pallottino–Nguyen 1993).
  This is **strictly stronger** than any single Berge path.

A Berge path (or an A* lower bound) can **bound, order, or prune** a P-graph search, but it **cannot
replace** the axiomatic AND-closure feasibility check (MSG/SSG/ABB). The audit found this AND-semantics
enforced and correct in pgraph, and it **must stay**:
`msg::close_producible` `.all()` at `hymeko_pgraph/src/msg.rs:198`; MSG forward pass `msg.rs:145`;
`ssg::is_feasible_base` `ssg.rs:129–141`; `ssg_dm::Enumerator::recurse` `ssg_dm.rs:210–216`; the ABB
reachability bound `abb.rs:295–299`.

## 2. Duplicated topology — the migration surface (category 6 / 2)
`hymeko_pgraph` currently **materialises adjacency at least four times** and hand-rolls reachability,
none of it borrowing `IncidenceIndex`:

| what | where | → target |
|---|---|---|
| `incoming` / `outgoing` `BTreeMap<DeclId,BTreeSet>` | `schema.rs:92,95` | borrow `IncidenceIndex` (Bᵀ/B) |
| `edges` (owned copy of the signed incidence) | `schema.rs:84` | borrow `Ir.arcs` / incidence iterator |
| `in_degree`/`out_degree` (O(E) scans) | `schema.rs:210,215` | `IncidenceIndex::degree` (O(1)) |
| `predecessors`/`successors`, `inputs`/`outputs`/`producers`/`consumers` | `schema.rs:172,184`; `lowering.rs:122–160` | `PGraphView` over incidence |
| `build_producers`, `build_forward_adj` (×2, one **verbatim duplicate**) | `axioms.rs:340,353`; `axiom_extensions.rs:221` | one borrowed incidence view |
| `reaches_any` — OR-reachability BFS, **defined twice verbatim** | `axioms.rs:363`; `axiom_extensions.rs:229` | `direct_ir::reachable_nodes` (directed variant) |
| `meta_resolve::build_incidence` (rebuilds directed signed incidence) | `meta_resolve.rs:403` | borrow `IncidenceIndex`; per-unit cost stays (cat-4) |

**Caveat for the reachability migration:** `direct_ir::reachable_nodes` is **undirected** Berge
reachability; pgraph's `reaches_any` walks the **directed** schema forward. Migration needs either a
directed reachable entry point in `direct_ir` (a `DirectedProducerConsumer`-style policy over the
incidence — the P1.1 `informed` policy layer already models this orientation) or a sign-orientation filter.
This is exactly the kind of primitive `direct_ir` should own.

## 3. Must stay in pgraph (categories 3/4/5) — the domain kernel
- **AND-closure feasibility**: `close_producible`/`close_consumable` (`msg.rs:189,212`), the MSG
  reduction+composition, `is_feasible_base`, `ssg_dm::recurse`, the ABB reachability bound, and the
  transductive wrapper `reachability::close_producible_under_rule` (`reachability.rs:77`).
- **Classification**: `schema.nodes` (M/O), `raws`/`products`, `meta_resolve::classify_materials/units`
  (`:338,375`), `IsaWalk::reaches` (`meta_resolve.rs:536`) — a DFS over the `<isa>` **inheritance**
  relation, a *different* relation from the incidence hypergraph, so not a `direct_ir` duplicate.
- **Cost / multi-objective policy**: `costs`/`cost_dimensions`/`cost_vectors` (`lowering.rs:101–110`),
  `abb::effective_cost` (`:108`), `cost_weights`, and the regimes (`regime.rs`).
- **Result/scratch** buffers and **compat** endpoints (`pgip_io`, `cli`, `dump`, `builder`).

## 4. Target architecture (design only)
```rust
pub struct PGraphView<'ir, 'idx> {
    traversal: HyperTraversal<'idx, 'ir>,   // borrows Ir + IncidenceIndex: adjacency/degree/reachability
    // + P-graph classification (M/O/raw/product), cost, regime policy — borrowed, not a second graph
}
```
`PGraphView` replaces the six owned maps and the two BFS copies with borrowed incidence queries; the
AND-closure feasibility engine, classification, and cost/regime policy stay pgraph-owned. The seam already
exists: **`friedler.rs` is the exemplar** — the DFS lives in `hymeko_graph` (`CyclePruner`,
`friedler.rs:85,139`); only the axiom predicate (A0/A1/A3) lives in pgraph (`extend_ok`/`emit_ok`
`:144,162`). The rest of pgraph's topology should delegate the same way.

## 5. Should a directed B/F-hyperpath or AND/OR relaxation feed P-graph bounds? (design)
For an ABB **lower bound**, plain Berge A* is a *weak* relaxation: it proves connectivity, not
producibility, so it under-bounds the true synthesis cost and can mislead pruning. Two stronger,
producibility-aware relaxations are worth evaluating **as bounds only** (not a second search engine):
- **Backward B-hyperpath / OR-reachability from the product set** (the existing `close_producible`
  backward flavour): a *necessary* condition — a unit not on any product-reaching hyperpath can be pruned.
  This is a sound admissible pruning bound and already lives in pgraph (`abb.rs:295`); it could reuse a
  `direct_ir` directed-reachability primitive for its topology half.
- **AND/OR shortest-hyperpath (Knuth's generalisation of Dijkstra to grammar/hypergraph problems)**: gives
  a *tighter* admissible lower bound on producibility depth/cost than Berge distance, because it respects
  the AND-of-tail. This is the correct target if a stronger ABB bound is wanted — but it is a **domain**
  algorithm (it needs the AND semantics), so it belongs in pgraph consuming `direct_ir` primitives, **not**
  in the general Berge search. **No second search engine is introduced**; this is a bound design.

Verdict: keep the general **Berge** A*/Dijkstra in `hymeko_graph`; if P-graph wants a tighter bound, add a
pgraph-side **AND/OR hyperpath bound** that borrows `direct_ir` topology — a future, separately-approved
step, not part of P1.1.

## 6. Entry conditions for the production PGRAPH migration (separate approval)
1. A `direct_ir` **directed reachability** primitive (a policy over the incidence) exists and is V&V'd —
   the one capability the `reaches_any` migration needs that undirected `reachable_nodes` lacks.
2. `PGraphView` is introduced **additively** (borrowing) alongside the current `PGraphSchema`, with a
   byte-identical parity gate: the six P-graph dumps, `.pgip` round-trip, and MSG/SSG/ABB results identical
   before/after, per fixture.
3. An explicit `APPROVED-GRAPH-EDIT` (and, since `hymeko_pgraph` is core in this repo's CORE.YAML, the
   pgraph-side approval) scoped to the adjacency/reachability delegation **only** — never touching the
   AND-closure feasibility kernel, classification, or cost/regime policy.
4. The migration removes the duplicated adjacency/`reaches_any` copies **in the same change** that adds the
   borrowed view (no long-lived double ownership).
