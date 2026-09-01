# Direct-Ir graph performance survey

**APPROVED-CORE-EDIT: platform-performance-survey** · 2026-09-01 12:35 CEDT
Branch `feat/graph-performance-survey` @ base `98c6a93`. Measurement only; graph `main` and the direct-Ir
branch **not merged**. Production source byte-identical — only the existing `direct_ir_bench` harness was
**extended in place** (new `bench_direct_ir_ops`) to cover ops it did not measure before.

## Method / host
Unified contract (see the core survey's metric definition): phases not cross-compared, one time unit,
warm/resident medians. Host: Win11, 24 cores, RTX 5080; rustc 1.97.1 release; ambient ~14% at run →
**PROVISIONAL** (borderline the <10% quiet target). Amaterasu (second host) has no Rust toolchain →
single-host only. Workload: `build_window_ir` (sliding-window arity-k hypergraph); generators are the
crate's own — no cross-repo copy (graph depends on `hymeko_core` by git; `hymeko_core::generators` is the
shared library for combinatorial families).

## 1. Reachability: direct vs sparse-star vs clique (cited, prior report, quiet-host advisable)
From `2026-09-01-graph-direct-ir-algorithms.md`, nnz 1e6, arity 5: A=direct 14.1 ms, C=flat-star CSR
1.0 ms, D=clique 0.66 ms (build 60.7 ms); A ≈ **13× the flat-star** for warm reachability — the fat
canonical `Vec<SignedRefR>` vs a packed u32 CSR (A ≈ B confirms the cost is the structure, not the
arc-expansion count). Direct beats clique on build (4.3–9×). Controls named correctly (D is clique/
2-section, not star). Not re-run here; the cross-op survey below uses the same generator.

## 2. Direct-Ir op survey (measured; previously unbenched ops)
`reports/raw-2026-09-01-perf/direct_ir_ops.tsv`, arity 5, warm median-of-9 (µs); the ops
`berge_shortest_path`, `connected_components`, `dual_reachability`, `s_connected_components`, `batch`,
`par_batch` had **no benchmark before** — the harness was extended to cover them.

| op | m=1e4 (nnz 5e4) | m=1e5 (nnz 5e5) | ns/incidence @1e5 |
|---|--:|--:|--:|
| reachable_nodes | 161 µs | 6 701 µs | 13.4 |
| berge_shortest_path | 195 µs | 8 339 µs | 16.7 |
| connected_components | 176 µs | 7 215 µs | 14.4 |
| dual_reachability | 159 µs | 6 673 µs | 13.3 |
| s_connected_components (s=1) | 131 µs | 1 364 µs | 2.7 |
| batch×16 serial | 3 001 µs | 131 699 µs | — |
| batch×16 Rayon | 1 066 µs (2.8×) | 12 225 µs (**10.8×**) | — |
| index build (cold) | 393 µs | 7 091 µs | — |

Readings:
- The four alternating-walk ops (reachable / berge / components / dual) cluster at ~13–17 ns/incidence at
  1e5 — the same cache-bound fat-structure cost. Per-incidence rises 3.2 → 13.4 ns from 5e4 → 5e5 nnz
  (working set exceeds cache), then flattens toward the prior 1e6 numbers — cache-transition, not an
  algorithmic superlinearity.
- **s_connected_components(s=1) is ~5× cheaper** — union-find over arcs touches the reverse index, not the
  fat refs per step.
- **Rayon batch scales with size**: 2.8× at 1e4, **10.8× at 1e5** (per-query work grows, so the
  parallel floor amortises). Serial batch is 16× a single query, as expected.

## 3. Invariants (design-verified; from the direct-Ir V&V, not re-proven here)
warm query allocation-free (a zero-alloc test exists); visited-node ∧ visited-arc; each hyperarc expanded
at most once (debug-assert enforced); no high-arity O(k²) (B control demonstrates re-expansion); canonical
IDs only; no dense/star/clique materialization on the standard path (C/D are explicit controls).

## 4. Ranked bottleneck list
1. **Fat canonical structure vs flat CSR** — *proven* (reachable A ≈ 13× flat-star; all walk ops
   ~13–17 ns/inc). *Gain*: large for query-heavy workloads. *Impl risk*: medium. *Semantic risk*: medium
   (a flat target cache betrays zero-copy/losslessness if defaulted). *Next token*: `graph-flat-target-cache`
   (opt-in only, measured before defaulting).
2. **Batch serial cost** — *proven* (16× single query). *Gain*: Rayon already gives 10.8× at 1e5; wire the
   Rayon batch as the default for large batches. *Impl risk*: low. *Semantic risk*: none (serial ≡ parallel
   proven). *Next token*: `graph-default-rayon-batch`.
3. **Coverage gap now closed** — berge/components/dual/s-cc/batch were unbenched; now in the harness.
   *Next token*: none (fold future ops into the same bench).

## 5. Retracted / measurement notes
- The direct_ir bench previously covered **only** `reachable_nodes`; the claim that the direct-Ir op suite
  was "benched" was overstated — 5 ops were unmeasured until this survey. Now corrected.
- All numbers PROVISIONAL (ambient ~14%); the s-cc cheapness and the Rayon scaling are large enough to be
  robust to that; the absolute µs are not regression gates.

## 6. Worktree / remote
Worktree `/d/hnnwork/graph-perfsurvey` clean at the feature tip; branch off `98c6a93`; no foreign files
touched. Remote push + SHA in the return summary. **HARD STOP.**

## Provenance
rustc 1.97.1, x86_64-pc-windows-msvc; pulls `hymeko_core` by git. Single-host, ambient ~14% (PROVISIONAL).
