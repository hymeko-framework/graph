# graph

Graph theory and hypergraph extensions for analysis and modelling with HyMeKo.

`direct_ir::PageRank` ranks canonical IR nodes without materializing a pairwise
graph. `PageRank::build(&ir, PageRankModel::default())` prepares an undirected
vertex–hyperarc–vertex probability walk. `run(config, &mut workspace,
personalization)` uses reusable `PageRankWorkspace` buffers and returns scores in
`NodeId` order, iteration count, an L1 error bound and explicit convergence status.
Reaching the iteration cap returns `converged=false` rather than claiming success.

The default includes self-return and incidence multiplicity. `RankFlow` also
offers explicit minus-to-plus and In-to-Out routing; algebraic sign and port
direction remain independent. `RankWeights::Component(i)` selects one nonnegative
numeric weight-vector component. Dangling nodes redistribute to the normalized
personalization, uniform when absent. Every canonical node record is included;
non-node arc targets are rejected. Each iteration takes O(nodes + arcs +
incidences), with no clique expansion, and a warm run allocates no heap storage.

The default `PageRank::build` now consumes Core's numeric incidence projection
and shared gather/scatter row operations. Target resolution and sign/port routing
happen during preparation. Undirected unit incidence needs no coefficient array;
directed unit incidence stores role bits, and weighted modes store normalized rates.
The compensated arithmetic, selected vector component and convergence contract
are unchanged. A fused per-arc apply needs no additional arc-sized scratch vector.

An existing `PreparedIr` or incidence index can share the same topology with
several consumers:

```rust
let projection = prepared.incidences()
    .prepare_projection(hymeko::ir::incidence::IncidenceDomain::Nodes)?;
let rank = PageRank::from_projection(projection.clone(), model)?;
```

Projection storage is O(incidences + arcs), with native-size indices preserving
the canonical ID range. Normalizations remain specific to each PageRank model.
The earlier timing tables precede this integration; implementation and correctness
evidence is in `reports/2026-10-18-native-graph-comparison/shared-incidence/report.pdf`.

For directed hyperarcs, construct `PageRankModel { flow: RankFlow::InToOut,
..Default::default() }` for operator ports, or choose `RankFlow::MinusToPlus`
for algebraic incidence roles. A hyperarc may have several source and destination
slots: their mass is gathered and scattered without enumerating every source–target
pair. Neutral/unspecified slots are inactive in the corresponding directed mode.
An arc without destination mass is excluded from outgoing normalization; nodes
without a remaining outgoing transition use the dangling-node rule. The published
timing table currently characterizes the undirected mode only.

The existing performance-evidence client has an explicit `--suite ranking` mode.
It separates preparation, complete PageRank convergence and prepared reachability
from graph construction. Cells require `HYMEKO_RANK_BENCH=1` and remain ignored in
normal tests. Local unpublished revision sets may be checked through
`--source-manifest`; files and binaries are hashed and source drift aborts a run.
See `reports/2026-10-17-pagerank/report.pdf` for exact semantics, tested cases,
measurement boundaries and the shared archive receipt.

Signed edge-list tools share `signed_graph::{Loader, SignedEdgeListLoader}`.
`load` accepts a buffered stream; `load_path` uses the same implementation for a
file. Compatibility rules preserve the former example loaders' malformed-row and
self-loop skipping, individual space/tab/comma separators, edge order, signs, and
one-vertex empty-input convention. An unrepresentable vertex count returns an error.

`signed_graph::SignedGraphGenerator` implements Core's `GraphGenerator` and
`SyntheticGenerator`. It reproduces the existing seeded LCG fixtures while writing
directly into the final edge/sign buffers. Probabilities use per-mille units.
Cycle integration tests share typed runners and explicit comparison strategies;
Core's `util::measurement` owns timing statistics and allocation instrumentation.

`hymeko_pgraph::PGraphSchemaBuilder` builds an explicit-ID schema from material IDs,
unit IDs and numbered edges. Validation and adjacency remain owned by
`PGraphSchema::try_new`; conflicting node roles are reported, while repeated edge
IDs retain the map constructor's last-write behavior. The named, domain-level
construction API remains `PgraphBuilder`. Unit tests use a shared declarative
schema case so their inputs and expected failures stay visible.

`direct_ir::HopHeuristic` borrows a prepared DeclId-indexed hop table;
`MaxHeuristic` composes two borrowed bounds and combines their guarantees by
conjunction. Hop tables must describe exact undirected distances to the query's
goal on the same IR, and the supplied step bound must be valid for the cost model.
Construction checks numeric bounds and finite estimates; it does not certify the
table's topology or its coupling to the cost model.

Informed-search tests share borrowed cases and static Dijkstra/A* strategies.
Evidence clients share checksum/work verification and typed TSV result rows;
timing entry points remain explicitly opt-in. The declaration-permutation test
compiles two reordered sources, checks actual ID changes and remapped references,
and rejects use of a stale dense node ID. See
`reports/2026-10-14-informed-search-runners.pdf` for validation and remaining limits.

These additions currently form an uncommitted Core/Graph integration change.
The checked-in Core revision predates the new APIs: publish the provider and pin
Graph to that revision before claiming a portable build. The task report records
the command-line local Core override used for verification; no path override is
added to the repository manifests. See `reports/2026-10-14-shared-support.pdf`
and its archive under `by-repository/graph/branch-feat%2Fnewest-core-e1eee8418382`.

Two crates, laid out the way [`hymeko/core`](http://amaterasu.local:3000/hymeko/core)
is:

| Crate | What it is | Depends on |
|---|---|---|
| [`hymeko_graph`](hymeko_graph/) | The generic layer. Signed graphs, CSR, DFS and BFS, cycle and walk enumeration, top-k search, colour coding, community detection, SIMD spine kernels, and the `CyclePruner` seam | `rayon`, `dashmap` |
| [`hymeko_pgraph`](hymeko_pgraph/) | The P-graph layer. Material/Operating-Unit overlay on the HyMeKo IR, the A1–A5 axiom checker, Friedler's axioms as a `CyclePruner`, MSG/SSG generation, ABB solver | `hymeko_graph`, `hymeko_core`, `parser` |

Both came out of `hymeko_framework_rust`, which is retired. **This repository
is the canonical home for both.**

```
cargo test                       # 378 tests, no C toolchain needed
cargo test -p hymeko_graph       # 167
cargo test -p hymeko_pgraph      # 211
```

## Nothing here needs SQLite

`.pgip` is P-graph Studio's file format, and Studio stores its models in
SQLite databases — so reading one means `rusqlite`, and `rusqlite` with
`bundled` compiles SQLite from C. That is a fine price for *interoperating
with one external tool*, and no price at all to pay for using a graph
library.

So `pgip` is an **opt-in feature**. The default build of `hymeko_pgraph`
resolves 114 crates and not one of them is `rusqlite` or `libsqlite3-sys`;
there is no C in it. The `.pgip` reader, its round-trip tests and the two
binaries that use it are all behind the feature:

```
cargo build -p hymeko_pgraph --features pgip
```

211 tests run without it, 216 with — the five extra being the Studio
round-trips.

## Where the line is

`hymeko_pgraph` → `hymeko_graph`, one way. Nothing in `hymeko_graph` knows
what a Material or an Operating Unit is.

**Traversal is the graph library's.** DFS, BFS, bidirectional BFS, connected
components, CSR adjacency, the bitset helpers — all in
`hymeko_graph::traversal`. `hymeko_pgraph` has no traversal of its own and
should not grow any. What looks like traversal in `msg.rs` is not: producible
and consumable closures are AND-fixpoints, where a unit fires only when
*every* input is already reachable. That is hypergraph B-connectivity, and no
graph BFS computes it.

**Friedler is the P-graph library's.** `hymeko_pgraph::FriedlerAxiomPruner`
implements `hymeko_graph::CyclePruner`, so cycle enumeration rejects
infeasible process loops during the DFS rather than after. It shares
`PNodeKind` with the rest of `hymeko_pgraph`. It used to live in
`hymeko_graph` with its own copy of that enum — the doc comment there said, in
so many words, that the duplicate existed to avoid a dependency. Reversing the
dependency removed the need for both.

Within `hymeko_pgraph`, note that the pruner and `axioms` are not two spellings
of one thing. `axioms` validates A1–A5 against a lowered model and reports
which failed and why — a verdict you read. The pruner is a predicate in a hot
loop, and only A0/A1/A3 have a cycle-enumeration reading at all.

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

There is **one remaining duplication** across the three, and it is not
resolved: `hotaru::hypergraph` carries its own balance predicates, BFS,
components and pruner trait — roughly 400 lines that restate what
`hymeko_graph` already has, for hypergraphs instead of graphs. It stays that
way only because `hotaru` is dependency-free by default and this crate needs
`rayon` and `dashmap`. Collapsing it means deciding which of those two
properties matters more.

## Layout

```
hymeko_graph/     the signed-graph crate, its tests and examples
hymeko_pgraph/    the P-graph crate, its tests and two binaries
data/             fixtures both crates' tests read with include_str!
benchmarks/       every bench, as a separate crate with its own workspace root
```

`benchmarks/` is parked: the sources are all there, unchanged, in a crate
with its own workspace root so neither library resolves criterion. Wiring it
up properly is a later job.

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
