# Informed Berge evidence and bounded-enumeration repair

**Date:** 2026-09-04 CEST

**Status:** approved for implementation

**Approval:** the user explicitly approved the three production corrections after executable
public-API probes reproduced each defect: “jóváhagyom ez fontos”.

## Outcome

Create a durable, Kato-compatible evidence package for informed Berge search and bounded Berge
path/cycle enumeration. Correct the three production defects discovered before measurement, then
measure only the corrected canonical entry points.

## Approved production boundary

The only production file that may change is `hymeko_graph/src/direct_ir/berge_enum.rs`:

1. cycle identity must include the alternating node and hyperarc rings, invariant under rotation
   and reflection, so parallel hyperarcs remain distinct;
2. reaching `max_depth` must be observable as `Some(BudgetKind::Depth)` unless a stronger global
   result/expansion cap stops the run; and
3. `max_results = 0` must emit no item and report `Some(BudgetKind::Results)`.

No public signature, type layout, core IR layout, dependency, Cargo manifest or lockfile changes.
Rollback is one production commit.

## Verification contract

- three parallel rank-2 arcs produce three distinct length-2 Berge cycles;
- rotation/reflection still deduplicate traversal direction;
- the existing long-chain depth fixture reports `Depth`;
- zero-result path and cycle budgets return empty results and report `Results`;
- all existing direct-IR and informed-search tests remain green.

## Evidence contract

The stable suite and artifact directory use `informed-berge-evidence`, not a roadmap phase code.
Every performance cell runs in a fresh release process with two warmups and nine retained samples.
It records median, Q1, Q3, worst, operation CPU, process wall/CPU/RSS, deterministic result hashes
and algorithm-work observations where the public API exposes them.

### Informed search

- Dijkstra, Manhattan/Euclidean A*, weighted metric/discrete/combined A* and the admissible
  transport-hyperedge case;
- near/far grid goals, weighted obstacle maps and an adversarial corridor;
- exact full path hash, optimal cost bits, hop count and `SearchStats`;
- heuristic preparation reported separately from warm query time;
- Dijkstra/BFS/exhaustive oracle checks remain normal tests, not benchmark assertions.

### Berge shortest path and bounded enumeration

- unit-cost BFS Berge shortest path versus Dijkstra on the same canonical IR;
- path/cycle output hashed outside each timed boundary;
- layered, cyclic and high-arity workloads with explicit finite budgets;
- result count and stopping reason retained for every cell;
- fixed-budget comparisons are labelled as such; no unbounded enumeration or universal throughput
  claim.

## D*/LPA* boundary

There is no canonical production D*/LPA* implementation. This work must not relabel repeated A* as
D*. A later design may add LPA* for a fixed start with changing external costs or D* Lite for a
moving start, but only behind its own production/API approval and fresh-Dijkstra equivalence suite.

## Acceptance

- the three reproduced regressions pass through public APIs;
- all new cell outputs pass schema, sample-count, quartile, checksum, semantic-parity and non-zero
  RSS validation;
- generated TSV/SVG hashes match;
- prior evidence artifacts remain byte-identical;
- root fmt, Clippy `-D warnings`, check, build, tests and doctests pass;
- the separate benchmark workspace compiles and all four Criterion smoke targets pass; and
- local and remote branch heads match after push.
