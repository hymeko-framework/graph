# graph

Graph theory and hypergraph extensions for analysis and modelling with HyMeKo.

Two crates, laid out the way [`hymeko/core`](http://amaterasu.local:3000/hymeko/core)
is:

| Crate | What it is | Depends on |
|---|---|---|
| [`hymeko_graph`](hymeko_graph/) | Signed-graph cycle and walk enumeration with axiomatic pruning: top-k search, colour coding, community detection, SIMD spine kernels | `rayon`, `dashmap` |
| [`hymeko_pgraph`](hymeko_pgraph/) | The P-graph framework — Material/Operating-Unit overlay on the HyMeKo IR, axiom checker, MSG/SSG generation, ABB solver, `.pgip` round-trip with P-graph Studio | `hymeko_core`, `parser`, `rusqlite` |

Both came out of `hymeko_framework_rust`, which is retired. **This repository
is the canonical home for both.**

```
cargo test                       # 324 tests
cargo test -p hymeko_graph       # 177
cargo test -p hymeko_pgraph      # 147
cd benchmarks && cargo bench     # needs crates.io
```

## They do not depend on each other

Worth knowing before reaching for either, because both mention Friedler's
axioms and they mean different things by it.

`hymeko_graph::FriedlerAxiomPruner` encodes A0/A1/A3 as **cycle enumeration
pruning rules** — cheap structural checks consulted during a DFS, so the
search costs what the feasible set costs rather than what the full set does.
It knows nothing about HyMeKo; you hand it a `Vec<NodeKind>`.

`hymeko_pgraph::axioms` checks **A1–A5 against a real P-graph** lowered from a
description, and can say which axiom failed and why. It is a verdict on a
model, not a filter inside a search.

One is a predicate in a hot loop. The other is an analysis you read.

## And `hotaru::hypergraph`?

A third thing, in a third repo, and the distinction matters because the names
all sound alike:

| | shape |
|---|---|
| `hymeko_graph` | pairwise signed graphs — `edges: Vec<(u32, u32)>` |
| `hymeko_pgraph` | bipartite P-graphs over the HyMeKo IR |
| [`hotaru::hypergraph`](http://amaterasu.local:3000/hymeko/hotaru) | hypergraphs as hypergraphs — Berge paths, Levi/2-section/dual/s-line, transversals, acyclicity, s-connectivity |

A hypergraph reaches `hymeko_graph`'s algorithms only by being star-expanded
first, which is what `BipartiteOnlyPruner` exists for. If your question is
*"which cycles satisfy this axiom, in a graph too big to enumerate naively"*,
it is `hymeko_graph`. If it is *"is this process model feasible, and why
not"*, it is `hymeko_pgraph`. If it is *"what does this relation structure
look like, and where is it fragile"*, it is `hotaru`.

`hotaru` is dependency-free by default and these two are not, so nothing here
depends on it and it depends on nothing here. The ~400 lines of overlap
(balance predicates, breadth-first distances, components, the idea of a pruner
consulted during the search) is duplicated on purpose.

## Layout

```
hymeko_graph/     the signed-graph crate, its tests and examples
hymeko_pgraph/    the P-graph crate, its tests and two binaries
data/             fixtures both crates' tests read with include_str!
benchmarks/       every bench, as a separate crate with its own workspace root
```

`benchmarks/` is not a workspace member on purpose. Criterion's dependency
closure is around a hundred and twenty crates including a wasm-bindgen
toolchain, and requiring all of it to run `cargo test` on a graph library is
the wrong trade. The bench sources are unchanged; running them needs
crates.io.

## Notes on the extraction

- Every source file is byte-identical to the monorepo except `hymeko_graph/src/lib.rs`,
  whose doc comment was stale — it claimed the cycle enumerator still lived in
  `hymeko_py/src/cycles.rs`.
- `hymeko_pgraph`'s path dependencies on `../hymeko_core` and `../parser`
  became git dependencies on the live split-out repos.
- `data/pgraph/Chapter4/example4_3.pgip` was **not** brought across. It is a
  32 MB SQLite blob that no test, bench or example references; everything else
  under `data/` came, and the whole directory is 835 KB. Fetch it from the
  monorepo if a Studio round-trip needs it.
- `proptest` is dropped from both crates' dev-dependencies: declared, never
  used.
- `hymeko_pgraph_py`, the PyO3 binding, stayed behind. It is a packaging
  concern rather than analysis, and needs a decision about where Python
  bindings live in the split-out world.
