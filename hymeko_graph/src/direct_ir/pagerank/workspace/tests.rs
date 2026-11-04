// Frozen source SHA-256: d17badffe5b1308db9c73e0ff808e8fcdb081c8ff845b03481b6e8938ee1503d
use super::*;

// Frozen pre-change normalization and finish loops. The Sum primitive itself
// is unchanged and separately covered by the existing IEEE-pattern contracts.
// This oracle does not call the new fused sweep or InputNormalization.
struct BeforeSweep;
impl BeforeSweep {
    // Frozen from run_krylov_initialized before workspace extraction:
    // krylov.rs SHA-256 0411a15a1f12bad5208a4f61c2ab344052744e94dbac44712a0c89130695e8cc.
    // The candidate admission is independent of prepare_candidate; normalization
    // below also predates InputNormalization and does not call production staging.
    fn candidate(scores: &mut [f64]) -> Result<(), PageRankError> {
        if scores.iter().any(|x| !x.is_finite()) {
            return Err(PageRankError::NumericalMass);
        }
        for x in scores.iter_mut() {
            *x = x.max(0.0);
        }
        let mass = Sum::of(scores.iter().copied());
        if !mass.is_finite() || mass <= 0.0 {
            return Err(PageRankError::NumericalMass);
        }
        for score in scores {
            *score /= mass;
        }
        Ok(())
    }

    fn normalize(scores: &[f64], outgoing: &[f64], weighted: bool) -> (Vec<f64>, f64) {
        let mut scaled = vec![0.0; scores.len()];
        let mut dangling = Sum::default();
        for (node, &degree) in outgoing.iter().enumerate() {
            scaled[node] = if degree == 0.0 {
                dangling.add(scores[node]);
                0.0
            } else if weighted {
                scores[node]
            } else {
                scores[node] / degree
            };
        }
        (scaled, dangling.total())
    }
    fn finish(
        scores: &mut [f64],
        incoming: &[Sum],
        personalization: &[f64],
        teleport: f64,
    ) -> Result<f64, PageRankError> {
        let mut change = Sum::default();
        let mut mass = Sum::default();
        for ((score, incoming), &personalization) in
            scores.iter_mut().zip(incoming).zip(personalization)
        {
            let next = incoming.total() + teleport * personalization;
            if !next.is_finite() || next < 0.0 {
                return Err(PageRankError::NumericalMass);
            }
            change.add((next - *score).abs());
            mass.add(next);
            *score = next;
        }
        if (mass.total() - 1.0).abs() > 1e-8 {
            return Err(PageRankError::NumericalMass);
        }
        Ok(change.total())
    }
    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|v| v.to_bits()).collect()
    }
}

#[test]
fn solver_candidate_admission_preserves_bits_errors_and_staging() {
    let tiny = f64::from_bits(1);
    let mut cases = vec![
        vec![],
        vec![1.0],
        vec![0.0, -0.0],
        vec![-1.0, -tiny],
        vec![tiny, tiny, -tiny, -0.0],
        vec![f64::MIN_POSITIVE, 1.0, 1e-17, 1e-17],
        vec![1e300, 1.0, -1e300, 1e-310],
        vec![f64::MAX, f64::MAX],
        vec![-1.0, f64::MAX, f64::MAX],
        vec![-1.0, f64::MAX, f64::MAX, f64::NAN],
        vec![-1.0, tiny, f64::INFINITY],
        vec![-0.0, -tiny, tiny, 0.0],
    ];
    for invalid in [
        f64::NAN,
        f64::from_bits(0xfff8_0000_0000_0001),
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        for position in 0..4 {
            let mut values = vec![-0.0, -1.0, 0.25, 0.75];
            values[position] = invalid;
            cases.push(values);
        }
    }
    // Fixed IEEE-pattern population: signed finite normals and subnormals,
    // spanning enough exponents to exercise compensated summation and clipping.
    let mut state = 0xCA11_DA7Eu64;
    for size in [2, 3, 17, 65, 257] {
        for _ in 0..8 {
            let values = (0..size)
                .map(|_| {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let sign = state & (1u64 << 63);
                    let exponent = ((state >> 32) % 1900) << 52;
                    f64::from_bits(sign | exponent | (state & ((1u64 << 52) - 1)))
                })
                .collect();
            cases.push(values);
        }
    }
    for weighted in [false, true] {
        let mut actual = PageRankWorkspace::new();
        for values in &cases {
            let size = values.len();
            let degrees: Vec<_> = (0..size).map(|i| (i % 7) as f64).collect();
            actual.initialize(size, None).unwrap();
            actual.scores.copy_from_slice(values);
            actual.scaled.fill(-17.0);
            actual.incoming.fill(Sum::default());
            for incoming in &mut actual.incoming {
                incoming.add(3.0);
            }
            actual.dangling = -23.0;
            let mut expected = values.clone();
            let verdict = BeforeSweep::candidate(&mut expected);
            assert_eq!(actual.prepare_candidate(&degrees, weighted), verdict);
            assert_eq!(
                BeforeSweep::bits(&actual.scores),
                BeforeSweep::bits(&expected)
            );
            if verdict.is_ok() {
                let (scaled, dangling) = BeforeSweep::normalize(&expected, &degrees, weighted);
                assert_eq!(
                    BeforeSweep::bits(&actual.scaled),
                    BeforeSweep::bits(&scaled)
                );
                assert_eq!(actual.dangling.to_bits(), dangling.to_bits());
                assert!(
                    actual
                        .incoming
                        .iter()
                        .all(|value| value.total().to_bits() == 0)
                );
            } else {
                assert!(actual.scaled.iter().all(|value| *value == -17.0));
                assert!(actual.incoming.iter().all(|value| value.total() == 3.0));
                assert_eq!(actual.dangling, -23.0);
            }
        }
        // Reusing the same workspace after the entire population clears any
        // rejected candidate's partial state without allocating an alternate IR.
        actual.initialize(3, None).unwrap();
        actual.scores.copy_from_slice(&[-1.0, 2.0, 2.0]);
        actual
            .prepare_candidate(&[0.0, 2.0, 0.0], weighted)
            .unwrap();
        assert_eq!(actual.scores, [0.0, 0.5, 0.5]);
        assert_eq!(actual.dangling, 0.5);
        assert!(actual.incoming.iter().all(|value| value.total() == 0.0));
    }
}

#[test]
fn fused_sweep_matches_frozen_order_and_stages_next_input_bitwise() {
    for nodes in [1, 2, 3, 17, 257] {
        for weighted in [false, true] {
            for personalized in [false, true] {
                let distribution: Vec<_> = (1..=nodes).map(|n| n as f64).collect();
                let degrees: Vec<_> = (0..nodes)
                    .map(|n| {
                        if n % 7 == 0 {
                            0.0
                        } else {
                            1.0 + (n % 5) as f64
                        }
                    })
                    .collect();
                let mut actual = PageRankWorkspace::new();
                actual
                    .initialize(nodes, personalized.then_some(distribution.as_slice()))
                    .unwrap();
                actual.prepare_input(&degrees, weighted);
                let (scaled, dangling) = BeforeSweep::normalize(&actual.scores, &degrees, weighted);
                assert_eq!(
                    BeforeSweep::bits(&actual.scaled),
                    BeforeSweep::bits(&scaled)
                );
                assert_eq!(actual.dangling.to_bits(), dangling.to_bits());
                for _ in 0..3 {
                    for (i, accumulator) in actual.incoming.iter_mut().enumerate() {
                        let value = 0.8 * actual.personalization[nodes - 1 - i];
                        accumulator.add(value * 0.25);
                        accumulator.add(value * 0.75);
                    }
                    let mut scores = actual.scores.clone();
                    let reference = BeforeSweep::finish(
                        &mut scores,
                        &actual.incoming,
                        &actual.personalization,
                        0.2,
                    )
                    .unwrap();
                    let change = actual.finish_update(0.2, &degrees, weighted).unwrap();
                    assert_eq!(change.to_bits(), reference.to_bits());
                    assert_eq!(
                        BeforeSweep::bits(&actual.scores),
                        BeforeSweep::bits(&scores)
                    );
                    let (scaled, dangling) = BeforeSweep::normalize(&scores, &degrees, weighted);
                    assert_eq!(
                        BeforeSweep::bits(&actual.scaled),
                        BeforeSweep::bits(&scaled)
                    );
                    assert_eq!(actual.dangling.to_bits(), dangling.to_bits());
                    assert!(
                        actual
                            .incoming
                            .iter()
                            .all(|a| a.total().to_bits() == 0.0f64.to_bits())
                    );
                }
            }
        }
    }
}

#[test]
fn numerical_errors_preserve_score_writes_and_reinitialization_clears_state() {
    for invalid in [-1.0, f64::NAN, f64::INFINITY, 2.0] {
        let mut actual = PageRankWorkspace::new();
        actual.initialize(3, None).unwrap();
        actual.prepare_input(&[0.0, 2.0, 1.0], false);
        actual.incoming[0].add(0.2);
        actual.incoming[1].add(invalid);
        actual.incoming[2].add(0.6);
        let mut reference = actual.scores.clone();
        let expected = BeforeSweep::finish(
            &mut reference,
            &actual.incoming,
            &actual.personalization,
            0.2,
        );
        assert_eq!(actual.finish_update(0.2, &[0.0, 2.0, 1.0], false), expected);
        assert_eq!(
            BeforeSweep::bits(&actual.scores),
            BeforeSweep::bits(&reference)
        );
        actual.initialize(2, None).unwrap();
        actual.prepare_input(&[1.0, 0.0], false);
        assert_eq!(actual.scaled, vec![0.5, 0.0]);
        assert_eq!(actual.dangling, 0.5);
        assert!(actual.incoming.iter().all(|a| a.total() == 0.0));
    }
    let mut empty = PageRankWorkspace::new();
    empty.initialize(0, None).unwrap();
    empty.prepare_input(&[], false);
    assert_eq!(empty.dangling, 0.0);
    assert_eq!(
        empty.finish_update(0.0, &[], false),
        Err(PageRankError::NumericalMass)
    );
}

#[test]
fn invalid_regrouped_state_restores_checkpoint_and_runs_canonical_update() {
    use crate::direct_ir::pagerank::{PageRank, PageRankModel};
    use crate::ir_test_support::Case;
    use hymeko::ir::ir::IrBuilder;
    let rows: Vec<Vec<usize>> = (0..32).map(|start| (start..start + 32).collect()).collect();
    let borrowed: Vec<_> = rows.iter().map(Vec::as_slice).collect();
    let ir = Case::unsigned(64, &borrowed).build().unwrap();
    let model = PageRank::build(&ir, PageRankModel::default()).unwrap();
    assert!(model.plan_stats().is_some());
    let mut expected = PageRankWorkspace::new();
    expected.initialize(64, None).unwrap();
    expected.prepare_input(&model.outgoing, false);
    let change = model.update(0.85, &mut expected).unwrap();
    let mut actual = PageRankWorkspace::new();
    actual.initialize(64, None).unwrap();
    actual.prepare_input(&model.outgoing, false);
    // Fault injection into the speculative input, while the saved scores remain valid.
    actual.scaled[3] = f64::NAN;
    let (actual_change, certified) = model.update_ranges(0.85, &mut actual).unwrap();
    assert!(certified);
    assert_eq!(actual_change.to_bits(), change.to_bits());
    assert_eq!(
        BeforeSweep::bits(&actual.scores),
        BeforeSweep::bits(&expected.scores)
    );
}

#[test]
fn degree_bound_scatter_matches_canonical_bits_for_generated_inputs() {
    use crate::direct_ir::pagerank::{PageRank, PageRankConfig, PageRankModel};
    use crate::ir_test_support::Case;
    use hymeko::ir::ir::IrBuilder;
    for nodes in [1, 2, 3, 17, 64] {
        let mut rows: Vec<Vec<usize>> = (1..nodes).map(|i| vec![i - 1, i]).collect();
        if nodes == 1 {
            rows.push(vec![0, 0]);
        }
        let borrowed: Vec<_> = rows.iter().map(Vec::as_slice).collect();
        let ir = Case::unsigned(nodes, &borrowed).build().unwrap();
        let fast = PageRank::build(&ir, PageRankModel::default()).unwrap();
        assert!(fast.incoming_term_bound().is_some_and(|n| n <= 2));
        let mut reference = PageRank::build(&ir, PageRankModel::default()).unwrap();
        reference.incoming_terms = None;
        for seed in 0..16 {
            let p: Vec<_> = (0..nodes)
                .map(|i| {
                    if (i + seed) % 5 == 0 {
                        1e-200
                    } else {
                        ((i * 7 + seed) % 13 + 1) as f64
                    }
                })
                .collect();
            let mut a = PageRankWorkspace::new();
            let mut b = PageRankWorkspace::new();
            let config = PageRankConfig {
                tolerance: 1e-12,
                ..Default::default()
            };
            let actual = fast.run(config, &mut a, Some(&p)).unwrap();
            let expected = reference.run(config, &mut b, Some(&p)).unwrap();
            assert_eq!(
                BeforeSweep::bits(actual.scores),
                BeforeSweep::bits(expected.scores)
            );
            assert_eq!(actual.progress, expected.progress);
        }
    }
    // Long gather is still compensated even when each destination receives one term.
    let members: Vec<_> = (0..64).collect();
    let ir = Case::unsigned(64, &[&members]).build().unwrap();
    let fast = PageRank::build(&ir, PageRankModel::default()).unwrap();
    let mut reference = PageRank::build(&ir, PageRankModel::default()).unwrap();
    reference.incoming_terms = None;
    let p: Vec<_> = (0..64).map(|i| if i == 0 { 1.0 } else { 1e-17 }).collect();
    let mut a = PageRankWorkspace::new();
    let mut b = PageRankWorkspace::new();
    assert_eq!(
        BeforeSweep::bits(
            fast.run(PageRankConfig::default(), &mut a, Some(&p))
                .unwrap()
                .scores
        ),
        BeforeSweep::bits(
            reference
                .run(PageRankConfig::default(), &mut b, Some(&p))
                .unwrap()
                .scores
        )
    );
}
