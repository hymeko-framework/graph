//! Tests for the programmatic [`PgraphBuilder`] (Stage P-engine
//! Slice 1, 2026-05-19).
//!
//! The builder is the in-process path for constructing a
//! [`LoweredPGraph`] from Python (via PyO3) or from Rust code that
//! doesn't want to round-trip through `.hymeko` source or `.pgip`
//! SQLite files. These tests pin:
//!
//!   1. Empty builder fails cleanly.
//!   2. HDA built programmatically reproduces the canonical ABB
//!      optimum $\{$Mixer, Reactor$\}$ at cost 350 (Methane vented).
//!   3. Multi-cost dimensions are alphabetised at build time.
//!   4. Unknown material references fail with a named error.
//!   5. Duplicate names fail.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use hymeko_pgraph::abb::{AbbOptions, solve_with_options};
use hymeko_pgraph::builder::{BuilderError, MaterialKind, PgraphBuilder};
use hymeko_pgraph::msg::maximal_structure;

#[test]
fn empty_builder_build_fails_no_materials() {
    let b = PgraphBuilder::new();
    let err = b.build().expect_err("empty must fail");
    assert!(matches!(err, BuilderError::NoMaterials));
}

#[test]
fn builder_with_materials_but_no_units_fails() {
    let mut b = PgraphBuilder::new();
    b.add_material("a", MaterialKind::Raw).unwrap();
    let err = b.build().expect_err("no units must fail");
    assert!(matches!(err, BuilderError::NoUnits));
}

#[test]
fn duplicate_material_name_fails() {
    let mut b = PgraphBuilder::new();
    b.add_material("a", MaterialKind::Raw).unwrap();
    let err = b
        .add_material("a", MaterialKind::Intermediate)
        .expect_err("duplicate must fail");
    assert!(matches!(err, BuilderError::DuplicateName(ref s) if s == "a"));
}

#[test]
fn unit_name_collides_with_material_name_fails() {
    let mut b = PgraphBuilder::new();
    b.add_material("Mixer", MaterialKind::Raw).unwrap();
    let err = b
        .add_unit("Mixer", &[], &[], 0.0)
        .expect_err("unit/material name collision");
    assert!(matches!(err, BuilderError::DuplicateName(_)));
}

#[test]
fn unit_referencing_unknown_material_fails_at_build_time() {
    let mut b = PgraphBuilder::new();
    b.add_material("a", MaterialKind::Raw).unwrap();
    b.add_material("b", MaterialKind::Product).unwrap();
    b.add_unit("u1", &["a"], &["c_unknown"], 1.0).unwrap();
    let err = b.build().expect_err("unknown material must fail");
    match err {
        BuilderError::UnknownMaterial { unit, target } => {
            assert_eq!(unit, "u1");
            assert_eq!(target, "c_unknown");
        }
        other => panic!("expected UnknownMaterial, got {other:?}"),
    }
}

#[test]
fn build_hda_reproduces_textbook_abb_optimum() {
    // Constructed in code; must match the .hymeko/.pgip result of
    // {Mixer, Reactor, Disposal} at cost 400.
    let mut b = PgraphBuilder::new();
    b.add_material("Toluene", MaterialKind::Raw).unwrap();
    b.add_material("H2", MaterialKind::Raw).unwrap();
    b.add_material("Mix", MaterialKind::Intermediate).unwrap();
    b.add_material("Benzene", MaterialKind::Product).unwrap();
    b.add_material("Methane", MaterialKind::Intermediate)
        .unwrap();
    b.add_unit("Mixer", &["Toluene", "H2"], &["Mix"], 100.0)
        .unwrap();
    b.add_unit("Reactor", &["Mix"], &["Benzene", "Methane"], 250.0)
        .unwrap();
    b.add_unit("DirectSynth", &["Toluene", "H2"], &["Benzene"], 800.0)
        .unwrap();
    b.add_unit("Disposal", &["Methane"], &[], 50.0).unwrap();
    let graph = b.build().expect("HDA must build");

    // Sanity: 5 materials + 4 units.
    assert_eq!(graph.materials.len(), 5);
    assert_eq!(graph.units.len(), 4);
    assert_eq!(graph.raws.len(), 2);
    assert_eq!(graph.products.len(), 1);

    let m = maximal_structure(&graph);
    let sol = solve_with_options(&graph, &m, AbbOptions::default())
        .expect("HDA must have a feasible ABB optimum");
    let names: BTreeSet<String> = sol
        .units
        .iter()
        .map(|d| graph.decl_to_name[d].clone())
        .collect();
    // Canonical optimum: {Mixer, Reactor} at 350 (Methane vented; the
    // Disposal sink reaches no product and is excluded from the maximal
    // structure). Pre-2026-05-27 the buggy strict default gave
    // {Mixer,Reactor,Disposal} at 400.
    let expected: BTreeSet<String> = ["Mixer", "Reactor"].iter().map(|s| s.to_string()).collect();
    assert_eq!(names, expected);
    assert!(
        (sol.cost - 350.0).abs() < 1e-9,
        "cost must be 350, got {}",
        sol.cost
    );
}

#[test]
fn multi_cost_dimensions_alphabetised_at_build_time() {
    let mut b = PgraphBuilder::new();
    b.add_material("a", MaterialKind::Raw).unwrap();
    b.add_material("b", MaterialKind::Product).unwrap();
    // Deliberately add in non-alphabetic order: zulu, alpha, mike.
    let mc: BTreeMap<String, f64> = [("zulu", 5.0), ("alpha", 1.0), ("mike", 3.0)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    b.add_unit_multi_cost("u1", &["a"], &["b"], 0.0, mc)
        .unwrap();
    let graph = b.build().expect("must build");
    assert_eq!(
        graph.cost_dimensions,
        vec!["alpha".to_string(), "mike".to_string(), "zulu".to_string()]
    );
    // Per-unit vector should be alpha=1, mike=3, zulu=5 in that order.
    let u_decl = graph.name_to_decl["u1"];
    assert_eq!(graph.cost_vectors[&u_decl], vec![1.0, 3.0, 5.0]);
}

#[test]
fn multi_cost_with_missing_dim_pads_to_zero() {
    // Two units. u1 declares {capex, co2}; u2 declares {capex, opex}.
    // After build, cost_dimensions = [capex, co2, opex] alphabetised;
    // u1's vector pads opex to 0, u2's pads co2 to 0.
    let mut b = PgraphBuilder::new();
    b.add_material("raw", MaterialKind::Raw).unwrap();
    b.add_material("prod", MaterialKind::Product).unwrap();
    b.add_unit_multi_cost(
        "u1",
        &["raw"],
        &["prod"],
        10.0,
        [("capex", 100.0), ("co2", 5.0)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect(),
    )
    .unwrap();
    b.add_unit_multi_cost(
        "u2",
        &["raw"],
        &["prod"],
        20.0,
        [("capex", 200.0), ("opex", 50.0)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect(),
    )
    .unwrap();
    let graph = b.build().unwrap();
    assert_eq!(
        graph.cost_dimensions,
        vec!["capex".to_string(), "co2".to_string(), "opex".to_string()]
    );
    let u1 = graph.name_to_decl["u1"];
    let u2 = graph.name_to_decl["u2"];
    // u1 = (capex=100, co2=5, opex=0)
    assert_eq!(graph.cost_vectors[&u1], vec![100.0, 5.0, 0.0]);
    // u2 = (capex=200, co2=0, opex=50)
    assert_eq!(graph.cost_vectors[&u2], vec![200.0, 0.0, 50.0]);
}

#[test]
fn programmatic_hda_matches_hymeko_lowered_hda() {
    // The graph we just built programmatically must be semantically
    // equivalent to the one obtained by parsing data/pgraph/hda.hymeko.
    use parser::parse_description;
    use std::path::PathBuf;
    let mut b = PgraphBuilder::new();
    b.add_material("Toluene", MaterialKind::Raw).unwrap();
    b.add_material("H2", MaterialKind::Raw).unwrap();
    b.add_material("Mix", MaterialKind::Intermediate).unwrap();
    b.add_material("Benzene", MaterialKind::Product).unwrap();
    b.add_material("Methane", MaterialKind::Intermediate)
        .unwrap();
    b.add_unit("Mixer", &["Toluene", "H2"], &["Mix"], 100.0)
        .unwrap();
    b.add_unit("Reactor", &["Mix"], &["Benzene", "Methane"], 250.0)
        .unwrap();
    b.add_unit("DirectSynth", &["Toluene", "H2"], &["Benzene"], 800.0)
        .unwrap();
    b.add_unit("Disposal", &["Methane"], &[], 50.0).unwrap();
    let g_built = b.build().unwrap();

    let hda_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph/hda.hymeko");
    let src = std::fs::read_to_string(&hda_path).unwrap();
    let desc = parse_description(&src).unwrap();
    let g_parsed = hymeko_pgraph::lower(&desc).unwrap();

    // Name-keyed equivalence (DeclIds differ but names match).
    let names_built: BTreeSet<String> = g_built
        .units
        .iter()
        .map(|d| g_built.decl_to_name[d].clone())
        .collect();
    let names_parsed: BTreeSet<String> = g_parsed
        .units
        .iter()
        .map(|d| g_parsed.decl_to_name[d].clone())
        .collect();
    assert_eq!(names_built, names_parsed);

    // Same ABB result.
    let m1 = maximal_structure(&g_built);
    let m2 = maximal_structure(&g_parsed);
    let s1 = solve_with_options(&g_built, &m1, AbbOptions::default()).unwrap();
    let s2 = solve_with_options(&g_parsed, &m2, AbbOptions::default()).unwrap();
    assert!((s1.cost - s2.cost).abs() < 1e-9);
}
