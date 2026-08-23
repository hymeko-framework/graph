//! Integration tests for the meta-model → P-graph adapter
//! ([`hymeko_pgraph::compile_to_lowered`]). These compile real `.hymeko`
//! files through `hymeko_core` (resolving `@"includes"` + `using` aliases) and
//! assert the `<isa>` classification, the hybrid unit rule, and that the
//! resulting graph drives MSG correctly.

use std::collections::BTreeSet;
use std::path::PathBuf;

use hymeko_pgraph::abb::{AbbOptions, solve_with_options};
use hymeko_pgraph::msg::maximal_structure;
use hymeko_pgraph::{MetaResolveError, compile_sources, compile_to_lowered};

fn pgraph_data(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Names of a decl set, for order-independent assertions.
fn names(
    g: &hymeko_pgraph::LoweredPGraph,
    set: &BTreeSet<hymeko::common::ids::DeclId>,
) -> BTreeSet<String> {
    set.iter().map(|d| g.decl_to_name[d].clone()).collect()
}

#[test]
fn example_3_1_classifies_via_isa_and_hybrid_units() {
    let g = compile_to_lowered(&pgraph_data("data/prgraph_ex_3_1.hymeko"))
        .expect("example 3.1 compiles");

    // M, R, P recovered from <isa> ancestry.
    assert_eq!(
        names(&g, &g.materials),
        ["A", "B", "C", "D", "E", "F", "G"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    assert_eq!(names(&g, &g.raws), BTreeSet::from(["A".into(), "B".into()]));
    assert_eq!(names(&g, &g.products), BTreeSet::from(["G".into()]));

    // All five @-edges are units via the *structural* branch of the hybrid
    // rule (they carry no explicit <isa> process, but their arcs are all
    // material-targeted).
    assert_eq!(
        names(&g, &g.units),
        ["u1", "u2", "u3", "u4", "u5"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );

    // Signed incidence, by name (queries from the schema refactor).
    let io = |u: &str| -> (BTreeSet<String>, BTreeSet<String>) {
        let d = g.name_to_decl[u];
        (
            g.inputs(d)
                .iter()
                .map(|m| g.decl_to_name[m].clone())
                .collect(),
            g.outputs(d)
                .iter()
                .map(|m| g.decl_to_name[m].clone())
                .collect(),
        )
    };
    assert_eq!(io("u1"), (set(["B"]), set(["D"])));
    assert_eq!(io("u2"), (set(["F"]), set(["D", "E"])));
    assert_eq!(io("u3"), (set(["E"]), set(["G"])));
    assert_eq!(io("u4"), (set(["D"]), set(["C", "G"])));
    assert_eq!(io("u5"), (set(["A", "C"]), set(["G"])));

    // No explicit costs in the source ⇒ default 1.0.
    for u in &g.units {
        assert!((g.costs[u] - 1.0).abs() < 1e-9);
    }
}

#[test]
fn msg_prunes_the_unproduced_f_branch() {
    let g = compile_to_lowered(&pgraph_data("data/prgraph_ex_3_1.hymeko")).unwrap();
    // F is declared intermediate but produced by no unit, so u2 (consumes F)
    // and u3 (consumes E, only produced by u2) are structurally infeasible.
    let m = maximal_structure(&g);
    assert_eq!(
        names(&g, &m.units),
        ["u1", "u4", "u5"].iter().map(|s| s.to_string()).collect()
    );
    // The surviving structure still reaches the product G via ABB.
    let sol = solve_with_options(&g, &m, AbbOptions::default()).unwrap();
    assert!(sol.cost > 0.0 && sol.cost.is_finite());
    assert!(g.products.iter().all(|p| g.materials.contains(p)));
}

#[test]
fn compile_sources_matches_path_based_compile() {
    // In-memory (filesystem-free) compile — the WASM entry — must agree with
    // the path-based compile on the same content.
    let instance = std::fs::read_to_string(pgraph_data("data/prgraph_ex_3_1.hymeko")).unwrap();
    let meta = std::fs::read_to_string(pgraph_data("data/meta_pgraph.hymeko")).unwrap();
    let g = compile_sources(
        "prgraph_ex_3_1.hymeko",
        &[
            ("prgraph_ex_3_1.hymeko", instance.as_str()),
            ("meta_pgraph.hymeko", meta.as_str()),
        ],
    )
    .expect("in-memory compile");
    assert_eq!(g.materials.len(), 7);
    assert_eq!(g.units.len(), 5);
    assert_eq!(g.raws.len(), 2);
    assert_eq!(g.products.len(), 1);
}

#[test]
fn meta_alone_yields_empty_pgraph() {
    // The meta-model declares the archetypes but no instances <isa> them.
    let g = compile_to_lowered(&pgraph_data("data/meta_pgraph.hymeko")).unwrap();
    assert!(g.materials.is_empty());
    assert!(g.units.is_empty());
}

#[test]
fn non_meta_file_errors_with_missing_archetype() {
    // hda.hymeko uses the literal-tag idiom (no pgraph meta archetypes), so the
    // meta-model adapter must reject it loudly rather than mis-lower.
    let hda = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph/hda.hymeko");
    match compile_to_lowered(&hda) {
        Err(MetaResolveError::MissingArchetype(_)) => {}
        other => panic!("expected MissingArchetype, got {other:?}"),
    }
}

fn set<const N: usize>(xs: [&str; N]) -> BTreeSet<String> {
    xs.iter().map(|s| s.to_string()).collect()
}
