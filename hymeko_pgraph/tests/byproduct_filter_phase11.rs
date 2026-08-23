//! Phase 11 (2026-05-20): NAS-quality filter via by-product
//! injection on empirically dominated HSIKAN architecture choices.
//!
//! Pins the structural behaviour of
//! `data/hsikan/sweep_msg_byproduct_dominated.hymeko`:
//!
//!   * strict-mode MSG drops `train_short` AND `model_h32`
//!     (both produce un-consumed by-products);
//!   * relaxed-mode MSG keeps all 8 units;
//!   * ABB-selected architectures differ between modes;
//!   * canonical S1..S5 PASS on the full schema (Friedler 1992 only
//!     forbids un-touched M-nodes, not by-products);
//!   * extension E-NoExcess FAILS on the two by-product M-nodes.
//!
//! The 5-seed training-time AUC validation lives in
//! `hymeko_neuro/tests/test_byproduct_filter_e2e.py` (Python).

use std::collections::BTreeSet;

use hymeko_pgraph::{
    AbbOptions, AxiomBundle, ExtensionAxiomBundle, ExtensionAxiomViolation,
    abb_solve, lower,
};
use hymeko_pgraph::msg::{MaximalStructureOptions, maximal_structure_with_options};
use parser::parse_description;

const FIXTURE: &str = include_str!(
    "../../data/hsikan/sweep_msg_byproduct_dominated.hymeko"
);

fn lower_fixture() -> hymeko_pgraph::LoweredPGraph {
    let d = parse_description(FIXTURE).expect("parser must accept fixture");
    lower(&d).expect("lower must succeed")
}

#[test]
fn strict_msg_drops_both_dominated_producers() {
    let p = lower_fixture();
    let m = maximal_structure_with_options(
        &p, MaximalStructureOptions { strict_no_excess: true },
    );
    let kept: BTreeSet<String> = m
        .units
        .iter()
        .map(|u| p.decl_to_name[u].clone())
        .collect();
    assert!(!kept.contains("train_short"),
        "strict MSG must drop train_short (produces un-consumed wasted_potential): kept={kept:?}");
    assert!(!kept.contains("model_h32"),
        "strict MSG must drop model_h32 (produces un-consumed unused_capacity): kept={kept:?}");
    // Six survivors: m4/m16/m64 × h8/h16 × train_long.
    assert_eq!(kept.len(), 6, "kept={kept:?}");
}

#[test]
fn relaxed_msg_keeps_all_eight_units() {
    let p = lower_fixture();
    let m = maximal_structure_with_options(
        &p, MaximalStructureOptions { strict_no_excess: false },
    );
    assert_eq!(m.units.len(), 8);
}

#[test]
fn strict_abb_picks_m4_h8_long() {
    let p = lower_fixture();
    let m = maximal_structure_with_options(
        &p, MaximalStructureOptions { strict_no_excess: true },
    );
    let sol = abb_solve(&p, &m).expect("strict ABB must succeed");
    let names: BTreeSet<String> = sol
        .units
        .iter()
        .map(|u| p.decl_to_name[u].clone())
        .collect();
    let expected: BTreeSet<String> = ["cycle_topk_m4", "model_h8", "train_long"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(names, expected, "strict ABB pick: {names:?}");
    assert!((sol.cost - 150.0).abs() < 1e-9, "strict ABB cost: {}", sol.cost);
}

#[test]
fn relaxed_abb_picks_dominated_m4_h8_short() {
    let p = lower_fixture();
    let m = maximal_structure_with_options(
        &p, MaximalStructureOptions { strict_no_excess: false },
    );
    let sol = hymeko_pgraph::abb::solve_with_options(
        &p, &m,
        AbbOptions { strict_no_excess: false, ..AbbOptions::default() },
    )
    .expect("relaxed ABB must succeed");
    let names: BTreeSet<String> = sol
        .units
        .iter()
        .map(|u| p.decl_to_name[u].clone())
        .collect();
    let expected: BTreeSet<String> = ["cycle_topk_m4", "model_h8", "train_short"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(names, expected, "relaxed ABB pick: {names:?}");
    assert!((sol.cost - 60.0).abs() < 1e-9, "relaxed ABB cost: {}", sol.cost);
}

#[test]
fn canonical_passes_on_full_schema_extension_flags_both_byproducts() {
    let p = lower_fixture();
    let canonical = AxiomBundle::new(p.raws.iter().copied(), [])
        .validate(&p.schema, &p.products);
    canonical.expect(
        "canonical Friedler S1..S5 must pass on the byproduct-injected fixture"
    );

    let extension = ExtensionAxiomBundle::new(p.raws.iter().copied())
        .validate(&p.schema, &p.products);
    let v = extension.expect_err(
        "extension bundle must catch the by-product M-nodes"
    );
    // Pull the names of the un-reaching offenders out and assert
    // BOTH our injected by-products are flagged.
    let offenders: BTreeSet<String> = v.iter().flat_map(|x| match x {
        ExtensionAxiomViolation::NonReachingMaterials { offenders } => {
            offenders.iter().map(|d| p.decl_to_name[d].clone()).collect::<Vec<_>>()
        }
        _ => vec![],
    }).collect();
    assert!(offenders.contains("unused_capacity"),
        "extension must flag unused_capacity: {offenders:?}");
    assert!(offenders.contains("wasted_potential"),
        "extension must flag wasted_potential: {offenders:?}");
}
