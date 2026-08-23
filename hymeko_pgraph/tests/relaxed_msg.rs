//! Tests for the relaxed-MSG option (Stage 2026-05-19, post-P-mo).
//!
//! When `MaximalStructureOptions::strict_no_excess = false`, MSG's
//! backward pass keeps a unit if *at least one* output is useful
//! (product or consumed by some surviving unit). The strict default
//! requires *every* output to be useful — the Friedler 1992 form.
//!
//! The textbook P-graph Studio chapter examples
//! (`data/pgraph/Chapter*/example*.pgip`) use the relaxed semantics
//! by default. Chapter4/example4_3 is the canonical case where
//! strict-MSG cascade-drops every unit while relaxed-MSG keeps the
//! 29-unit maximal structure.

use std::collections::BTreeSet;
use std::path::PathBuf;

use hymeko_pgraph::abb::{AbbOptions, solve_with_options};
use hymeko_pgraph::lower;
use hymeko_pgraph::msg::{
    MaximalStructureOptions, maximal_structure, maximal_structure_with_options,
};
use parser::parse_description;

fn fixture(name: &str) -> String {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let p = PathBuf::from(manifest)
        .parent()
        .unwrap()
        .join("data/pgraph")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"))
}

#[test]
fn strict_msg_default_byte_identical() {
    // Pre-2026-05-19 callers used `maximal_structure(p)`. That entry
    // point must return the same result as
    // `maximal_structure_with_options(p, default)`.
    let src = fixture("hda.hymeko");
    let desc = parse_description(&src).unwrap();
    let p = lower(&desc).unwrap();
    let strict_default = maximal_structure(&p);
    let strict_explicit = maximal_structure_with_options(&p, MaximalStructureOptions::default());
    assert_eq!(strict_default.units, strict_explicit.units);
    assert_eq!(strict_default.materials, strict_explicit.materials);
}

#[test]
fn relaxed_keeps_more_or_equal_units() {
    // Mathematical invariant: the relaxed criterion is a *weakening*
    // of strict, so for any P-graph the relaxed MSG superset-equals
    // the strict MSG.
    let src = fixture("hda.hymeko");
    let desc = parse_description(&src).unwrap();
    let p = lower(&desc).unwrap();
    let strict = maximal_structure_with_options(
        &p,
        MaximalStructureOptions {
            strict_no_excess: true,
        },
    );
    let relaxed = maximal_structure_with_options(
        &p,
        MaximalStructureOptions {
            strict_no_excess: false,
        },
    );
    let s: BTreeSet<_> = strict.units.iter().collect();
    let r: BTreeSet<_> = relaxed.units.iter().collect();
    assert!(
        s.is_subset(&r),
        "strict MSG must be a subset of relaxed MSG; strict={:?} relaxed={:?}",
        strict.units,
        relaxed.units
    );
}

#[test]
fn chapter4_3_canonical_keeps_29_units() {
    // Book Example 3.3 (35 declared units; Fig. 4.13 shows a
    // non-degenerate maximal structure). The canonical MSG keeps 29 of
    // the 35 (the other 6 are forward-infeasible). Pre-2026-05-27 the
    // strict default wrongly collapsed this to 0 units.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph/Chapter4/example4_3.hymeko");
    if !path.exists() {
        return;
    }
    let src = std::fs::read_to_string(&path).unwrap();
    let desc = parse_description(&src).unwrap();
    let p = lower(&desc).unwrap();

    let canonical = maximal_structure_with_options(&p, MaximalStructureOptions::default());
    assert_eq!(
        canonical.units.len(),
        29,
        "canonical MSG must keep 29 units on Example 3.3; got {}",
        canonical.units.len()
    );
    // The explicit relaxed flag is the same as the default (canonical).
    let relaxed = maximal_structure_with_options(
        &p,
        MaximalStructureOptions {
            strict_no_excess: false,
        },
    );
    assert_eq!(relaxed.units.len(), 29);
}

#[test]
fn chapter6_canonical_abb_optimum_is_9() {
    // Example 6.1 (the costed twin of Example 3.2, same 7-unit maximal
    // structure). The canonical ABB optimum is {O2, O5, O7} at cost 9.0.
    // The pre-2026-05-27 buggy default returned {O1, O3, O6} at 18.0 — a
    // suboptimal answer forced by a 3-unit (too-small) maximal structure.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph/Chapter6/example6_1.hymeko");
    if !path.exists() {
        return;
    }
    let src = std::fs::read_to_string(&path).unwrap();
    let desc = parse_description(&src).unwrap();
    let p = lower(&desc).unwrap();
    let m = maximal_structure(&p); // canonical default
    assert_eq!(
        m.units.len(),
        7,
        "canonical MSG keeps all 7 units (book Fig. 3.6)"
    );
    let abb = solve_with_options(&p, &m, AbbOptions::default())
        .expect("canonical ABB must find a Chapter6 optimum");
    let names: BTreeSet<_> = abb
        .units
        .iter()
        .map(|d| p.decl_to_name[d].clone())
        .collect();
    let expected: BTreeSet<_> = ["O2", "O5", "O7"].iter().map(|s| s.to_string()).collect();
    assert_eq!(
        names, expected,
        "Chapter6 canonical ABB units must be {{O2,O5,O7}}"
    );
    assert!(
        (abb.cost - 9.0).abs() < 1e-9,
        "Chapter6 ABB cost must be 9.0, got {}",
        abb.cost
    );
}
