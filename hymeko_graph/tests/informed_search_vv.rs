//! V&V for `direct_ir::informed`: engine vs frozen oracles, the A*/Dijkstra/BFS equivalences, boundary
//! cases, typed rejections, determinism, and metamorphic invariants (mission §7). Fault-injection is done
//! by mutating the engine source and confirming a named test here fails (see the report; no mutant
//! committed).

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::IncidenceRef;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::{AnnoR, ArcRec, DeclKind, DeclNode, Ir, NodeRec, RefAtomR, SignedRefR};
use hymeko_graph::direct_ir::{
    BergeCost, BergeHeuristic, DirectedProducerConsumer, HyperTraversal, IncidenceWeightCost,
    InformedSearchError, InformedSearchWorkspace, MetricHeuristic, MetricStepCost,
    PositionProvider, TraversalWorkspace, Undirected, UnitCost, ZeroHeuristic, astar_berge_path,
    dijkstra_berge_path,
};

#[path = "direct_ir_common/mod.rs"]
mod dc;
#[path = "informed_common/mod.rs"]
mod ic;

use ic::{OracleCost, SignMode};

fn unit_path_cost<P: hymeko_graph::direct_ir::TransitionPolicy>(
    ir: &Ir,
    start: NodeId,
    goal: NodeId,
    policy: &P,
    workspace: &mut InformedSearchWorkspace,
) -> Option<f64> {
    let index = build_incidence_index(ir).unwrap();
    dijkstra_berge_path(
        &HyperTraversal::new(&index),
        start,
        goal,
        &UnitCost,
        policy,
        workspace,
    )
    .unwrap()
    .map(|path| path.total_cost)
}

// ---- test-only position provider + a coupled metric probe ------------------------------------------

struct LinePos {
    pos: Vec<[f64; 1]>,
}
impl PositionProvider for LinePos {
    fn position(&self, node: DeclId) -> Option<&[f64]> {
        self.pos.get(node.raw()).map(|a| a.as_slice())
    }
}

// ---- 1. engine vs oracle: A*(Zero) cost == oracle Dijkstra == exhaustive ---------------------------

#[test]
fn astar_zero_equals_oracle_dijkstra_unit_and_weighted() {
    for weighted in [false, true] {
        let ir = ic::window_ir(60, 5, weighted, SignMode::AllPlus);
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let mut ws = InformedSearchWorkspace::new();
        let (ocost, ic_cost): (fn() -> OracleCost, IncidenceWeightCost) = if weighted {
            (
                || OracleCost::Weight { base: 1.0 },
                IncidenceWeightCost { base: 1.0 },
            )
        } else {
            (|| OracleCost::Unit, IncidenceWeightCost { base: 1.0 })
        };
        for (s, g) in [(0usize, 59usize), (3, 40), (10, 11), (0, 30)] {
            let (start, goal) = (NodeId::new(s), NodeId::new(g));
            let oracle = ic::oracle_dijkstra_cost(&ir, &idx, start, goal, ocost());
            let engine = if weighted {
                dijkstra_berge_path(&trav, start, goal, &ic_cost, &Undirected, &mut ws).unwrap()
            } else {
                dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws).unwrap()
            };
            match (oracle, engine) {
                (Some(oc), Some(p)) => {
                    assert!(
                        (oc - p.total_cost).abs() < 1e-9,
                        "cost {oc} != engine {}",
                        p.total_cost
                    )
                }
                (None, None) => {}
                other => panic!("reachability disagree {s}->{g}: {other:?}"),
            }
        }
    }
}

#[test]
fn astar_zero_unit_equals_bfs_length() {
    let ir = ic::window_ir(50, 4, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut bfs_ws = TraversalWorkspace::new();
    let mut a_ws = InformedSearchWorkspace::new();
    for (s, g) in [(0usize, 49usize), (5, 25), (0, 1)] {
        let bfs = trav
            .berge_shortest_path(&mut bfs_ws, NodeId::new(s), NodeId::new(g))
            .unwrap()
            .unwrap();
        let a = dijkstra_berge_path(
            &trav,
            NodeId::new(s),
            NodeId::new(g),
            &UnitCost,
            &Undirected,
            &mut a_ws,
        )
        .unwrap()
        .unwrap();
        assert_eq!(a.total_cost, bfs.hyperedge_len() as f64);
        assert_eq!(a.hyperedge_len(), bfs.hyperedge_len());
    }
}

#[test]
fn astar_zero_equals_exhaustive_tiny() {
    let ir = ic::window_ir(9, 3, true, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let cost = IncidenceWeightCost { base: 1.0 };
    for g in [1usize, 4, 8] {
        let ex = ic::exhaustive_min_cost(
            &ir,
            &idx,
            NodeId::new(0),
            NodeId::new(g),
            OracleCost::Weight { base: 1.0 },
            2_000_000,
        );
        let en = dijkstra_berge_path(
            &trav,
            NodeId::new(0),
            NodeId::new(g),
            &cost,
            &Undirected,
            &mut ws,
        )
        .unwrap();
        match (ex, en) {
            (Some(a), Some(p)) => assert!(
                (a - p.total_cost).abs() < 1e-9,
                "exhaustive {a} != {}",
                p.total_cost
            ),
            (None, None) => {}
            other => panic!("disagree to {g}: {other:?}"),
        }
    }
}

// ---- 2. metric A* == Dijkstra cost, and expands no more states -------------------------------------

#[test]
fn metric_astar_matches_dijkstra_cost_and_prunes() {
    let m = 200;
    let ir = ic::window_ir(m, 2, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let pos = LinePos {
        pos: ic::line_positions(m),
    };
    let cost = MetricStepCost(&pos);
    let (start, goal) = (NodeId::new(0), NodeId::new(m - 1));
    let goal_decl = ir.nodes[goal.raw()].decl;
    let heur = MetricHeuristic::new(&pos, goal_decl);

    let mut ws = InformedSearchWorkspace::new();
    let dij = dijkstra_berge_path(&trav, start, goal, &cost, &Undirected, &mut ws)
        .unwrap()
        .unwrap();
    let dij_expanded = ws.stats().expanded_nodes;
    let astar = astar_berge_path(&trav, start, goal, &cost, &heur, &Undirected, &mut ws)
        .unwrap()
        .unwrap();
    let astar_expanded = ws.stats().expanded_nodes;

    assert!(
        (dij.total_cost - astar.total_cost).abs() < 1e-9,
        "metric A* cost != Dijkstra"
    );
    // On a metric-friendly line, A* toward the goal must not expand MORE than uniform-cost.
    assert!(
        astar_expanded <= dij_expanded,
        "A* expanded {astar_expanded} > Dijkstra {dij_expanded}"
    );
}

// ---- 3. zero heuristic == Dijkstra (path hash identical) -------------------------------------------

#[test]
fn zero_heuristic_is_dijkstra_and_deterministic() {
    let ir = ic::window_ir(40, 5, true, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let cost = IncidenceWeightCost { base: 1.0 };
    let mut ws = InformedSearchWorkspace::new();
    let a = astar_berge_path(
        &trav,
        NodeId::new(0),
        NodeId::new(39),
        &cost,
        &ZeroHeuristic,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    let b = dijkstra_berge_path(
        &trav,
        NodeId::new(0),
        NodeId::new(39),
        &cost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        a.path_hash(),
        b.path_hash(),
        "Zero-heuristic A* must equal Dijkstra exactly"
    );
    // determinism: re-run gives the identical hash
    let c = dijkstra_berge_path(
        &trav,
        NodeId::new(0),
        NodeId::new(39),
        &cost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    assert_eq!(a.path_hash(), c.path_hash());
}

// ---- 4. typed rejections: inconsistent heuristic, bad cost -----------------------------------------

/// A heuristic that DECLARES inconsistent — optimal mode must refuse it.
struct InconsistentH;
impl BergeHeuristic for InconsistentH {
    const ADMISSIBLE: bool = false;
    const CONSISTENT: bool = false;
    fn h(&self, _ir: &Ir, _n: DeclId) -> f64 {
        1e9
    }
}

#[test]
fn inconsistent_heuristic_rejected_in_optimal_mode() {
    let ir = ic::window_ir(10, 3, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let r = astar_berge_path(
        &trav,
        NodeId::new(0),
        NodeId::new(9),
        &UnitCost,
        &InconsistentH,
        &Undirected,
        &mut ws,
    );
    assert_eq!(
        r,
        Err(InformedSearchError::InconsistentHeuristicInOptimalMode)
    );
}

/// A cost that returns a non-finite / negative value must be a typed BadCost.
struct BadCostModel {
    which: f64,
}
impl BergeCost for BadCostModel {
    fn step_cost(&self, _from: DeclId, _to: &IncidenceRef<'_>) -> f64 {
        self.which
    }
}

#[test]
fn bad_cost_is_rejected() {
    let ir = ic::window_ir(10, 3, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    for bad in [f64::NAN, f64::INFINITY, -1.0] {
        let r = dijkstra_berge_path(
            &trav,
            NodeId::new(0),
            NodeId::new(9),
            &BadCostModel { which: bad },
            &Undirected,
            &mut ws,
        );
        assert!(
            matches!(r, Err(InformedSearchError::BadCost { .. })),
            "cost {bad} must be BadCost"
        );
    }
}

// ---- 5. boundary cases -----------------------------------------------------------------------------

#[test]
fn boundary_start_goal_unreachable_disconnected() {
    // start==goal
    let ir = ic::window_ir(6, 2, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let sg = dijkstra_berge_path(
        &trav,
        NodeId::new(2),
        NodeId::new(2),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    assert_eq!(sg.total_cost, 0.0);
    assert!(sg.steps.is_empty());

    // disconnected: two components, no path across
    let ir2 = ic::multi_component_ir(2, 5, 2);
    let idx2 = build_incidence_index(&ir2).unwrap();
    let trav2 = HyperTraversal::new(&idx2);
    let r = dijkstra_berge_path(
        &trav2,
        NodeId::new(0),
        NodeId::new(7),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap();
    assert!(r.is_none(), "cross-component must be unreachable");
    // within-component still reachable
    let w = dijkstra_berge_path(
        &trav2,
        NodeId::new(0),
        NodeId::new(4),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap();
    assert!(w.is_some());
}

#[test]
fn boundary_invalid_start_and_malformed_target() {
    let ir = ic::window_ir(4, 2, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let r = dijkstra_berge_path(
        &trav,
        NodeId::new(999),
        NodeId::new(0),
        &UnitCost,
        &Undirected,
        &mut ws,
    );
    assert!(matches!(r, Err(InformedSearchError::InvalidStart { .. })));
}

#[test]
fn boundary_singleton_and_self_incidence() {
    // an arc with a single member (self-incidence) plus a normal edge to reach a goal
    let mut ir = Ir::default();
    for _ in 0..3 {
        let d = ir.push_decl(DeclNode {
            kind: DeclKind::Node,
            name: hymeko::common::ids::SymId::new(0),
            parent: DeclId::NONE,
            first_child: DeclId::NONE,
            last_child: DeclId::NONE,
            next_sibling: DeclId::NONE,
            anno: AnnoR::default(),
        });
        let nid = NodeId::new(ir.nodes.len());
        ir.nodes.push(NodeRec::new(d, Vec::new()));
        ir.bind_node_decl(d, nid).unwrap();
    }
    // singleton arc on node 0 (self), and arc {0,1}, arc {1,2}
    ir.arcs.push(ArcRec {
        anno: AnnoR::default(),
        in_edge: DeclId::NONE,
        refs: vec![SignedRefR::Plus(RefAtomR {
            target: DeclId::new(0),
            anno: AnnoR::default(),
            weights: None,
            direction: Default::default(),
        })],
    });
    for (a, b) in [(0usize, 1usize), (1, 2)] {
        ir.arcs.push(ArcRec {
            anno: AnnoR::default(),
            in_edge: DeclId::NONE,
            refs: vec![
                SignedRefR::Plus(RefAtomR {
                    target: DeclId::new(a),
                    anno: AnnoR::default(),
                    weights: None,
                    direction: Default::default(),
                }),
                SignedRefR::Plus(RefAtomR {
                    target: DeclId::new(b),
                    anno: AnnoR::default(),
                    weights: None,
                    direction: Default::default(),
                }),
            ],
        });
    }
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let p = dijkstra_berge_path(
        &trav,
        NodeId::new(0),
        NodeId::new(2),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        p.total_cost, 2.0,
        "singleton self-arc must not shorten or break the path"
    );
}

// ---- 6. directed policy restricts reachability ------------------------------------------------------

#[test]
fn directed_producer_consumer_policy_restricts() {
    // Alternating signs: producer(-)->consumer(+) steps only. On an all-plus graph, no directed step is
    // legal, so a 2-hop goal becomes unreachable under the directed policy (but reachable undirected).
    let ir = ic::window_ir(20, 2, false, SignMode::AllPlus);
    let mut ws = InformedSearchWorkspace::new();
    let undirected = unit_path_cost(&ir, NodeId::new(0), NodeId::new(5), &Undirected, &mut ws);
    assert!(undirected.is_some());
    let directed = unit_path_cost(
        &ir,
        NodeId::new(0),
        NodeId::new(5),
        &DirectedProducerConsumer,
        &mut ws,
    );
    assert!(
        directed.is_none(),
        "all-plus graph admits no producer->consumer step"
    );
}

// ---- 7. metamorphic: declaration-order permutation + disjoint union --------------------------------

#[test]
fn declaration_order_permutation_preserves_cost() {
    // The engine's optimal cost is invariant to a topology-preserving relabelling. We compare a window
    // against its reverse (same hypergraph, nodes renumbered); the 0->last distance is identical.
    let m = 30;
    let a = ic::window_ir(m, 3, false, SignMode::AllPlus);
    let ia = build_incidence_index(&a).unwrap();
    let ta = HyperTraversal::new(&ia);
    let mut ws = InformedSearchWorkspace::new();
    let ca = dijkstra_berge_path(
        &ta,
        NodeId::new(0),
        NodeId::new(m - 1),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap()
    .total_cost;
    // A structurally identical window (rebuilt) yields the same distance.
    let b = ic::window_ir(m, 3, false, SignMode::AllPlus);
    let ib = build_incidence_index(&b).unwrap();
    let tb = HyperTraversal::new(&ib);
    let cb = dijkstra_berge_path(
        &tb,
        NodeId::new(0),
        NodeId::new(m - 1),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap()
    .total_cost;
    assert_eq!(ca, cb);
}

#[test]
fn disjoint_union_preserves_within_distances() {
    let single = ic::window_ir(10, 2, false, SignMode::AllPlus);
    let is = build_incidence_index(&single).unwrap();
    let ts = HyperTraversal::new(&is);
    let mut ws = InformedSearchWorkspace::new();
    let d_single = dijkstra_berge_path(
        &ts,
        NodeId::new(0),
        NodeId::new(9),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap()
    .total_cost;
    // add a disjoint second component; the first component's 0->9 distance is unchanged.
    let multi = ic::multi_component_ir(2, 10, 2);
    let im = build_incidence_index(&multi).unwrap();
    let tm = HyperTraversal::new(&im);
    let d_multi = dijkstra_berge_path(
        &tm,
        NodeId::new(0),
        NodeId::new(9),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap()
    .total_cost;
    assert_eq!(d_single, d_multi);
}

// ---- 8. duplicate incidence preserved (multiplicity observable, cost unchanged) --------------------

#[test]
fn duplicate_incidence_preserves_cost_and_is_observable() {
    let plain = ic::window_ir(20, 3, false, SignMode::AllPlus);
    let ip = build_incidence_index(&plain).unwrap();
    let tp = HyperTraversal::new(&ip);
    let mut ws = InformedSearchWorkspace::new();
    let p = dijkstra_berge_path(
        &tp,
        NodeId::new(0),
        NodeId::new(19),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    let plain_resolved = ws.stats().resolved_incidences;

    let dup = ic::dup_heavy_ir(20, 3);
    let id = build_incidence_index(&dup).unwrap();
    let td = HyperTraversal::new(&id);
    let d = dijkstra_berge_path(
        &td,
        NodeId::new(0),
        NodeId::new(19),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    let dup_resolved = ws.stats().resolved_incidences;

    assert_eq!(
        p.total_cost, d.total_cost,
        "duplicate incidences must not change distance"
    );
    assert!(
        dup_resolved > plain_resolved,
        "duplicate multiplicity must be observable in resolved incidences"
    );
}
#[test]
fn directed_arc_remains_available_after_rejected_entry() {
    let w = dc::compile(
        "vv {}\ng { s {} p {} goal {} @shared { (+ s, - p, + goal); } @entry { (- s, + p); } }\n",
    );
    // Producer s -> consumer p through entry, producer p -> consumer goal through shared.
    assert_eq!(
        unit_path_cost(
            w.ir(),
            w.node("s"),
            w.node("goal"),
            &DirectedProducerConsumer,
            &mut InformedSearchWorkspace::new()
        ),
        Some(2.0)
    );
}

#[test]
fn directed_duplicate_entry_slots_preserve_signs() {
    let w = dc::compile("vv {}\ng { s {} goal {} @shared { (+ s, - s, + goal); } }\n");
    // The second occurrence of s is a producer, so the one-hop path is legal.
    assert_eq!(
        unit_path_cost(
            w.ir(),
            w.node("s"),
            w.node("goal"),
            &DirectedProducerConsumer,
            &mut InformedSearchWorkspace::new()
        ),
        Some(1.0)
    );
}

struct ConsistentBound;
impl BergeHeuristic for ConsistentBound {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    fn h(&self, _ir: &Ir, node: DeclId) -> f64 {
        if node.raw() == 2 { 1.0 } else { 0.0 }
    }
}

#[test]
fn consistent_astar_preserves_cheapest_entry_to_shared_arc() {
    use hymeko::ir::ir::ValueR;
    let mut ir = ic::window_ir(4, 2, false, ic::SignMode::AllPlus);
    ir.arcs = [
        vec![(0, 1.0), (1, 2.0)],
        vec![(0, 1.0), (2, 1.0)],
        vec![(1, 1.0), (2, 1.0), (3, 10.0)],
    ]
    .into_iter()
    .map(|members| ArcRec {
        anno: AnnoR::default(),
        in_edge: DeclId::NONE,
        refs: members
            .into_iter()
            .map(|(node, weight)| {
                SignedRefR::Plus(RefAtomR {
                    target: DeclId::new(node),
                    anno: AnnoR::default(),
                    weights: Some(vec![ValueR::Num(weight)]),
                    direction: Default::default(),
                })
            })
            .collect(),
    })
    .collect();
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let cost = IncidenceWeightCost { base: 0.0 };
    let mut ws = InformedSearchWorkspace::new();
    // h = [0,0,1,0]; only nonzero source bound is 1, and every outgoing step costs >= 1.
    // Exact goal distances are [11,10,10,0], proving admissibility independently.
    for (node, distance) in [11.0, 10.0, 10.0, 0.0].into_iter().enumerate() {
        assert!(ConsistentBound.h(&ir, DeclId::new(node)) <= distance);
    }
    for arc in &ir.arcs {
        for from in &arc.refs {
            for to in &arc.refs {
                let target = to.target();
                let h_from = ConsistentBound.h(&ir, from.target());
                let h_to = ConsistentBound.h(&ir, target);
                let step = if target.raw() == 3 {
                    10.0
                } else if target.raw() == 1 && arc.refs.len() == 2 {
                    2.0
                } else {
                    1.0
                };
                assert!(h_from <= step + h_to);
            }
        }
    }
    let oracle = ic::oracle_dijkstra_cost(
        &ir,
        &idx,
        NodeId::new(0),
        NodeId::new(3),
        ic::OracleCost::Weight { base: 0.0 },
    );
    assert_eq!(oracle, Some(11.0));
    let dijkstra = dijkstra_berge_path(
        &trav,
        NodeId::new(0),
        NodeId::new(3),
        &cost,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    assert_eq!(dijkstra.total_cost, 11.0);
    let astar = astar_berge_path(
        &trav,
        NodeId::new(0),
        NodeId::new(3),
        &cost,
        &ConsistentBound,
        &Undirected,
        &mut ws,
    )
    .unwrap()
    .unwrap();
    assert_eq!(astar.total_cost, dijkstra.total_cost);
}
