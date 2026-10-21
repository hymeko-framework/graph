//! Regression: [`crate::dump::analyze_source`] on the Gömb toy P-graph.

use hymeko_pgraph::{DumpAlgorithm, analyze_source};

const GOMB_TOY: &str = include_str!("../../data/hsikan/sweep_msg_gomb.hymeko");

#[test]
fn gomb_toy_msg_non_empty() {
    let j = analyze_source(GOMB_TOY, DumpAlgorithm::Msg);
    assert!(j.ok, "{j:?}");
    assert!(!j.msg_units.is_empty());
}

#[test]
fn gomb_toy_ssg_lists_structures() {
    let j = analyze_source(GOMB_TOY, DumpAlgorithm::Ssg);
    assert!(j.ok, "{j:?}");
    let sols = j.ssg_structures.as_ref().expect("ssg");
    assert!(!sols.is_empty(), "{j:?}");
}

#[test]
fn gomb_toy_abb_has_solution() {
    let j = analyze_source(GOMB_TOY, DumpAlgorithm::Abb);
    assert!(j.ok, "{j:?}");
    let abb = j.abb.as_ref().expect("abb solution");
    assert!(!abb.units.is_empty());
    assert!(abb.cost > 0.0);
}

fn source_variants(
    source: &str,
    algorithm: DumpAlgorithm,
    regime: &dyn hymeko_pgraph::regime::Regime,
    options: hymeko_pgraph::AbbOptions,
) -> Vec<hymeko_pgraph::dump::PgraphAnalysisJson> {
    use hymeko_pgraph::dump::{
        SsgAlgorithm, analyze_source_with_regime, analyze_source_with_regime_full,
        analyze_source_with_regime_topk,
    };
    let mut results = vec![analyze_source_with_regime(
        source,
        algorithm,
        regime,
        options.clone(),
    )];
    results.extend(
        [0, 1, 3].map(|k| {
            analyze_source_with_regime_topk(source, algorithm, regime, options.clone(), k)
        }),
    );
    results.extend(
        [
            (0, SsgAlgorithm::Brute),
            (3, SsgAlgorithm::Brute),
            (1, SsgAlgorithm::DecisionMapping),
            (3, SsgAlgorithm::DecisionMapping),
        ]
        .map(|(k, ssg)| {
            analyze_source_with_regime_full(source, algorithm, regime, options.clone(), k, ssg)
        }),
    );
    results
}

#[test]
fn source_variants_keep_frozen_json_and_error_stages() {
    use hymeko_pgraph::{
        AbbOptions,
        regime::{CANONICAL, NO_EXCESS, Regime},
    };
    let neutral = GOMB_TOY.replacen("-G_mem", "~G_mem", 1);
    let regimes: [&dyn Regime; 2] = [&CANONICAL, &NO_EXCESS];
    let mut fingerprints = [0xcbf29ce484222325_u64; 8];
    for (stage, source) in ["", neutral.as_str(), GOMB_TOY].into_iter().enumerate() {
        for algorithm in [DumpAlgorithm::Msg, DumpAlgorithm::Ssg, DumpAlgorithm::Abb] {
            for regime in regimes {
                for weights in [None, Some(vec![0.5])] {
                    let options = AbbOptions {
                        cost_weights: weights.clone(),
                        ..AbbOptions::default()
                    };
                    let rows = source_variants(source, algorithm, regime, options);
                    for (fingerprint, row) in fingerprints.iter_mut().zip(rows) {
                        assert_eq!(row.ok, stage == 2);
                        assert_eq!(row.parse_error.is_some(), stage == 0);
                        assert_eq!(row.lower_error.is_some(), stage == 1);
                        assert_eq!(row.strict_no_excess, regime.name() != "canonical");
                        assert_eq!(row.cost_weights_echo, weights);
                        if stage == 1 {
                            assert_eq!(row.description, "Gomb_Sweep_MSG");
                            assert_eq!(
                                row.lower_error.as_deref(),
                                Some(
                                    "unit `gomb_fast` has a neutral (~) reference; P-graph edges must be + or -"
                                )
                            );
                        }
                        for byte in serde_json::to_vec(&row).unwrap() {
                            *fingerprint =
                                (*fingerprint ^ u64::from(byte)).wrapping_mul(0x100000001b3);
                        }
                    }
                }
            }
        }
    }
    // Frozen against the pre-refactor dump.rs source saved in the task evidence.
    let expected: [u64; 8] = [
        6667203794409334753,
        6667203794409334753,
        6667203794409334753,
        8517452500412257245,
        6667203794409334753,
        8517452500412257245,
        841332433765072571,
        16063358277250660975,
    ];
    assert_eq!(fingerprints, expected);
}
