# hymeko_pgraph

The **P-graph framework** over the HyMeKo IR: a bipartite Material /
Operating-Unit overlay on the canonical signed-incidence hypergraph, an axiom
checker, MSG/SSG generation, an accelerated branch-and-bound solver, and
`.pgip` round-trip with P-graph Studio.

This is the domain layer. The generic side —  signed graphs, CSR, DFS and BFS,
cycle enumeration, the `CyclePruner` seam — is
[`hymeko_graph`](../hymeko_graph/), which this crate depends on and which knows
nothing about Materials or Operating Units.

## Two readings of Friedler's axioms, and they are not the same

Friedler, Tarján, Huang and Fan (1992) characterised feasible
process-synthesis structures with five axioms. This crate implements them
twice on purpose, because two different questions are being asked.

**[`axioms`](src/axioms.rs) — a verdict on a model.** `AxiomBundle::validate`
checks A1–A5 against a lowered P-graph and returns an `AxiomViolation` saying
which failed and why. You read the answer.

**[`friedler`](src/friedler.rs) — a predicate in a hot loop.**
`FriedlerAxiomPruner` implements `hymeko_graph::CyclePruner`, so cycle
enumeration rejects infeasible process loops *during* the DFS rather than
after materialising them:

| | |
|---|---|
| **A0** bipartite alternation | rejected at `extend_ok`; an M-M or O-O step cannot occur |
| **A1** required products | checked at `emit_ok`; the cycle must touch a product |
| **A3** valid operating units | rejected at `extend_ok` against a whitelist |
| **A4 / A5** | degenerate at cycle-enumeration time |

Only A0/A1/A3 have a cycle-enumeration reading at all, which is the clearest
sign these are different tools rather than one duplicated.

The pruner shares [`PNodeKind`](src/schema.rs) with the rest of the crate. It
used to live in `hymeko_graph` with a private copy of that enum — the doc
comment there said the duplicate existed so the graph crate need not depend on
this one. Putting the pruner on this side removed the need for both.

## No traversal here

DFS, BFS, bidirectional BFS, connected components and CSR adjacency are
`hymeko_graph::traversal`'s. This crate has none of its own and should not
grow any.

What looks like traversal in [`msg`](src/msg.rs) is not. `close_producible`
and `close_consumable` are **AND-fixpoints**: a unit fires only when *every*
one of its inputs is already reachable, and the closure iterates to a fixed
point. That is hypergraph B-connectivity, and no graph BFS computes it —
reachability along any single edge is exactly the thing it must not do.

## Building

```
cargo test -p hymeko_pgraph
cargo run --example axiom_effect        # what each axiom prunes, timed
cargo run --example strategy_pattern    # composing pruners
```

`.pgip` support pulls `rusqlite` with `bundled`, which compiles SQLite from C.
It is behind the opt-in `pgip` feature. The default library build and the
wasm32 dependency path contain no SQLite. Enable the interoperability tools
explicitly with `cargo build -p hymeko_pgraph --features pgip`.

Test fixtures live in [`../data/`](../data/) and are read with
`include_str!("../../data/…")`. One file did not come across from the
monorepo: `data/pgraph/Chapter4/example4_3.pgip`, a 32 MB SQLite blob that
nothing references. The two `.pgip` samples `tests/pgip_io.rs` does read,
Chapter3 and Chapter6, are here; that test skips gracefully if they are not.

## Citations

- F. Friedler, K. Tarján, Y.-W. Huang, L. T. Fan,
  *Combinatorial algorithms for process synthesis*, Computers & Chemical
  Engineering, 16, 1992.
- F. Friedler, K. Tarján, Y.-W. Huang, L. T. Fan,
  *Graph-theoretic approach to process synthesis: axioms and theorems*,
  Chemical Engineering Science, 47(8), 1992.
