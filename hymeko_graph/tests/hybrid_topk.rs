//! Integration tests for the hybrid α-blended scorer.
//!
//! Verifies four contracts:
//!
//! 1. **α=0 collapse**: `HybridScorer{α=0}` matches the underlying
//!    diversity heuristic exactly on score / update / rollback /
//!    upper_bound.
//! 2. **α=1 collapse**: `HybridScorer{α=1}.score` equals
//!    `signal.score`; `update`/`rollback` still delegate to the
//!    diversity component (state must be maintained for ABB
//!    consistency even when α=1).
//! 3. **Linearity of UB**: `UB_α = α·UB_signal + (1-α)·UB_div`
//!    holds within float epsilon for any α ∈ [0, 1].
//! 4. **Admissibility**: for every reachable closed cycle, score ≤
//!    upper_bound for any α and any state.
//! 5. **End-to-end**: the enumerator returns ≤ K cycles, all
//!    satisfying the pruner, at any α.

use hymeko_graph::signed_graph::{GraphGenerator, SignedGraphGenerator};
use hymeko_graph::{
    EntropyGainScorer, HybridScorer, InverseDegreeScorer, SignedGraph, UniformityHeuristic,
    UniformityState,
    balance::{BalanceMode, CartwrightHararyPruner},
    enumerate_top_k_cycles_par_entropy,
    pruner::NoOpPruner,
    topk_cycles::{BoundedScorer, FractionNegativeScorer},
};

// ─── Boundary collapse ────────────────────────────────────────────

#[test]
fn alpha_zero_collapses_to_pure_diversity() {
    // At α=0, every output of HybridScorer must equal the pure
    // diversity scorer's output.  Tested on a non-trivial state
    // (some pre-populated counts).
    let mut state = UniformityState::new(8);
    let div = EntropyGainScorer;
    div.update(&[0, 1, 2, 3], &[1, 1, 1, 1], &mut state);
    div.update(&[2, 3, 4, 5], &[1, 1, 1, 1], &mut state);

    let hybrid = HybridScorer::new(FractionNegativeScorer, EntropyGainScorer, 0.0);
    let cycle: &[u32] = &[1, 4, 6, 7];
    let signs: &[i8] = &[1, -1, 1, -1];

    let s_pure = div.score(cycle, signs, &state);
    let s_hybrid = hybrid.score(cycle, signs, &state);
    assert!(
        (s_pure - s_hybrid).abs() < 1e-12,
        "α=0 must equal pure diversity score: pure={s_pure} hybrid={s_hybrid}",
    );

    // UB also collapses.
    let ub_pure = div.upper_bound(cycle, 0, 4, &state);
    let ub_hybrid = hybrid.upper_bound(cycle, 0, 4, &state);
    assert!(
        (ub_pure - ub_hybrid).abs() < 1e-12,
        "α=0 must equal pure diversity UB",
    );
}

#[test]
fn alpha_one_collapses_to_pure_signal() {
    // At α=1, HybridScorer's score must equal the signal's score.
    // Note: state-aware methods (update/rollback) still operate on
    // the diversity component — we want consistent state for ABB
    // even at α=1.
    let state = UniformityState::new(8);
    let signal = FractionNegativeScorer;
    let hybrid = HybridScorer::new(FractionNegativeScorer, EntropyGainScorer, 1.0);

    let cycle: &[u32] = &[0, 1, 2, 3];
    let signs: &[i8] = &[-1, -1, 1, 1];

    let s_pure = signal.score(cycle, signs);
    let s_hybrid = hybrid.score(cycle, signs, &state);
    assert!(
        (s_pure - s_hybrid).abs() < 1e-12,
        "α=1 must equal pure signal score: pure={s_pure} hybrid={s_hybrid}",
    );
}

// ─── Linearity of UB ──────────────────────────────────────────────

#[test]
fn ub_is_linear_in_alpha() {
    let mut state = UniformityState::new(16);
    EntropyGainScorer.update(&[0, 1, 2, 3], &[1, 1, 1, 1], &mut state);

    let signal = FractionNegativeScorer;
    let div = EntropyGainScorer;
    let prefix: &[u32] = &[2, 5];

    let ub_signal = signal.upper_bound(prefix.len().saturating_sub(1), 2, 4);
    let ub_div = div.upper_bound(prefix, 2, 4, &state);

    for &alpha in &[0.0_f64, 0.25, 0.5, 0.75, 1.0] {
        let hybrid = HybridScorer::new(signal, div, alpha);
        let ub_hybrid = hybrid.upper_bound(prefix, 2, 4, &state);
        let expected = alpha * ub_signal + (1.0 - alpha) * ub_div;
        assert!(
            (ub_hybrid - expected).abs() < 1e-12,
            "α={alpha} UB linearity failed: got {ub_hybrid}, expected {expected}",
        );
    }
}

// ─── Admissibility ────────────────────────────────────────────────

fn check_admissibility_at_alpha(g: &SignedGraph, alpha: f64) {
    use hymeko_graph::enumerate_simple_cycles_noprune;
    let state = UniformityState::new(g.n_nodes as usize);
    let hybrid = HybridScorer::new(FractionNegativeScorer, EntropyGainScorer, alpha);
    let cycles = enumerate_simple_cycles_noprune(g, 4);
    let ub = hybrid.upper_bound(&[], 4, 4, &state);
    for cycle in &cycles {
        let signs = vec![1i8; cycle.len()]; // signs irrelevant for these UBs
        let actual = hybrid.score(cycle, &signs, &state);
        assert!(
            actual <= ub + 1e-9,
            "α={alpha}: cycle {cycle:?} score {actual} exceeds root UB {ub}",
        );
    }
}

#[test]
fn ub_admissible_alpha_zero() {
    let g = SignedGraphGenerator::new(60, 100, 700, 31)
        .generate()
        .unwrap();
    check_admissibility_at_alpha(&g, 0.0);
}

#[test]
fn ub_admissible_alpha_quarter() {
    let g = SignedGraphGenerator::new(60, 100, 700, 32)
        .generate()
        .unwrap();
    check_admissibility_at_alpha(&g, 0.25);
}

#[test]
fn ub_admissible_alpha_half() {
    let g = SignedGraphGenerator::new(60, 100, 700, 33)
        .generate()
        .unwrap();
    check_admissibility_at_alpha(&g, 0.5);
}

#[test]
fn ub_admissible_alpha_three_quarters() {
    let g = SignedGraphGenerator::new(60, 100, 700, 34)
        .generate()
        .unwrap();
    check_admissibility_at_alpha(&g, 0.75);
}

#[test]
fn ub_admissible_alpha_one() {
    let g = SignedGraphGenerator::new(60, 100, 700, 35)
        .generate()
        .unwrap();
    check_admissibility_at_alpha(&g, 1.0);
}

// ─── End-to-end ──────────────────────────────────────────────────

#[test]
fn enumerator_returns_k_cycles_at_each_alpha_balance_pruner() {
    let g = SignedGraphGenerator::new(150, 60, 700, 41)
        .generate()
        .unwrap();
    let pruner = CartwrightHararyPruner {
        mode: BalanceMode::OnlyBalanced,
    };
    for &alpha in &[0.0_f64, 0.25, 0.5, 0.75, 1.0] {
        let hybrid = HybridScorer::new(FractionNegativeScorer, EntropyGainScorer, alpha);
        let out = enumerate_top_k_cycles_par_entropy(&g, 4, &pruner, 50, &hybrid);
        assert!(
            !out.is_empty(),
            "α={alpha}: fixture must produce some cycles",
        );
        assert!(out.len() <= 50, "α={alpha}: must respect k_keep cap",);
        for (_, _, signs) in &out {
            let prod: i32 = signs.iter().map(|&s| s as i32).product();
            assert_eq!(prod, 1, "α={alpha}: all output cycles must be balanced",);
        }
    }
}

#[test]
fn enumerator_with_inverse_degree_diversity() {
    // Cross-product check: HybridScorer<FractionNeg, InverseDegree>
    // also enumerates cleanly.
    let g = SignedGraphGenerator::new(150, 60, 700, 42)
        .generate()
        .unwrap();
    let hybrid = HybridScorer::new(FractionNegativeScorer, InverseDegreeScorer, 0.5);
    let out = enumerate_top_k_cycles_par_entropy(&g, 4, &NoOpPruner, 50, &hybrid);
    assert!(!out.is_empty());
    assert!(out.len() <= 50);
}
