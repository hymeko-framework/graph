//! Integration test for the CSR-aligned sign lookup change in
//! `topk_cycles.rs`.
//!
//! Two regression guards:
//!
//! 1. **HashMap parity**: every (u, v) sign that
//!    `SignedGraph::build_sign_lookup` returns must round-trip
//!    through `build_csr_with_signs` + a CSR scan. The unit tests in
//!    the `signed_graph` module already cover this on a hand-checked
//!    fixture; this integration test extends it to a 200-vertex
//!    random signed graph at production-relevant density.
//!
//! 2. **Sequential / parallel cycle-set parity**: the rayon-parallel
//!    `enumerate_top_k_per_vertex_cycles_par` and the sequential
//!    `enumerate_top_k_per_vertex_cycles` must agree on the
//!    canonicalised set of cycles emitted under the same pruner /
//!    scorer config. Confirms that the rayon scratch-hoist refactor
//!    (per-fold-task `Scratch` struct replacing per-iteration
//!    allocations) preserves the algorithm's output exactly.

mod cycle_common;
use cycle_common::{CycleCase, ParallelPerVertex, SerialPerVertex, StructuralParity, TestStrategy};
use hymeko_graph::signed_graph::{GraphGenerator, SignedGraphGenerator};

use hymeko_graph::{
    balance::{BalanceMode, CartwrightHararyPruner},
    topk_cycles::scorers,
};

#[test]
fn parallel_and_sequential_balance_pruner_satisfy_same_invariants() {
    let graph = SignedGraphGenerator::new(200, 50, 700, 42)
        .generate()
        .unwrap();
    let pruner = CartwrightHararyPruner {
        mode: BalanceMode::OnlyBalanced,
    };
    let case = CycleCase {
        graph: &graph,
        pruner: &pruner,
        length: 4,
        keep: 8,
    };
    case.compare(
        &SerialPerVertex,
        &ParallelPerVertex,
        &StructuralParity {
            balanced: true,
            max_count_drift: 0.03,
            min_overlap: Some(0.85),
        },
    );
}

#[test]
fn parallel_and_sequential_no_pruner_match_in_count() {
    use hymeko_graph::{
        enumerate_top_k_per_vertex_cycles_noprune, enumerate_top_k_per_vertex_cycles_par_noprune,
    };
    let g = SignedGraphGenerator::new(200, 50, 700, 7)
        .generate()
        .unwrap();
    let seq = enumerate_top_k_per_vertex_cycles_noprune(&g, 4, 8, scorers::fraction_negative);
    let par = enumerate_top_k_per_vertex_cycles_par_noprune(&g, 4, 8, scorers::fraction_negative);
    StructuralParity {
        balanced: false,
        max_count_drift: 0.03,
        min_overlap: None,
    }
    .verify(4, &seq, &par);
}

#[test]
fn csr_with_signs_round_trips_every_hashmap_entry_dense_fixture() {
    // 200-vertex 5%-density graph: ~2k edges, exercises the dedup
    // path + multi-row CSR layout.
    let g = SignedGraphGenerator::new(200, 50, 700, 99)
        .generate()
        .unwrap();
    let lk = g.build_sign_lookup();
    let (row_ptr, col_idx, signs_csr) = g.build_csr_with_signs();
    for (&(u, v), &s) in lk.iter() {
        let row_u = {
            let lo = row_ptr[u as usize] as usize;
            let hi = row_ptr[u as usize + 1] as usize;
            col_idx[lo..hi]
                .iter()
                .position(|&x| x == v)
                .map(|pos| signs_csr[lo + pos])
        };
        let row_v = {
            let lo = row_ptr[v as usize] as usize;
            let hi = row_ptr[v as usize + 1] as usize;
            col_idx[lo..hi]
                .iter()
                .position(|&x| x == u)
                .map(|pos| signs_csr[lo + pos])
        };
        assert_eq!(row_u, Some(s), "CSR scan ({u},{v}) != HashMap sign {s}");
        assert_eq!(row_v, Some(s), "CSR scan ({v},{u}) != HashMap sign {s}");
    }
}
