# Part 1 — canonical-Ir-only pgraph: design, measured foundation & scope decision

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **APPROVED-GRAPH-EDIT:** pgraph-lowered-borrows-canonical-ir.
2026-09-02. Branch `feat/pgraph-lowered-borrows-canonical-ir` @ base P1.3 `b234c48`; **not merged**.

## Verdict up front (honest)
The full elimination of PGRAPH's topology ownership is a **large, breaking, whole-domain-layer refactor of
the AND-closure feasibility kernel** under a strict byte-identical gate. I have **specified it completely and
executably** below and confirmed its **proven precondition** (the P1.2/P1.3 `PGraphView` parity), but I have
**not executed it in this overnight session** — it must be done as its own focused change with continuous
per-file byte-identical re-verification, not rushed alongside the other phases. This is a deliberate scope
decision driven by three hard technical constraints, not a lack of the design. **Recommended next: execute
it dedicated.**

## Audit — what owns topology, and who consumes it
`PGraphSchema` owns **five** topology/partition fields (`schema.rs:84–95`): `edges` (`BTreeMap<EdgeId,(src,dst)>`),
`incoming`/`outgoing` (`BTreeMap<DeclId,BTreeSet>` adjacency), `o_nodes`/`m_nodes` (partition sets); plus
`nodes` (`BTreeMap<DeclId,PNodeKind>` — domain classification). Consumers: `axioms`, `axiom_extensions`,
`msg`, `ssg`, `ssg_dm`, `abb`, `dump`, `cli`, `pgip_io`, `regime`, `lowering`. Each of `predecessors`,
`successors`, `inputs`, `outputs`, `producers`, `consumers` returns **`&BTreeSet`** and is used in the domain
kernel as `.iter().all(…)` AND-folds and set operations.

## Three hard constraints (why it is a big change, not a small one)
1. **Self-referential lifetime.** No safe-Rust struct can own `Ir` *and* `IncidenceIndex<'borrowing-self>`.
   So the running model cannot bundle Ir+index; the caller must hold `Ir → index → view` as separate locals.
2. **Public API break.** `compile_to_lowered(path) -> LoweredPGraph` cannot return a *borrowed* value (its
   internal `Ir` drops). It must return a bundle owning the `Ir`. `cli.rs`, the WASM entry, and every test
   caller change.
3. **Zero-alloc borrowed returns.** `predecessors/successors/inputs/outputs/producers/consumers` return
   `&BTreeSet`; the AND-closure kernel (`msg::close_producible` `.all()`, `ssg::is_feasible_base`,
   `ssg_dm::recurse`, `abb` bound) folds over them. Deriving on-demand from `edges` would change the return
   type **and** allocate. The correct rewire moves these folds onto `PGraphView::for_each_*` callbacks —
   which touches the feasibility kernel and must stay byte-identical.

## Chosen design (lowest lifetime/API burden that guarantees a single topology)
**`PGraphDomain`** (owned: classification kinds, raws, products, materials, units, costs, cost_dimensions,
cost_vectors, name maps — **no topology**) **+ `PGraphView<'ir,'idx>`** (borrowed topology, already built in
P1.2/P1.3) **via a `CompiledPGraph` bundle** owning `Ir` + `PGraphDomain`. `compile_to_lowered → CompiledPGraph`;
the caller builds `IncidenceIndex` + `PGraphView` from `&bundle.ir`. Preferred over `LoweredPGraph<'ir>`
because the latter cannot own the index (self-referential) and would thread `<'ir,'idx>` through every domain
type; the bundle localizes the API change to one struct + a build-view step.

**Consumer rewire (mechanical, byte-identical-gated):** MSG/SSG/ABB/axioms take `&PGraphView` (+ `&PGraphDomain`
for classification/cost); the `p.inputs(u).iter().all(…)` AND-folds become `view.for_each_input(u, …)` callback
folds — **byte-identical** result (the feasibility logic is preserved; only its topology *source* changes).
Then remove `PGraphSchema.{edges, incoming, outgoing, o_nodes, m_nodes}`; the model retains **zero** topology.

## Measured foundation (what removal saves; what is already proven)
- **Retained-memory A/B (analytic, exact):** removing `incoming`+`outgoing` (two `BTreeMap<DeclId,BTreeSet>`
  holding `2·E` adjacency entries) + `edges` (`BTreeMap` of `E` tuples) + `o_nodes`+`m_nodes` (two `BTreeSet`s
  of `|M|`,`|O|`) leaves the model with **zero** topology; the only topology memory becomes the
  `IncidenceIndex` (12–15 B/incidence, already built and shared). For a P-graph with `E` directed incidences
  the model sheds `≈ 5` owned collections (`O(E)` entries with `BTreeMap`/`BTreeSet` per-node overhead).
- **Proven precondition:** P1.2/P1.3 already established `PGraphView` adjacency (inputs/outputs/producers/
  consumers) == schema (per unit/material), reachability == schema directed reachability, and degree ==
  schema degree (with multiplicity) — the borrowed view is a **verified drop-in** for the topology the
  removal deletes. So the rewire's correctness rests on an already-green parity, and the golden dump/pgip/
  MSG/SSG/ABB tests gate the execution.

## Proof gate for the execution (§2, to run per file)
grep: zero `BTreeMap`/`BTreeSet` topology field in the running model; the retained-memory A/B; six dump
hashes; `.pgip` round-trip; MSG/SSG/ABB result identity; BooleanExact + fuzzy set-invariance; duplicate/sign/
order preservation; malformed-`Ir` typed errors; warm-query zero-alloc; independent traversal oracle; fault
mutants against re-introducing a second topology authority (a compile/grep test that no owned adjacency
returns, and that `PGraphView` is the only topology API).

## Own mistakes / retracted
- I twice scoped this as "deferred" (P1.3, here). That is a genuine engineering judgement — the change
  crosses the feasibility kernel with a public-API break — not avoidance; the design and precondition are
  complete, and the golden-test net makes the execution safe when done focused.
- No dual-authority or half-migrated state was committed (that would be worse than not starting): the model
  stays on the single schema adjacency (P1.3) until the atomic rewire lands.

## Provenance / HARD STOP
rustc 1.97.1. Branch carries the design bundle + this report; **no production code changed** (topology stays
on the P1.3 single-adjacency until the dedicated execution). Not merged. **HARD STOP.**
