# P1.3 — pgraph adjacency delegation & duplication removal

**STRATEGY-ID:** HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P1.3 · **APPROVED-GRAPH-EDIT:**
pgraph-adjacency-delegation. 2026-09-02. Branch `feat/pgraph-adjacency-delegation` @ `1a03350`
(remote-verified); **not merged** to main (graph main = `9655f07`). Additive/consolidation; no new graph
model, no dense/star/clique, no snapshot/transaction; core & sibling repos untouched.

## 1. Entry gate (Phase 0)
Re-read remote refs; ancestry `d4086a0 ≺ 9655f07`; re-ran the P1.2 gates (fmt/clippy, pgraph 232/0, pgip
6/6) green; **fast-forward-only** promoted graph main `d4086a0..9655f07` (P1.2). P1.3 branched off the
promoted main.

## 2. Read-only consumer / compatibility inventory (§1.1)
- `PGraphSchema` / `LoweredPGraph` carry **no serde derive** — not on any wire; `.pgip` is a SQLite
  emission over `inputs/outputs`, byte-checked by the round-trip test. Internal field changes have **no wire
  impact** (verified — no public field deleted).
- `schema.incoming/outgoing` is the **single** canonical derived adjacency (one legit source). The
  **duplication** was the axiom-layer rebuilds: `axioms::build_forward_adj` (`==` `schema.successors`),
  `axioms::build_producers` (`==` `schema.predecessors(m)` for a material, by the bipartite invariant), and
  the **verbatim** `axiom_extensions::{build_forward_adj, reaches_any}` copies.
- Consumers: A2 used `build_producers`; A4 + E-StrictNoExcess used `build_forward_adj`+`reaches_any`;
  MSG/SSG/ABB read `LoweredPGraph::{inputs,outputs,producers,consumers}` (queries over the schema — not
  side tables). `in_degree/out_degree` scan `edges`.

## 3. Consolidation (§1.3, one atomic commit)
One `pub(crate) axioms::reaches_any(schema, from, targets)` — BFS over `schema.successors`, the sole
reachability. **Deleted:** `axioms::build_forward_adj`, `axioms::build_producers`,
`axiom_extensions::build_forward_adj`, `axiom_extensions::reaches_any`. `check_a2` →
`!schema.predecessors(m).is_empty()`; `check_a4` / `check_strict_no_excess` → the shared `reaches_any`.
**Net −68 LOC** (31 insertions, 99 deletions) across the two axiom files.

## 4. Invariants proven (§1.4) — the byte-identity gate
The full pgraph suite is **235/0/3** (was 232 + 3 new parity tests): the **six dump hashes**, `.pgip`
round-trip, MSG/SSG/ABB result sets, book validation, and BooleanExact closure are **unchanged** (the
golden tests are the byte-identity gate; all green after the change). Additionally
`adjacency_delegation_vv` proves: `PGraphView.forward_reachable` == the schema's directed reachability (the
exact A4 predicate) for every unit and material across four graph shapes; `PGraphView.in/out_degree` ==
`schema.in/out_degree` (with multiplicity); duplicate incidence preserved as multiplicity (deduped set = 1,
degree = 2). Single canonical topology; `PGraphView` borrows `&Ir`/`&IncidenceIndex` (cannot outlive them);
sign/weight/annotation and incidence multiplicity preserved.

## 5. schema.incoming/outgoing removal — **DEFERRED** (honest)
The *full* removal of the schema adjacency requires `LoweredPGraph` to **borrow** the `Ir`+`IncidenceIndex`
— it is Ir-independent by design (that is *why* the schema adjacency exists) — which lifetime-parameterizes
the whole domain layer (MSG/SSG/ABB/pgip/dumps). That cannot be done atomically with a byte-identical
guarantee this phase without large risk, so it is **deferred** to a scoped follow-up
(`pgraph-lowered-borrows-ir`). What P1.3 removes is the real duplication — the axiom-layer rebuilds + the
verbatim copies — while `schema.incoming/outgoing` remains the single non-duplicated source, and
`PGraphView` is the borrowed Ir-based equivalent (proven equal), used by the AND/OR path.

## 6. Fault injection (§1.5; no mutant committed, git-clean restore)
Mutating `view.rs`, each caught by a named parity test: **input-sign swap** (→ closure parity), **producer/
consumer swap** (→ `producers_set` parity in andor_vv), **dropped slot** (→ closure parity), **wrong
arc-owner** (→ reachability parity), **reachability-direction** (→ reachability parity). **Dedup** guarded by
the multiplicity test; **compat-field-as-authoritative** structurally guarded — `PGraphView` holds no
compatibility field, it queries the incidence directly.

## 7. Own mistakes / retracted
- The reachability-parity test first compared `forward_reachable` (which includes the start node) against
  the schema BFS (which excludes it), giving a false failure on product-materials; fixed by excluding the
  start on both sides (apples-to-apples).
- No architecture claim changed; the AND-closure feasibility kernel, classification, cost/regime policy are
  untouched (they stay domain logic).

## 8. Next minimal phase
`pgraph-lowered-borrows-ir`: lifetime-parameterize `LoweredPGraph<'ir>` to borrow the `Ir`+`IncidenceIndex`
and remove `schema.incoming/outgoing` entirely (delegating all adjacency to `PGraphView`), gated on the same
byte-identical dump/pgip/MSG/SSG/ABB re-verification — the change that actually reduces *retained*
topological memory (P1.3 removes per-check rebuild *allocations*, not retained bytes).

## Provenance / HARD STOP
rustc 1.97.1; pulls `hymeko_core` == core `5fe2c88`. Graph V&V: fmt clean; clippy `--workspace
--all-features --all-targets -D warnings` green; pgraph **235/0/3**. Branch pushed + verified, **not merged**.
HARD STOP on this phase's own scope; P1.4/P2 proceed on their own branches.
