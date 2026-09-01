# Workload matrix — direct-Ir informed Berge search

Created-at 2026-09-01 20:25 CEST · P1.1. Small/medium workloads are **source-first** (real `.hymeko` →
parser → `Ir`), reusing `direct_ir_common::compile` and the `sunflower_src`/`hub_src` generators
(direct_ir_families.rs). Large workloads are **deterministically generated** via `build_window_ir`
(direct_ir_bench.rs) and new weighted/signed generators. No new workload crate (§6.1 reuse).

## Sizes (incidences) × arity
| incidences | arity 2 | arity 5 | arity 20 |
|--:|:--:|:--:|:--:|
| 10² | ✓ src | ✓ gen | ✓ gen |
| 10³ | ✓ src | ✓ gen | ✓ gen |
| 10⁴ | — | ✓ gen | ✓ gen |
| 10⁵ | — | ✓ gen | ✓ gen |
| 10⁶ | — | ✓ gen (headline) | — |

## Topology families
uniform (window) · hub/skew (`hub_src`) · sunflower (`sunflower_src`) · Steiner (core `generators::steiner`)
· Mycielski (design/gen) · multi-component grid · long chain (metric-friendly).

## Domain-flavoured (source-first where a fixture exists)
computer-network / VLAN / service-dependency · robot chain / humanoid / hexapod · spatial/metric (positions
attached via a `PositionProvider` keyed by DeclId). Where no committed fixture exists, a generated analogue
is used and labelled `gen` (not claimed as a real domain measurement).

## Signs & weights
`+ / − / ~` mixes; weighted (incidence `weights`) and unweighted (unit). Duplicate-incidence-heavy variant.
Disconnected (multi-component) variant. Metric-friendly (positions consistent with a short path) and
metric-misleading (positions that mislead a geometric heuristic) variants.

## Per-cell measured columns (§11)
µs/ms (explicit unit) · incidences/sec · expanded nodes · expanded arcs · resolved incidences · queue
push/pop/stale-pop/decrease · allocations/query · retained bytes · peak RSS · heuristic precompute (separate
column, never folded into warm query) · time-to-first · total time · path cost · path hash. Break-even query
count computed from (precompute) / (per-query saving).

## What is measured vs designed this phase
Implemented + measured: window (uniform) + hub + sunflower at 10²–10⁵ (10⁶ headline for the winning cell),
arity 2/5/20, unit + weighted, metric-friendly + metric-misleading, disconnected, duplicate-heavy; A*(Zero)
vs A*(metric) vs BFS vs test-Dijkstra oracle. Mycielski/full domain fixtures and the 10⁶×arity-20 corner are
**classified here and deferred** with the reason (fixture/compile budget), never silently dropped (§ live
observability / no silent caps).
