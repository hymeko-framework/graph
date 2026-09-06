# Independent Codex audit — P3.4 architecture, duplication and performance

> Historical audit of the September 3 revision. Section 23 records the September 6 integration into the canonical Graph checkout and supersedes the old retained-worktree instructions.

**Audit date:** 2026-09-03 CEST

**Repository:** `hymeko/graph`

**Audited ref:** `feat/graph-matroid-navigation-p34`

**Audited commit:** `ce841343b428ba1cfa274e225ae5646606eb8a50`

**Expected and verified main:** `b234c4871807dc255854cc4b50d1458547dca7d3`

**Audit branch:** `feat/codex-p34-independent-audit`

## Executive verdict

P3.4's semantic core is substantially sound, but the release-wide `COMPLETE` verdict is
**REFUTED**. The two matroids genuinely share one production DSU, `MatroidRef` is a
one-word zero-copy binding, and the independently recomputed rank and T/S cardinality
results agree. However:

- the committed benchmark workspace did not compile at the audited tip;
- all 51 committed peak-RSS cells are zero, so peak RSS was not measured;
- the advertised Levi break-even compares unlike lifecycles and reduces the alleged
  query to reading a cached scalar;
- the 24-thread result is fixed-work **independent-query batch parallelism**, not a
  parallelized single graph query;
- the six-figure allocation totals are dominated by workload/IR construction, not by a
  warm matroid operation;
- several exact timing and scaling headlines do not reproduce on the audit host; and
- the promised fundamental-operations matrix omits most of the requested operations.

The audit found one qualifying production semantic defect: incremental rank-2 and Levi
sessions accepted elements from a foreign canonical `Ir` snapshot, and rank-2 could
panic on a mismatched node universe or out-of-range arc. Failing tests were committed
first; the minimal fix binds a stream to one snapshot and returns typed errors. No
topology copy or steady-state allocation was added. The separate benchmark lock was
also corrected to the already tested root revisions.

The complete per-claim table is
[`2026-09-03-codex-p34-claims.tsv`](data/2026-09-03-codex-p34-claims.tsv), the complete
duplication inventory is
[`2026-09-03-codex-p34-duplication.tsv`](data/2026-09-03-codex-p34-duplication.tsv), and
all independent numeric evidence is in
[`2026-09-03-codex-p34-rerun.tsv`](data/2026-09-03-codex-p34-rerun.tsv) and
[`2026-09-03-codex-p34-allocations.tsv`](data/2026-09-03-codex-p34-allocations.tsv).

## 1. Entry integrity and remote verification

The audit began from an isolated clone with a clean checkout before any edit. Remote
resolution at entry was:

| Ref | Verified object |
|---|---|
| `origin/feat/graph-matroid-navigation-p34` | `ce841343b428ba1cfa274e225ae5646606eb8a50` |
| `origin/main` | `b234c4871807dc255854cc4b50d1458547dca7d3` |

`origin/main` is an ancestor of the audited feature. The audit branch was created
directly from `ce84134`; main was neither checked out for editing nor merged. The
original P3.4 report, TSVs and figures remain byte-identical; their frozen hashes are in
[`2026-09-03-codex-p34-artifact-sha256.tsv`](data/2026-09-03-codex-p34-artifact-sha256.tsv).

The user's pre-existing core and graph checkouts were read only for fleet context and
were not changed. At entry, the pre-existing core checkout was dirty and 29 commits
behind its remote main; the pre-existing graph checkout was clean and 37 commits behind.
Those states were preserved rather than normalized.

The final pushed audit object and `ls-remote` equality are reported in the terminal
handoff because the report commit itself determines that object ID.

## 2. Host and baseline population

Host: Windows 11 Home build 26200, Intel Core Ultra 9 275HX, 24 logical processors,
63.42 GiB RAM, rustc/cargo 1.97.1, Git 2.55.0. The initial 39% ambient-load window was
rejected. Independent timings were taken after a five-second observation averaging
12.0% host CPU with a 20.1% maximum. This is a non-laboratory workstation run; the
distribution and A/A controls matter more than sub-nanosecond headline precision.

Before source changes, fmt, clippy, check, build, unit tests, integration tests and
doctests were green. Tests were enumerated by name:

| Population | Named | Passed | Ignored |
|---|---:|---:|---:|
| unit (`hymeko_graph` 143, `hymeko_pgraph` 47) | 190 | 190 | 0 |
| integration targets | 351 | 340 | 11 |
| doctests | 5 | 3 | 2 |
| **baseline total** | **546** | **533** | **13** |
| **post-audit total** | **552** | **537** | **15** |

The post-audit increase is four passing semantic regression tests and two deliberately
ignored independent measurement tests. All post-fix gates are green, including all 552
named tests under their intended ignored/non-ignored policy.

Benchmark smoke was not green at baseline. `cargo bench --workspace --no-run` in the
separate `benchmarks` workspace failed at eight imports of `hymeko::ir::incidence`.
Its lock selected core `75c6c7b2` (before the incidence module) and parser `284c490d`,
whereas the root selected compatible core `70b4f364` and parser `e28a2051`. After the
lock correction, all four Criterion targets (`graph_bench`, `incidence_bench`,
`pgraph_bench`, `spine_bench`) build, enumerate and pass their smoke runs.

## 3. Dependency duplication

Root `cargo tree -d` contains exactly one `hymeko_core` source and one parser source:

- `hymeko_core` `70b4f364d9559602d2d229cde5bfe0d8b8cc5113`;
- parser `e28a205147c48273980e63aa3338ca286974a6ed`.

Normal third-party version duplication remains (for example `hashbrown`), but no second
core or parser authority exists in the production workspace. Verdict: **CONFIRMED** for
single-source production dependencies. The stale benchmark lock was a reproducibility
defect, not a simultaneous duplicate in one resolved graph.

## 4. Architecture, ownership and production duplication

The ownership map is
[`ownership.mmd`](../docs/plans/2026-09-03-codex-p34-audit/ownership.mmd). Canonical
topology remains in core `Ir` plus `IncidenceIndex`; graph borrows it for direct-Ir and
matroid operations; PGraph lowers schema while retaining canonical IDs.

P3.4 did **not** duplicate rank-2 and Levi union/find logic. Both call the single
`GraphicMatroidWorkspace` in `hymeko_graph/src/matroid/workspace.rs` (parent, rank and
generation `u32` arrays; path halving; union by rank; generation-stamped reset).
Verdict: **CONFIRMED**.

Notable overlaps:

| Mechanism | Classification | Result |
|---|---|---|
| `GraphicMatroidWorkspace` used by rank-2 and Levi | `FALSE-POSITIVE` | one shared production implementation |
| one-shot DSU in `direct_ir/s_connect.rs` | `CONSOLIDATABLE` | different hyperarc-ground-set contract/lifecycle; no defect justified a rewrite |
| generation resets in traversal, informed search, matroid and PGraph AND/OR | `CONSOLIDATABLE` | policy resembles, state and lifecycle differ; no cross-crate abstraction added |
| node, signed-CSR, s-connect and Levi component routines | `FALSE-POSITIVE` | distinct ground sets, outputs and adjacency relations |
| rank-2 Kruskal vs cycle enumeration | `FALSE-POSITIVE` | only one production weighted-forest authority |
| direct-Ir reachability vs CSR/PGraph reachability | mostly `FALSE-POSITIVE` | different direction, representation or output contracts |
| PGraph schema/view reachability overlap | `CONSOLIDATABLE` | migration overlap with parity coverage |
| P3.4 sparse-star builder vs existing benchmark star builder | `CONSOLIDATABLE` | test infrastructure duplication, not production topology |
| P3.4 clique pair list vs existing deduplicated clique CSR | `BENCHMARK-CONTROL` / `CONSOLIDATABLE` | related mechanism, intentionally different materialization |
| Steiner and Mycielski workload builders across graph/PGraph tests | `CONSOLIDATABLE` | same combinatorics, different output types |
| per-test-binary allocation counters and tiny TSV writers | `TRIVIAL` | isolation requires separate global allocators; abstraction value is low |
| SVG output | `FALSE-POSITIVE` | the pre-audit code already centralizes emission in `svg_plot.py` |

No `HARMFUL` duplicated production algorithm was demonstrated. The full list includes
files, functions, approximate line counts and exact contracts in the duplication TSV.
Independent audit/oracle code intentionally repeats mechanisms and is not a candidate
for production reuse.

There is documentation drift outside P3.4's source mechanics: core README still names
the removed `hymeko_hnn` member; graph's manifest comment says graph/PGraph do not depend
on each other despite the current dependency; and older Hotaru ownership text conflicts
with the newer platform strategy that assigns hypergraph algorithms to graph and HNN to
Nagare. This audit maps but does not silently resolve that cross-repository policy.

## 5. Intentional oracle and control duplication

The rank-2 `OracleDsu` and independent Kruskal in `matroid_vv.rs`, the Levi `OracleDsu`
in `matroid_levi_vv.rs`, and the new eager-DSU audit oracle are
`INTENTIONAL-ORACLE`. They reconstruct endpoints and connectivity without calling the
production matroid rank implementation. Replacing them with the production workspace
would introduce a common-mode correctness hole.

The eager DSU and materialized sparse-star/clique structures in
`p34_repr_bench.rs` are `BENCHMARK-CONTROL`. They should remain independent, though their
lifecycle must be made symmetric before performance comparisons. Sparse star and clique
must not be consolidated with each other because one is a bipartite incidence graph and
the other a 2-section.

## 6. Facade and provenance

`MatroidRef` has one field, `&IncidenceIndex`; obtains canonical `Ir` only through
`IncidenceIndex::ir()`; has no arbitrary `new(&ir, &index)` pairing; is `Copy`; owns no
topology; and is not serialized. Its size is one machine word (8 bytes here). A counted
loop of 100,000 constructions allocated 0 objects and 0 bytes. Construction is one
reference copy with no loop. Verdict: **CONFIRMED**.

Only binding is O(1). Index construction, workspace capacity growth, rank/basis scans
and output materialization are separate non-constant operations. The committed
`facade_ns=0.00` value is below timer resolution and is **REFUTED** as timing evidence;
the structural and allocation proof establishes the binding claim.

Production outputs are canonical `HyperArcId` values or exact `(HyperArcId, slot)` Levi
coordinates. Raw declaration IDs are translated through canonical mappings before dense
DSU indexing. The incremental-session provenance hole described in section 17 was the
exception and is now rejected.

## 7. Rank-2 semantics

Verdict: **CONFIRMED-WITH-QUALIFICATION** after the provenance correction.

- exactly two node incidences are required; multiway arcs return `WrongCardinality`;
- self-loops return `SelfLoop`, while a duplicate parallel edge is dependent;
- non-node targets and non-finite weights return typed errors;
- rank equals `N-C`, the output is acyclic, and exhaustive I1/I2/I3 tests pass;
- min/max weighted bases match an independent Kruskal oracle;
- equal-weight order is deterministic through canonical arc-ID tie-breaking;
- incremental insertion equals full rebuild; deletion is neither implemented nor
  claimed.

The original implementation did not bind the incremental session to an `Ir` snapshot
and indexed unchecked arcs. Those cases now return `ForeignSnapshot`, `UnknownArc`, or
`NodeUniverseMismatch`, rather than accepting mixed topology or panicking.

## 8. Levi semantics

Verdict: **CONFIRMED-WITH-QUALIFICATION** after the same snapshot correction.

The implicit undirected graph contains every canonical node and every canonical
hyperarc. DSU coordinates are `node.raw()` and `N + arc.raw()` with checked `u32`
overflow. Isolated nodes and zero-incidence hyperarcs are singleton vertices. A target
that does not map to a canonical node is a typed error. Duplicate incidences remain
distinct canonical `(arc, slot)` ground elements: the first may join components and the
parallel duplicate is dependent.

An independent eager DSU recomputed `rank = N + A - C`:

| Workload | Independent components | Rank |
|---|---:|---:|
| STS(31) | 1 | 185 |
| STS(127) | 1 | 2,793 |
| STS(511) | 1 | 43,945 |
| STS(1023) | 1 | 175,273 |
| two disjoint STS(7) | 2 | 26 |
| random rank-3 (1M incidences) | 1,323 | 532,010 |
| multiway transport | 205 | 195,076 |

Every production result was asserted equal. The STS(1023) value follows independently
from `N=1023`, `A=174251`, `C=1`. The original large STS check mostly reused the formula
and shared machinery; the audit supplies the independent large-workload oracle.

The Levi basis is only a connectivity certificate. No production consumer uses it as a
replacement for shortest/weighted paths, signs, T/S propagation, B-family semantics or
message passing.

## 9. Claim-by-claim performance verdicts

All independent timing rows use two warmups, nine measured samples, median, Q1, Q3 and
worst; fine comparisons alternate A/B order and include A/A controls. Important claims:

| Claim | Rerun | Verdict |
|---|---|---|
| facade is one-word, zero-allocation O(1) binding | 8 B; 0 allocations/bytes for 100k | **CONFIRMED** |
| `facade_ns=0.00` proves it | below timer resolution | **REFUTED** |
| path forest about 8.8 ns/arc | A/A 9.42 and 9.25 ns/arc | **CONFIRMED-WITH-QUALIFICATION** |
| grid forest about 26 ns/arc | A/A 28.11 and 30.76 ns/arc; 9.4% null drift | **CONFIRMED-WITH-QUALIFICATION** |
| weighted basis costs 3–7x | path 2.02x; grid 0.97x | **REFUTED** |
| STS1023 Levi about 16.9 ns/incidence | 18.60; A/A ratio 0.997 | **CONFIRMED-WITH-QUALIFICATION** |
| random1e6 Levi about 25.6 ns/incidence | 29.19; A/A ratio 0.964 | **CONFIRMED-WITH-QUALIFICATION** |
| STS1023 24-thread speedup about 16.1x | 12.91x | **REFUTED** |
| map512 24-thread speedup about 13.6x | 11.21x | **REFUTED** |
| random1e6 24-thread speedup about 9.9x | 9.30x | **CONFIRMED-WITH-QUALIFICATION** |
| result is single-query acceleration | 64 serial searches distributed by `par_iter` | **REFUTED** |
| operation-local CPU utilization | child-lifetime CPU / one-operation median wall | **NOT-COMPARABLE** |
| peak RSS was captured | all 51 committed values are 0.0 | **REFUTED** |
| index is about 12 B/incidence | STS 12.01; random 12.80 incl. row/capacity | **CONFIRMED-WITH-QUALIFICATION** |
| rank-2 workspace is 12 B/node | three `u32` arrays | **CONFIRMED** |
| Levi workspace is 12 B/(N+A) | exact in both large cases | **CONFIRMED** |
| `b_per_levi_ground` divides by N+A | code divides by incidence count | **REFUTED** |
| six-figure allocations are matroid cost | 79.5–83.4% of bytes are IR construction | **REFUTED** |
| Levi break-even Q≥2 / Q≥1 | lifecycle and question differ | **NOT-COMPARABLE** |
| sparse star/clique are structurally distinct | bipartite CSR vs pair list | **CONFIRMED-WITH-QUALIFICATION** |
| sparse star loses hyperedge identity | it retains the `N+arc` vertex | **REFUTED** |
| label strings/bytes guard representation swaps | no structural kind field | **REFUTED** |
| chain cardinalities 1500/1500/197/10 | exact independent reproduction | **CONFIRMED** |
| T/S warm time is a median/distribution | committed code takes min of 7 | **REFUTED** |
| fundamental operation inventory is covered | most operations absent | **REFUTED** |
| benchmark workspace smoke was green | failed at eight imports | **REFUTED** |
| rank-2 and Levi share a DSU | same workspace calls in both | **CONFIRMED** |
| P3.4 is complete | multiple missing/invalid evidence classes | **REFUTED** |

## 10. CPU scaling classification

`p34_cell.rs` constructs the same graph and exactly 64 start nodes at every thread count.
`par_batch_reachable_counts` performs `starts.par_iter().map_init(...)`, with each item
calling the ordinary serial `reachable_nodes`. Checksums are identical for every thread
count: STS1023 65,472; random1e6 12,715,392; map512 16,751,104.

Classification: fixed-work **independent-query batch parallelism**. It is valid batch
strong-scaling evidence for those 64 queries, but not evidence that one graph query was
parallelized. The local pool used at most the host's 24 logical processors. Scaling
flattens between 16 and 24 threads; memory-bandwidth saturation is plausible but remains
**UNMEASURED** because no hardware-counter profile was taken.

The PowerShell process CPU counter covers child setup, workload/IR/index construction,
warmups and all samples, while `op_wall_s` represents one median operation. Dividing one
by the other is **NOT-COMPARABLE**. The harness also suppressed child stderr, did not
check exit status and could omit missing rows. It cannot support operation-local CPU
utilization.

## 11. Allocation and retained-memory decomposition

Operation-local thread allocation counts are valid for the serial measured cells. They
must not be generalized to total Rayon allocation.

| Workload/stage | Allocations | Allocated bytes | Retained bytes |
|---|---:|---:|---:|
| STS1023 IR construction | 174,295 | 88,222,968 | — |
| STS1023 index | 4 | 6,281,228 | 6,277,132 |
| STS1023 facade ×100k | 0 | 0 | 8 structural |
| STS1023 Levi workspace reserve | 3 | 2,103,288 | 2,103,288 |
| STS1023 fresh output | 17 | 8,388,544 | 4,194,304 |
| STS1023 warm workspace + output | 0 | 0 | 6,297,592 |
| random1e6 IR construction | 666,735 | 251,188,288 | — |
| random1e6 index | 4 | 13,599,996 | 12,799,992 |
| random1e6 facade ×100k | 0 | 0 | 8 structural |
| random1e6 Levi workspace reserve | 3 | 6,399,996 | 6,399,996 |
| random1e6 fresh output | 19 | 33,554,368 | 16,777,216 |
| random1e6 warm workspace + output | 0 | 0 | 23,177,212 |

The original whole-pipeline totals attribute 83.4% of STS1023, 79.8% of random1e6 and
79.5% of map512 allocated bytes to canonical IR construction alone. Its `search_ws_b`
also includes output allocation. The corrected conclusion is that warm Levi workspace
reuse is allocation-free; fresh output materialization is material; and the headline
allocation counts are primarily construction costs.

The T/S chain decomposition is: source string 3,008 allocations / 307,129 B;
parse+resolve+IR 76,880 / 10,778,153 B; PGraph lowering 23,691 / 2,203,636 B; index 4 /
72,040 B (54,008 retained). A warm propagation still returns an owned `Availability`:
Boolean/Gödel allocate 250 objects / 65,312 B, Product 34 / 39,968 B and Łukasiewicz 2 /
36,160 B.

## 12. Peak RSS

Verdict: **UNMEASURED** for actual peak values and **REFUTED** for the claim that the
committed harness captured them. Every committed `peak_rss_mb` value is `0.0`. Therefore
baseline process RSS, IR, index, workspace, basis output, control and total process peak
cannot be decomposed from this dataset. Fresh child processes prevent allocator state
from carrying between cells, but a zero peak and unvalidated exit are not evidence.

## 13. Break-even recalculation

The committed calculation does not implement lifecycle-equivalent
`total(Q)=preparation+Q*warm_query`:

- direct recomputes components per query;
- star and clique reconstruct a DSU per query;
- Levi preparation computes a basis, then a separate `levi_components` call performs
  another full Θ(incidences) build;
- Levi's measured query only `black_box`s the resulting cached `usize`.

There is no arbitrary-pair certificate-query API. A cached global component count is not
a source-to-target reachability query and is not a DSU `find`. The Q≥2/Q≥1 Levi and
sparse-star/clique crossings are therefore **NOT-COMPARABLE**.

The like-for-like replacement has every representation prepare and cache the same
global component count, then perform the same scalar read. All 100,000-read medians are
0.193–0.208 ns, so no query-dependent crossing exists; only preparation differs:

| Workload | direct ms | Levi ms | sparse-star ms | clique ms |
|---|---:|---:|---:|---:|
| STS1023 | 5.460 | 9.314 | 13.368 | 16.841 |
| random1e6 | 59.185 | 27.220 | 44.916 | 46.637 |
| map512 | 38.663 | 21.936 | 37.709 | 47.007 |

For this question, the cheapest preparation remains cheapest at every Q. The original
figure script also maps both an actual baseline crossing and `NEVER` to zero, conflating
two states. The corrected figure plots preparation rather than inventing a crossing.

## 14. Sparse-star and clique controls

Sparse star is genuinely bipartite and O(nnz); its CSR stores both directions, so the
actual adjacency column has two entries per incidence, not the report's “one”. The
`N+arc` vertex preserves hyperarc grouping/identity, while exact slot, sign and weight
are lost. Clique is O(Σk²), emits node pairs and loses the originating multiway hyperarc;
its P3.4 control does not deduplicate pairs across arcs.

Connectivity parity holds in the audited workloads, and retained-byte methods use
vector capacities. The controls are structurally different. Nevertheless, M10's label
swap protection is **REFUTED**: strings and different byte values do not form a
machine-checkable `representation_kind`; labels can be swapped without changing either
structure. Because this is report/control metadata and the break-even experiment is
already invalid, the audit records the defect rather than expanding production API.

## 15. T/S norms

The benchmark calls production `hymeko_pgraph::andor::propagate`; it does not copy the
fixpoint. Independent known answers check T and S for BooleanExact, Gödel, Product and
Łukasiewicz. The production loop initializes raw materials to 1 and others to the
S-identity, applies per-unit reliability, uses strict `next > current + eps`, re-enqueues
consumers and returns `NonConvergent` at the configured cap. Boolean is exact;
independent fuzzy runs use epsilon `1e-9`, the production-derived cap and reliability
0.9.

Chain-1500 producible counts are exactly Boolean 1500, Gödel 1500, Product 197 and
Łukasiewicz 10: **CONFIRMED**. They are PGraph propagation results only, not matroid or
A* evidence. The committed “warm” metric takes the minimum of seven calls and exposes no
IQR: its distribution claim is **REFUTED**. Its warm allocation totals also cover seven
owned result objects, not one allocation-free fixpoint.

## 16. Fundamental operation/query coverage

The P3.4 table measures six families: default/create, preallocation, node insertion,
rank-2 hyperarc insertion, clone and canonical hash. Two rows are `NOT IMPLEMENTED` and
two are boundary-only. It does not separately measure incidence insertion, canonical ID
lookup, declaration→record mapping, hierarchy traversal, arc/node incidence traversal,
exact coordinate resolution, path-index construction/lookup, incidence-index
construction/validation, graph queries, result materialization or supported
serialization/deserialization.

Several numbers are best-of/single aggregate rather than contract-compliant
five-sample median/IQR/worst distributions; allocation-byte fields are also misleading
for operations such as clone. This subsection is **REFUTED** as complete. Canonical
deletion and transactions correctly remain `NOT IMPLEMENTED`; reconstruction was not
relabeled as mutation.

## 17. Corrections and rejected edits

Committed sequence after the plan and independent evidence:

1. `9136353` adds red tests for foreign-snapshot incremental rank-2 and Levi sessions.
2. `257fcca` adds red tests for rank-2 out-of-range and node-universe panic paths.
3. `b8e5afe` binds each stream to a canonical snapshot, checks node count and replaces
   unchecked arc indexing with `UnknownArc`; exact valid outputs remain unchanged.
4. `a471e1b` aligns the separate benchmark lock to core `70b4f364` and parser
   `e28a2051`, matching the root workspace.

The source fix is justified by a reproduced semantic defect, not by microbenchmark
gain. It adds only borrowed snapshot/count state, no topology copy and no allocation.
The benchmark-lock edit is a benchmark-invalidating reproducibility correction.

Rejected production edits:

- no shared abstraction for domain-specific generation resets without two equivalent
  production consumers;
- no merge of oracle DSUs into production;
- no consolidation of sparse-star and clique controls;
- no sorting/capacity optimization for weighted bases because no production consumer or
  stable benefit was demonstrated;
- no single-query parallel search change inferred from batch throughput;
- no broad benchmark API redesign inside the semantic correction.

## 18. Corrective before/after performance

Five process-order-alternating rounds were run; every point is the existing benchmark's
median of 15 incremental rebuild samples.

| Workload | Before `ce84134` median of round medians | After `a471e1b` | Delta | A/A context |
|---|---:|---:|---:|---|
| path65k | 644.5 µs | 661.6 µs | +2.65% | below 3% acceptance boundary |
| grid256 | 3,696.0 µs | 3,994.1 µs | +8.07% | within independently observed 9.43% grid null drift |

The grid point exceeds 3% numerically but does not separate from A/A noise; no measured
regression is attributed. Valid output checksums/ranks are unchanged, and warm
steady-state allocation remains zero.

## 19. Remaining untrusted or unmeasured claims

- actual peak RSS and its stage decomposition: **UNMEASURED**;
- operation-local CPU utilization from the committed child telemetry:
  **NOT-COMPARABLE**;
- single-query parallel acceleration: **REFUTED** by source structure;
- arbitrary-pair connectivity and any certificate-query complexity: **UNMEASURED**
  because no such certificate API exists;
- hardware explanation for path/grid latency and high-thread flattening:
  **UNMEASURED**; cache locality/bandwidth remain hypotheses;
- exact published STS/map scaling ratios on this host: **REFUTED**;
- break-even for a future lifecycle-equivalent pair-connectivity API:
  **UNMEASURED**;
- cross-repository ownership statements: internally inconsistent and require a separate
  documentation decision.

## 20. Files changed by the audit

Production/reproducibility:

- `hymeko_graph/src/matroid/error.rs`
- `hymeko_graph/src/matroid/levi.rs`
- `hymeko_graph/src/matroid/rank2.rs`
- `benchmarks/Cargo.lock`

Tests and measurement harnesses:

- `hymeko_graph/tests/matroid_vv.rs`
- `hymeko_graph/tests/matroid_levi_vv.rs`
- `hymeko_graph/tests/codex_p34_audit.rs`
- `hymeko_pgraph/tests/codex_p34_audit.rs`

Plan, report, data and generated audit assets:

- `docs/plans/2026-09-03-codex-p34-audit/plan.md`
- `docs/plans/2026-09-03-codex-p34-audit/plan.mmd`
- `docs/plans/2026-09-03-codex-p34-audit/ownership.mmd`
- `reports/2026-09-03-codex-p34-independent-audit.md`
- `reports/data/2026-09-03-codex-p34-rerun.tsv`
- `reports/data/2026-09-03-codex-p34-claims.tsv`
- `reports/data/2026-09-03-codex-p34-duplication.tsv`
- `reports/data/2026-09-03-codex-p34-allocations.tsv`
- `reports/data/2026-09-03-codex-p34-artifact-sha256.tsv`
- `reports/figures/codex-p34-batch-scaling.svg`
- `reports/figures/codex-p34-like-for-like-components.svg`
- `scripts/codex_p34_audit_figures.ps1`

No original P3.4 report, data file or figure was overwritten.

## 21. Exact reproduction commands

Run from PowerShell. All task state remains below the allowed D: tree:

```powershell
$env:CARGO_HOME = 'D:\h\codex-graph-p34-audit\tmp\cargo-home'
$env:CARGO_TARGET_DIR = 'D:\h\codex-graph-p34-audit\target'
$env:TEMP = 'D:\h\codex-graph-p34-audit\tmp'
$env:TMP = 'D:\h\codex-graph-p34-audit\tmp'
Set-Location 'D:\h\codex-graph-p34-audit\checkout'

cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo check --workspace --all-features --all-targets
cargo build --workspace --all-features --all-targets
cargo test --workspace --all-features --all-targets
cargo test --doc --workspace --all-features

# Write graph audit raw TSV first, then append PGraph T/S rows.
cargo test -p hymeko_graph --release --test codex_p34_audit `
  independent_graph_measurements -- --ignored --nocapture --test-threads=1
cargo test -p hymeko_pgraph --release --test codex_p34_audit `
  independent_ts_measurements -- --ignored --nocapture --test-threads=1

# Benchmark workspace after the lock correction.
cargo bench --manifest-path benchmarks/Cargo.toml --workspace --no-run
cargo bench --manifest-path benchmarks/Cargo.toml --workspace -- --test

# Regenerate only the independent corrected figures.
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts/codex_p34_audit_figures.ps1
```

The two measurement tests regenerate their graph/PGraph sections. The four
`correction_before_after` summary rows in the checked-in rerun TSV were produced by five
alternating invocations of the following existing benchmark in the retained detached
before/after worktrees; each invocation writes its own worktree-local P3.3 TSV:

```powershell
cargo test --manifest-path 'D:\h\codex-graph-p34-audit\repro-a\graph-before\Cargo.toml' `
  -p hymeko_graph --release --test matroid_bench measure_matroid_latency `
  -- --ignored --nocapture --test-threads=1
cargo test --manifest-path 'D:\h\codex-graph-p34-audit\repro-b\graph-after\Cargo.toml' `
  -p hymeko_graph --release --test matroid_bench measure_matroid_latency `
  -- --ignored --nocapture --test-threads=1
```

Alternate which worktree runs first on successive rounds. Extract the `incr_us` values
for `path-65k` and `grid-256`, then take the median of the five round medians. Do not
mistake the overwrite produced by a raw-data regeneration for the preserved
before/after summary.

## 22. Final state and retained paths

The intended final state is a clean `feat/codex-p34-independent-audit` worktree whose
upstream remote branch resolves to the same final report commit. `origin/main` remains
`b234c487` and the audited feature remains `ce841343`. Final hashes are verified after
this report is committed and pushed, and are included in the terminal handoff.

Retained by explicit policy; nothing was automatically deleted:

- `D:\h\codex-graph-p34-audit\checkout`
- `D:\h\codex-graph-p34-audit\target`
- `D:\h\codex-graph-p34-audit\tmp`
- `D:\h\codex-graph-p34-audit\artifacts`
- `D:\h\codex-graph-p34-audit\repro-a`
- `D:\h\codex-graph-p34-audit\repro-b`


## 23. Canonical Core/Graph integration, 2026-09-06

Updated: 2026-09-06T04:05:17+02:00

The user requested integration of useful old work and explicitly permitted Graph, while restricting further actions to Core and Graph. This phase uses the existing canonical checkout at `D:/hakiko_ai_ws/03_implementation/hymeko_framework/graph`, on `audit/platform-2026-09-05`. No new checkout or branch was created. The paths and alternate-worktree procedures in the historical sections above are provenance, not current setup instructions.

The normal merge combines current `4a59c238af4a4af86e43fa905b96606284b34d10` with the old audit tip `83774b8bdb170ace87506a02c0655397f806a0b8`. The matroid implementation and its independent tests/evidence are integrated into their existing Graph owner. Conflicts preserve the current directed slot handling, entry-dependent policy check and consistent-A* arc-entry-g dominance guard. Goal pruning stops a path at its requested endpoint; cycle identity includes alternating node/arc IDs and keeps parallel hyperarcs distinct. The budget regression matrix now distinguishes the four expansions needed by goal-directed paths from the six needed by the cycle search. Existing focused tests cover these differences.

Core's nine requested branches were integrated and published on `main` at **`92383b38a7c56c197300d9b85e81f637135a8743`**. Every requested incoming tip was checked as an ancestor. Graph's workspace manifest and both Cargo lockfiles now use that exact Core revision. Both lockfile diffs change only the Core source revision; the Parser remains `284c490d68232973a006bcba974279fed6f89ce8` and registry versions stay fixed. The stale Graph registry pointer now names `../hymeko/ecosystem.hymeko`; the contract's old no-Core-dependency sentence is aligned with its already established dependency direction.

### Verification in the canonical checkout

- Before the Core-pin refresh: complete Graph workspace **556 passed / 0 failed / 21 ignored**; focused integration **61 passed / 1 ignored**; all-feature/all-target Clippy and formatting passed.
- Against published Core `92383b38`: native/unit/integration groups **552 passed / 0 failed / 19 ignored**. The initial doctest stage encountered `E0463` during a separate benchmark workspace rebuild in the shared target; the same doctest command then passed **4 / 0 / 2** with no source change. The final serialized complete workspace rerun passed **556 / 0 / 21**, including all doctests (64 result groups; build 42.17 s). All-feature/all-target Clippy then passed with `-D warnings` (12.10 s).
- Separate benchmark workspace: `cargo check --benches --release --locked --offline -j 4 --target-dir <existing graph/target>` **PASS**, 37.57 s. The parent vendor omits rand; running the same manifest outside that parent config consumed the existing Cargo registry cache offline. No vendor/source repository or dependency version was changed. This is compilation evidence, not new benchmark timing evidence.
- Symbols: **7 packages / 482 public symbols**, generated index checked. Formatting and diff whitespace checks pass.
- Duplication measured **512 clone groups / 12124 cloned lines / 149 files**. This Graph checkout has no installed pre-commit hook or prior duplication baseline, so no passing duplication delta gate is claimed. Provider preflight was run explicitly for 149 added files and returned existing-name/path candidates (exit 1); the DISCOVERY disposition retains the imported implementations/tests in their reviewed existing Graph owners. This is not reported as a zero-candidate preflight pass.

Full command logs are retained in the canonical Core's existing `.git/worktree-cleanup-20260906` archive (`graph-final-core-tests*.log`, `graph-final-core-doctests.log`, `graph-final-core-benches.log`, `graph-final-core-clippy.log`). Historical performance claims above retain their original evidence and limitations. No new performance campaign, formal proof, WASM verification or other consumer-repository validation is claimed.

The source merge is committed normally after verification, and published to the existing Graph audit branch. Graph main and other branch refs are not rewritten or deleted. Rollback, if later requested, is a normal first-parent revert of this merge after checking user work; both original parent refs remain available. Core integration details and the nine-branch disposition are in `../core/reports/2026-09-06-worktree-consolidation.md`.

DISCOVERY: existing direct_ir remains the graph navigation owner; matroid is imported from its audited Graph branch rather than recreated; Core owns canonical incidence, and the provider ecosystem map supersedes the retired Core workspace registry. Extend this existing independent audit report to record the integration and verification.


## 24. Graph single-branch consolidation, 2026-09-06

The user subsequently requested consolidating Graph into one branch. This supersedes section 23's audit-only publication boundary. The sole active target is `main` in the existing canonical Graph checkout, published through an ordinary fast-forward update. All current local and remote non-main Graph branch tips are checked as ancestors before their names are retired. Existing history and every commit remain reachable from main; no new worktree, branch, clone, tag or source backup is created.

Seven of the nine listed branch tips were already included by `978c39ea`. The remaining `31930875` contributes only a deferred PGraph topology-ownership design, and `e710e84b` contributes the existing P1.4 axiom/AND-OR timing harness and report. Both are integrated by normal Git merge. The design remains unimplemented; Core's newer PreparedIr changes its historical lifetime assumptions, as the imported report now notes. Historical timings are not relabeled as current measurements. The harness's misleading claim of an asserted cross-revision A/B comparison is corrected: it prints observations, and does not make that assertion.

Production source, manifests, Core/Parser pins, both lockfiles and the separate benchmark workspace are byte-identical in Git to the previously verified `978c39ea` integration. Core stays at `92383b38`, Parser at `284c490d`. Changes in this step are the imported test/design/evidence plus consolidation notes. The earlier separate benchmark compilation therefore remains applicable. Full workspace verification includes the new timing harness; its successful execution is not an independent performance proof.

### Branch names and exact pre-retirement tips

The nine requested branches and older fully integrated Graph branches are consolidated into main. `-` means that the name existed only on the other side. These exact refs permit reconstruction with `git branch <name> <tip>` and a normal push if later requested; no extra archival branch is needed.

<!-- graph-branch-tips:start -->
| Branch | Local tip | Remote tip |
|---|---|---|
| `audit/platform-2026-09-05` | `978c39eae94132cf02d1eae1a4040d608ef12d73` | `978c39eae94132cf02d1eae1a4040d608ef12d73` |
| `chore/canonical-graph-governance` | `-` | `610d1eb1d30c9b22d4daa2758e54473cf64e191a` |
| `chore/graph-clippy` | `-` | `61185f0b5d57e27cea155045ab65b31f2c143777` |
| `chore/graph-fmt` | `-` | `a2a13cd28461526830904c1bf3ce242d8e5766d9` |
| `feat/canonical-owner-hyper-operator` | `-` | `250115cf1115f3bd0b65e65d79b08b23802dfcc6` |
| `feat/canonical-owner-signed-tuples` | `-` | `ec58a3d998999f8f86f0162bbc00ad6ca5129c76` |
| `feat/canonical-owner-spine-extension` | `-` | `5e9bd957e6fa14e335a6a72c2fd7ca42a25ef5d4` |
| `feat/codex-p34-independent-audit` | `-` | `83774b8bdb170ace87506a02c0655397f806a0b8` |
| `feat/direct-ir-informed-berge-search` | `d4086a04aeb9f31db42147642324e78a2b4d1499` | `d4086a04aeb9f31db42147642324e78a2b4d1499` |
| `feat/graph-direct-ir-algorithms` | `98c6a93212613dd4053a3b3a04c36eee8b5d48ad` | `98c6a93212613dd4053a3b3a04c36eee8b5d48ad` |
| `feat/graph-matroid-navigation-p33` | `cead2c8e50b6fbc5b006d9bb5f25dff267515923` | `cead2c8e50b6fbc5b006d9bb5f25dff267515923` |
| `feat/graph-matroid-navigation-p34` | `ce841343b428ba1cfa274e225ae5646606eb8a50` | `ce841343b428ba1cfa274e225ae5646606eb8a50` |
| `feat/graph-navigation-performance-p31` | `3bcab17a5171a9efcb5b18fc6a6017c4b90be088` | `3bcab17a5171a9efcb5b18fc6a6017c4b90be088` |
| `feat/graph-navigation-performance-p32` | `1c6d568dfa3d16ba3eb245440a0d8b8efe008cd2` | `1c6d568dfa3d16ba3eb245440a0d8b8efe008cd2` |
| `feat/graph-performance-survey` | `cbaadd438b313b9ec2abfaec05d590082a301c06` | `cbaadd438b313b9ec2abfaec05d590082a301c06` |
| `feat/graph-query-performance-matrix` | `e710e84b13911c46e4b9d792f54d9b52321fc2ab` | `e710e84b13911c46e4b9d792f54d9b52321fc2ab` |
| `feat/graph-steiner-mycielski-navigation-bench` | `cc5b895c83bed1e4cc3cf8b4941cdd16b6f8e9d2` | `cc5b895c83bed1e4cc3cf8b4941cdd16b6f8e9d2` |
| `feat/p1-lint-cleanup` | `c35b9f29e81f4dd63c9f56835ba951fd7648c465` | `c35b9f29e81f4dd63c9f56835ba951fd7648c465` |
| `feat/pgraph-adjacency-delegation` | `b234c4871807dc255854cc4b50d1458547dca7d3` | `b234c4871807dc255854cc4b50d1458547dca7d3` |
| `feat/pgraph-direct-ir-and-or-algebra` | `9655f07755e271c0f2ee4361a526b14e0205af8c` | `9655f07755e271c0f2ee4361a526b14e0205af8c` |
| `feat/pgraph-lowered-borrows-canonical-ir` | `31930875bc2cfdd75f743b87e7d963a87c764af6` | `31930875bc2cfdd75f743b87e7d963a87c764af6` |
<!-- graph-branch-tips:end -->

Deletion is conditional on all tips being ancestors of the committed and remotely verified main. Remote deletions are atomic and guarded by each exact expected tip, so a concurrent branch advance stops the operation. Local deletion uses Git's merged-branch check. The original remote main base is `b234c487`; existing local main has been safely advanced to the tested integration before the two remaining branches are merged.

### Discovery

```text
DISCOVERY LEDGER ? Graph ?gak egyes?t?se
  find:    rg --files -> megl?v? auditterv ?s jelent?s
  grep:    median_us / chain / fanout / fanin -> megl?v? AND/OR m?r?tesztek
  symbols: SYMBOLS.md -> AxiomBundle, AndOrWorkspace, PGraphView
  read:    a k?t hi?nyz? ?g teljes diffje ?s a jelenlegi auditterv
  verdict: EXTEND az audittervet ?s jelent?st; a r?gi m?r?tesztet beemelem,
           mert az axi?maellen?rz?s ?s az AND/OR-z?r?s k?l?n m?r?s?t tartalmazza.
```

The older timing harness has some fixture/timing duplication already acknowledged by its original report. It is retained as historical experiment evidence with its production calls intact, without starting an unrelated test-framework refactor during branch consolidation. This phase adds no production duplicate.


### Final main verification

- Complete Graph workspace: **557 passed / 0 failed / 21 ignored**, including doctests and the imported `perf_matrix` harness; `cargo test --workspace --release --all-features --locked --offline --no-fail-fast -j 4 -- --test-threads=4`. This adds one historical measurement harness execution to the previous 556-test inventory.
- `cargo clippy --workspace --all-features --all-targets --release --locked --offline -j 4 -- -D warnings`: **PASS**, 3.86 s.
- `cargo fmt --all --check`, symbol index check, diff whitespace checks: **PASS**. Production source/manifests/locks have no diff from the tested 978c39ea.
- Duplication measurement after the imported harness: **512 clone groups / 12201 cloned lines / 150 files**, versus **512 / 12124 / 149** before it. The 77-line growth is recorded; no Graph duplication baseline is invented or changed.
- Provider preflight checks the six imported files; overlap candidates are resolved by the DISCOVERY disposition rather than claimed absent. The normal Git merge commit records that disposition. Graph still has no installed hook, so explicit checks are the evidence.

Full local command logs are in this checkout's `.git/graph-main-consolidation-tests.log`, `.git/graph-main-consolidation-clippy.log`, and `.git/graph-main-consolidation-preflight.log`. The authoritative source is the normal merge commit, published on main. Publication and exact-tip branch retirement are verified against the server after the commit.
