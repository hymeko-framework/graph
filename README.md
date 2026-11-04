# graph

Hypergraph and graph algorithms over the canonical HyMeKo IR. `direct_ir`
walks the signed-incidence hypergraph that
[core](https://github.com/hymeko-framework/core) compiles, in place: Berge
paths, components, informed search, PageRank and matroids run without star or
clique expansion and without a second graph model. A separate `SignedGraph`
API serves explicitly pairwise signed graphs. Iterative solvers come from
[tensor](https://github.com/hymeko-framework/tensor).

## Crates

| Crate | Contents | Framework dependencies |
|---|---|---|
| [`hymeko_graph`](hymeko_graph/) | Canonical-IR traversal (`direct_ir`), Berge paths, Dijkstra/A*, components, PageRank, Levi and binary matroids, cell topology, signed-graph cycle and walk enumeration with axiomatic pruning, top-k search | core, tensor |
| [`hymeko_pgraph`](hymeko_pgraph/) | P-graph process networks over the IR: Material/Operating-Unit schema, A1-A5 axioms, MSG/SSG enumeration, constrained decision mapping, the ABB solver and the `pgraph` command line | graph, core, parser |
| [`hymeko_sbsh`](hymeko_sbsh/) | Spatial bounded-width signed hypergraphs: an N-dimensional orthtree over positioned IR declarations and elimination-width certificates | graph, core |

## Use

```toml
[dependencies]
hymeko_core = { git = "https://github.com/hymeko-framework/core.git" }
hymeko_graph = { git = "https://github.com/hymeko-framework/graph.git" }
```

```rust
use std::path::Path;

use hymeko::module_store::{execution::CompiledModel, source_provider::MemProvider};
use hymeko::util::real_parser::RealParser;
use hymeko_graph::direct_ir::{PageRank, PageRankConfig, PageRankModel, PageRankWorkspace};

let source = "net_description {} net { a {} b {} c {} @link { ( +a, -b, -c ); } }";
let sources = MemProvider::default().with_file("net.hymeko", source);
let model = CompiledModel::compile(sources, RealParser, Path::new("net.hymeko"))
    .map_err(|e| format!("{e:?}"))?; // ModuleLoadError is Debug, not Error

let rank = PageRank::build(&model.program().ir, PageRankModel::default())?;
let mut workspace = PageRankWorkspace::new();
let result = rank.run(PageRankConfig::default(), &mut workspace, None)?;
assert!(result.progress.converged);
// result.scores holds one probability per canonical node, in NodeId order.
```

PageRank reuses its workspace across runs and reports an L1 error bound;
reaching the iteration cap returns `converged == false`. The contracts of
these and the other algorithms are in [docs/api-notes.md](docs/api-notes.md).

## Recorded results: native connected components

On the retained NiTech kato21 campaign (20-21 October 2026), HyMeKo was faster
on all seven selected workloads and all 35 paired process comparisons. Ratios
below are Hypergraphz time / HyMeKo time for the same component-partition task.

| Workload | Vertices | Hyperarcs | Incidences | HyMeKo (ms) | Hypergraphz (ms) | Time ratio |
|---|---:|---:|---:|---:|---:|---:|
| chain65k | 65,536 | 65,535 | 131,070 | 2.335 | 8.848 | 3.79x |
| hub32_1m | 65,536 | 31,250 | 1,000,000 | 4.496 | 67.152 | 14.94x |
| components8_1m | 65,536 | 31,250 | 1,000,000 | 8.863 | 62.337 | 7.03x |
| steiner255 | 255 | 10,795 | 32,385 | 0.237 | 0.632 | 2.67x |
| complete32_3 | 32 | 4,960 | 14,880 | 0.085 | 0.160 | 1.88x |
| sunflower1024 | 32,769 | 1,024 | 33,792 | 0.600 | 4.072 | 6.79x |
| mycielski7 | 767 | 22,196 | 44,392 | 0.435 | 0.972 | 2.24x |

CPU: AMD Ryzen Threadripper 7980X; Ubuntu 24.04.3; Rust 1.97.1. Each algorithm
used one thread, pinned to logical CPU 2. Each cell ran in five fresh processes,
with two warmups and nine calibrated samples per process. Values are the median
of the five process medians; they are not single-request tail latencies.

The comparison includes fresh operation workspace, normalized output copying
and temporary-result destruction. Graph and reverse-index construction are
excluded. Full normalized partitions were checked for equality in each process.
The complete campaign has 66 cells, 330 processes and 2,970 raw samples; only
connected components have an external comparison in this campaign.

These are deterministic synthetic inputs. They do not establish universal
performance superiority. Source parsing/file loading and other systems' graph
construction are outside this comparison. Whole-process RSS includes setup and,
in the native comparison, both resident graphs. The publication packaging is
not a new benchmark run.

Evidence: [PDF report](docs/evidence/graph-algorithms/report.pdf),
[measurements](docs/evidence/graph-algorithms/measurements.csv),
[raw samples](docs/evidence/graph-algorithms/raw-samples.csv),
[provenance and hashes](docs/evidence/graph-algorithms/provenance.json).
The existing [campaign runner](scripts/performance_evidence/campaign.py) and
[native adapter](hymeko_graph/tests/performance_evidence/native_adapter.zig)
retain the measurement implementation. Timing cells are explicitly opt-in. The pinned reference is
[Hypergraphz e45b7b8](https://github.com/yamafaktory/hypergraphz/tree/e45b7b80e3ce302937a56760d9fe76016ef08e26),
compiled as a native C-ABI library with Zig `0.17.0-dev.242+5d55999d2`,
`ReleaseFast`, target `x86_64-linux-gnu`, CPU `baseline`.

The campaign parser, provenance and host-admission contracts have focused
Python tests. Some need NumPy, Matplotlib or NetworkX, and the host checks
need a POSIX system:

```sh
python -m unittest discover -s scripts/performance_evidence -p 'test_*.py'
```

## Nothing here needs SQLite

`.pgip` is P-graph Studio's file format, and Studio stores its models in
SQLite databases — so reading one means `rusqlite`, and `rusqlite` with
`bundled` compiles SQLite from C. That is a fine price for *interoperating
with one external tool*, and no price at all to pay for using a graph
library.

So `pgip` is an **opt-in feature**. No crate in the default build of
`hymeko_pgraph` is `rusqlite` or `libsqlite3-sys`; there is no C in it. The `.pgip` reader, its round-trip tests and the two
binaries that use it are all behind the feature:

```
cargo build -p hymeko_pgraph --features pgip
```

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

## Build and test

```sh
cargo test --locked
cargo test --locked -p hymeko_graph --test pagerank --test matroid_binary
cargo build --locked -p hymeko_pgraph --features pgip
```

`cargo dev-test` runs the correctness suites against a dynamic-library build,
which shortens incremental rebuilds; see [dev/README.txt](dev/README.txt).

## Layout

```
hymeko_graph/     canonical hypergraph and signed-graph algorithms, tests and examples
hymeko_pgraph/    P-graph semantics, tests and optional binaries
hymeko_sbsh/      spatial orthtree overlays and width certificates
data/             fixtures the tests read with include_str!
benchmarks/       Criterion benches, as a separate crate with its own workspace root
scripts/          the performance-evidence campaign runner and its tests
docs/             API notes and the retained evidence
```

`benchmarks/` has its own workspace so Criterion stays outside the library
build.

## License

MIT; see [LICENSE](LICENSE).
