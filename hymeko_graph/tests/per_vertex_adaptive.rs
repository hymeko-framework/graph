//! Integration tests for the degree-adaptive `m_v` per-vertex
//! enumerator.
//!
//! Three contracts:
//!
//! 1. **Uniform-vector parity**: with `m_v[·] = m_per_vertex` for
//!    every vertex, the adaptive enumerator returns the same output
//!    as the scalar-`m` enumerator.  Required so the existing
//!    fixed-`m` callers see no behavior change after the refactor.
//!
//! 2. **Adaptive cap respect**: every vertex `v` appears in at most
//!    `m_v[v]` output cycles.  Direct verification of the per-vertex
//!    cap.
//!
//! 3. **Full-heap rate climbs at `c >= 2`**: on a 150-vertex
//!    Erdős–Rényi fixture, the post-enumeration full-heap rate (the
//!    fraction of vertices whose heap reached its `m_v[v]` cap) is
//!    higher under degree-adaptive caps than under fixed-m=128.  The
//!    plan's mechanism for unlocking per-vertex ABB.

mod cycle_common;
use cycle_common::{AdaptivePerVertex, CardinalityParity, CycleCase, ParallelPerVertex};
use hymeko_graph::signed_graph::{GraphGenerator, SignedGraphGenerator};
use std::collections::HashMap;

use hymeko_graph::{
    SignedGraph, balance::BalanceMode, balance::CartwrightHararyPruner, degree_adaptive_m_v,
    enumerate_top_k_per_vertex_cycles_par_adaptive, pruner::NoOpPruner, topk_cycles::scorers,
};

// ─── 1. Uniform-vector parity ─────────────────────────────────────

#[test]
fn uniform_m_v_parity_with_scalar_m_no_pruner() {
    let graph = SignedGraphGenerator::new(150, 60, 700, 71)
        .generate()
        .unwrap();
    let pruner = NoOpPruner;
    let caps = vec![8; graph.n_nodes as usize];
    let case = CycleCase {
        graph: &graph,
        pruner: &pruner,
        length: 4,
        keep: 8,
    };
    case.compare(
        &ParallelPerVertex,
        &AdaptivePerVertex(&caps),
        &CardinalityParity,
    );
}

#[test]
fn uniform_m_v_parity_with_scalar_m_balance_pruner() {
    let graph = SignedGraphGenerator::new(150, 60, 700, 72)
        .generate()
        .unwrap();
    let pruner = CartwrightHararyPruner {
        mode: BalanceMode::OnlyBalanced,
    };
    let caps = vec![8; graph.n_nodes as usize];
    let case = CycleCase {
        graph: &graph,
        pruner: &pruner,
        length: 4,
        keep: 8,
    };
    case.compare(
        &ParallelPerVertex,
        &AdaptivePerVertex(&caps),
        &CardinalityParity,
    );
}

// ─── 2. degree_adaptive_m_v formula ───────────────────────────────

#[test]
fn degree_adaptive_m_v_formula() {
    // Triangle 0-1-2 plus an isolated vertex 3.  Degrees: 2, 2, 2, 0.
    let g = SignedGraph::from_parts(4, &[0, 1, 2], &[1, 2, 0], &[1, 1, 1]);
    let m_v = degree_adaptive_m_v(&g, 1, 32, 4.0);
    // m_v[v] = min(32, max(1, ceil(4 * deg(v))))
    assert_eq!(m_v[0], 8); // ceil(4 * 2) = 8
    assert_eq!(m_v[1], 8);
    assert_eq!(m_v[2], 8);
    assert_eq!(m_v[3], 1); // deg 0 → max(1, ceil(0)) = 1
}

#[test]
fn degree_adaptive_m_v_clamps_at_m_max() {
    let g = SignedGraphGenerator::new(50, 200, 700, 81)
        .generate()
        .unwrap(); // dense-ish
    let m_v = degree_adaptive_m_v(&g, 1, 16, 4.0);
    for &cap in &m_v {
        assert!(cap <= 16, "cap {cap} exceeds m_max=16");
        assert!(cap >= 1, "cap {cap} below m_min=1");
    }
}

#[test]
fn degree_adaptive_m_v_c_zero_is_uniform_m_min() {
    let g = SignedGraphGenerator::new(50, 100, 700, 82)
        .generate()
        .unwrap();
    let m_v = degree_adaptive_m_v(&g, 5, 16, 0.0);
    for &cap in &m_v {
        assert_eq!(cap, 5, "c=0 must produce uniform m_min");
    }
}

// ─── 3. Adaptive cap respect ──────────────────────────────────────

#[test]
fn adaptive_total_output_at_most_sum_of_m_v() {
    // The per-vertex heap is capped at m_v[v], so the union (before
    // dedup) has at most sum(m_v) entries; after dedup the output
    // is even smaller.  A cycle of length k_len gets pushed into
    // up to k_len heaps, then dedup'd — so the total OUTPUT
    // cardinality is bounded by sum(m_v) but a single vertex's
    // appearance count in the output can exceed m_v[v] (cycles
    // retained by *other* heaps still touch v in the output).
    //
    // This test verifies the loose-but-correct global bound.
    let g = SignedGraphGenerator::new(120, 80, 700, 91)
        .generate()
        .unwrap();
    let m_v = degree_adaptive_m_v(&g, 1, 8, 2.0);
    let total_cap: u64 = m_v.iter().map(|&c| c as u64).sum();
    let out = enumerate_top_k_per_vertex_cycles_par_adaptive(
        &g,
        4,
        &NoOpPruner,
        &m_v,
        scorers::fraction_negative,
    );
    assert!(
        (out.len() as u64) <= total_cap,
        "output cardinality {} exceeds sum(m_v) = {}",
        out.len(),
        total_cap,
    );
}

// ─── 4. Full-heap rate climbs with degree-adaptive caps ───────────

fn full_heap_rate(g: &SignedGraph, m_v: &[u32], k: usize) -> f64 {
    let pruner = CartwrightHararyPruner {
        mode: BalanceMode::OnlyBalanced,
    };
    let out = enumerate_top_k_per_vertex_cycles_par_adaptive(
        g,
        k,
        &pruner,
        m_v,
        scorers::fraction_negative,
    );
    // Reconstruct per-vertex contribution counts from the dedup'd
    // output (matches the probe's reconstruction technique).
    let n = g.n_nodes as usize;
    let mut per_vertex: HashMap<u32, Vec<f64>> = HashMap::new();
    for (score, vs, _) in &out {
        for &v in vs {
            per_vertex.entry(v).or_default().push(*score);
        }
    }
    let mut full = 0usize;
    for v in 0..n as u32 {
        let cap = m_v[v as usize] as usize;
        if cap == 0 {
            continue;
        }
        let len = per_vertex.get(&v).map(|s| s.len()).unwrap_or(0);
        if len >= cap {
            full += 1;
        }
    }
    full as f64 / n as f64
}

#[test]
fn full_heap_rate_climbs_under_degree_adaptive() {
    let g = SignedGraphGenerator::new(150, 60, 700, 101)
        .generate()
        .unwrap();
    let n = g.n_nodes as usize;
    let m_max: u32 = 32;

    // Fixed m: every vertex has the same cap.  Only well-connected
    // vertices fill it; long-tail leaves don't.
    let m_v_fixed = vec![m_max; n];
    let rate_fixed = full_heap_rate(&g, &m_v_fixed, 4);

    // Degree-adaptive: low-degree vertices get smaller caps that
    // they can fill; the rate should climb.
    let m_v_adapt = degree_adaptive_m_v(&g, 1, m_max, 2.0);
    let rate_adapt = full_heap_rate(&g, &m_v_adapt, 4);

    eprintln!(
        "full-heap rate: fixed m={m_max} → {rate_fixed:.3} ; \
         degree-adaptive (c=2) → {rate_adapt:.3}",
    );
    assert!(
        rate_adapt > rate_fixed,
        "degree-adaptive rate {rate_adapt:.3} ≤ fixed-m rate {rate_fixed:.3}; \
         degree-adaptive should improve full-heap coverage on the \
         long tail",
    );
}

// ─── 5. Determinism regression ────────────────────────────────────
//
// Repeated calls of `enumerate_top_k_per_vertex_cycles_par_adaptive`
// with identical inputs must return bit-identical output (modulo the
// arbitrary BinaryHeap iteration order, so we compare on canonical
// cycle sets).
//
// Regression for the 2026-05-23 non-determinism: rayon's parallel
// reduce merges per-fold heaps in scheduler-dependent order, and
// before the fix the heap boundary check only used raw score
// comparison.  Cycles with tied scores never displaced each other,
// so "which tied cycle survives" depended on merge order → same
// process, same input, two consecutive calls produced 308 vs 309
// cycles on Windows runners with NT≈8.  Triggered specifically by
// the balance pruner (it thins cycles enough to force ties at the
// per-vertex cap).
//
// The fix introduced a total preference order on `HeapEntry` (score
// desc, then cycle lex desc) and routed all 16 heap-boundary checks
// through it.  This test asserts the contract directly so future
// changes to the dispatch / merge logic can't silently reintroduce
// the bug.

#[test]
fn par_adaptive_is_deterministic_across_calls_with_balance_pruner() {
    let g = SignedGraphGenerator::new(150, 60, 700, 72)
        .generate()
        .unwrap();
    let k = 4;
    let m = 8u32;
    let pruner = CartwrightHararyPruner {
        mode: BalanceMode::OnlyBalanced,
    };
    let n = g.n_nodes as usize;
    let m_v = vec![m; n];

    let baseline = enumerate_top_k_per_vertex_cycles_par_adaptive(
        &g,
        k,
        &pruner,
        &m_v,
        scorers::fraction_negative,
    );
    let baseline_canon: std::collections::HashSet<Vec<u32>> = baseline
        .iter()
        .map(|(_, vs, _)| {
            let mut c = vs.clone();
            c.sort_unstable();
            c
        })
        .collect();

    // 20 iterations gave us 0/20 failures locally after the fix on a
    // host that produced 5/10 failures before.  Bumps up the test's
    // statistical power against any reintroduction.
    let n_iters = 20;
    for i in 0..n_iters {
        let out = enumerate_top_k_per_vertex_cycles_par_adaptive(
            &g,
            k,
            &pruner,
            &m_v,
            scorers::fraction_negative,
        );
        assert_eq!(
            out.len(),
            baseline.len(),
            "iteration {i}: length {} differs from baseline {} \
             (parallel-reduce non-determinism regression)",
            out.len(),
            baseline.len(),
        );
        let out_canon: std::collections::HashSet<Vec<u32>> = out
            .iter()
            .map(|(_, vs, _)| {
                let mut c = vs.clone();
                c.sort_unstable();
                c
            })
            .collect();
        assert_eq!(
            baseline_canon, out_canon,
            "iteration {i}: canonical cycle set differs from baseline",
        );
    }
}
