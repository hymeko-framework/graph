//! Source-first V&V for the optional `direct_ir` layer: bounded Berge path / cycle enumeration,
//! s-connected components, and serial/parallel batch reachability.

mod direct_ir_common;

use std::collections::BTreeSet;

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::build_incidence_index;

use hymeko_graph::direct_ir::{
    BudgetKind, DirectIrError, EnumBudget, HyperTraversal, TraversalWorkspace,
    batch_reachable_counts, enumerate_berge_cycles, enumerate_berge_paths,
    par_batch_reachable_counts, s_connected_components,
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
    for cap in 0..=3 {
        let budget = EnumBudget::new(8, cap, 100_000);
        let out =
            enumerate_berge_paths(&index, w.node("a"), Some(w.node("d")), budget).expect("ok");
        assert_eq!(out.items.len(), cap.min(2), "cap={cap}");
        assert_eq!(out.bound, (cap <= 2).then_some(BudgetKind::Results));
    }
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

    let zero_cycles = enumerate_berge_cycles(&index, w.node("a"), EnumBudget::new(8, 0, 100_000))
        .expect("zero-result cycle budget");
    assert!(zero_cycles.items.is_empty());
    assert_eq!(zero_cycles.bound, Some(BudgetKind::Results));
}

const PARALLEL_ARCS: &str = r#"
vv {}
g {
    a {} b {}
    @e1 { (+ a, + b); }
    @e2 { (+ a, + b); }
    @e3 { (+ a, + b); }
}
"#;

#[test]
fn berge_cycles_keep_parallel_arc_identity() {
    let w = compile(PARALLEL_ARCS);
    let index = build_incidence_index(w.ir()).expect("index");
    let out = enumerate_berge_cycles(&index, w.node("a"), EnumBudget::new(2, 100, 1_000))
        .expect("cycles");
    assert_eq!(out.items.len(), 3, "one cycle for each pair of arcs");
    assert!(out.bound.is_none());

    let pairs: BTreeSet<Vec<usize>> = out
        .items
        .iter()
        .map(|cycle| {
            assert_eq!(cycle.nodes.len(), 2);
            assert_eq!(cycle.arcs.len(), 2);
            let mut arcs: Vec<usize> = cycle.arcs.iter().map(|arc| arc.raw()).collect();
            arcs.sort_unstable();
            arcs
        })
        .collect();
    let mut expected = BTreeSet::new();
    for (a, b) in [("e1", "e2"), ("e1", "e3"), ("e2", "e3")] {
        let mut arcs = vec![w.edge_arc(a).raw(), w.edge_arc(b).raw()];
        arcs.sort_unstable();
        expected.insert(arcs);
    }
    assert_eq!(pairs, expected);
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

// Declaration order puts the depth-pruned branch before the two short arcs.
const DEEP_THEN_SHORT: &str = r#"
vv {}
g {
    a {} b {} c {} d {} t {}
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
    @e3 { (+ c, + d); }
    @e4 { (+ a, + t); }
    @e5 { (+ a, + t); }
}
"#;

#[test]
fn berge_depth_pruning_preserves_later_paths_and_cycles() {
    let w = compile(DEEP_THEN_SHORT);
    let index = build_incidence_index(w.ir()).expect("index");
    let (start, goal) = (w.node("a"), w.node("t"));
    let short_arcs = BTreeSet::from([w.edge_arc("e4"), w.edge_arc("e5")]);
    // Goal-directed paths stop at t: four expansions (e1, e2, e4, e5).
    // Cycles still need six (e1, e2, e4, e5, e5, e4); e3 is depth-pruned.
    for (results, expansions, path_count, cycle_count, path_bound, cycle_bound) in [
        (100, 6, 2, 1, BudgetKind::Depth, BudgetKind::Depth),
        (1, 100, 1, 1, BudgetKind::Results, BudgetKind::Results),
        (100, 2, 0, 0, BudgetKind::Expansions, BudgetKind::Expansions),
        (100, 4, 2, 1, BudgetKind::Depth, BudgetKind::Expansions),
    ] {
        let budget = EnumBudget::new(2, results, expansions);
        let paths = enumerate_berge_paths(&index, start, Some(goal), budget).expect("paths");
        let cycles = enumerate_berge_cycles(&index, start, budget).expect("cycles");
        assert_eq!(
            (paths.bound, cycles.bound),
            (Some(path_bound), Some(cycle_bound))
        );
        assert_eq!(
            (paths.items.len(), cycles.items.len()),
            (path_count, cycle_count)
        );
        let mut emitted_arcs = BTreeSet::new();
        for path in paths.items {
            assert_eq!(path.nodes().collect::<Vec<_>>(), [start, goal]);
            assert_eq!(path.hyperedge_len(), 1);
            assert!(short_arcs.contains(&path.steps[0].arc));
            assert!(emitted_arcs.insert(path.steps[0].arc));
        }
        if path_count == 2 {
            assert_eq!(emitted_arcs, short_arcs);
        }
        for cycle in cycles.items {
            assert_eq!(
                cycle.nodes.iter().copied().collect::<BTreeSet<_>>(),
                BTreeSet::from([start, goal])
            );
            assert_eq!(cycle.nodes.len(), 2);
            assert_eq!(cycle.arcs.len(), 2);
            assert_eq!(cycle.arcs.into_iter().collect::<BTreeSet<_>>(), short_arcs);
        }
    }
}

#[test]
fn berge_zero_budgets_report_the_first_applicable_bound() {
    let w = compile(DEEP_THEN_SHORT);
    let index = build_incidence_index(w.ir()).expect("index");
    for (budget, bound) in [
        (EnumBudget::new(8, 0, 100), BudgetKind::Results),
        (EnumBudget::new(0, 0, 0), BudgetKind::Results),
        (EnumBudget::new(0, 100, 0), BudgetKind::Depth),
        (EnumBudget::new(2, 100, 0), BudgetKind::Expansions),
    ] {
        for goal in [None, Some(w.node("t"))] {
            let paths = enumerate_berge_paths(&index, w.node("a"), goal, budget).expect("paths");
            assert!(paths.items.is_empty(), "budget={budget:?}, goal={goal:?}");
            assert_eq!(paths.bound, Some(bound));
        }
        let cycles = enumerate_berge_cycles(&index, w.node("a"), budget).expect("cycles");
        assert!(cycles.items.is_empty(), "budget={budget:?}");
        assert_eq!(cycles.bound, Some(bound));
    }
}

#[test]
fn berge_cycles_cover_exact_and_exceeded_budgets() {
    let w = compile(TRIANGLE);
    let index = build_incidence_index(w.ir()).expect("index");
    for (budget, count, bound) in [
        (EnumBudget::new(3, 0, 6), 0, Some(BudgetKind::Results)),
        (EnumBudget::new(3, 1, 6), 1, Some(BudgetKind::Results)),
        (EnumBudget::new(3, 2, 6), 1, None),
        (EnumBudget::new(3, 2, 5), 1, Some(BudgetKind::Expansions)),
        (EnumBudget::new(2, 2, 6), 0, Some(BudgetKind::Depth)),
    ] {
        let out = enumerate_berge_cycles(&index, w.node("a"), budget).expect("cycles");
        assert_eq!(
            (out.items.len(), out.bound),
            (count, bound),
            "budget={budget:?}"
        );
    }
}

#[test]
fn berge_finished_frontiers_do_not_report_depth_or_expansion_bounds() {
    let w = compile("vv {} g { a {} b {} isolated {} @e { (+ a, + b); } }");
    let index = build_incidence_index(w.ir()).expect("index");
    let budget = EnumBudget::new(1, 100, 1);
    let paths = enumerate_berge_paths(&index, w.node("a"), Some(w.node("b")), budget)
        .expect("exact depth and expansion completion");
    assert_eq!((paths.items.len(), paths.bound), (1, None));
    assert_eq!(
        paths.items[0].nodes().collect::<Vec<_>>(),
        [w.node("a"), w.node("b")]
    );
    for (cap, bound) in [(100, None), (0, Some(BudgetKind::Results))] {
        let budget = EnumBudget::new(0, cap, 0);
        let paths = enumerate_berge_paths(&index, w.node("isolated"), None, budget).expect("paths");
        let cycles = enumerate_berge_cycles(&index, w.node("isolated"), budget).expect("cycles");
        assert!(paths.items.is_empty() && cycles.items.is_empty());
        assert_eq!((paths.bound, cycles.bound), (bound, bound));
    }
}

#[test]
fn berge_zero_budget_still_validates_endpoints() {
    let w = compile(DIAMOND);
    let index = build_incidence_index(w.ir()).expect("index");
    let invalid = NodeId::new(usize::MAX);
    let budget = EnumBudget::new(0, 0, 0);
    for (start, goal) in [(invalid, None), (w.node("a"), Some(invalid))] {
        assert!(
            matches!(enumerate_berge_paths(&index, start, goal, budget), Err(DirectIrError::InvalidStart { node }) if node == invalid)
        );
    }
    assert!(
        matches!(enumerate_berge_cycles(&index, invalid, budget), Err(DirectIrError::InvalidStart { node }) if node == invalid)
    );
}

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
