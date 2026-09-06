# Informed A* and Berge evidence — same-host reproduction

Date: 2026-09-04 CEST

Repository tip: `12fd6cd80e529a29cefdc45434c25fb671231736`

Compared baseline: `reports/informed-berge-evidence/2026-09-04-local-baseline`

## Verdict

The semantic and algorithmic evidence reproduces exactly. Absolute wall-clock
headlines do not reproduce to a narrow 3% tolerance on this uncontrolled
workstation, but the within-run conclusions and speedup classes remain intact.
There is no reproduced correctness or performance blocker for continued static
graph development.

The effective measurement client and independent oracle blobs are identical to
the baseline (`a6f4a30b...` and `fdc93d64...`). After removing only the run and
process identifiers, the new `results.tsv` and `search-work.tsv` have zero rows
different from the baseline. Thus reachability, cost, hop/item count, full path
hash, enumeration bounds, expansions, incidence resolutions, queue operations,
stale pops and reopenings all reproduce exactly.

## Measurement integrity

- quiet-host gate: 15% average, 21% maximum CPU;
- 48 isolated release child processes, all successful;
- two warmups and nine retained samples per cell;
- all external peak-RSS values are non-zero, from 5.00 to 261.25 MiB;
- all 12 retained TSV/SVG hashes verify;
- the first invocation failed closed on unavailable mDNS; the successful run
  used Cargo CLI Git fetching with a process-local URL rewrite from
  `amaterasu.local` to `192.168.1.228`. No repository remote or global Git
  configuration was changed.

The successful effective invocation was:

```powershell
$env:GIT_CONFIG_COUNT='1'
$env:GIT_CONFIG_KEY_0='url.ssh://git@192.168.1.228:2222/.insteadOf'
$env:GIT_CONFIG_VALUE_0='ssh://git@amaterasu.local:2222/'
$env:CARGO_NET_GIT_FETCH_WITH_CLI='true'
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts/informed_berge_evidence/run.ps1 `
  -RunDate '2026-09-04-repro-KYBERSAMAN' `
  -ScratchRoot 'D:\h\codex-graph-p34-audit\repro-current'
```

## Timing reproduction

Across all 48 matched cells, the median of new/baseline wall-time ratios is
1.0519. The range is 0.9852–1.3414; only five baseline/reproduction IQR pairs
overlap. This is substantial evidence that exact absolute microsecond values
should not be promoted as portable regression thresholds from these two
workstation sessions.

The important within-run ratios remain stable:

| Workload/subject | Baseline speedup | Reproduction speedup |
|---|---:|---:|
| grid d20 A* L1 / Dijkstra | 30.48x | 30.27x |
| grid d40 A* L1 / Dijkstra | 73.59x | 72.61x |
| grid d80 A* L1 / Dijkstra | 173.18x | 178.70x |
| transport128 discrete A* / Dijkstra | 5.63x | 5.30x |
| path65k Berge BFS / unit Dijkstra | 1.76x | 1.62x |
| grid256 Berge BFS / unit Dijkstra | 1.96x | 1.85x |
| random1e6 Berge BFS / unit Dijkstra | 3.47x | 3.44x |
| map512 Berge BFS / unit Dijkstra | 1.67x | 1.65x |

The weighted map cases also reproduce the negative result: metric, discrete and
combined A* do not prune and remain slower than Dijkstra. The STS(1023)
single-hop Berge BFS ratio changes from 2227x to 2507x; that special case is
dominated by Dijkstra's unnecessary heap work and is not generalized.

Bounded path/cycle enumeration reproduces every result count, output hash and
stopping bound. Reproduction medians range from 62.943 microseconds for the
grid6 path cell to 2.868 ms for STS(31) cycles; the 16-parallel-arc regression
still returns all 120 distinct two-arc cycles.

## D*/LPA* boundary

D*, D* Lite and LPA* remain unimplemented. Repeated A* was not measured or
reported as an incremental algorithm. A future implementation still needs a
typed change-set contract and equality with a fresh Dijkstra result after every
update.
