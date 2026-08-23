//! Integration tests for the signed-incidence process-info queries that
//! replaced the `unit_inputs` / `unit_outputs` side tables
//! (2026-05-25 hypergraph-queries refactor). The per-unit input/output
//! sets are now *derived* from the directed edge set via
//! [`LoweredPGraph::inputs`] / [`LoweredPGraph::outputs`], so these
//! tests pin that the derivation matches the source incidence.

use std::collections::BTreeSet;
use std::path::PathBuf;

use hymeko_pgraph::abb::{solve_with_options, AbbOptions};
use hymeko_pgraph::builder::{MaterialKind, PgraphBuilder};
use hymeko_pgraph::lower;
use hymeko_pgraph::msg::maximal_structure;
use parser::parse_description;

/// `inputs(u)` / `outputs(u)` recover the declared signature, by name.
#[test]
fn inputs_outputs_recover_declared_signature() {
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
    let g = b.build().unwrap();

    let by_name = |n: &str| g.name_to_decl[n];
    let names = |s: &BTreeSet<_>| -> BTreeSet<String> {
        s.iter().map(|d| g.decl_to_name[d].clone()).collect()
    };

    assert_eq!(
        names(g.inputs(by_name("Mixer"))),
        BTreeSet::from(["Toluene".to_string(), "H2".to_string()])
    );
    assert_eq!(
        names(g.outputs(by_name("Mixer"))),
        BTreeSet::from(["Mix".to_string()])
    );
    assert_eq!(
        names(g.outputs(by_name("Reactor"))),
        BTreeSet::from(["Benzene".to_string(), "Methane".to_string()])
    );
    // Material-side dual: Mix is produced by Mixer, consumed by Reactor.
    assert_eq!(
        names(g.schema.successors(by_name("Mix"))),
        BTreeSet::from(["Reactor".to_string()])
    );
}

/// A disposal-sink unit (no products) has an empty `outputs` set — this
/// locks the empty-vs-absent contract the old `or_default()` lookup
/// guaranteed and the MSG relaxed branch relies on.
#[test]
fn disposal_sink_has_empty_outputs() {
    let mut b = PgraphBuilder::new();
    b.add_material("Toluene", MaterialKind::Raw).unwrap();
    b.add_material("Benzene", MaterialKind::Product).unwrap();
    b.add_material("Methane", MaterialKind::Intermediate)
        .unwrap();
    b.add_unit("Reactor", &["Toluene"], &["Benzene", "Methane"], 250.0)
        .unwrap();
    b.add_unit("Disposal", &["Methane"], &[], 50.0).unwrap();
    let g = b.build().unwrap();

    let disposal = g.name_to_decl["Disposal"];
    assert!(g.outputs(disposal).is_empty());
    assert_eq!(g.inputs(disposal).len(), 1);
    assert!(g.inputs(disposal).contains(&g.name_to_decl["Methane"]));
}

/// End-to-end: parsing + lowering the HDA `.hymeko` fixture still drives
/// MSG + ABB to a solution, now reading I/O purely through the queries.
#[test]
fn hda_lowers_and_solves_via_queries() {
    let hda_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("data/pgraph/hda.hymeko");
    let src = std::fs::read_to_string(&hda_path).unwrap();
    let desc = parse_description(&src).unwrap();
    let g = lower(&desc).unwrap();

    // Every unit's inputs/outputs are bipartite-consistent (materials).
    for u in &g.units {
        for m in g.inputs(*u).iter().chain(g.outputs(*u).iter()) {
            assert!(g.materials.contains(m), "I/O endpoint must be a material");
        }
    }

    let m = maximal_structure(&g);
    let sol = solve_with_options(&g, &m, AbbOptions::default()).unwrap();
    assert!(sol.cost > 0.0);
}
