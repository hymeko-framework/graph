//! Book-validation regression spine (2026-05-27, Pimentel report).
//!
//! Asserts the canonical maximal structure / solution-structure count /
//! ABB optimum for each worked example in Friedler, Orosz, Pimentel
//! Losada, *P-graphs for Process Systems Engineering* (Springer), against
//! the values stated in the book. Every assertion here would have FAILED
//! against the pre-2026-05-27 implementation, which used a non-canonical
//! strict-no-excess default and a raw-reachability forward pass that
//! dropped structurally-valid cycles.
//!
//! | Example | book maximal | book count / optimum | source |
//! |---------|--------------|----------------------|--------|
//! | 4.1     | 7 {u2,u3,u4,u5,u6,u8,u10} | — | p.41 |
//! | 3.2     | 7            | 19 solution-structures | Fig. 3.4/3.6 |
//! | 3.3     | 29           | 3465 solution-structures | Fig. 4.13 |
//! | 6.1     | 7            | ABB {O2,O5,O7} = 9 | Ch.6 |
//! | 14.1    | 12           | ABB {u1,u4,u8,u11} = 16 | Table 14.1 |

use std::collections::BTreeSet;
use std::path::PathBuf;

use hymeko_pgraph::abb::{AbbOptions, solve_with_options};
use hymeko_pgraph::msg::maximal_structure;
use hymeko_pgraph::{lower, ssg_dm_enumerate};
use parser::parse_description;

fn load(rel: &str) -> Option<hymeko_pgraph::lowering::LoweredPGraph> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph")
        .join(rel);
    let src = std::fs::read_to_string(&path).ok()?;
    Some(lower(&parse_description(&src).expect("parse")).expect("lower"))
}

fn msg_names(p: &hymeko_pgraph::lowering::LoweredPGraph) -> BTreeSet<String> {
    maximal_structure(p)
        .units
        .iter()
        .map(|d| p.decl_to_name[d].clone())
        .collect()
}

fn abb_names_cost(p: &hymeko_pgraph::lowering::LoweredPGraph) -> (BTreeSet<String>, f64) {
    let msg = maximal_structure(p);
    let sol = solve_with_options(p, &msg, AbbOptions::default()).expect("ABB must find an optimum");
    let names: BTreeSet<String> = sol
        .units
        .iter()
        .map(|d| p.decl_to_name[d].clone())
        .collect();
    (names, sol.cost)
}

fn name_set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn example4_1_maximal_structure_is_book_seven_units() {
    let Some(p) = load("Chapter4/example4_1.hymeko") else {
        return;
    };
    // Book p.41: maximal structure = {u2,u3,u4,u5,u6,u8,u10}. The cycle
    // {u3,u6,u10} (u3 produces product B) was wrongly dropped pre-fix.
    assert_eq!(
        msg_names(&p),
        name_set(&["u2", "u3", "u4", "u5", "u6", "u8", "u10"]),
        "Example 4.1 canonical maximal structure must match the book"
    );
}

#[test]
fn example3_2_maximal_seven_and_nineteen_solution_structures() {
    let Some(p) = load("Chapter3/example3_2.hymeko") else {
        return;
    };
    let msg = maximal_structure(&p);
    assert_eq!(
        msg.units.len(),
        7,
        "Example 3.2 maximal structure has 7 units (Fig. 3.6)"
    );
    // Book Fig. 3.4: 19 solution-structures. The decision-mapping SSG
    // (structural, S4-enforcing) reproduces this; the brute SSG over-counts
    // (it does not enforce S4).
    let dm = ssg_dm_enumerate(&p, &msg);
    assert_eq!(
        dm.len(),
        19,
        "Example 3.2 must have 19 solution-structures (book Fig. 3.4)"
    );
}

#[test]
fn example3_3_maximal_29_and_3465_solution_structures() {
    let Some(p) = load("Chapter4/example4_3.hymeko") else {
        return;
    };
    let msg = maximal_structure(&p);
    assert_eq!(
        msg.units.len(),
        29,
        "Example 3.3 maximal structure has 29 units (Fig. 4.13)"
    );
    let dm = ssg_dm_enumerate(&p, &msg);
    assert_eq!(
        dm.len(),
        3465,
        "Example 3.3 must have 3465 solution-structures"
    );
}

#[test]
fn example6_1_abb_optimum_is_nine() {
    let Some(p) = load("Chapter6/example6_1.hymeko") else {
        return;
    };
    assert_eq!(
        maximal_structure(&p).units.len(),
        7,
        "Example 6.1 maximal structure has 7 units"
    );
    let (names, cost) = abb_names_cost(&p);
    assert_eq!(
        names,
        name_set(&["O2", "O5", "O7"]),
        "Example 6.1 ABB optimum structure"
    );
    assert!(
        (cost - 9.0).abs() < 1e-9,
        "Example 6.1 ABB optimum cost is 9, got {cost}"
    );
}

#[test]
fn example14_1_abb_optimum_is_sixteen() {
    let Some(p) = load("book/example14_1.hymeko") else {
        return;
    };
    // All 12 units are backward-reachable from a product, so the canonical
    // maximal structure keeps all of them (incl. the non-bootstrappable
    // cycle units the old raw-reachability forward pass wrongly dropped to 5).
    assert_eq!(
        maximal_structure(&p).units.len(),
        12,
        "Example 14.1 maximal structure has 12 units"
    );
    let (names, cost) = abb_names_cost(&p);
    assert_eq!(
        names,
        name_set(&["u1", "u4", "u8", "u11"]),
        "Example 14.1 ABB optimum structure"
    );
    assert!(
        (cost - 16.0).abs() < 1e-9,
        "Example 14.1 ABB optimum cost is 16, got {cost}"
    );
}
