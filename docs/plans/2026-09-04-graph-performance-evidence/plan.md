# Graph performance evidence — justification and migration plan

**Created:** 2026-09-04 CEST

**Branch:** `feat/codex-p34-independent-audit`

**Base:** `2247f2d44e700e36aeea633320d2fca36bc249a5`

## Decision sought under CORE.YAML

`CORE.YAML` protects the `hymeko_graph` and `hymeko_pgraph` crates and requires an
explicit approval after a written justification and migration plan. The proposed work
changes measurement clients under `hymeko_graph/tests`; it does not change a production
module, public API, dependency, canonical topology, algorithm, or output contract.

Approval is requested for this exact protected edit surface:

- add the integration-test entry point `hymeko_graph/tests/performance_evidence.rs`;
- add test-only modules below `hymeko_graph/tests/performance_evidence/` for statistics,
  telemetry and representation controls.

The historical `p34_cell.rs`, P3.4 claim harnesses and their raw data remain unchanged.

No `hymeko_graph/src`, `hymeko_pgraph/src`, Cargo manifest/lock, or core repository file
will be changed. If implementation discovers that a production API or dependency change
is needed, work stops for a new approval rather than expanding this boundary.

## Why the protected test edit is justified

The independent audit established that the algorithms are usable but the committed
measurement evidence is not release-grade:

1. all 51 peak-RSS values are zero because `PeakWorkingSet64` is read only after exit;
2. process-lifetime CPU is divided by a one-operation median wall interval;
3. the batch cell has only three measured samples in the original harness;
4. the break-even paths do not use equivalent preparation/query lifecycles;
5. the alleged Levi O(1) query is only a cached scalar read and no certificate-query API
   exists;
6. representation identity is prose rather than typed structural metadata; and
7. graph's IR-operation table duplicates core ownership while still omitting most core
   operations.

Script-only correction can recover whole-process peak RSS by polling a direct child, but
it cannot place CPU/RSS boundaries around the operation or emit phase milestones. The
new isolated `performance_evidence` client supplies those observation points without
rewriting the historical P3.4 cell. This is measurement instrumentation, not production
behavior.

## Measurement contracts

### Statistics

- two untimed warmups;
- nine measured samples (hard minimum five);
- raw per-sample rows plus median, Q1, Q3 and worst;
- deterministic integer checksums for every cell;
- alternating order and an A/A null where ratios below 10% are interpreted;
- no single-shot or best-of value presented as a benchmark;
- timer-resolution-limited operations are batched and normalized, not printed as zero.

### CPU and RSS

The PowerShell harness will invoke the compiled `performance_evidence` test executable directly.
It will fail closed on missing output, non-zero exit, duplicate cell identity or checksum
drift; stderr remains visible in diagnostics.

Two scopes will be kept separate:

- **operation scope:** process CPU delta and wall time captured inside the cell around
  each measured operation sample;
- **process scope:** child wall/CPU plus a polled maximum working set from process start
  to exit.

The cell will emit working-set milestones before workload creation, after canonical Ir,
after `IncidenceIndex`, after workspace/output preparation, and after measurement where
Windows exposes them. External polling supplies the independent whole-child peak. A
same-binary `--list` process supplies process-start control overhead. Unsupported
platform metrics are `UNMEASURED`, never zero-filled.

### Scaling

The work remains exactly 64 independent serial reachability queries for every thread
count. It will be named `independent_query_batch`, not single-query acceleration.
Thread counts are `1,2,4,8,16,min(24, logical_processors)`. The query set, graph,
preparation and checksum must match at every count. Speedup uses the median operation
wall of the same workload's one-thread cell; efficiency is speedup/thread count.

### Comparable representation measurement

The representation experiment asks one question only: compute the global canonical
node component count from a prepared representation.

- common input: the same canonical `Ir`;
- preparation: build the representation and reserve reusable workspace exactly once;
- query: recompute the component count from that representation into reused workspace;
- output: one `usize` count for every variant;
- lifecycle: no variant reads a count cached during preparation;
- total: `prep + Q * warm_query` with unrounded raw values;
- memory: representation, workspace and output retained bytes remain separate.

Variants are represented by a test-only `RepresentationKind` enum (`CanonicalIndex`,
`ImplicitLevi`, `SparseStar`, `Clique2Section`) whose stable key is written to TSV. Sparse
star remains bipartite O(nnz); clique remains O(sum k^2). Levi reports
`C_node = C_levi - zero_incidence_arcs`. No strict-O(1) certificate query or pair
connectivity claim is made.

### IR operations

Canonical Ir belongs to `hymeko/core`, not graph. This suite will not grow another graph-owned IR
benchmark implementation. The graph report will instead:

- pin the exact resolved core revision (`70b4f364`);
- enumerate the operation coverage of that revision's core Criterion benches;
- run those benches from a detached, read-only pinned core worktree with all target/temp
  output under `D:\h\codex-graph-p34-audit`;
- mark operations absent at that revision `UNMEASURED` or `NOT IMPLEMENTED` rather than
  filling them with graph-local reconstructions.

This removes a duplicated measurement authority rather than expanding it.

## Durable directory layout and versioned artifacts

The roadmap code `P3.4` appears only as provenance in the report, not as the suite name.
The audited 2026-09-03 claim set remains untouched. New artifacts are grouped by purpose
and run date:

```text
hymeko_graph/tests/
  performance_evidence.rs
  performance_evidence/
    statistics.rs
    telemetry.rs
    representations.rs
scripts/performance_evidence/
  run.ps1
  figures.ps1
reports/performance-evidence/2026-09-04/
  report.md
  manifest.tsv
  data/
    cells.tsv
    samples.tsv
    rss.tsv
    representations.tsv
    core-ir-coverage.tsv
  figures/
    batch-scaling.svg
    process-memory.svg
    representation-total.svg
```

Future runs receive a new dated report directory. Stable runner and test paths therefore
do not encode a roadmap phase or version number.

The manifest records source commit, binary hash, workload seed, compiler/profile, host,
logical processors, warmups, samples and exact command for every data family.

## Implementation sequence

1. Commit this plan without touching protected files.
2. After explicit approval, write discriminating harness tests for sample count, quartile
   order, child failure, missing metric, representation identity and semantic parity.
3. Add the isolated `performance_evidence` cell with per-sample operation CPU/wall and
   phase RSS while preserving all established operation checksums.
4. Make the PowerShell runner poll the direct child, validate exit/output, and preserve
   samples rather than suppressing them.
5. Add the lifecycle-equivalent representation measurement with typed identities.
6. Run the pinned core-owned IR benchmark inventory; do not copy it into graph.
7. Execute measurements only after a quiet-host gate, validate every TSV schema and
   checksum, regenerate the date-scoped figures, and write the evidence report.
8. Run root fmt, clippy, check, build, tests, doctests and separate benchmark smoke.
9. Commit instrumentation, raw evidence and report separately; push only the audit
   branch after local/remote hash equality is verified.

## Acceptance and rollback

Acceptance requires:

- no production source/API/dependency diff;
- old P3.4 and audit artifacts retain their SHA-256 values;
- at least nine valid samples per reported cell;
- `q1 <= median <= q3 <= worst` for every summary;
- non-zero, finite Windows RSS for successful non-empty cells and external/self peak
  consistency explained;
- process CPU and operation CPU never mixed in one utilization formula;
- identical workload/query/checksum at every scaling thread count;
- representation parity for every workload and no cached-result query;
- all repository and benchmark-workspace gates green.

Rollback is commit-level revert of test instrumentation, scripts and evidence artifacts. The
production library remains byte-identical throughout, so rollback cannot alter runtime
behavior.

## Approved follow-up amendment: Dijkstra and comparability closure

**Approved:** 2026-09-04 CEST, explicitly by the user after review of the first evidence run.

**Approval text:** “Jóváhagyom a Dijkstra slot-iteráció minimalista production javítását, a
benchmark-korrekciókat és az újramérést; core IR layoutváltás nélkül.”

The first evidence review found two benchmark weaknesses and one bounded production hot path:

1. Dijkstra evidence checks only path length even though `WeightedBergePath::path_hash()` can pin
   the full canonical path and cost.
2. The representation cell returns the same scalar, but its canonical path constructs a complete
   `ComponentLabeling` while the controls compute only a count. That confounds representation and
   output work.
3. `astar_berge_path` iterates `incident_arcs(node)` and then calls `u_slot_in`, which linearly
   rescans the same node row for every newly activated arc. On STS(1023) this executes 57,303,686
   slot comparisons for 348,502 target relaxations and changes the documented linear walk into a
   high-degree `sum(degree^2)` term.

The approved production surface is exactly
`hymeko_graph/src/direct_ir/informed.rs`: zip the already-parallel `incident_arcs` and
`incident_slots` slices and pass the matching slot directly to `resolve`; remove the now-unused
linear `u_slot_in` helper. Public types, signatures, ordering, cost/policy semantics, topology and
dependencies remain unchanged. Existing path hashes must remain byte-identical across the change.
Rollback is the single production-fix commit.

The approved test/evidence surface is the existing `performance_evidence` client and scripts:

- use the full `path_hash()` as Dijkstra's deterministic checksum;
- emit exact `SearchStats` work counters for every Dijkstra cell;
- replace the label-producing canonical representation query with a test-only count-only kernel
  over canonical arcs and the same control DSU lifecycle used by other count-only variants;
- report canonical workspace bytes instead of `UNMEASURED`;
- add fixed-near-one-million-incidence rank-2/3/8/32/128 representation workloads; and
- preserve the first run under `reports/performance-evidence/2026-09-04`, writing the corrective
  run to `reports/performance-evidence/2026-09-04-dijkstra-comparability`.

The canonical count kernel is benchmark-control code only. It neither becomes a production
algorithm nor creates retained topology: it scans the borrowed canonical `Ir`, unions each
hyperarc's node members in reusable scratch and returns one `usize`, exactly like the other
variants. The separate production `components` operation cell continues measuring the public full
labeling API.

Follow-up acceptance requires all original gates plus: pre/post Dijkstra path hashes match on all
five operation workloads; Dijkstra work counters are non-zero and deterministic; every
representation returns the same count; each representation query returns only the count; all five
arity cells finish without an unreported OOM/skip; the old evidence directory and all historical
P3.4 artifacts remain byte-identical; and no core repository, Cargo manifest/lock or production
file outside the one approved graph source is changed.
