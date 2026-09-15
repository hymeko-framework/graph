//! Source-first V&V for `hymeko_graph::direct_ir`.
//!
//! Every fixture goes the whole way: `*.hymeko` text -> `RealParser` -> `ModuleStore::compile`
//! -> canonical `Ir` -> `IncidenceIndex` -> a direct-Ir algorithm -> canonical-ID result. An
//! independent `O(V·E)` fixpoint oracle (which does **not** use `IncidenceIndex`) checks
//! reachability; Berge paths and components are checked against hand-reasoned expectations.

mod direct_ir_common;

use std::collections::BTreeSet;

use hymeko::common::ids::{HyperArcId, NodeId};
use hymeko::ir::incidence::build_incidence_index;

use hymeko_graph::direct_ir::DirectIrError;
use hymeko_graph::direct_ir::{
    HyperTraversal, TraversalWorkspace, connected_components, dual_reachability,
};

use direct_ir_common::{World, compile, oracle_reachable};

fn reachable_set(w: &World, start: NodeId) -> BTreeSet<usize> {
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index builds");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();
    let set = trav.reachable_nodes(&mut ws, start).expect("reachable");
    set.nodes().map(|n| n.raw()).collect()
}

// ----------------------------------------------------------------------------------------------
// golden fixtures
// ----------------------------------------------------------------------------------------------

const MULTIWAY: &str = r#"
vv {}
g {
    n0 {} n1 {} n2 {} n3 {} n4 {}
    @e0 { (+ n0, - n1, ~ n2); }
    @e1 { (+ n2, + n3); }
    lonely {}
}
"#;

#[test]
fn reachable_matches_independent_oracle_multiway() {
    let w = compile(MULTIWAY);
    let start = w.node("n0");
    let got = reachable_set(&w, start);
    let want = oracle_reachable(w.ir(), start);
    assert_eq!(
        got, want,
        "direct reachable set must equal the O(V·E) oracle"
    );
    // n0..n3 reachable through the two hyperedges; n4/lonely are not.
    assert!(got.contains(&w.node("n3").raw()));
    assert!(!got.contains(&w.node("n4").raw()));
    assert!(!got.contains(&w.node("lonely").raw()));
}

#[test]
fn signs_do_not_gate_reachability() {
    // Only `-` and `~` incidences on the path from a to c; both must still connect.
    let src = r#"
vv {}
g {
    a {} b {} c {}
    @e0 { (- a, ~ b); }
    @e1 { (~ b, - c); }
}
"#;
    let w = compile(src);
    let got = reachable_set(&w, w.node("a"));
    assert!(
        got.contains(&w.node("c").raw()),
        "sign must not gate reachability"
    );
    assert_eq!(got, oracle_reachable(w.ir(), w.node("a")));
}

const CHAIN: &str = r#"
vv {}
g {
    a {} b {} c {} d {} x {}
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
    @e3 { (+ c, + d); }
}
"#;

#[test]
fn berge_shortest_path_length_nodes_and_lossless_steps() {
    let w = compile(CHAIN);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();

    let path = trav
        .berge_shortest_path(&mut ws, w.node("a"), w.node("d"))
        .expect("ok")
        .expect("a reaches d");
    assert_eq!(path.hyperedge_len(), 3, "a->d is three hyperedges");
    let node_ids: Vec<usize> = path.nodes().map(|n| n.raw()).collect();
    assert_eq!(
        node_ids,
        vec![
            w.node("a").raw(),
            w.node("b").raw(),
            w.node("c").raw(),
            w.node("d").raw()
        ]
    );
    // Each END of each step resolves back to a full incidence via (arc, slot): nothing is dropped,
    // and now that includes the entry coordinate.
    for step in &path.steps {
        for end in [step.from, step.to] {
            let iref = index.resolve(step.arc, end.slot);
            assert_eq!(iref.sign, end.sign, "step sign matches the incidence");
            assert_eq!(
                iref.target,
                ir.nodes[end.node.raw()].decl,
                "step node matches target"
            );
        }
    }
}

#[test]
fn berge_shortest_path_trivial_and_unreachable() {
    let w = compile(CHAIN);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();

    let trivial = trav
        .berge_shortest_path(&mut ws, w.node("a"), w.node("a"))
        .expect("ok")
        .expect("a reaches itself");
    assert_eq!(trivial.hyperedge_len(), 0);

    let none = trav
        .berge_shortest_path(&mut ws, w.node("a"), w.node("x"))
        .expect("ok");
    assert!(none.is_none(), "x is isolated -> unreachable");
}

#[test]
fn weighted_incidence_survives_into_the_step() {
    let src = r#"
vv {}
g {
    a {} b {}
    @e0 { (+ a [0.87], + b); }
}
"#;
    let w = compile(src);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();
    let path = trav
        .berge_shortest_path(&mut ws, w.node("a"), w.node("b"))
        .expect("ok")
        .expect("a reaches b");
    let step = path.steps.last().expect("one step");
    let iref = index.resolve(step.arc, step.to.slot);
    // b is the reached node; a carries the weight -- the point is that the step's (arc,slot)
    // coordinate resolves to a live incidence whose weights are reachable (nothing was flattened
    // away by traversal). Verify the weighted incidence is visible through the same index.
    let a_decl = ir.nodes[w.node("a").raw()].decl;
    let weighted = (0..ir.arcs[step.arc.raw()].refs.len())
        .map(|s| index.resolve(step.arc, s as u32))
        .find(|r| r.target == a_decl)
        .expect("a is a member of the arc");
    assert!(
        weighted.weights.is_some(),
        "a's weight is preserved and reachable"
    );
    let _ = iref;
}

#[test]
fn connected_components_partition() {
    let src = r#"
vv {}
g {
    a {} b {} c {} p {} q {} iso {}
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
    @f1 { (+ p, + q); }
}
"#;
    let w = compile(src);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let mut ws = TraversalWorkspace::new();
    let cc = connected_components(&index, &mut ws).expect("components");

    assert_eq!(cc.same_component(w.node("a"), w.node("c")), Some(true));
    assert_eq!(cc.same_component(w.node("a"), w.node("p")), Some(false));
    assert_eq!(cc.same_component(w.node("p"), w.node("q")), Some(true));
    assert_eq!(cc.same_component(w.node("a"), w.node("iso")), Some(false));
    // at least: {a,b,c}, {p,q}, {iso} are distinct
    assert!(
        cc.count() >= 3,
        "at least three components, got {}",
        cc.count()
    );
}

#[test]
fn dual_reachability_links_arcs_sharing_a_node() {
    let src = r#"
vv {}
g {
    a {} b {} c {} d {} e {}
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
    @e3 { (+ d, + e); }
}
"#;
    let w = compile(src);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let mut ws = TraversalWorkspace::new();

    let e1 = w.edge_arc("e1");
    let reach = dual_reachability(&index, &mut ws, e1).expect("dual");
    let reached: BTreeSet<usize> = reach.arcs().iter().map(|a| a.raw()).collect();
    assert!(reached.contains(&w.edge_arc("e1").raw()));
    assert!(reached.contains(&w.edge_arc("e2").raw()), "e1,e2 share b");
    assert!(!reached.contains(&w.edge_arc("e3").raw()), "e3 is disjoint");
}

#[test]
fn duplicate_incidence_is_a_single_visit_but_observable_multiplicity() {
    // a referenced twice in the same arc: reachable set counts it once; the index still sees 2.
    let src = r#"
vv {}
g {
    a {} b {}
    @e0 { (+ a, + a, + b); }
}
"#;
    let w = compile(src);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let start = w.node("b");
    let got = reachable_set(&w, start);
    // a appears once in the reachable set
    assert_eq!(got.iter().filter(|&&n| n == w.node("a").raw()).count(), 1);
    // but its incidence multiplicity is observable through the index degree
    let a_decl = ir.nodes[w.node("a").raw()].decl;
    assert_eq!(
        index.degree(a_decl),
        2,
        "duplicate incidence is still observable"
    );
}

#[test]
fn self_incidence_and_singleton_arc_terminate() {
    // singleton arc (one member) and a node repeated -- the walk must simply terminate.
    let src = r#"
vv {}
g {
    a {} b {}
    @solo { (+ a); }
    @dup  { (+ a, + a, + b); }
}
"#;
    let w = compile(src);
    let got = reachable_set(&w, w.node("a"));
    assert!(got.contains(&w.node("b").raw()));
    assert_eq!(got, oracle_reachable(w.ir(), w.node("a")));
}

#[test]
fn invalid_start_is_a_typed_error() {
    let w = compile(CHAIN);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();
    let bogus = NodeId::new(ir.nodes.len() + 100);
    assert!(matches!(
        trav.reachable_nodes(&mut ws, bogus),
        Err(DirectIrError::InvalidStart { .. })
    ));

    let bad_arc = HyperArcId::new(ir.arcs.len() + 100);
    assert!(matches!(
        dual_reachability(&index, &mut ws, bad_arc),
        Err(DirectIrError::InvalidStartArc { .. })
    ));
}

#[test]
fn warm_workspace_is_reused_across_queries() {
    // Running many queries on one workspace must give stable, correct results (generation reuse).
    let w = compile(CHAIN);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();
    let a = w.node("a");
    let first: BTreeSet<usize> = trav
        .reachable_nodes(&mut ws, a)
        .unwrap()
        .nodes()
        .map(|n| n.raw())
        .collect();
    for _ in 0..64 {
        let again: BTreeSet<usize> = trav
            .reachable_nodes(&mut ws, a)
            .unwrap()
            .nodes()
            .map(|n| n.raw())
            .collect();
        assert_eq!(again, first, "repeated warm queries are stable");
    }
}
