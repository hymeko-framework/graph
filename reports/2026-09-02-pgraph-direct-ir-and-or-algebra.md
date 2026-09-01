# P1.2 — direct-Ir P-graph AND/OR algebra (T-norm / S-norm)

**GOVERNING-STRATEGY:** ../core/docs/hymeko-platform-strategy.md · **STRATEGY-ID:**
HYMEKO-PLATFORM-2026-09-01-v1 · **ROADMAP-TASK:** P1.2. 2026-09-02 00:19 CEST.
**APPROVED-GRAPH-EDIT:** pgraph-direct-ir-and-or-algebra. Additive; branch
`feat/pgraph-direct-ir-and-or-algebra`, **not promoted**. Core production source untouched; no new graph
representation; no core Ir/wire/dependency change. The P1.1 direct-Ir A* work is untouched (orthogonal base).

## 0. Entry gate
Verified the P1.1 branch (`d4086a0`) and graph main by `ls-remote`; ancestry `c35b9f2 ≺ d4086a0`; re-ran the
graph gate suite (fmt/clippy/tests 466/0/6) green; **fast-forward-only** promoted graph main
`c35b9f2..d4086a0`. New branch off the promoted main.

## 1. PGraphView ownership & memory
`PGraphView<'a,'ir,'idx>` (`hymeko_pgraph/src/view.rs`) — a **borrowed** domain view: it holds only
`&IncidenceIndex` + borrowed classification sets (`&units/&materials/&raws/&products`). **Zero owned
topology.** All directed adjacency is answered by querying the canonical signed incidence in place:
`inputs(u)` = the `-` refs of `u`'s arc, `outputs(u)` = the `+` refs, `producers(m)/consumers(m)` via
`IncidenceIndex::incident_arcs(m)` + the sign of `m`'s slot (owner = `arc.in_edge`). Sign convention
verified against `meta_resolve::build_incidence`; a `~` unit ref is a typed `ViewError::NeutralUnitRef`.
Hot-path queries are **allocation-free callbacks** (`for_each_{input,output,producer,consumer}`), preserving
duplicate incidences; deduped `*_set` methods exist for parity. Memory: `O(1)` owned (borrows only) plus the
already-built `IncidenceIndex`.

## 2. Removed duplication — **deferred** (parity gate established)
The physical removal of the duplicated adjacency (`schema.incoming/outgoing/edges`, `in/out_degree` O(E)
scans, `axioms/axiom_extensions::build_forward_adj` ×2, the two verbatim `reaches_any` copies) is
**deferred to a tightly-scoped follow-up**, and here is the honest reason: it rewires core-protected
`LoweredPGraph`/`PGraphSchema`/`axioms` onto `PGraphView`, which can move the six P-graph dump hashes, the
`.pgip` round-trip, and the MSG/SSG/ABB goldens. Correctness-first, I did not rush that into this phase.
**What P1.2 delivers is the precondition:** the parity gate is now GREEN — `PGraphView` adjacency is proven
byte-identical to the schema (`andor_vv::parity_check`, per unit/material, over five graph shapes), so the
removal is *unblocked*. The removal plan and its verification gate are §12 (next minimal phase).

## 3. BooleanExact parity (the load-bearing proof)
`close_producible` is exactly a Boolean AND/OR fixpoint: `ready(u)=⋀ inputs`, a material joins the closure if
raw **or** any producer fires. `andor::propagate` with `BooleanExact` (`T=min`, `S=max`, `{0,1}`) reproduces
it: **`Availability::producible()` == `close_producible(units, raws)`**, asserted across `MULTI_AND`,
`MULTI_OR`, `CYCLE_NO_RAW`, `DUP_INPUT`, `CHAIN`, `REVERSE_CHAIN` (`andor_vv`), and cross-checked at every
benchmark size (`andor_bench`, n=100..2000). Feasibility and MSG/SSG/ABB result sets are therefore unchanged
by construction — they consume exactly this closure, which is bit-identical, and their code is untouched
(full pgraph suite **232/0/3** green, including the six dump hashes and `.pgip` round-trip).

## 4. Norms & numeric policy
`AndOrNorm` (monomorphised — no runtime mode boolean, no `Box<dyn>` in the fixpoint): **BooleanExact**
(exact) and opt-in **Gödel** (min/max), **Product** (`ab` / `a+b−ab`), **Łukasiewicz** (`max(0,a+b−1)` /
`min(1,a+b)`). Fuzzy modes are **ordering-only**: with full reliability their producible **set** is proven
identical to BooleanExact (`fuzzy_norms_are_ordering_only_same_producible_set`) — they may give a
relaxation / priority / bound / order but never change the exact result set. The `+/-/~` sign is an
incidence **role**, not a fuzzy truth value. `NaN`/`Inf`/out-of-`[0,1]` ⇒ typed `NormError::BadValue`; fold
order deterministic (sorted `BTreeSet` iteration); duplicate incidence stays semantic (**Product does not
deduplicate** — proven by the multiplicity assertion). BooleanExact is finite; fuzzy uses `ε` +
iteration-limit ⇒ typed `NonConvergent`.

## 5. Full / incremental complexity
Full closure: the monotone worklist visits each unit when woken — `O(units + incidences)` on the
exit-independent path, versus `close_producible`'s re-scan `O(rounds·units)`. Warm loop **allocation-free**
(dense `DeclId`-indexed, generation-stamped `avail`/`queue`/`pending`; `take`/restore keeps capacity —
proven by `andor_alloc`: two warm calls allocate identically and result-sized, not iteration-scaled).
Incremental (before/after) is **designed, not built** this phase (no snapshot layer; §12).

## 6. MSG/SSG/ABB result identity
Unchanged — untouched code consuming a bit-identical closure. The full pgraph suite (book validation,
workloads suite, MSG/SSG/ABB, dump hashes, `.pgip` round-trip) is **232/0/3** green on the branch.

## 7. First / best / all latency
`andor_bench` (chain raw→product, unit cost, median-of-9, single host PROVISIONAL):

| n | units | A `close_producible` µs | C worklist µs | C iterations | A/C |
|--:|--:|--:|--:|--:|--:|
| 100 | 99 | 7.1 | 4.4 | 99 | 1.6× |
| 300 | 299 | 58.9 | 16.1 | 299 | 3.7× |
| 1000 | 999 | 192.2 | 72.4 | 999 | 2.7× |
| 2000 | 1999 | 407.0 | 153.4 | 1999 | 2.7× |

C iterations = n−1 (each unit fires once). A is at its **best** here (forward declaration lets one in-place
pass add the whole chain); a reverse-declared chain would widen the gap toward O(n²) vs O(n). MSG/SSG/ABB
first/best/all latency was **not** re-timed (their code is unchanged); the closure they consume is faster
and identical.

## 8. Allocation & peak RSS
Warm fixpoint loop: **0 allocations** (workspace reused); a call's only allocation is the owned
`Availability` result (`O(materials)`). Peak RSS for the largest bench cell (2000 materials) ≪ 16 GiB.

## 9. A* integration admissibility verdict
Kept **separate** from the P1.1 A* primary cost, as directed. The T/S availability is a **feasibility
filter / tie-break**, not a metric cost; it must **not** enter `h(n)` as an admissible bound without a
proof (an inadmissible fuzzy score in optimal mode is exactly what the P1.1 engine's consistency gate
refuses). The P1.1 metric/discrete heuristics are unchanged. A producibility-aware **admissible lower
bound** for ABB is a pgraph-side AND/OR (Knuth) hyperpath borrowing `direct_ir` topology — designed in
`../graph-berge` `docs/pgraph-as-graph-extension.md`, not built here (no second search engine).

## 10. Winning approach per workload
- **Long dependency chains / large closures:** the BooleanExact worklist (C) — 2.7–3.7× faster than the
  re-scan, exact set identical; the gap grows with reverse/random declaration order.
- **Small graphs:** both are sub-10 µs; either is fine.
- **Uncertainty / ranking (not the exact set):** the fuzzy norms as an ordering signal (they never change
  the exact producible set) — a bound/priority for search, measured to agree with BooleanExact on the set.

## 11. Own mistakes / retracted hypotheses
- **Wrong lowering path first:** I initially compiled the tag-style `hda.hymeko` through `lower_resolved`
  (the `<isa>` archetype path) and hit `MissingArchetype("raw")`; the tag style is the AST-`lower` path.
  Corrected to `<isa>`-archetype sources with the `meta_pgraph.hymeko` include, which yields both the `Ir`
  and the `LoweredPGraph` from one compile.
- **M3 fault initially not caught:** dropping consumer propagation was masked because seeding all units +
  FIFO processes a forward chain in dependency order; a **reverse-declared** chain exposes it. Added
  `REVERSE_CHAIN`; M3 now caught.
- **`pending` Vec in the hot loop:** the first draft allocated per firing; moved it into the workspace
  (`take`/restore) to keep the warm loop allocation-free.
- No architecture claim changed; the exact AND-closure stays domain logic (not replaced by Berge
  connectivity).

## 12. Next minimal phase — the consolidation removal (approval scope + entry conditions)
`pgraph-adjacency-delegation`: in **one** commit, delete the two verbatim `reaches_any` copies and the
`build_forward_adj`/`build_producers` duplicates, and rewire `LoweredPGraph::{inputs,outputs,producers,
consumers}` + the A4/A5 reachability probes onto `PGraphView` (a directed-reachability entry it already
provides via `forward_reachable`), removing `PGraphSchema.incoming/outgoing` where fully replaced. **Entry
conditions:** (1) the parity gate of this report (established); (2) a **byte-identical** re-verification of
the six dump hashes, `.pgip` round-trip, and MSG/SSG/ABB goldens *per fixture* before/after in the same
commit; (3) the AND-closure feasibility kernel, classification, and cost/regime policy **untouched**; (4) no
long-lived double ownership. Fault-injection to re-run: T/S swap, wrong identity, dropped propagation,
premature convergence (all caught here); dedup and fuzzy-as-exact-pruning (structurally guarded).

## Fault-injection (no mutant committed; git-clean restore)
Engine mutants each caught by a named test: **M1** BooleanExact T/S swap (→ `multi_input_and_missing`),
**M2** wrong AND identity (→ closure parity), **M3** dropped consumer propagation (→ closure parity via
`REVERSE_CHAIN`), **M5** premature-convergence threshold (→ closure parity). **M4** wrong-dedup and **M6**
fuzzy-as-exact-pruning are structurally guarded by the duplicate-multiplicity assertion and the
fuzzy-invariance test respectively.

## Provenance / HARD STOP
rustc 1.97.1, x86_64-pc-windows-msvc; pulls `hymeko_core` == core `5fe2c88`. Graph V&V: fmt clean; clippy
`-p hymeko_pgraph --all-features --all-targets -D warnings` green; pgraph suite **232/0/3** (incl. six dump
hashes, `.pgip` round-trip, MSG/SSG/ABB, book validation). Branch **not promoted**; core/pgraph domain
kernel/Nagare/Hotaru untouched. **HARD STOP** after this report — the consolidation removal (§12) and any
transaction/snapshot layer are **not** started.
