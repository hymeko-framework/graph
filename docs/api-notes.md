# Graph API notes

Contracts of selected `hymeko_graph` and `hymeko_pgraph` APIs and of the
performance-evidence clients. The [README](../README.md) gives the overview.
Timing cells and measurement suites are opt-in; the campaign retained with this
repository is in [evidence/graph-algorithms](evidence/graph-algorithms/).

## Binary matroids

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
outputs. Remaining columns are physically realizable alternatives. The provider solver is reused;
the example owns only argument/source/result transport. Focused validation is
`cargo test --locked --offline -p hymeko_graph --test matroid_binary --example binary_matroid`.

## Metric tours

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
an interval-certified approximation guarantee.

## PageRank and its evidence clients

`direct_ir::PageRank` ranks canonical IR nodes without materializing a pairwise
graph. `PageRank::build(&ir, PageRankModel::default())` prepares an undirected
vertex–hyperarc–vertex probability walk. `run(config, &mut workspace,
personalization)` uses reusable `PageRankWorkspace` buffers and returns scores in
`NodeId` order, iteration count, an L1 error bound and explicit convergence status.
Reaching the iteration cap returns `converged=false` rather than claiming success.

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

The same controller also provides `--suite external_ranking` and Linux-only
`--suite memory`. External PageRank combines each framework's native incidence
export with the pinned SciPy GMRES solver; these rows are explicitly composed
workflows. Full-vector and independent residual checks preserve the canonical
node domain, isolates and duplicate-hyperedge multiplicity.

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

Mathematical background for the GMRES solver is in the tensor repository's
`docs/gmres/mathematical-summary.pdf`.

## Signed graphs and P-graph schemas

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

## Informed search

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

Informed-search tests share borrowed cases and static Dijkstra/A* strategies.
Evidence clients share checksum/work verification and typed TSV result rows;
timing entry points remain explicitly opt-in. The declaration-permutation test
compiles two reordered sources, checks actual ID changes and remapped references,
and rejects use of a stale dense node ID.

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

To take a measurement, use the existing controller, for example on Linux
with the prepared release test executable and a host-specific quiet policy:

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
by this extension.

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
request. Opaque provenance in the request is echoed, not interpreted.

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
