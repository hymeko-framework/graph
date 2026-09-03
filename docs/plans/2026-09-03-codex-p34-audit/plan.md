# Independent Codex audit plan — P3.4

**Created-at:** 2026-09-03 18:03 CEST  
**ETA:** multi-hour forensic audit; source/claim reconstruction first, quiet-host
measurement cells only after semantic comparability and harness boundaries are
established.  
**Audited commit:** `ce841343b428ba1cfa274e225ae5646606eb8a50`  
**Expected main:** `b234c4871807dc255854cc4b50d1458547dca7d3`  
**Audit branch:** `feat/codex-p34-independent-audit`

## Scope and invariants

Audit the P3.4 architecture, semantic duplication, provenance, timing boundaries,
allocation/RSS attribution, scaling classification, representation comparisons and
break-even claims. Treat committed reports, TSVs and figures as claims, not evidence.
Preserve independent oracles and benchmark controls. Do not edit production code unless
a reproduced semantic defect, benchmark-invalidating defect, harmful production
duplication or measured regression first supplies a failing discriminating test.

Task state is confined to:

- `D:\h\codex-graph-p34-audit\checkout`
- `D:\h\codex-graph-p34-audit\target`
- `D:\h\codex-graph-p34-audit\tmp`
- `D:\h\codex-graph-p34-audit\artifacts`
- `D:\h\codex-graph-p34-audit\repro-a`
- `D:\h\codex-graph-p34-audit\repro-b`

The user's existing `core` and `graph` checkouts remain untouched. No directory is
automatically deleted.

## Entry evidence captured before this plan

- Remote graph refs match exactly: feature `ce841343`, main `b234c487`; main is an
  ancestor of the feature.
- Root workspace resolves exactly one `hymeko_core` (`70b4f364`) and one `parser`
  (`e28a2051`).
- Host: Windows 11 build 26200, Intel Core Ultra 9 275HX, 24 logical processors,
  63.42 GiB RAM; rustc 1.97.1, cargo 1.97.1.
- Baseline gates: fmt, clippy (`--workspace --all-features --all-targets -D warnings`),
  check and build are green.
- Test inventory: 546 named tests = 190 unit + 351 integration-target + 5 doctests.
  533 ran green; 13 are explicit ignored (11 integration-target, 2 doctest).
- Benchmark smoke is **failed**: the independent `benchmarks/Cargo.lock` pins
  `hymeko_core` at `75c6c7b2`, before `ir::incidence` existed. Building the benchmark
  workspace fails at eight `hymeko::ir::incidence` imports. The root lock instead pins
  compatible core `70b4f364`. This is a baseline finding, not repaired in the plan
  commit.
- Ambient CPU load was 39%; no performance result from that window is accepted.

## Work packages

1. Build a repository-wide implementation and consumer map for DSU, traversal,
   connectivity, construction controls, workloads, telemetry, break-even, T/S norms,
   TSV and SVG machinery. Classify each overlap with the mandated verdict taxonomy.
2. Reconstruct `MatroidRef`, rank-2 and Levi contracts directly from production source;
   inspect exact ID translation, allocation sites, workspace lifecycle and output
   materialization.
3. Reconstruct every reported metric's start/end boundary and semantic question.
   Reject unlike-for-like comparisons before rerunning them.
4. Create independent reproductions under `repro-a` and `repro-b`, using new raw data,
   alternating order and A/A controls where the comparison is fine-grained.
5. Decompose retained bytes, operation-local allocations and fresh-process peak RSS.
   Classify the 24-thread result as batch parallelism or single-query acceleration from
   code and fixed-work checks.
6. Add a failing test or discriminating benchmark before any corrective implementation.
   Keep the smallest justified correction only if exact output and memory/performance
   gates pass.
7. Produce the requested report and TSVs without overwriting the audited claim set,
   verify branch/worktree/remote state, push only the audit branch, then hard stop.

## Planned deliverables

- `reports/2026-09-03-codex-p34-independent-audit.md`
- `reports/data/2026-09-03-codex-p34-rerun.tsv`
- `reports/data/2026-09-03-codex-p34-claims.tsv`
- `reports/data/2026-09-03-codex-p34-duplication.tsv`
- `reports/data/2026-09-03-codex-p34-allocations.tsv`
- architecture/ownership map in the audit report and Mermaid plan
- corrected figures only when newly generated raw data changes a plotted result

## Change boundary and rollback

`CORE.YAML` items touched by this plan commit: none. The audit branch is disposable;
each later evidence, regression-test and optional correction phase is a separate commit.
Rollback is commit-level revert. No dependency change is planned. Production source
remains unchanged unless the evidence gate above is satisfied.

