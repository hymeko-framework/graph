//! Source-first V&V for the optional `direct_ir` layer: bounded Berge path / cycle enumeration,
//! s-connected components, and serial/parallel batch reachability.

mod direct_ir_common;

use std::collections::BTreeSet;

use hymeko::ir::incidence::build_incidence_index;

use hymeko_graph::direct_ir::{
    BudgetKind, EnumBudget, HyperTraversal, TraversalWorkspace, batch_reachable_counts,
    enumerate_berge_cycles, enumerate_berge_paths, par_batch_reachable_counts,
    s_connected_components,
};

use direct_ir_common::compile;

const DIAMOND: &str = r#"
vv {}
g {
    a {} b {} c {} d {}
    @e1 { (+ a, + b); }
    @e2 { (+ a, + c); }
    @e3 { (+ b, + d); }
    @e4 { (+ c, + d); }
}
"#;

#[test]
fn berge_paths_enumerates_both_simple_paths() {
    let w = compile(DIAMOND);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let budget = EnumBudget::new(8, 100, 100_000);
    let out = enumerate_berge_paths(&index, w.node("a"), Some(w.node("d")), budget).expect("ok");
    assert_eq!(
        out.items.len(),
        2,
        "two simple a->d paths through the diamond"
    );
    assert!(out.bound.is_none(), "finished within budget");
    // both paths are three-hop start..d with distinct arc sets
    for p in &out.items {
        assert_eq!(p.hyperedge_len(), 2);
        assert_eq!(p.nodes().last().unwrap(), w.node("d"));
    }
}

#[test]
fn berge_paths_budget_bound_is_reported() {
    let w = compile(DIAMOND);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let budget = EnumBudget::new(8, 1, 100_000); // cap results at 1
    let out = enumerate_berge_paths(&index, w.node("a"), Some(w.node("d")), budget).expect("ok");
    assert_eq!(out.items.len(), 1);
    assert_eq!(
        out.bound,
        Some(BudgetKind::Results),
        "truncation is reported, not silent"
    );
}

const TRIANGLE: &str = r#"
vv {}
g {
    a {} b {} c {}
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
    @e3 { (+ c, + a); }
}
"#;

#[test]
fn berge_cycles_dedup_to_one_canonical_cycle() {
    let w = compile(TRIANGLE);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let budget = EnumBudget::new(8, 100, 100_000);
    let out = enumerate_berge_cycles(&index, w.node("a"), budget).expect("ok");
    // the triangle has exactly one canonical Berge cycle (both traversal directions dedup to one)
    assert_eq!(
        out.items.len(),
        1,
        "one canonical cycle after reflection/rotation dedup"
    );
    let cyc = &out.items[0];
    assert_eq!(cyc.nodes.len(), 3);
    assert_eq!(cyc.arcs.len(), 3);
    let nodes: BTreeSet<usize> = cyc.nodes.iter().map(|n| n.raw()).collect();
    assert_eq!(
        nodes,
        BTreeSet::from([w.node("a").raw(), w.node("b").raw(), w.node("c").raw()])
    );
}

const SLINE: &str = r#"
vv {}
g {
    x {} y {} z {} w {}
    @e1 { (+ x, + y); }
    @e2 { (+ x, + y); }
    @e3 { (+ y, + z); }
    @e4 { (+ z, + w); }
}
"#;

#[test]
fn s_connected_components_s1_is_one_component() {
    let w = compile(SLINE);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let cc = s_connected_components(&index, 1);
    assert_eq!(cc.s(), 1);
    // every arc shares at least one node with a neighbour -> a single 1-component
    assert_eq!(cc.count(), 1);
}

#[test]
fn s_connected_components_s2_separates_by_overlap() {
    let w = compile(SLINE);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let cc = s_connected_components(&index, 2);
    assert_eq!(cc.s(), 2);
    // e1,e2 share both x and y (|∩|=2) -> together; e3,e4 share only one node with anyone -> alone
    assert_eq!(
        cc.same_component(w.edge_arc("e1"), w.edge_arc("e2")),
        Some(true)
    );
    assert_eq!(
        cc.same_component(w.edge_arc("e1"), w.edge_arc("e3")),
        Some(false)
    );
    assert_eq!(cc.count(), 3, "{{e1,e2}}, {{e3}}, {{e4}}");
}

#[test]
fn batch_reachable_serial_equals_parallel_and_singletons() {
    let w = compile(DIAMOND);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let starts = [w.node("a"), w.node("b"), w.node("c"), w.node("d")];

    let serial = batch_reachable_counts(&index, &starts).expect("serial");
    let parallel = par_batch_reachable_counts(&index, &starts).expect("parallel");
    assert_eq!(
        serial, parallel,
        "parallelism changes speed, never the answer"
    );

    // cross-check against single-query reachable_nodes
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();
    for (i, &s) in starts.iter().enumerate() {
        let one = trav.reachable_nodes(&mut ws, s).unwrap().len();
        assert_eq!(one, serial[i]);
    }
    // in the diamond every node reaches all four
    assert!(serial.iter().all(|&c| c == 4));
}
