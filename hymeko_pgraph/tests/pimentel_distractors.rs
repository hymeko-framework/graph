//! Pimentel benchmark (`data/pgraph/Chapter6/pimentel_distractors.hymeko`)
//! validation, 2026-06-03.
//!
//! Jean Pimentel sent a hand-prepared P-graph test problem
//! (`feedback/Testing problem description.docx`) augmenting Chapter 6
//! Example 6.1 with three distractor operating units (O8, O9, O10) plus
//! two extra raws (Q, P) and two intermediates (M, N). The distractors
//! violate Friedler axioms A2 / A4 (and A5 transitively) and MUST be
//! filtered out by the MSG stage. The remaining canonical problem has
//! expected outputs:
//!
//!   MSG: {O1, O2, O3, O4, O5, O6, O7}     (7 of 10 units)
//!   SSG: 19 combinatorially feasible structures (via decision-mapping)
//!   ABB top-3 by cost:
//!     #1 {O2, O5, O7} cost  9
//!     #2 {O1, O4}     cost 12
//!     #3 {O1, O3}     cost 13
//!
//! This test fixes the expected values into a cargo-runnable assertion
//! so any future refactor that breaks the distractor filter, the
//! decision-mapping SSG, the cost-rank, or the byproduct-cycle
//! feasibility check (load-bearing for #2 and #3) fails CI.

use hymeko_pgraph::abb::{solve_top_k_with_regime, AbbOptions};
use hymeko_pgraph::msg::maximal_structure_with_regime;
use hymeko_pgraph::regime::CANONICAL;
use hymeko_pgraph::{lower, ssg_dm_enumerate};
use parser::parse_description;

const FIXTURE_PATH: &str = "../data/pgraph/Chapter6/pimentel_distractors.hymeko";

fn load_fixture() -> hymeko_pgraph::LoweredPGraph {
    let src = std::fs::read_to_string(FIXTURE_PATH)
        .unwrap_or_else(|e| panic!("read {FIXTURE_PATH}: {e}"));
    let desc = parse_description(&src)
        .unwrap_or_else(|e| panic!("parse pimentel_distractors.hymeko: {e:?}"));
    lower(&desc).unwrap_or_else(|e| panic!("lower: {e}"))
}

fn unit_names(p: &hymeko_pgraph::LoweredPGraph,
              units: &std::collections::BTreeSet<hymeko::common::ids::DeclId>) -> Vec<String> {
    let mut v: Vec<String> = units.iter()
        .map(|d| p.decl_to_name[d].clone())
        .collect();
    v.sort();
    v
}

#[test]
fn pimentel_msg_filters_three_distractors() {
    let p = load_fixture();
    let m = maximal_structure_with_regime(&p, &CANONICAL);
    let names = unit_names(&p, &m.units);
    assert_eq!(
        names,
        vec!["O1", "O2", "O3", "O4", "O5", "O6", "O7"]
            .iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "axiom filter should strip O8/O9/O10; got {names:?}"
    );
}

#[test]
fn pimentel_ssg_decision_mapping_count_is_nineteen() {
    let p = load_fixture();
    let m = maximal_structure_with_regime(&p, &CANONICAL);
    let sols = ssg_dm_enumerate(&p, &m);
    // Decision-mapping returns each solution-structure exactly once.
    // Pimentel's docx specifies 19 for this graph.
    assert_eq!(
        sols.len(),
        19,
        "expected 19 decision-mapping solution-structures; got {}",
        sols.len()
    );
}

#[test]
fn pimentel_abb_top_three_match_docx() {
    let p = load_fixture();
    let m = maximal_structure_with_regime(&p, &CANONICAL);
    let top = solve_top_k_with_regime(
        &p, &m, 3, AbbOptions::default(), &CANONICAL,
    );
    assert_eq!(top.len(), 3, "expected 3 top solutions; got {}", top.len());

    // #1: {O2, O5, O7} cost 9.0
    assert_eq!(unit_names(&p, &top[0].units), vec!["O2", "O5", "O7"]);
    assert!((top[0].cost - 9.0).abs() < 1e-9,
            "#1 cost: expected 9.0, got {}", top[0].cost);

    // #2: {O1, O4} cost 12.0  (byproduct cycle: O1 makes F that O4 uses)
    assert_eq!(unit_names(&p, &top[1].units), vec!["O1", "O4"]);
    assert!((top[1].cost - 12.0).abs() < 1e-9,
            "#2 cost: expected 12.0, got {}", top[1].cost);

    // #3: {O1, O3} cost 13.0  (byproduct cycle: O1 makes F that O3 uses)
    assert_eq!(unit_names(&p, &top[2].units), vec!["O1", "O3"]);
    assert!((top[2].cost - 13.0).abs() < 1e-9,
            "#3 cost: expected 13.0, got {}", top[2].cost);
}

#[test]
fn pimentel_distractor_axioms_report_correct_offenders() {
    // The canonical-axiom certificate on the FULL schema must FAIL
    // with the specific offenders Pimentel listed: J (S2), O10 (S4).
    // analyze_source_with_regime reads + lowers internally, so we
    // skip the load_fixture() helper here.
    let json = hymeko_pgraph::analyze_source_with_regime(
        &std::fs::read_to_string(FIXTURE_PATH).unwrap(),
        hymeko_pgraph::DumpAlgorithm::Abb,
        &CANONICAL,
        AbbOptions::default(),
    );
    assert_eq!(json.canonical_full.status, "FAIL",
               "canonical certificate must FAIL on the distractor-augmented fixture");

    let tags: std::collections::BTreeSet<String> =
        json.canonical_full.violation_tags.iter().cloned().collect();
    assert!(tags.contains("S2"), "expected S2 violation; tags={tags:?}");
    assert!(tags.contains("S4"), "expected S4 violation; tags={tags:?}");

    // S2 should name raw:J as offender; S4 should name O10.
    let offenders: std::collections::BTreeMap<String, Vec<String>> =
        json.canonical_full.offenders.iter().cloned().collect();
    let s2 = offenders.get("S2").expect("S2 offenders list missing");
    assert!(s2.iter().any(|o| o == "raw:J"),
            "S2 offenders should include raw:J; got {s2:?}");
    let s4 = offenders.get("S4").expect("S4 offenders list missing");
    assert!(s4.iter().any(|o| o == "O10"),
            "S4 offenders should include O10; got {s4:?}");
}
