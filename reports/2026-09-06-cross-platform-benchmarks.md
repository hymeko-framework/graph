# Graph cross-platform benchmark campaign

Created-at: 2026-09-06 13:12 +02:00. Completed measurements: 2026-09-06 13:19 +02:00.

## Scope and ownership

The user's earlier Graph/search benchmark instruction is executed here. The canonical Graph checkout is `D:/hakiko_ai_ws/03_implementation/hymeko_framework/graph`, branch `feat/graph-cross-platform-benchmarks`, based on freshly fetched `main` `2b619a39c9982f47db7cbe49bba6dd8c7f9b4579`. No worktree or clone was created. Core production, Graph production, manifests, locks, Simulator and other consumers remain unchanged. CORE.YAML items touched: empty.

The independent website belongs in the existing `hymeko/experiment_viewer` repository at `/srv/apps/experiment-viewer` on Amaterasu. No website files or dependencies belong to Core. Its four-format plan compiled before implementation. Creating the actual app files was rejected by automatic approval review twice, including the narrower configuration-only command without deletion. The only reason supplied was `blocked by policy`; no OS permission failure occurred. The SSH user can write the directory. Website implementation/deployment is **blocked, not complete**. No new application or dependencies were installed.

## Discovery

```text
DISCOVERY LEDGER ? Graph cross-platform benchmark campaign
  find:    rg --files graph/scripts graph/docs/plans -> existing performance_evidence and informed_berge_evidence runners/plans
  grep:    rg EVIDENCE_ graph/hymeko_graph/tests -> existing machine-readable benchmark clients
  symbols: graph/SYMBOLS.md -> existing traversal, search and matroid algorithm owners
  read:    both run.ps1 files; performance_evidence.rs; informed_berge_evidence.rs; statistics.rs; telemetry.rs
  verdict: EXTEND graph/scripts/performance_evidence with a portable campaign driver; reuse existing Rust clients
DISCOVERY LEDGER ? Independent benchmark website
  find:    find /srv/apps/experiment-viewer -> existing frontend, sources.py, tests and deployment
  grep:    sources.py discovery implementations -> report/plan indexing, no typed benchmark reader
  symbols: ecosystem.hymeko parser-backed --repos/--edges uses; experiment_viewer has no SYMBOLS.md -> Core and Graph owners confirmed
  read:    experiment_viewer README.md, sources.py, Dockerfile, docker-compose.yml, deploy/README.md
  verdict: EXTEND /srv/apps/experiment-viewer with benchmark-next; existing document SPA does not provide the requested Next.js metric view
```

The map reader returned 14 repository identities and 14 `uses` relations. Graph owns the production graph algorithms; this campaign calls its existing test clients. The existing document viewer is a Python SPA despite its `experiments-next` container name. No existing Next.js source was found there.

## Measurement design

Exactly 59 cells, three independent fresh-process repetitions per platform, two untimed warmups and nine retained samples per successful cell. Run 2 reverses cell order. The existing client calibrates each batch against a 250ms probe target; the recorded per-operation values are **iteration batch means**, not request-tail latency. Pooling the three runs yields 27 samples per successful platform/cell. The ordered-sample quartiles use indexes `floor(n/4)`, `floor(n/2)`, `floor(3*n/4)`; worst is the largest batch mean.

Coverage:

- Full existing 48-cell informed Berge matrix: grid distances 20/40/80 with unit Dijkstra and L1/L2 A*; weighted maps 32/128/512 and corridor128 with metric/discrete/combined A*; transport128; discrete-hop preparation.
- BFS/unit-Dijkstra pairs on path65k, grid256, STS1023, random1e6 and map512.
- Existing bounded path/cycle enumeration on layered10, grid6/grid5, STS31 and parallel16. Existing depth/result/expansion limits remain authoritative.
- Reachability, components, rank-2 spanning forest and Levi basis on path65k and STS511.
- Exactly 64 independent reachability queries on STS511 at 1/2/4 threads. This is independent query throughput, not parallel acceleration of a single search.

Rank-2 spanning forest on STS511 is explicitly not applicable; it is never represented as a zero-duration result. Thus each platform has 58 timed cells plus one not-applicable cell, each invoked three times. PGraph ABB, all possible graph APIs and the separate representation-expansion matrix are outside this campaign. The initial plan's 61-cell arithmetic was corrected to 59 before finalization; workload scope did not change.

## Provenance and execution

All Rust measurement sources and resolved dependencies are identical across platforms: Graph `2b619a39`, Core `92383b38`, Parser `284c490d`. `source-provenance.json` records Git-blob SHA256 values, lock and compiler. Both native and cross builds use Rust 1.97.1, four compile jobs and the existing Cargo/target caches. Build source changes were only uncommitted plans and the external Python measurement driver, listed in each campaign JSON; no measured Rust source changed.

Windows: Intel Core Ultra 9 275HX, 24 logical processors, native MSVC release executable. Quiet-host CPU samples were 6,12,14,8,15 percent (mean 11). No local Docker/QEMU was used. Existing `graph/target` was reused.

Amaterasu: AMD Ryzen 7 3700X, 16 logical processors. Existing checkout `/home/kyberszittya/hymeko-distributed-admission-fix-20260905/dependency-refresh/consumers/graph` was clean and refreshed directly from the same host's Gitea bare repository to `2b619a39`. The host SSH deployment key is limited to experiment_viewer; the read-only Gitea file source avoids copying credentials or relaxing SSH verification. Native execution uses a user systemd scope with MemoryMax=16GiB, no swap, CPUQuota=400%. A native pre-run ambient CPU series was not captured; background workloads were present. These measurements therefore do not support controlled regression or hardware-normalized speedup claims.

ARM64: compiled natively on Amaterasu and executed there in actual ARM64 Debian Docker containers via explicitly mounted native `qemu-aarch64-static`. Image, executable architecture and QEMU hash/version are recorded. Each ephemeral cell container has network disabled, a read-only filesystem/binary mounts, four CPU quota and 16GiB memory/swap cap. No host-global binfmt registration was made. Native and emulated campaigns are serialized; no website build ran beside either campaign.

The runtime driver records each exact command, per-process wall time and observed peak working set/VmHWM. Linux/QEMU observations are polled every 50ms; they are sampled peaks, not a claim of perfect instantaneous capture. The existing Linux Rust telemetry intentionally returns unavailable operation CPU and phase RSS; these remain `NA`, rather than invented zeros. Whole-process RSS includes fixture/preparation/runtime memory, not memory per graph operation.

Raw files and aggregates: `reports/cross-platform-benchmarks/2026-09-06/`. Each platform has `campaign.json`, two correctness logs and 177 process logs. `progress.json` is only a completion counter, not evidence of success; `campaign.json` appears only when the full run succeeds. `platform-summary.csv`, `process-resources.csv` and `verification.json` are generated by the same campaign owner after cross-platform checks. They are suitable for extraction by an independent website without source edits or duplicated metric definitions.

## Verification

All three platforms: each existing evidence client passed its non-ignored correctness tests: informed 4 passed/4 ignored; algorithm 6 passed/2 ignored. The ignored benchmark cells are invoked explicitly by the campaign. These counts do not replace or inflate the previously passing full Graph V&V population.

The portable driver has four fast unit tests covering the complete matrix and malformed/missing/duplicate sample rows, drift, invalid numbers and fabricated summaries. Four additional negative controls using the actual Windows campaign reject source mismatch, checksum drift, duplicate repetition and a missing cell. Every raw Windows log also passed the stricter final parser. Windows/native exact result and work-counter parity passed, as did A*/Dijkstra optimal-cost and BFS/Dijkstra reachability/hop/unit-cost checks.

Formatting, focused Python static checks, generated symbol index and the eight committed Graph governance checks passed. Production source and dependency diffs are empty. Built-in token clone measurement: **512 clones / 12,201 cloned lines before and after; delta 0 / 0**, source files 150 to 152. No new dependencies.

## Completion and limitations

Final three-platform results, timings and artifact hashes are recorded below. The independent Next.js website remains blocked by automatic approval review; its absence must not be confused with missing Unix permissions or benchmark failure.


## Final three-platform result

All **531 benchmark processes exited successfully**: 522 timed invocations plus nine explicit not-applicable rank-2/STS511 invocations. There are **174 pooled metric rows and 4,698 raw timing samples**. All 531 logs reparse exactly to their archived structured records. Cross-run/platform result hashes, graph dimensions and search work counters match. All **432 informed-client result rows** match the committed, independently reproduced 2026-09-04 reference exactly. No numerical substitution, source/API repair, fallback algorithm or dependency update was required.

| Platform | Processes | Timed cases | Whole-process peak RSS | Sum of process wall times |
|---|---:|---:|---:|---:|
| Windows native | 177 | 58 x 3 | 265.37 MiB | 465.78 s |
| Amaterasu native | 177 | 58 x 3 | 261.61 MiB | 489.83 s |
| Amaterasu ARM64 Docker/QEMU | 177 | 58 x 3 | 276.74 MiB | 644.94 s |

All observed memory is far below the 16GiB budget. Process-wall sums exclude builds, orchestration gaps and report generation. Correctness tests passed 10 per platform, 30 in total; the deliberately ignored benchmark entry points were invoked by the measurement driver. The benchmark work completed earlier than the plan's 45--75 minute estimate; all declared cells and repeats ran. No unrelated service was stopped to obtain these values.

Two illustrative within-platform median ratios (Dijkstra time divided by A* time):

| Workload / heuristic | Windows | Amaterasu native | ARM64 QEMU |
|---|---:|---:|---:|
| grid200 d80 / Manhattan A* | 176.03x | 146.29x | 90.82x |
| transport128 / discrete A* | 5.70x | 5.58x | 4.37x |

These are workload-specific comparisons with equal optimal costs, not framework-wide speedup claims. Weighted-map cases and every negative/neutral result remain in the complete CSV. Cross-platform ratios are not hardware-normalized comparisons, and QEMU results are not native ARM hardware performance.

Plot: [selected search medians](cross-platform-benchmarks/2026-09-06/search-platforms.svg). Complete [numeric summary](cross-platform-benchmarks/2026-09-06/platform-summary.csv), [resource summary](cross-platform-benchmarks/2026-09-06/process-resources.csv), [verification receipt](cross-platform-benchmarks/2026-09-06/verification.json) and [historical parity](cross-platform-benchmarks/2026-09-06/historical-parity.json).

The exact original external measurement driver is retained as a hashed text transcript, `measurement-driver.py.txt`. The final owning script adds stricter validation, cross-platform aggregation and formatting; the Rust timing clients and measured operations are unchanged. `driver-provenance.json` identifies the original bytes. All campaign evidence bytes are preserved through Git with a scoped `.gitattributes` rule.

Recompute the summary without rerunning a benchmark:

```sh
python3 scripts/performance_evidence/campaign.py --summarize reports/cross-platform-benchmarks/2026-09-06
python3 -m unittest discover -s scripts/performance_evidence -p test_campaign.py -v
```

Build the same clients with `cargo test --release --locked --offline -j 4 -p hymeko_graph --test informed_berge_evidence --test performance_evidence --no-run`; add `--target aarch64-unknown-linux-gnu` with the recorded cross image for ARM64. The campaign CLI takes explicit executable paths, a new output directory and `--platform windows|native|arm64`; ARM64 additionally takes `--qemu`. Every actual runtime command is retained in each platform's `campaign.json`.

## Changed files and delivery

Changes are limited to `.gitattributes`, generated `SYMBOLS.md`, the four-format Graph plan, `scripts/performance_evidence/campaign.py`, `scripts/performance_evidence/test_campaign.py`, this report and its dated evidence directory. A generated file inventory records exact added/deleted line counts; no production source, Cargo manifest/lock, or Core file changed.

The independent website remains unfinished because of the documented automatic review rejection. SSH/sudo access is not the blocker. The Core folder contains no added website code, configuration or dependencies.


Cleanup completed: all per-cell and build containers were removed; SHA256-verified executable/QEMU staging and temporary LaTeX compiler files were removed. Existing reusable Cargo caches and target volumes remain. No worktree/clone was created and no global SSH, Node, Docker-builder or binfmt configuration changed. Prior Core raw duplicates from the earlier cleanup rejection remain outside this task's changed files.
