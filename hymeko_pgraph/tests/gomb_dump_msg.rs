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
