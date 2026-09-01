//! Metamorphic and generated-family V&V for `direct_ir`, all source-first through the real parser.
//!
//! Metamorphic relations (the result is invariant under a transformation that preserves the
//! hypergraph up to naming): declaration-order permutation, disjoint union, duplicate incidence.
//! Generated families: sunflower (shared core), hub-skew, a multi-component grid. Each cross-checks
//! against the independent `O(V·E)` oracle or a hand-reasoned expectation.

mod direct_ir_common;

use std::collections::BTreeSet;
use std::fmt::Write as _;

use hymeko::ir::incidence::build_incidence_index;

use hymeko_graph::direct_ir::{
    HyperTraversal, TraversalWorkspace, connected_components, s_connected_components,
};

use direct_ir_common::{World, compile, oracle_reachable};

fn reach_names(w: &World, start: &str) -> BTreeSet<usize> {
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();
    trav.reachable_nodes(&mut ws, w.node(start))
        .expect("reachable")
        .nodes()
        .map(|n| n.raw())
        .collect()
}

// ---- metamorphic: declaration-order permutation --------------------------------------------

#[test]
fn metamorphic_declaration_order_permutation() {
    let src1 = r#"
vv {}
g { a {} b {} c {}
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
}
"#;
    // same hypergraph, declarations and edges reordered (DeclIds will differ)
    let src2 = r#"
vv {}
g {
    @e2 { (+ b, + c); }
    c {} b {} a {}
    @e1 { (+ a, + b); }
}
"#;
    let w1 = compile(src1);
    let w2 = compile(src2);
    // reachability by NAME is invariant even though the underlying DeclIds are permuted
    let names1: BTreeSet<&str> = ["a", "b", "c"]
        .into_iter()
        .filter(|n| reach_names(&w1, "a").contains(&w1.node(n).raw()))
        .collect();
    let names2: BTreeSet<&str> = ["a", "b", "c"]
        .into_iter()
        .filter(|n| reach_names(&w2, "a").contains(&w2.node(n).raw()))
        .collect();
    assert_eq!(names1, names2);
    assert_eq!(names1, BTreeSet::from(["a", "b", "c"]));

    // component count is invariant to declaration order
    let cc1 = {
        let ir = w1.ir();
        let idx = build_incidence_index(ir).unwrap();
        let mut ws = TraversalWorkspace::new();
        connected_components(&idx, &mut ws).unwrap().count()
    };
    let cc2 = {
        let ir = w2.ir();
        let idx = build_incidence_index(ir).unwrap();
        let mut ws = TraversalWorkspace::new();
        connected_components(&idx, &mut ws).unwrap().count()
    };
    assert_eq!(cc1, cc2);
}

// ---- metamorphic: disjoint union ------------------------------------------------------------

#[test]
fn metamorphic_disjoint_union_preserves_within_and_adds_components() {
    let a_only = r#"
vv {}
g { a {} b {} @e1 { (+ a, + b); } }
"#;
    let both = r#"
vv {}
g {
    a {} b {} p {} q {} r {}
    @e1 { (+ a, + b); }
    @f1 { (+ p, + q); }
    @f2 { (+ q, + r); }
}
"#;
    let wa = compile(a_only);
    let wb = compile(both);

    // reachability within the A component is unchanged by adding a disjoint B component
    let a_reach_alone = reach_names(&wa, "a").len();
    let a_reach_union = reach_names(&wb, "a").len();
    assert_eq!(
        a_reach_alone, a_reach_union,
        "disjoint B does not change A's reachability"
    );

    // p's component in the union is exactly {p,q,r}
    let idx = build_incidence_index(wb.ir()).unwrap();
    let mut ws = TraversalWorkspace::new();
    let cc = connected_components(&idx, &mut ws).unwrap();
    assert_eq!(cc.same_component(wb.node("a"), wb.node("p")), Some(false));
    assert_eq!(cc.same_component(wb.node("p"), wb.node("r")), Some(true));
}

// ---- generated families --------------------------------------------------------------------

/// A sunflower: `petals` hyperedges each over the shared `core` nodes plus one private petal node.
fn sunflower_src(core: usize, petals: usize) -> String {
    let mut s = String::from("vv {}\ng {\n");
    for c in 0..core {
        let _ = writeln!(s, "    c{c} {{}}");
    }
    for p in 0..petals {
        let _ = writeln!(s, "    p{p} {{}}");
    }
    for p in 0..petals {
        let members: String = (0..core)
            .map(|c| format!("+ c{c}"))
            .chain(std::iter::once(format!("+ p{p}")))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(s, "    @e{p} {{ ({members}); }}");
    }
    s.push_str("}\n");
    s
}

#[test]
fn sunflower_is_connected_and_s_core_binds_all_petals() {
    let w = compile(&sunflower_src(2, 5));
    // every petal reachable from any core node
    let got = reach_names(&w, "c0");
    let want = oracle_reachable(w.ir(), w.node("c0"));
    assert_eq!(got, want);
    for p in 0..5 {
        assert!(got.contains(&w.node(&format!("p{p}")).raw()));
    }
    // with a 2-node core, every pair of petal edges shares both core nodes => one s=2 component
    let idx = build_incidence_index(w.ir()).unwrap();
    let sc = s_connected_components(&idx, 2);
    assert_eq!(sc.count(), 1, "shared 2-core binds all petals at s=2");
}

/// A hub node shared by `spokes` two-member arcs.
fn hub_src(spokes: usize) -> String {
    let mut s = String::from("vv {}\ng {\n    hub {}\n");
    for i in 0..spokes {
        let _ = writeln!(s, "    n{i} {{}}");
    }
    for i in 0..spokes {
        let _ = writeln!(s, "    @e{i} {{ (+ hub, + n{i}); }}");
    }
    s.push_str("}\n");
    s
}

#[test]
fn hub_reaches_all_spokes_and_degree_is_observable() {
    let spokes = 32;
    let w = compile(&hub_src(spokes));
    let got = reach_names(&w, "n0");
    let want = oracle_reachable(w.ir(), w.node("n0"));
    assert_eq!(got, want);
    for i in 0..spokes {
        assert!(got.contains(&w.node(&format!("n{i}")).raw()));
    }
    // the hub's incidence multiplicity is observable through the index degree
    let idx = build_incidence_index(w.ir()).unwrap();
    let hub_decl = w.ir().nodes[w.node("hub").raw()].decl;
    assert_eq!(
        idx.degree(hub_decl),
        spokes,
        "hub is incident to every spoke arc"
    );
}

#[test]
fn many_small_components_grid() {
    // k independent 2-node edges -> k components, none reaching another
    let mut s = String::from("vv {}\ng {\n");
    let k = 12;
    for i in 0..k {
        let _ = writeln!(s, "    a{i} {{}} b{i} {{}}");
    }
    for i in 0..k {
        let _ = writeln!(s, "    @e{i} {{ (+ a{i}, + b{i}); }}");
    }
    s.push_str("}\n");
    let w = compile(&s);
    let idx = build_incidence_index(w.ir()).unwrap();
    let mut ws = TraversalWorkspace::new();
    let cc = connected_components(&idx, &mut ws).unwrap();
    assert!(cc.count() >= k as u32, "at least {k} components");
    assert_eq!(cc.same_component(w.node("a0"), w.node("a1")), Some(false));
    assert_eq!(cc.same_component(w.node("a3"), w.node("b3")), Some(true));
}
