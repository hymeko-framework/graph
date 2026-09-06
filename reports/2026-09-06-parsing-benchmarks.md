# Parsing and graph ingestion benchmarks

Created-at: 2026-09-06 17:43 +02:00. Updated 18:43 +02:00. Status: phase breakdown verified; supplementary measurements and publication pending.

## Explicit phase breakdown

The user's 18:36 request authorizes measurement and website publication. Five additional phases extend the existing client: `generate_ir` (existing deterministic generator including direct IR construction), `emit_source` (IR to HyMeKo text), `read_source` (local UTF-8 file, warm OS cache), `dijkstra_prepared` (compiled IR and index ready; fresh query workspace), and directly measured `file_to_dijkstra` (read, fresh compilation, index and query). Original five-stage evidence remains immutable under its original revision. New stages use a separate `stages/` campaign with its own measured revision; results are never pooled across source revisions. File creation, sync, validation and final object destruction are outside the timed operation. Intermediate destruction remains inside. External-map import is not implemented or measured in this benchmark-only extension.

The existing generators directly construct IR, so topology generation and direct IR construction are explicitly a combined measurement. AST-to-IR remains a separate source-ingestion path. These rows are not additive partitions: complete pipeline timings are measured directly. Process RSS includes setup, validation and retained sampler objects, not a per-stage memory delta.

```text
DISCOVERY LEDGER — benchmark phase breakdown
  find:    rg --files ../graph/scripts ../graph/hymeko_graph/tests -> existing campaign.py, parsing.rs, statistics.rs
  grep:    rg -n 'measure_factory|mod statistics' ../graph/hymeko_graph/tests -> existing factory sampler and parsing client
  symbols: rg -n 'Campaign|Evidence|parsing|map_grid' ../graph/SYMBOLS.md -> existing Campaign, CampaignSummary and Evidence owners
  read:    parsing.rs, statistics.rs, campaign.py, test_campaign.py, existing plan and report; website model.ts, catalog.ts, explorer.tsx
  verdict: EXTEND existing Graph parsing client, campaign, plan/report and /srv/apps/benchmark
```

Fresh fetch: Graph f6bd760 equals task upstream, contains current main eae7a57. Parser 284c490d equals master. Core checkout 2d43e82 diverges from current main f305ade and remains read-only; the measured Graph lock remains fc3eda9b/284c490d. Remote Graph checkout is the existing detached f6bd760 at `/home/kyberszittya/hymeko-distributed-admission-fix-20260905/dependency-refresh/consumers/graph`, refreshed from its host's read-only Gitea source. Website `/srv/apps/benchmark` is local-only Git, HEAD 84196e5, clean before this extension, with no remote. No new checkout/worktree; no production or dependency edits.

Verification at 18:43: 9 focused Rust tests passed, 5 benchmark entrypoints ignored in ordinary testing. Full release workspace tests including doctests passed. Focused clippy passed. Eight Python campaign tests passed. Symbols check passed. Duplication 517 clones / 12272 cloned lines before and after: delta 0 / 0. Temporary-file tests verify exact bytes, invalid UTF-8 rejection and cleanup; extended phase tests exercise all ten phases and nine samples. Test-only expects remain outside production.

The original missing remote matrix is running as `hymeko-parsing-original-20260906.service` (18:39 launch): native followed serially by ARM64 parser-only. Logs: `reports/parsing-benchmarks/2026-09-06/native-driver.log` and `arm64-driver.log` in the remote checkout. Windows original full matrix finished 18:15, 240/240 processes. The earlier ARM64 full-compile map512 smoke exceeded 180 seconds; it has no accepted latency row. Original accepted target is 240 Windows + 240 native + 48 ARM64 processes. Supplementary target is 240 Windows + 240 native processes; ARM64 supplementary stages are not measured.

This extends the existing Graph evidence client and driver with actual HyMeKo source ingestion for all 16 distinct source topologies in the 59-cell Graph campaign. The five stages are parser AST construction; AST interning/resolution/lowering; fresh ModuleStore compilation; incidence index; and directly measured source-to-Dijkstra. The production parser and Core compile pipeline are called through public APIs. No production implementation or dependency is added.

## Discovery and ownership

- find: existing parser/benches/parser_bench.rs, Graph informed_common, informed_berge_evidence.rs, statistics.rs, campaign.py and Core writer/ModuleStore.
- grep: parse, source, writer, serialization, workload and measurement symbols.
- symbols: Graph SYMBOLS.md and canonical ecoread --repos / --edges uses, using the existing parser-backed executable.
- read: parser grammar, RealParser, ModuleStore, intern_ast, build_index_sym, lower_to_ir, IrWriter, informed fixtures, factory sampler and campaign validator.
- verdict: EXTEND existing Graph evidence. The existing IrWriter emits an arc placeholder, so it cannot reproduce these workloads. Add a private fixture-only textual projection with explicit rejection of unsupported annotations, directions, bases, nonnumeric values and non-node targets. Preserve nodes, signs, weighted incidence order and node positions; add one named edge declaration per original synthetic arc. This is not a general-purpose serializer.

Fresh fetch succeeded in Core, Graph and parser through the existing Amaterasu host IP (hostname DNS failed initially; no persistent SSH configuration change). Fresh Graph main eae7a57 was merged into the existing feat/graph-cross-platform-benchmarks checkout; preceding evidence was preserved. Graph now uses the upstream branch=main declaration, locked Core fc3eda9b73e4f171aada5c79751af481759c068e and parser 284c490d68232973a006bcba974279fed6f89ce8. New worktrees/clones: zero. Core and Simulator files: untouched by this extension.

## Measurement contract

Resident UTF-8 source is generated outside timing. Source FNV-1a-64, bytes, graph dimensions, revisions, executable SHA256, complete raw samples and process peak RSS accompany results. FNV identifies deterministic fixture bytes; it is not a cryptographic artifact integrity claim. The final artifact manifest uses SHA256.

The original synthetic graphs omit edge declarations; valid source necessarily creates them. These frontend costs are measured honestly and are not attributed to query speed. AST-to-IR calls the public single-module interning/index/lowering APIs, excluding ModuleStore validation and hashing. Full compile includes those costs and starts from a fresh store every time. Source-to-Dijkstra also uses a fresh index and search workspace. All source topology/sign/weight/position checks and original-vs-parsed query comparisons precede timings. Final returned-object destruction is excluded by the existing factory sampler; intermediate destruction remains included. File I/O is excluded. No cache-hit compile is substituted. No sum of medians is called end-to-end time.

Each cell: 2 warmups, 9 factory samples; accepted campaign: 3 fresh processes per cell per platform. Planned matrix: 16 workloads x 5 phases x 3 process runs x 3 platforms = 720 processes / 240 pooled rows / 6480 samples. Windows, Amaterasu native, ARM64 Docker/QEMU on Amaterasu are separate platforms. No native ARM hardware speed claim and no SOTA ranking follow from these measurements.

## Verification and initial resource probe

Focused Rust integration client: 8 passed, 5 benchmark entries explicitly ignored by the ordinary test runner and invoked separately. Python campaign validator: 6 passed, including missing/incorrect source identity and exact coverage of existing graph source topologies. Focused clippy and ruff passed. Duplication: 517 clones / 12272 cloned lines before and after; delta 0 / 0 (one private test module added). Full workspace validation and remote platform runs are in progress, not yet accepted.

Windows largest-input smoke completed: map512 full compilation median 4.1610282 s, maximum whole-process peak 3021.40234375 MiB; random1e6 compilation 2.6433603 s, 1717.92578125 MiB. These are warmup-plus-nine-sample smoke results, not the final three-process summary. Logs and exact dirty-source provenance are under parsing-benchmarks/2026-09-06/windows-smoke. The 16 GiB cap was respected.

## Files and rollback

Changes: existing four-format cross-platform plan, informed_berge_evidence.rs module wiring, private informed_common/parsing.rs, existing campaign.py and test_campaign.py, evidence .gitattributes entry, this report and dated evidence. Generated symbol index is checked; no public API is added. Tests use explicit expect/unwrap for fixture failures; these do not enter production. Rollback is the scoped benchmark commit and its evidence. Existing reusable targets are retained; transient remote compiler files and extracted executables are removed after verified delivery.

The independent website at /srv/apps/benchmark already reads Graph platform-summary.csv plus manifests. Its parsing table is pending campaign completion. No website code is added to Core.


2026-09-06 17:53: The first Windows full campaign was interrupted after 17 complete cells, before its process provenance was durably saved. These logs remain diagnostic only. The driver now saves and atomically replaces a checkpoint after each cell, verifies source/binary/platform/runtime and raw records on resume, and skips completed cells. Seven Python tests pass, including mismatched-checkpoint rejection. The previous fragment is excluded from the accepted matrix. A remote build initially failed because Git's file transport did not retain the safe-directory setting; using an explicitly scoped upload-pack for the read-only local Gitea source resolves that transport boundary without global Git settings.
