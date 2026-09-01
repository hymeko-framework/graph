//! Freeze the informed-search oracles (mission §7), independently of the engine.
//!
//! These pin the *reference* implementations: the linear-scan Dijkstra and the exhaustive Berge-path
//! enumerator agree with the existing BFS `berge_shortest_path` on unit cost, and with each other on
//! tiny graphs. Committed before the A* engine so the engine is checked against a frozen reference.

use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::direct_ir::{HyperTraversal, TraversalWorkspace};

#[path = "direct_ir_common/mod.rs"]
mod dc;
#[path = "informed_common/mod.rs"]
mod ic;

use ic::{OracleCost, SignMode};

/// Unit-cost oracle Dijkstra distance must equal the BFS Berge hyperedge length, for every reachable
/// pair on a window workload (the two engines are independent code).
#[test]
fn oracle_dijkstra_unit_equals_bfs_length_window() {
    let ir = ic::window_ir(40, 5, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = TraversalWorkspace::new();
    use hymeko::common::ids::NodeId;
    for start in [0usize, 3, 17] {
        for goal in [start, start + 1, 20, 39] {
            let bfs = trav
                .berge_shortest_path(&mut ws, NodeId::new(start), NodeId::new(goal))
                .unwrap();
            let orc =
                ic::oracle_dijkstra_cost(&ir, &idx, NodeId::new(start), NodeId::new(goal), OracleCost::Unit);
            match (bfs, orc) {
                (Some(p), Some(c)) => assert_eq!(
                    p.hyperedge_len() as f64,
                    c,
                    "unit oracle cost != BFS length for {start}->{goal}"
                ),
                (None, None) => {}
                other => panic!("BFS/oracle reachability disagree for {start}->{goal}: {other:?}"),
            }
        }
    }
}

/// The exhaustive enumerator and the oracle Dijkstra agree on tiny graphs, unit and weighted.
#[test]
fn exhaustive_equals_oracle_dijkstra_tiny() {
    use hymeko::common::ids::NodeId;
    let ir = ic::window_ir(8, 3, true, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    for cost in [OracleCost::Unit, OracleCost::Weight { base: 1.0 }] {
        for goal in [1usize, 4, 7] {
            let d = ic::oracle_dijkstra_cost(&ir, &idx, NodeId::new(0), NodeId::new(goal), cost);
            let e = ic::exhaustive_min_cost(&ir, &idx, NodeId::new(0), NodeId::new(goal), cost, 500_000);
            match (d, e) {
                (Some(a), Some(b)) => {
                    assert!((a - b).abs() < 1e-9, "dijkstra {a} != exhaustive {b} to {goal}")
                }
                (None, None) => {}
                other => panic!("reachability disagree to {goal}: {other:?}"),
            }
        }
    }
}

/// Source-first (real parser) sanity: a hub compiles and the oracle reaches every spoke from the hub.
#[test]
fn oracle_source_first_hub_reaches_all() {
    use std::fmt::Write;
    let spokes = 6usize;
    let mut s = String::from("vv {}\ng {\n    hub {}\n");
    for i in 0..spokes {
        let _ = writeln!(s, "    n{i} {{}}");
    }
    for i in 0..spokes {
        let _ = writeln!(s, "    @e{i} {{ (+ hub, + n{i}); }}");
    }
    s.push_str("}\n");
    let w = dc::compile(&s);
    let idx = build_incidence_index(w.ir()).unwrap();
    let hub = w.node("hub");
    for i in 0..spokes {
        let goal = w.node(&format!("n{i}"));
        let c = ic::oracle_dijkstra_cost(w.ir(), &idx, hub, goal, OracleCost::Unit);
        assert_eq!(c, Some(1.0), "hub must reach spoke n{i} in one hop");
    }
    // cross-check reachability against the committed oracle_reachable
    let reach = dc::oracle_reachable(w.ir(), hub);
    assert_eq!(reach.len(), spokes + 1, "hub reaches itself + all spokes");
}
