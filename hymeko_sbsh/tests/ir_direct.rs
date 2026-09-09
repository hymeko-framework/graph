//! The fully IR-direct path: a model that declares its own poses needs no external side table.
//!
//! This is the case the crate should make easy, because the IR already holds the coordinates —
//! a numeric list attribute compiles to a child declaration whose `anno.value` is `ValueR::List`.

use std::path::Path;

use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::direct_ir::PositionProvider;
use hymeko_sbsh::{IrPositions, OrthConfig, SbshScene};

const SRC: &str = r#"
vv {}
g { a { position [0.0, 0.0]; }
    b { position [1.0, 0.0]; }
    c { position [2.0, 0.0]; }
    d { position [8.0, 8.0]; }
    e { position [9.0, 8.0]; }
    f { }
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
    @e3 { (+ d, + e, ~ f); }
}
"#;

#[test]
fn a_scene_builds_from_the_model_alone_with_no_external_position_table() {
    let provider =
        hymeko::module_store::source_provider::MemProvider::default().with_file("/m.hymeko", SRC);
    let mut store = hymeko::module_store::module_store::ModuleStore::new(
        provider,
        hymeko::util::real_parser::RealParser,
    );
    let prog = store.compile(Path::new("/m.hymeko")).expect("compiles");
    let ir = &prog.ir;

    // The ONLY thing the caller supplies is which attribute carries the coordinate — vocabulary
    // that belongs to the model, not to the graph layer.
    let attr = store
        .it
        .get_id("position")
        .expect("the fixture declares `position`");
    let pos = IrPositions::from_attribute(ir, attr);

    assert_eq!(
        pos.len(),
        5,
        "five declarations carry a position; `f` does not"
    );

    let index = build_incidence_index(ir).expect("index builds");
    let scene = SbshScene::build(
        &index,
        &pos,
        2,
        &OrthConfig {
            max_depth: 6,
            drop_empty: true,
        },
    )
    .expect("scene builds from the IR alone");

    assert_eq!(scene.dim(), 2);
    assert_eq!(scene.decls().len(), 5);
    scene.tree().verify_laminar().expect("laminar");

    // The coordinates are the ones the model declared, read straight out of the IR.
    let a = (0..ir.decl_nodes.len())
        .map(hymeko::common::ids::DeclId::new)
        .find(|&d| store.it.resolve(ir.decl_nodes[d.raw()].name) == "a")
        .expect("decl a");
    assert_eq!(scene.point_of(a), Some([0.0, 0.0].as_slice()));

    // `f` declares no position, so it is outside the partition — and outside the primal graph.
    assert_eq!(pos.position(a).map(|p| p.len()), Some(2));
    let adj = scene.primal_adjacency(&index);
    let half: usize = adj.iter().map(|n| n.len()).sum();
    // e1, e2 are binary; e3 has three members but only d and e are positioned, so it contributes
    // a single edge rather than a triangle.
    assert_eq!(
        half, 6,
        "unpositioned members drop out of the primal graph, got {adj:?}"
    );
    assert_eq!(
        scene.width(&index),
        1,
        "two disjoint paths certify at width 1"
    );
}

#[test]
fn a_declaration_without_the_attribute_is_simply_absent() {
    let provider =
        hymeko::module_store::source_provider::MemProvider::default().with_file("/m.hymeko", SRC);
    let mut store = hymeko::module_store::module_store::ModuleStore::new(
        provider,
        hymeko::util::real_parser::RealParser,
    );
    let prog = store.compile(Path::new("/m.hymeko")).expect("compiles");
    let attr = store.it.get_id("position").unwrap();
    let pos = IrPositions::from_attribute(&prog.ir, attr);
    let names: Vec<&str> = pos
        .decls()
        .map(|d| store.it.resolve(prog.ir.decl_nodes[d.raw()].name))
        .collect();
    assert_eq!(
        names,
        vec!["a", "b", "c", "d", "e"],
        "`f` carries no position and is absent"
    );
}

#[test]
fn an_attribute_that_is_not_numeric_yields_no_position_rather_than_a_partial_one() {
    let src = r#"
vv {}
g { a { position [1.0, 2.0]; }
    b { position "not a vector"; }
    @e1 { (+ a, + b); }
}
"#;
    let provider =
        hymeko::module_store::source_provider::MemProvider::default().with_file("/m.hymeko", src);
    let mut store = hymeko::module_store::module_store::ModuleStore::new(
        provider,
        hymeko::util::real_parser::RealParser,
    );
    let prog = store.compile(Path::new("/m.hymeko")).expect("compiles");
    let attr = store.it.get_id("position").unwrap();
    let pos = IrPositions::from_attribute(&prog.ir, attr);
    assert_eq!(
        pos.len(),
        1,
        "a string attribute is not a coordinate; only `a` is positioned"
    );
}
