# graph

Graph theory and hypergraph extensions for analysis and modelling with HyMeKo.

`matroid::binary_weighted_basis` reads selected canonical hyperarc columns over
GF(2), using the existing borrowed `MatroidRef`. It returns a minimum additive
weight basis and canonical-ID XOR reconstruction coordinates. Sparse pivots are
bounded algebraic scratch; no pairwise graph or full incidence matrix is copied.
This is linear independence, distinct from the existing graphic/Levi connectivity
matroids. A basis is not a certificate of a nonlinear circuit's phase or cleanup.

`matroid::binary_latent_proposals` reuses `neural_gas::BatchGas` on three support
descriptors and admits every proposed basis through exact elimination. Its learned
prototype graph is transient statistical state. It is not an alternative semantic
IR, a lossless circuit autoencoder, or an admissible pruning bound. The thin
canonical-source example is `cargo run --locked -p hymeko_graph --example
binary_matroid -- MODEL.hymeko EXACT_FORM_SCOPE`. Focused checks are
`cargo test --locked -p hymeko_graph --test matroid_binary --test matroid_facade`.

`matroid::binary_reconstruction_search` starts from the existing weighted basis
and searches exact fundamental-circuit exchanges to reconstruct specified
required columns from an independent free prefix and realizable paid columns.
It counts paid columns actually used by those reconstructions; minimizing the
basis of the entire supplied space is a different objective. A bounded beam
retains at most eight certificates and evaluates at most 4096 exchanges over
at most 2048 ground columns. Exchanges remain within the initial active span
and do not increase the paid-column count. No global-optimum/ABB-bound or RL
claim is made. All returned coordinates retain original canonical arc IDs.

The existing example accepts optional whitespace-separated column costs:
`binary_matroid MODEL SCOPE WEIGHTS.txt`. A fourth argument `FREE:TARGETS:REQUIRED`
enables reconstruction search: the first FREE columns are available without
cost, the last TARGETS are reference-only, and the last REQUIRED are requested
outputs. Remaining columns are physically realizable alternatives. For the
composed inverse demonstration this is `16:10:7`. The provider solver is reused;
the example owns only argument/source/result transport. Focused validation is
`cargo test --locked --offline -p hymeko_graph --test matroid_binary --example binary_matroid`.
Cross-repository native evidence is in
`../transformation/reports/kaggle-quantum-composed-inverse-20261030/`; the shared
report archive stores it under Transformation's branch directory. These APIs
remain local uncommitted integration work until separately published.

`direct_ir::Christofides::solve(&index, &arc_costs, TourLimits::default())`
constructs a closed walk visiting every canonical node. `arc_costs` contains one
finite nonnegative symmetric traversal cost per canonical hyperarc. The returned
`BergeTour` contains the metric visiting order, the expanded incidence walk,
total cost, MST/matching costs and the number of odd tree vertices. Repeated
nodes and hyperarcs are allowed and each traversal is charged again.

This applies metric Christofides to shortest Berge distances; it does not solve
the simple Berge-Hamilton-cycle or pay-once hyperedge-cover problem. The classical
3/2 guarantee applies in exact arithmetic. The implementation uses the existing
canonical-IR Dijkstra engine for metric closure and path expansion. It stores a
derived quadratic distance table, with no second IR or alternate search engine.
Its initial exact minimum matching backend is bounded subset dynamic programming,
not polynomial-time blossom: default limits are 256 nodes, `2^20` matching states
and 1,000,000 returned steps. Exceeding a limit returns `TourError::Budget`;
there is no silent greedy fallback. General floating-point inputs do not carry
an interval-certified approximation guarantee. Correctness checks and full-route
comparison evidence are in `reports/2026-10-22-path-tour-comparison/`.

The larger-input follow-up is in `reports/2026-10-22-path-tour-scaling/report.pdf`:
262,144-node / 1,046,528-incidence path input and 24-, 32- and 64-node tours,
with the same production solver and default budgets. The 64-node street input
hits the matching-state cap; the 64-node corridor with transport hyperarc completes.
On the 24- and 32-node inputs the faster HyMeKo tour has a slightly higher cost
than the external Christofides composition. The report keeps latency, returned
cost, lower-bound certificates and unsupported budget cases separate.

`direct_ir::PageRank` ranks canonical IR nodes without materializing a pairwise
graph. `PageRank::build(&ir, PageRankModel::default())` prepares an undirected
vertex–hyperarc–vertex probability walk. `run(config, &mut workspace,
personalization)` uses reusable `PageRankWorkspace` buffers and returns scores in
`NodeId` order, iteration count, an L1 error bound and explicit convergence status.
Reaching the iteration cap returns `converged=false` rather than claiming success.

The existing informed evidence client also prepares a `--suite search_reuse`
control. It compares complete fixed-goal batches of 1, 8 and 64 unique sources:
native Berge/BFS on unit grids, Dijkstra, caller-heuristic A*, and Berge-assisted
A*. Each batch includes one reverse-index build, applicable caller/goal
preparation, fresh workspace reused within the batch, all complete paths and
temporary disposal. Only the canonical IR and deterministic query list are
resident beforehand. The reported time is **per complete batch**, not per query.
Small batches are prefixes of the same seeded query list. Full witnesses,
optimal costs, reachability and available search counters are retained; BFS
counter fields are `NA`, not fabricated zeros. Process memory includes reference
verification state and is not an isolated-memory comparison.

`assisted`, `routing` and `routing_unit` accept the existing `grid200_d20`,
`grid200_d40` and `grid200_d80` fixtures explicitly; their historical defaults
are unchanged. The first two new distances can therefore use the same canonical
export and external full-route clients. Example preparation, without timing:

```powershell
$env:HYMEKO_ROUTING_EXPORT = 'routing-inputs.json'
$env:HYMEKO_ROUTING_CASES = 'grid200_d20,grid200_d40,grid200_d80,map128,transport128'
cargo test -p hymeko_graph --test informed_berge_evidence --locked assisted::export_routing_input -- --exact
cargo test -p hymeko_graph --test informed_berge_evidence --locked --release --no-run
```

When a measurement is separately authorized, use the existing controller, for
example on Linux with the prepared release test executable and host-specific
quiet policy:

```text
python scripts/performance_evidence/campaign.py --platform native --suite search_reuse
  --informed <release-test-executable> --source-manifest <source-manifest.json>
  --quiet-config <quiet.json> --cpu <logical-cpu> --runs 5
  --workloads grid200_d20 --subjects batch_bfs_1 batch_astar_1 batch_assisted_1
  --output <new-output-directory>
```

Join the displayed lines into one command. The controller supplies the explicit
`HYMEKO_ASSISTED_BENCH=1` gate; the test entry point is ignored in ordinary tests.
The full native reuse matrix has 54 cells. External multi-query batches,
changing-goal reuse and directed cross-framework contracts are not implemented
by this extension. Preparation checks and the untimed canonical export are in
`reports/2026-10-27-search-followup/`; no new performance result is claimed.

Automatic solver selection is available through the same prepared operator:

```rust
let result = rank.run_with_solver(
    config, &mut workspace, personalization,
    PageRankSolver::Auto(PageRankAutoConfig::default()),
)?;
let decision = result.automatic.expect("Auto selection evidence");
```

Auto advances power iteration in four-update batches and predicts remaining work from its
observed contraction and the existing structural plan. It rechecks the decision
until power converges or selects a Krylov phase, comparing that estimate
with a bounded matrix-free GMRES phase. The default phase has at most 20 Arnoldi
steps (Tensor may restart internally) and
admits at most 64 MiB of requested basis payload. This cap excludes other scratch,
allocator overhead and retained capacity from previous calls. The Krylov solver
starts from the prefix's scores; if its canonically checked candidate is not yet
converged, power iteration continues within the original total iteration budget.
Nonserial workspaces use the existing parallel power path. All methods preserve
the same personalization, directed/weighted semantics and canonical output order.

`result.automatic` records the reason, selection-check count, predicted work, basis admission and actual
power/Krylov update counts. Predictions are deterministic heuristics, not timing
or convergence certificates. `run` and the default explicit strategy still use
power iteration; Auto is explicitly selected. The existing
`--suite ranking_solvers --subjects rank_auto` evidence client includes the probe,
selection, fresh workspace and full solve in one measured call. It retains the
decision in the results and rejects missing or inconsistent Auto provenance.
Historical Power/GMRES minima are not Auto measurements. New measurements remain
behind the existing explicit opt-in and quiet-host checks.

The same `ranking_solvers` and `memory` clients accept application models through
`--application-input <catalog.json>`. The catalog contains `root`, `cases`
(`app_name` to relative `.hymeko` path), and `files` (relative source/import paths
to SHA-256). Workloads and native strategies must be selected explicitly. The
controller verifies these source bytes before each child; Core's `CompiledModel`
loads the model and retains its canonical IR tables. Compilation is outside the
ranking timer. The memory client also retains the compiler-produced IR, including
its metadata and isolates; the incidence export serves independent verification,
and is not used to rebuild a simplified application IR. No solver or Auto policy
is changed by this input path.

Example selection: `--suite ranking_solvers --workloads app_model --subjects
rank_auto rank_hymeko rank_gmres --application-input catalog.json`, with the
existing binary, native parity library, source manifest and quiet-host arguments.
Memory selection uses `--suite memory --subjects hymeko-auto-fresh hymeko
hymeko-gmres-fresh` and the matching per-case incidence/rank exports. The compiled
export helper is `native::application_source_export` in `performance_evidence`;
it requires explicit `HYMEKO_APPLICATION_SOURCE` and `HYMEKO_APPLICATION_EXPORT`
paths and performs correctness checks without measuring. The six-model control
is documented in `reports/2026-10-30-application-pagerank/`.

`PageRankAutoConfig { quotient: PageRankQuotient::Pendant, ..Default::default() }`
optionally considers a mass-coordinate quotient for Tensor GMRES. For unit
undirected walks it groups true isolates and vertices with exactly one incidence
in the same hyperarc; all other vertices remain separate. Normalized
personalization must be equal inside every group. A cheap bound must guarantee
at least one-quarter fewer coordinates before partition construction is admitted.
The original incidence/range transition is still applied through lift/restrict;
only Krylov basis and orthogonalization dimensions shrink. Auto prices the extra
setup and vector work, keeps its power prefix, and certifies the expanded result
with the original canonical update. Unsupported or unprofitable cases retain the
full-domain strategy. This is an execution view over the same IR, not a second
graph model. Quotient preparation is lazy and warm buffers are reusable. The
default is `Disabled` pending measured evidence. The explicit timing subject is
`rank_auto_quotient`; isolated-memory subjects are
`hymeko-auto-quotient-fresh` and `hymeko-auto-quotient-reused`.

The completed 27 October Amaterasu control compares Auto, power, GMRES,
Hypergraphz and HyperNetX+SciPy on four graph families with two vertex numberings:
40 configurations, five fresh processes each, and 1,800 retained timing samples.
Auto has a lower median than both external frameworks on all eight inputs;
their time divided by Auto time ranges from 1.075 to 2.848 for Hypergraphz and
1.042 to 1.674 for HyperNetX+SciPy. Preparation, workspace allocation, selection
and the full solve are included; initial graph construction is excluded.
Explicit GMRES remains 4.4--5.4% faster than Auto on the two-member inputs.
These families informed the selection policy, so this is not a held-out
generalization study or a universal framework ranking. The PDF, raw records,
source identities and reproduction bundle are retained in
`reports/2026-10-27-pagerank-auto/` and the repository's Samba report archive.

The separate kato21 follow-up keeps that Auto policy fixed on four additional
synthetic families (32-member, 1000-member, mixed-arity and sunflower), each with
original and seed-17 numbering. Its 200 fresh processes put Auto within 1.4% of
the faster explicit HyMeKo solver by process-median aggregates. Auto beats both
external comparators on the two sunflower inputs; Hypergraphz and HyperNetX+SciPy
are faster on the other six. These are additional designed inputs, not an
independent real-world generalization study, and are not pooled with Amaterasu.

The same follow-up adds 120 isolated-memory processes on the original four
topologies with identity numbering. `--suite memory` accepts
`hymeko-auto-fresh` and `hymeko-auto-reused`, retaining checked selection decisions
and workspace lifetimes. Auto fresh has median solve-phase process peaks of
34.95--196.01 MiB, versus HyperNetX+SciPy at 4.9--12.2 times those values;
Hypergraphz has lower peaks on two of the four inputs. Retaining workspace
capacity does not lower RSS here. The full controls, source snapshot, raw records
and five-page PDF are in `reports/2026-10-27-pagerank-auto/followup/`.

A subsequent native control evaluated a tighter structural search bound and
two-sweep GMRES candidate preparation together. Although their correctness checks
passed, the 120-process before/after control did not reduce Auto medians: they
were 0.04--4.60% higher. A separate 30-process reversed-order repeat reproduced
the regions slowdown (Auto +4.43%, Power +7.77%, GMRES +3.50%). The pre-change
structural bound and five-sweep preparation have therefore been restored;
stronger candidate-error and warm-reuse checks remain. This control does not
attribute the slowdown to either individual edit. Both cohorts and the PDF are
retained in `reports/2026-10-27-pagerank-auto/bounded-control/`.

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

The same controller also provides `--suite external_ranking` and Linux-only
`--suite memory`. External PageRank combines each framework's native incidence
export with the pinned SciPy GMRES solver; these rows are explicitly composed
workflows. Full-vector and independent residual checks preserve the canonical
node domain, isolates and duplicate-hyperedge multiplicity.

For isolated memory evidence, `MemoryEvidence.split(source, destination)` in
`scripts/performance_evidence/memory.py` splits an existing canonical export into
one JSON and compact binary input per case. Pass the resulting `manifest.json`
as the controller's `--external-input`, together with the native evidence binary,
native library and pinned external Python environment. Each cell owns one
framework and releases fixture/oracle data after graph construction. Runtime,
input, retained graph, preparation and solve RSS are reported separately; Linux
phase peaks reset via `clear_refs=5`. Verification runs in the controller after
the child exits. Measurement requires the explicit `HYMEKO_MEMORY_BENCH=1` gate;
normal tests never run it. RSS includes allocator-retained pages and is not an
intrinsic live-object byte count. The five-process control and PDF are in
`reports/2026-10-24-isolated-memory/`.

`--suite ranking_phases` prepares a paired HyMeKo/HyperNetX+SciPy timing matrix:
the same four graph families and both numberings, each with uninstrumented and
phase-observed whole calls (32 cells, five processes per cell for a full control).
The observer records incidence extraction/binding, operator preparation,
solve/result handling and temporary disposal within each sampled call. It reuses
the existing production entry points and numerical checks. Phase totals are
checked against their own whole-call sample; independent phase medians must not
be added and presented as an end-to-end time. `phase-samples.csv` retains the
same-call observations alongside the standard whole-call summary.

Observed cells require `HYMEKO_PHASE_BENCH=1` in addition to the existing explicit
benchmark gate. Ordinary tests use fake clocks or boundary-only callbacks and
do not run these measurements. The prepared Linux command uses the existing
controller with `--suite ranking_phases --runs 5`, the native evidence binary,
Hypergraphz parity library, pinned external Python environment, canonical export,
source manifest, `--quiet-config` and `--cpu`. A one-process four-cell pilot on
`--workloads rank2_1m` must pass before the full matrix is started. The phase
study has release builds on Windows and Amaterasu/Linux. The Amaterasu phase
control completed 160 fresh processes with 1,440 retained samples and 720
same-call partitions. The solve/result stage accounts for 86--94% of HyMeKo's
observed call time. The Windows timing control still awaits a quiet host.
The host gate checks CPU activity, memory and interactive users. Declared Linux
service identities match an exact UID, command and cgroup; their CPU activity
remains subject to the same limits. Transient activity can be given a bounded
wait without weakening admission thresholds.

`PageRank::run_with_solver` now selects `PageRankSolver::PowerIteration`,
`OrderedPowerIteration`, or `Gmres { restart: 20 }` on the same prepared model.
`run` and the default strategy retain structural power iteration. The ordered
power strategy retains canonical incidence order. GMRES reuses Tensor's
`hymeko_numerics` implementation and the same structural range contraction as
power iteration when the prepared model selects it; directed, weighted and
unstructured inputs retain the ordered contraction. Signed Krylov vectors do not
pass through probability clipping or mass validation during operator application.
No transition matrix is constructed, and GMRES currently requires one worker.
Its basis costs O(nodes * restart) additional storage. The final
canonical probability update certifies the same global L1 bound; Tensor's L2
stopping flag alone cannot certify convergence. The iteration budget includes
this final update.

`--suite ranking_solvers` now compares four HyMeKo strategies (structural power,
ordered power, GMRES and Auto), Hypergraphz and HyperNetX+SciPy: 48 configurations
across the eight existing inputs. The earlier five-strategy control below
predates Auto. The default strategy remains structural power. That five-process
Amaterasu comparison, before
structural contraction was connected to GMRES, found GMRES faster on both
two-member and hub layouts, but slower on chains and regions. That comparison
does not measure the subsequent structural GMRES integration. Its matched follow-up
is summarized below; signed-vector and dense-oracle checks pass on Windows and Linux.
For the original two-member case (65,536 nodes, 500,000 arcs), median whole-call
times are 192.81 ms for structural power, 103.63 ms for GMRES, 248.69 ms for
Hypergraphz, and 112.00 ms for HyperNetX+SciPy. HyperNetX's median time is 1.081
times the GMRES time on that case; this is not a universal framework ranking.
The solver control contains 200 fresh processes and 1,800 retained samples.
Full-scale correctness checks also pass on both hosts. Native process RSS includes
both parity graphs and fixture validation; it does not isolate Krylov workspace memory.
Build identities, guarded pilot commands, checks and the PDF are retained under
`reports/2026-10-25-pagerank-multihost/` and the repository's Samba report archive.

The structural GMRES follow-up on Amaterasu retains the same inputs and L1 target:

| Input | GMRES before (ms) | GMRES with ranges (ms) | Hypergraphz (ms) | HyperNetX + SciPy (ms) |
| --- | ---: | ---: | ---: | ---: |
| Hub | 117.58 | 88.55 | 181.06 | 76.16 |
| Hub, relabeled | 162.20 | 97.75 | 220.19 | 102.23 |
| Eight regions | 133.47 | 90.14 | 166.36 | 100.71 |
| Eight regions, relabeled | 184.19 | 101.93 | 210.82 | 140.05 |

These are medians of five process medians: 100 fresh control processes and 900
retained batches, including the separately reported default-power controls.
All cases have 65,536 nodes, 31,250 hyperarcs and one million incidences. Reusing
the shared range contraction improves GMRES by 1.33--1.81 times, with unchanged
22/25 step counts. GMRES has a lower median than Hypergraphz in four cases and
HyperNetX+SciPy in three; the original hub remains faster with HyperNetX+SciPy.
Two CPU profiles confirm range execution and retain substantial GMRES/vector
cost. This does not establish a global solver default. Raw data, quartiles,
profiles, exact source/binary identities and the two-page PDF are in
`reports/2026-10-25-pagerank-multihost/structural-control/` and the Samba archive.

The workspace follow-up (25 October) makes PageRank retain Tensor's shared
`GmresWorkspace`, its right-hand side and output buffers. It passes an absolute
L2 target to avoid a duplicate right-hand-side norm; the final canonical L1
certificate is unchanged. A warm GMRES call allocates no storage in the focused
structured/directed checks. The workspace retains its basis capacity; this is
not a lower-RSS or newly measured speed claim.

The manifest and lockfile pin the published Tensor provider revision; no sibling
checkout override is required. Mathematical background is in the Tensor
repository's `docs/gmres/mathematical-summary.pdf`, with its LaTeX source beside
it. Detailed validation evidence is retained in Tensor's
`reports/2026-10-25-gmres-workspace/` and the Samba archive.

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

`PreparedBergeSearch::build(&index, goal, &mut traversal_workspace)` also retains
exact incidence links toward the goal and a relaxed hop bound per hyperarc.
`search_with_heuristic(start, &cost, &heuristic, &policy, &mut workspace)` uses
`f = g + max(caller_bound, hop_bound)` in the same A* engine. The supplied bound
must be admissible and consistent for this goal, cost model and policy. Entropy
only orders equal-f candidates. `search` supplies the zero caller bound.

Each query checks the recorded route's forward direction and costs before using
its cost as an initial upper bound. A rejected or overflowing optional route
falls back to ordinary search. Certified hyperarc bounds can skip whole member
expansions; equal-cost and floating-point-ambiguous candidates remain eligible.
`SearchStats` includes skipped arcs, witness checks and acceptance of the initial
bound. The prepared view borrows one immutable canonical IR; rebuilding it after
topology changes and its preparation cost must be counted in lifecycle benchmarks.
The initial kato21 timing control covers three fixed query pairs, five processes
per cell. Prepared assisted queries took 5.224 us on the 40,000-node grid,
2.840 ms on the 16,384-node weighted road graph, and 0.276 us on its one-hop
transport example. Relative to plain A*, the grid and transport queries were
2.40x and 1.64x faster; the weighted road query was 15.43% slower. Fresh
index/setup/search from resident IR took 7.033, 5.391 and 2.286 ms respectively,
and was slower than plain A* in all three cases. IR construction is excluded.
These are separate prepared-query and first-result boundaries, not universal
speedup claims. Raw samples, source identities and phase definitions are in
`reports/2026-10-22-berge-bound-timing/report.pdf` and its sibling evidence files.

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

## Native development with a shared library

Use `cargo dev-test --test pagerank` or `cargo dev-test --test informed_search_vv --test metric_tour` from this workspace. Twenty-seven existing correctness suites and two tools have a shared-library build; their source files and production implementations are reused directly. See [the development linkage contract](dev/README.txt). Allocation instrumentation and benchmark/evidence targets retain their original static builds.

## Partial P-graph constraints and binary lifetime frontiers

`ssg_dm::enumerate_constrained` extends the existing decision-mapping engine.
`SsgConstraints` accepts exclusive producer materials, exclusive unit groups,
forbidden units, nonnegative additive lower costs, a strict cost ceiling and a
partial-state cap. Defaults preserve the original SSG API's structure set and
order. Singleton producer choices are enumerated directly. The covering bound
uses committed cost plus the maximum of per-demand cheapest available producer
costs; a sum would double-count shared producers. Only certified lower costs
belong here. Output/node caps report incomplete enumeration.

The existing CLI exposes `pgraph constrain MODEL REQUEST.json`. It recompiles
the source, checks the semantic digest, and returns structures, counters and the
request. Opaque provenance is echoed, not interpreted as quantum semantics.
Quantum owns interpretation of register use and certificate scope. No Graph
dependency upgrade or second search implementation is required for this client.

`matroid::binary_frontier_search` searches paired input/output column orders,
using exact binary bases and XOR reconstruction from the canonical incidence.
At a gate it counts input rank before retirement and output rank after insertion.
Its beam/budget limits are explicit; it models separate linear banks, not a
complete reversible gate implementation. `binary_frontier_barrier` verifies a
sufficient grouping theorem proving full input-plus-output rank for every order.
The proof and its scope are documented with the API; unsuccessful certification
means unknown. This does not bound changed tensors or nonlinear state updates.

Thin clients remain in the existing `binary_matroid` example:

```text
binary_matroid --frontier MODEL INPUT_SCOPE OUTPUT_SCOPE PRODUCT_SCOPE [BEAM BUDGET]
binary_matroid --barrier MODEL INPUT_SCOPE OUTPUT_SCOPE PRODUCT_SCOPE GROUP_SIZE
```

The full24 Quantum fixture generated20 structures instead of a capped16384,
preserving12 executable programs. Its tensor99 passes a48-rank separate-bank
barrier; a46-capacity direct99 request excludes two networks before generation.
These are structural/algebraic results, not a runtime benchmark or a new Kaggle
score. Consumer report:
`../transformation/reports/kaggle-quantum-reverse-state-20261030/partial/report.pdf`.
