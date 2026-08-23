//! Integration tests for the decision-mapping SSG (`ssg_dm`).
//!
//! Two literature-anchored parity points from Friedler/Orosz/Pimentel,
//! *P-graphs for Process Systems Engineering*:
//!
//! - **Example 3.3** (`data/pgraph/Chapter4/example4_3`, 35 units): the
//!   book reports **3465 solution-structures**. The brute `ssg`
//!   ($2^{29}$ subsets on the relaxed MSG) cannot reach this; the
//!   decision-mapping SSG must reproduce it exactly.
//! - **Example 14.1** (`data/pgraph/book/example14_1`, 12 units): the
//!   book's ABB optimum is `{u1,u4,u8,u11}` at weight 16.
//!
//! Plus a soundness cross-check against the brute `ssg::is_feasible`
//! (every decision-mapping structure is forward-feasible) and a
//! contract-preservation guard (the brute `n>30` refusal is unchanged).

use std::collections::BTreeSet;
use std::path::PathBuf;

use hymeko::common::ids::DeclId;
use hymeko_pgraph::abb::{AbbOptions, solve_with_options};
use hymeko_pgraph::lower;
use hymeko_pgraph::msg::{MaximalStructureOptions, maximal_structure_with_options};
use hymeko_pgraph::ssg::{self, SolutionStructure, SsgOptions};
use hymeko_pgraph::ssg_dm;
use parser::parse_description;

const HDA_SRC: &str = include_str!("../../data/pgraph/hda.hymeko");

fn data_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph")
        .join(rel)
}

fn relaxed_msg_and_lowered(
    src: &str,
) -> (
    hymeko_pgraph::LoweredPGraph,
    hymeko_pgraph::MaximalStructure,
) {
    let p = lower(&parse_description(src).expect("parse")).expect("lower");
    let msg = maximal_structure_with_options(
        &p,
        MaximalStructureOptions {
            strict_no_excess: false,
        },
    );
    (p, msg)
}

fn unit_names(p: &hymeko_pgraph::LoweredPGraph, s: &BTreeSet<DeclId>) -> BTreeSet<String> {
    s.iter().map(|d| p.decl_to_name[d].clone()).collect()
}

fn no_duplicates(structs: &[SolutionStructure]) -> bool {
    let set: BTreeSet<&BTreeSet<DeclId>> = structs.iter().map(|s| &s.units).collect();
    set.len() == structs.len()
}

#[test]
fn hda_decision_mapping_structures() {
    // Hand-derived: Benzene is produced by {Reactor, DirectSynth}.
    //   {DirectSynth}                  — direct route
    //   {Reactor, Mixer}               — two-stage (Methane vented; S4 holds
    //                                    for both units, disposal NOT pulled in)
    //   {Reactor, DirectSynth, Mixer}  — both routes together
    // Note this differs from brute strict SSG, which force-includes the
    // Disposal sink — a unit that reaches no product (violates S4) and is
    // therefore not a true solution-structure.
    let (p, msg) = relaxed_msg_and_lowered(HDA_SRC);
    let s = ssg_dm::enumerate(&p, &msg);
    assert!(
        no_duplicates(&s),
        "each solution-structure must appear once"
    );
    assert_eq!(s.len(), 3, "HDA has 3 decision-mapping solution-structures");

    let names: BTreeSet<BTreeSet<String>> = s.iter().map(|ss| unit_names(&p, &ss.units)).collect();
    let want: BTreeSet<BTreeSet<String>> = [
        vec!["DirectSynth"],
        vec!["Reactor", "Mixer"],
        vec!["Reactor", "DirectSynth", "Mixer"],
    ]
    .into_iter()
    .map(|v| v.into_iter().map(String::from).collect())
    .collect();
    assert_eq!(names, want);
}

#[test]
fn decision_mapping_structures_are_brute_feasible() {
    // Soundness: every structure the decision-mapping SSG emits is
    // forward-feasible under the brute relaxed `is_feasible`.
    let (p, msg) = relaxed_msg_and_lowered(HDA_SRC);
    let relaxed = SsgOptions {
        strict_no_excess: false,
        require_at_least_one_unit: true,
    };
    for s in ssg_dm::enumerate(&p, &msg) {
        assert!(
            ssg::is_feasible(&p, &s.units, relaxed),
            "dm-structure {:?} must be brute-feasible (relaxed)",
            unit_names(&p, &s.units)
        );
    }
}

#[test]
fn example3_3_reproduces_3465_solution_structures() {
    let path = data_path("Chapter4/example4_3.hymeko");
    if !path.exists() {
        return; // converted fixture absent in this checkout — skip.
    }
    let src = std::fs::read_to_string(&path).unwrap();
    let (p, msg) = relaxed_msg_and_lowered(&src);
    // Relaxed MSG keeps 29 of the 35 declared units (see relaxed_msg.rs).
    assert_eq!(msg.units.len(), 29, "relaxed MSG units on Example 3.3");

    let structures = ssg_dm::enumerate(&p, &msg);
    assert!(no_duplicates(&structures), "exactly-once generation");
    assert_eq!(
        structures.len(),
        3465,
        "decision-mapping SSG must reproduce the book's 3465 solution-structures"
    );
}

#[test]
fn example14_1_abb_matches_book_optimum() {
    let path = data_path("book/example14_1.hymeko");
    let src = std::fs::read_to_string(&path).expect("Example 14.1 fixture must exist");
    let (p, msg) = relaxed_msg_and_lowered(&src);

    // Relaxed no-excess throughout (the book's / P-graph Studio's regime):
    // the relaxed MSG admits excess byproducts (e.g. A12), so ABB must use
    // the matching leaf criterion or it would reject every structure.
    let opts = AbbOptions {
        strict_no_excess: false,
        ..AbbOptions::default()
    };
    let sol = solve_with_options(&p, &msg, opts).expect("ABB must find an optimum");
    let names = unit_names(&p, &sol.units);
    let want: BTreeSet<String> = ["u1", "u4", "u8", "u11"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(names, want, "book optimum is {{u1,u4,u8,u11}}");
    assert!(
        (sol.cost - 16.0).abs() < 1e-9,
        "book optimum weight is 16, got {}",
        sol.cost
    );
}

#[test]
fn brute_ssg_still_refuses_above_30_units() {
    // Contract preservation: the existing brute `ssg` guard is unchanged.
    // A 31-unit maximal structure must still yield the empty refusal,
    // independent of the new decision-mapping path.
    let mut src =
        String::from("Big{}\ncontext {\n  P <material, product>;\n  R <material, raw>;\n");
    for i in 0..31 {
        src.push_str(&format!("  @u{i} <unit> 1 {{ (-R, +P); }}\n"));
    }
    src.push_str("}\n");
    let (p, msg) = relaxed_msg_and_lowered(&src);
    assert!(msg.units.len() > 30, "fixture must exceed the guard");
    assert!(
        ssg::enumerate(&p, &msg).is_empty(),
        "brute ssg must still refuse >30-unit structures"
    );
}
