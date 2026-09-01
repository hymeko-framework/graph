# P-graph ownership map (read-only audit input)

Created-at 2026-09-01 20:25 CEST · P1.1. This is the plan-side digest; the full narrative is
`docs/pgraph-as-graph-extension.md`. **Read-only** — no pgraph production code is changed this phase.

Target dependency/ownership: `hymeko_core::Ir → hymeko_graph::direct_ir → hymeko_pgraph domain extension`.

## Category legend
1 canonical IR data (borrow) · 2 exact sparse derived index (graph owns) · 3 P-graph classification (stays)
· 4 cost/multi-objective policy (stays) · 5 result/scratch (stays) · 6 duplicated topology (migrate to
direct_ir) · 7 compatibility surface.

## Duplicated topology / adjacency (cat 2/6 — migration targets)
- `PGraphSchema.incoming` `schema.rs:92`, `outgoing` `schema.rs:95` — owned `BTreeMap<DeclId,BTreeSet>`
  adjacency = `Bᵀ`/`B` columns/rows. → borrow `IncidenceIndex`.
- `PGraphSchema.edges` `schema.rs:84` (cat 1) — "this directed edge set **is** the signed incidence"; owned
  copy of `Ir` arc refs. → borrow `Ir.arcs` / incidence iterator.
- `in_degree`/`out_degree` `schema.rs:210,215` — **O(E) scans**. → `IncidenceIndex::degree` (O(1)).
- `predecessors`/`successors` `schema.rs:172,184`, `LoweredPGraph::{inputs,outputs,producers,consumers}`
  `lowering.rs:122–160` — adjacency accessors. → `PGraphView` over incidence.
- `axioms::build_producers` `axioms.rs:340`, `build_forward_adj` `axioms.rs:353`,
  `axiom_extensions::build_forward_adj` `axiom_extensions.rs:221` — **3rd/4th** adjacency copies (the last a
  **verbatim duplicate**). → one borrowed incidence view.
- `axioms::reaches_any` `axioms.rs:363` **and** `axiom_extensions::reaches_any` `axiom_extensions.rs:229` —
  a plain OR-reachability BFS, **defined twice verbatim** = the clean cat-6 duplicate of
  `direct_ir::reachable_nodes` (needs a **directed** reachable entry / sign-orientation filter, since
  direct_ir reachability is undirected).
- `meta_resolve::build_incidence` `meta_resolve.rs:403` — rebuilds the directed signed incidence into a
  pgraph `BTreeMap` (+ per-unit cost at `:408`, which is cat-4). → borrow `IncidenceIndex`; cost stays.

## Must STAY in pgraph (cat 3/4/5 — the domain kernel; NOT Berge connectivity)
- **AND-semantics feasibility closures** (an OperatingUnit is producible iff **all** inputs are):
  `msg::close_producible` `msg.rs:189` (`.all()` gate at `:198`), MSG reduction forward/backward pass
  `msg.rs:125,145`, `ssg::is_feasible_base` `ssg.rs:125–141`, `ssg_dm::Enumerator::recurse` queuing all
  non-raw inputs `ssg_dm.rs:210–216`, ABB reachability bound `abb.rs:295–299`, transductive wrapper
  `reachability::close_producible_under_rule` `reachability.rs:77`. **A Berge path is connectivity proof,
  not producibility.** A* may bound/order/prune but must not replace these.
- **Classification**: `schema.nodes` (M/O), `raws`/`products` (`lowering.rs:90,92`),
  `meta_resolve::classify_materials/units` `:338,375`, `IsaWalk::reaches` `meta_resolve.rs:536` (DFS over
  the `<isa>` inheritance relation — a *different* relation from the incidence hypergraph; stays).
- **Cost / multi-objective policy**: `costs`/`cost_dimensions`/`cost_vectors` `lowering.rs:101,105,110`,
  `abb::effective_cost` `:108`, `cost_weights` `:86`, regimes `regime.rs`.
- **Result/scratch**: MSG/SSG/ABB result structs and B&B `SearchState` `abb.rs:257`.
- **Compat**: `pgip_io` (feature `pgip`), `cli.rs`, `dump.rs`, `builder.rs`.

## The correct-delegation exemplar
`friedler.rs` already models the target seam: the DFS lives in `hymeko_graph` (`CyclePruner` trait,
`friedler.rs:85,139`); only the axiom **predicate** (A0/A1/A3) lives in pgraph (`extend_ok`/`emit_ok`
`:144,162`). The rest of pgraph's topology should delegate the same way.

## Target architecture (design only; no code this phase)
```
pub struct PGraphView<'ir, 'idx> {
    traversal: HyperTraversal<'idx, 'ir>,   // borrows Ir + IncidenceIndex — all adjacency/degree/reach
    // + P-graph-specific classification (M/O/raw/product), cost, policy — borrowed, not a 2nd graph
}
```
`PGraphView` replaces the six owned maps and the two BFS copies with borrowed incidence queries; the
AND-closure feasibility engine, classification, and cost/regime policy remain pgraph-owned.
