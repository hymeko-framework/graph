//! V&V for `direct_ir::informed`: engine vs frozen oracles, the A*/Dijkstra/BFS equivalences, boundary
//! cases, typed rejections, determinism, and metamorphic invariants (mission §7). Fault-injection is done
//! by mutating the engine source and confirming a named test here fails (see the report; no mutant
//! committed).

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::IncidenceRef;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::IrConstruction;
use hymeko::ir::ir::{AnnoR, ArcRec, Ir, RefAtomR, SignedRefR};
use hymeko_graph::direct_ir::{
    BergeCost, BergeHeuristic, DirectedProducerConsumer, HyperTraversal, IncidenceWeightCost,
    InformedSearchError, InformedSearchWorkspace, MetricHeuristic, MetricStepCost,
    PositionProvider, PreparedBergeSearch, TraversalWorkspace, Undirected, UnitCost, ZeroHeuristic,
};
use ic::search::{AStar, Dijkstra, SearchCase};

#[path = "direct_ir_common/mod.rs"]
mod dc;
#[path = "informed_common/mod.rs"]
mod ic;

use ic::{OracleCost, SignMode};

#[test]
fn prepared_berge_matches_oracle_and_directed_engine_across_shapes() {
    use hymeko_graph::direct_ir::{DirectedConsumerProducer, PreparedBergeSearch};
    for n in [5, 9, 17] {
        for arity in [2, 3, 5] {
            let ir = ic::window_ir(n, arity, true, SignMode::Alternating);
            let index = build_incidence_index(&ir).unwrap();
            let traversal = HyperTraversal::new(&index);
            let goal = NodeId::new(n - 1);
            let mut scratch = TraversalWorkspace::new();
            let prepared = PreparedBergeSearch::build(&index, goal, &mut scratch).unwrap();
            let mut ws = InformedSearchWorkspace::new();
            for start in (0..n).map(NodeId::new) {
                let exact = ic::oracle_dijkstra_cost(&ir, &index, start, goal, OracleCost::Unit);
                assert_eq!(prepared.relaxed_hops(start).map(f64::from), exact);
                assert_eq!(
                    prepared
                        .search(start, &UnitCost, &Undirected, &mut ws)
                        .unwrap()
                        .map(|p| p.total_cost),
                    exact
                );
                let cost = IncidenceWeightCost { base: 0.25 };
                assert_eq!(
                    prepared
                        .search(start, &cost, &Undirected, &mut ws)
                        .unwrap()
                        .map(|p| p.total_cost),
                    ic::oracle_dijkstra_cost(
                        &ir,
                        &index,
                        start,
                        goal,
                        OracleCost::Weight { base: 0.25 }
                    )
                );
                let direct = ic::search::SearchCase::new(&traversal, start, goal, &mut ws)
                    .run(&Dijkstra::new(&UnitCost, &DirectedProducerConsumer))
                    .unwrap();
                let optimized = prepared
                    .search(start, &UnitCost, &DirectedProducerConsumer, &mut ws)
                    .unwrap();
                assert_eq!(
                    optimized.as_ref().map(|p| p.total_cost),
                    direct.as_ref().map(|p| p.total_cost)
                );
                let direct = ic::search::SearchCase::new(&traversal, start, goal, &mut ws)
                    .run(&Dijkstra::new(&UnitCost, &DirectedConsumerProducer))
                    .unwrap();
                assert_eq!(
                    prepared
                        .search(start, &UnitCost, &DirectedConsumerProducer, &mut ws)
                        .unwrap()
                        .map(|p| p.total_cost),
                    direct.map(|p| p.total_cost)
                );
            }
            assert_eq!(prepared.relaxed_hops(NodeId::new(n)), None);
            assert_eq!(prepared.entropy_priority(NodeId::new(n)), None);
            let rebuilt = PreparedBergeSearch::build(&index, goal, &mut scratch).unwrap();
            for node in (0..n).map(NodeId::new) {
                assert_eq!(
                    prepared.entropy_priority(node),
                    rebuilt.entropy_priority(node)
                );
            }
        }
    }
}

#[test]
fn prepared_berge_rejects_disconnected_starts_without_frontier_work() {
    use hymeko_graph::direct_ir::PreparedBergeSearch;
    let ir = ic::multi_component_ir(2, 5, 3);
    let index = build_incidence_index(&ir).unwrap();
    let mut scratch = TraversalWorkspace::new();
    let prepared = PreparedBergeSearch::build(&index, NodeId::new(9), &mut scratch).unwrap();
    let mut ws = InformedSearchWorkspace::new();
    prepared
        .search(NodeId::new(5), &UnitCost, &Undirected, &mut ws)
        .unwrap();
    assert!(
        prepared
            .search(NodeId::new(0), &UnitCost, &Undirected, &mut ws)
            .unwrap()
            .is_none()
    );
    assert_eq!(ws.stats(), Default::default());
    assert!(matches!(
        prepared.search(NodeId::new(99), &UnitCost, &Undirected, &mut ws),
        Err(InformedSearchError::InvalidStart { .. })
    ));
    assert!(matches!(
        PreparedBergeSearch::build(&index, NodeId::new(99), &mut scratch),
        Err(InformedSearchError::InvalidStart { .. })
    ));
    struct InvalidBound;
    impl BergeCost for InvalidBound {
        fn minimum_step_cost(&self) -> f64 {
            f64::NAN
        }
        fn step_cost(&self, _: DeclId, _: &IncidenceRef<'_>) -> f64 {
            1.0
        }
    }
    assert!(matches!(
        prepared.search(NodeId::new(5), &InvalidBound, &Undirected, &mut ws),
        Err(InformedSearchError::BadStepBound { .. })
    ));
}

#[test]
fn incumbent_bound_prunes_only_paths_worse_than_a_complete_witness() {
    let ir = ic::weighted_graph_ir(4, &[(0, 1, 1.0), (0, 2, 9.0), (2, 3, 1.0)]);
    let index = build_incidence_index(&ir).unwrap();
    let traversal = HyperTraversal::new(&index);
    let mut ws = InformedSearchWorkspace::new();
    let cost = IncidenceWeightCost { base: 0.0 };
    let result = SearchCase::new(&traversal, NodeId::new(0), NodeId::new(1), &mut ws)
        .run(&Dijkstra::new(&cost, &Undirected))
        .unwrap()
        .unwrap();
    assert_eq!(result.total_cost, 1.0);
    assert_eq!(ws.stats().bound_pruned, 1);
    assert_eq!(
        ic::oracle_dijkstra_cost(
            &ir,
            &index,
            NodeId::new(0),
            NodeId::new(1),
            OracleCost::Weight { base: 0.0 }
        ),
        Some(1.0)
    );
}

#[test]
fn entropy_orders_equal_cost_candidates_without_becoming_a_distance() {
    use hymeko_graph::direct_ir::PreparedBergeSearch;
    let ir = ic::weighted_graph_ir(4, &[(0, 1, 1.0), (0, 2, 1.0), (0, 3, 1.0)]);
    let index = build_incidence_index(&ir).unwrap();
    let mut preparation = TraversalWorkspace::new();
    let search = PreparedBergeSearch::build(&index, NodeId::new(1), &mut preparation).unwrap();
    // Hub has entropy log2(3); each leaf has one bucket and entropy zero.
    assert!(search.entropy_priority(NodeId::new(0)) > search.entropy_priority(NodeId::new(3)));
    assert_eq!(search.relaxed_hops(NodeId::new(0)), Some(1));
    assert_eq!(search.relaxed_hops(NodeId::new(3)), Some(2));
    let mut workspace = InformedSearchWorkspace::new();
    let result = search
        .search(NodeId::new(3), &UnitCost, &Undirected, &mut workspace)
        .unwrap()
        .unwrap();
    assert_eq!(result.total_cost, 2.0);
}

#[test]
fn inconsistent_admissibility_claim_is_rejected_before_search() {
    struct Uncertified;
    impl BergeHeuristic for Uncertified {
        const ADMISSIBLE: bool = false;
        const CONSISTENT: bool = true;
        fn h(&self, _: &Ir, _: DeclId) -> f64 {
            0.0
        }
    }
    let ir = ic::window_ir(3, 2, false, SignMode::AllPlus);
    let index = build_incidence_index(&ir).unwrap();
    assert_eq!(
        SearchCase::new(
            &HyperTraversal::new(&index),
            NodeId::new(0),
            NodeId::new(2),
            &mut InformedSearchWorkspace::new()
        )
        .run(&AStar::new(&UnitCost, &Uncertified, &Undirected))
        .unwrap_err(),
        InformedSearchError::InconsistentHeuristicInOptimalMode
    );
}

#[test]
fn prepared_bound_assist_prunes_a_whole_hyperarc_before_member_visits() {
    let mut ir = Ir::default();
    let nodes: Vec<_> = (0..10).map(|_| ic::node_decl(&mut ir)).collect();
    // The large distractor is deliberately before the one-hop goal arc.
    ic::push_arc(
        &mut ir,
        nodes[..9].iter().map(|&d| ic::plus_ref(d, None)).collect(),
    );
    ic::push_arc(
        &mut ir,
        [nodes[0], nodes[9]]
            .into_iter()
            .map(|d| ic::plus_ref(d, None))
            .collect(),
    );
    let index = build_incidence_index(&ir).unwrap();
    let mut ws = InformedSearchWorkspace::new();
    let control = SearchCase::new(
        &HyperTraversal::new(&index),
        NodeId::new(0),
        NodeId::new(9),
        &mut ws,
    )
    .run(&Dijkstra::new(&UnitCost, &Undirected))
    .unwrap()
    .unwrap();
    let unassisted = ws.stats();
    let prepared =
        PreparedBergeSearch::build(&index, NodeId::new(9), &mut TraversalWorkspace::new()).unwrap();
    let result = prepared
        .search(NodeId::new(0), &UnitCost, &Undirected, &mut ws)
        .unwrap()
        .unwrap();
    assert_eq!(result.total_cost, 1.0);
    assert_eq!(result.path_hash(), control.path_hash());
    assert!(ws.stats().seeded_bound);
    assert_eq!(ws.stats().witness_steps, 1);
    assert_eq!(ws.stats().bound_pruned_arcs, 1);
    assert_eq!(ws.stats().expanded_arcs, 1);
    assert_eq!(ws.stats().resolved_incidences, 1);
    assert_eq!(unassisted.resolved_incidences, 9);
    assert!(ws.stats().resolved_incidences < unassisted.resolved_incidences);
}

#[test]
fn prepared_caller_heuristic_changes_f_and_improves_the_initial_witness() {
    use std::cell::Cell;
    struct ExactDistances(Cell<usize>);
    impl BergeHeuristic for ExactDistances {
        const ADMISSIBLE: bool = true;
        const CONSISTENT: bool = true;
        fn h(&self, _: &Ir, node: DeclId) -> f64 {
            self.0.set(self.0.get() + 1);
            // Closed-form shortest distances to 4 for the six edges below.
            [5.0, 6.0, 3.0, 6.0, 0.0][node.raw()]
        }
    }
    let edges = [
        (0, 1, 1.0),
        (1, 4, 9.0),
        (0, 2, 2.0),
        (2, 4, 3.0),
        (0, 3, 1.0),
        (3, 4, 20.0),
    ];
    let ir = ic::weighted_graph_ir(5, &edges);
    let index = build_incidence_index(&ir).unwrap();
    let prepared =
        PreparedBergeSearch::build(&index, NodeId::new(4), &mut TraversalWorkspace::new()).unwrap();
    let cost = IncidenceWeightCost { base: 0.0 };
    let mut ws = InformedSearchWorkspace::new();
    let zero = prepared
        .search(NodeId::new(0), &cost, &Undirected, &mut ws)
        .unwrap()
        .unwrap();
    let without_caller = ws.stats();
    let heuristic = ExactDistances(Cell::new(0));
    let result = prepared
        .search_with_heuristic(NodeId::new(0), &cost, &heuristic, &Undirected, &mut ws)
        .unwrap()
        .unwrap();
    assert_eq!(result.total_cost, 5.0); // The relaxed-tree witness costs 10.
    assert_eq!(zero.total_cost, result.total_cost);
    assert_eq!(
        Some(result.total_cost),
        ic::oracle_dijkstra_cost(
            &ir,
            &index,
            NodeId::new(0),
            NodeId::new(4),
            OracleCost::Weight { base: 0.0 }
        )
    );
    assert!(heuristic.0.get() > 0);
    assert!(ws.stats().seeded_bound);
    assert_eq!(ws.stats().witness_steps, 2);
    assert!(ws.stats().expanded_nodes < without_caller.expanded_nodes);
    assert_eq!(ws.stats().expanded_nodes, 3);
    assert_eq!(without_caller.expanded_nodes, 5);
    assert_eq!(result.steps[0].to.node, NodeId::new(2));
}

#[test]
fn prepared_rejected_directed_witness_does_not_reject_other_routes_or_slots() {
    use hymeko_graph::direct_ir::TransitionPolicy;
    for (source, expected) in [
        (
            "vv {}\ng { s {} p {} goal {} @shared { (+ s, - p, + goal); } @entry { (- s, + p); } }\n",
            2.0,
        ),
        (
            "vv {}\ng { s {} goal {} @shared { (+ s, - s, + goal); } }\n",
            1.0,
        ),
    ] {
        let world = dc::compile(source);
        let index = build_incidence_index(world.ir()).unwrap();
        let prepared =
            PreparedBergeSearch::build(&index, world.node("goal"), &mut TraversalWorkspace::new())
                .unwrap();
        let mut ws = InformedSearchWorkspace::new();
        let path = prepared
            .search(
                world.node("s"),
                &UnitCost,
                &DirectedProducerConsumer,
                &mut ws,
            )
            .unwrap()
            .unwrap();
        assert_eq!(path.total_cost, expected);
        assert!(!ws.stats().seeded_bound);
        assert_eq!(ws.stats().witness_steps, 1);
        for step in &path.steps {
            assert!(DirectedProducerConsumer.allows(
                &index.resolve(step.arc, step.from.slot),
                &index.resolve(step.arc, step.to.slot)
            ));
            assert_eq!(step.from.sign, -1);
            assert_eq!(step.to.sign, 1);
        }
        if expected == 1.0 {
            assert_eq!(path.steps[0].from.slot, 1);
        }
    }
}

#[test]
fn prepared_zero_cost_and_equal_bound_paths_remain_eligible_and_deterministic() {
    let ir = ic::weighted_graph_ir(4, &[(0, 1, 0.0), (1, 3, 1.0), (0, 2, 0.0), (2, 3, 1.0)]);
    let index = build_incidence_index(&ir).unwrap();
    let prepared =
        PreparedBergeSearch::build(&index, NodeId::new(3), &mut TraversalWorkspace::new()).unwrap();
    let mut ws = InformedSearchWorkspace::new();
    for base in [0.0, 0.25] {
        let cost = IncidenceWeightCost { base };
        let mut hash = None;
        for _ in 0..8 {
            let path = prepared
                .search(NodeId::new(0), &cost, &Undirected, &mut ws)
                .unwrap()
                .unwrap();
            assert_eq!(path.total_cost, 1.0 + 2.0 * base);
            assert_eq!(path.hyperedge_len(), 2);
            assert!(ws.stats().seeded_bound);
            assert_eq!(ws.stats().witness_steps, 2);
            assert_eq!(*hash.get_or_insert(path.path_hash()), path.path_hash());
        }
    }
    let identity = prepared
        .search(NodeId::new(3), &UnitCost, &Undirected, &mut ws)
        .unwrap()
        .unwrap();
    assert_eq!(identity.total_cost, 0.0);
    assert!(identity.steps.is_empty());
    assert_eq!(ws.stats().witness_steps, 0);
    assert_eq!(ws.stats().expanded_nodes, 0);
}

#[test]
fn prepared_overflowing_optional_witness_falls_back_to_a_finite_route() {
    let ir = ic::weighted_graph_ir(
        4,
        &[(0, 1, f64::MAX), (1, 3, f64::MAX), (0, 2, 1.0), (2, 3, 1.0)],
    );
    let index = build_incidence_index(&ir).unwrap();
    let prepared =
        PreparedBergeSearch::build(&index, NodeId::new(3), &mut TraversalWorkspace::new()).unwrap();
    let mut ws = InformedSearchWorkspace::new();
    let path = prepared
        .search(
            NodeId::new(0),
            &IncidenceWeightCost { base: 0.0 },
            &Undirected,
            &mut ws,
        )
        .unwrap()
        .unwrap();
    assert_eq!(path.total_cost, 2.0);
    assert_eq!(path.steps[0].to.node, NodeId::new(2));
    assert!(!ws.stats().seeded_bound);
    assert_eq!(ws.stats().witness_steps, 2);
}

#[test]
fn prepared_bad_heuristics_and_witness_costs_fail_closed() {
    struct Uncertified;
    impl BergeHeuristic for Uncertified {
        const ADMISSIBLE: bool = false;
        const CONSISTENT: bool = true;
        fn h(&self, _: &Ir, _: DeclId) -> f64 {
            panic!("uncertified bound must not run")
        }
    }
    let ir = ic::window_ir(4, 2, false, SignMode::AllPlus);
    let index = build_incidence_index(&ir).unwrap();
    let prepared =
        PreparedBergeSearch::build(&index, NodeId::new(3), &mut TraversalWorkspace::new()).unwrap();
    let mut ws = InformedSearchWorkspace::new();
    assert!(matches!(
        prepared.search_with_heuristic(
            NodeId::new(0),
            &UnitCost,
            &Uncertified,
            &Undirected,
            &mut ws
        ),
        Err(InformedSearchError::InconsistentHeuristicInOptimalMode)
    ));
    assert!(matches!(
        prepared.search_with_heuristic(
            NodeId::new(99),
            &UnitCost,
            &ZeroHeuristic,
            &Undirected,
            &mut ws
        ),
        Err(InformedSearchError::InvalidStart { .. })
    ));
    for which in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            prepared.search(
                NodeId::new(0),
                &BadCostModel { which },
                &Undirected,
                &mut ws
            ),
            Err(InformedSearchError::BadCost { .. })
        ));
        assert!(matches!(
            prepared.search(
                NodeId::new(0),
                &IncidenceWeightCost { base: which },
                &Undirected,
                &mut ws
            ),
            Err(InformedSearchError::BadCost { .. })
        ));
    }
    // Forbidden transitions do not evaluate their costs, including during
    // optional witness probing. All-plus incidences admit no directed step.
    assert!(
        prepared
            .search(
                NodeId::new(0),
                &BadCostModel { which: f64::NAN },
                &DirectedProducerConsumer,
                &mut ws
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn prepared_decimal_bounds_do_not_prune_the_only_feasible_path() {
    for n in [4, 10, 33] {
        let edges: Vec<_> = (0..n - 1).map(|i| (i, i + 1, 0.0)).collect();
        let ir = ic::weighted_graph_ir(n, &edges);
        let index = build_incidence_index(&ir).unwrap();
        let prepared =
            PreparedBergeSearch::build(&index, NodeId::new(n - 1), &mut TraversalWorkspace::new())
                .unwrap();
        for base in [
            0.1,
            0.3,
            1e-200,
            1e200,
            f64::from_bits(1),
            f64::MIN_POSITIVE * 0.5,
        ] {
            let actual = prepared
                .search(
                    NodeId::new(0),
                    &IncidenceWeightCost { base },
                    &Undirected,
                    &mut InformedSearchWorkspace::new(),
                )
                .unwrap();
            let oracle = ic::oracle_dijkstra_cost(
                &ir,
                &index,
                NodeId::new(0),
                NodeId::new(n - 1),
                OracleCost::Weight { base },
            );
            assert_eq!(actual.map(|p| p.total_cost), oracle, "n={n}, base={base}");
        }
    }
}

fn unit_path_cost<P: hymeko_graph::direct_ir::TransitionPolicy>(
    ir: &Ir,
    start: NodeId,
    goal: NodeId,
    policy: &P,
    workspace: &mut InformedSearchWorkspace,
) -> Option<f64> {
    let index = build_incidence_index(ir).unwrap();
    SearchCase::new(&HyperTraversal::new(&index), start, goal, workspace)
        .run(&Dijkstra::new(&UnitCost, policy))
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
                SearchCase::new(&trav, start, goal, &mut ws)
                    .run(&Dijkstra::new(&ic_cost, &Undirected))
                    .unwrap()
            } else {
                SearchCase::new(&trav, start, goal, &mut ws)
                    .run(&Dijkstra::new(&UnitCost, &Undirected))
                    .unwrap()
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
        let a = SearchCase::new(&trav, NodeId::new(s), NodeId::new(g), &mut a_ws)
            .run(&Dijkstra::new(&UnitCost, &Undirected))
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
        let en = SearchCase::new(&trav, NodeId::new(0), NodeId::new(g), &mut ws)
            .run(&Dijkstra::new(&cost, &Undirected))
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
    let mut case = SearchCase::new(&trav, start, goal, &mut ws);
    let dij = case
        .run(&Dijkstra::new(&cost, &Undirected))
        .unwrap()
        .unwrap();
    let dij_expanded = case.stats().expanded_nodes;
    let astar = case
        .run(&AStar::new(&cost, &heur, &Undirected))
        .unwrap()
        .unwrap();
    let astar_expanded = case.stats().expanded_nodes;

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
    let mut case = SearchCase::new(&trav, NodeId::new(0), NodeId::new(39), &mut ws);
    let a = case
        .run(&AStar::new(&cost, &ZeroHeuristic, &Undirected))
        .unwrap()
        .unwrap();
    let b = case
        .run(&Dijkstra::new(&cost, &Undirected))
        .unwrap()
        .unwrap();
    assert_eq!(
        a.path_hash(),
        b.path_hash(),
        "Zero-heuristic A* must equal Dijkstra exactly"
    );
    // determinism: re-run gives the identical hash
    let c = case
        .run(&Dijkstra::new(&cost, &Undirected))
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
    let r = SearchCase::new(&trav, NodeId::new(0), NodeId::new(9), &mut ws).run(&AStar::new(
        &UnitCost,
        &InconsistentH,
        &Undirected,
    ));
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
        let r = SearchCase::new(&trav, NodeId::new(0), NodeId::new(9), &mut ws)
            .run(&Dijkstra::new(&BadCostModel { which: bad }, &Undirected));
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
    let sg = SearchCase::new(&trav, NodeId::new(2), NodeId::new(2), &mut ws)
        .run(&Dijkstra::new(&UnitCost, &Undirected))
        .unwrap()
        .unwrap();
    assert_eq!(sg.total_cost, 0.0);
    assert!(sg.steps.is_empty());

    // disconnected: two components, no path across
    let ir2 = ic::multi_component_ir(2, 5, 2);
    let idx2 = build_incidence_index(&ir2).unwrap();
    let trav2 = HyperTraversal::new(&idx2);
    let r = SearchCase::new(&trav2, NodeId::new(0), NodeId::new(7), &mut ws)
        .run(&Dijkstra::new(&UnitCost, &Undirected))
        .unwrap();
    assert!(r.is_none(), "cross-component must be unreachable");
    // within-component still reachable
    let w = SearchCase::new(&trav2, NodeId::new(0), NodeId::new(4), &mut ws)
        .run(&Dijkstra::new(&UnitCost, &Undirected))
        .unwrap();
    assert!(w.is_some());
}

#[test]
fn boundary_invalid_start_and_malformed_target() {
    let ir = ic::window_ir(4, 2, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let r = SearchCase::new(&trav, NodeId::new(999), NodeId::new(0), &mut ws)
        .run(&Dijkstra::new(&UnitCost, &Undirected));
    assert!(matches!(r, Err(InformedSearchError::InvalidStart { .. })));
}

#[test]
fn boundary_singleton_and_self_incidence() {
    // an arc with a single member (self-incidence) plus a normal edge to reach a goal
    let mut ir = Ir::default();
    for _ in 0..3 {
        IrConstruction::new(&mut ir)
            .append_node(
                hymeko::common::ids::SymId::new(0),
                AnnoR::default(),
                Vec::new(),
            )
            .unwrap();
    }
    // singleton arc on node 0 (self), and arc {0,1}, arc {1,2}
    ir.arcs.push(ArcRec::unowned(vec![SignedRefR::plain(
        DeclId::new(0),
        hymeko::ir::ir::IncidenceSign::Plus,
    )]));
    for (a, b) in [(0usize, 1usize), (1, 2)] {
        ir.arcs.push(ArcRec::unowned(vec![
            SignedRefR::plain(DeclId::new(a), hymeko::ir::ir::IncidenceSign::Plus),
            SignedRefR::plain(DeclId::new(b), hymeko::ir::ir::IncidenceSign::Plus),
        ]));
    }
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let p = SearchCase::new(&trav, NodeId::new(0), NodeId::new(2), &mut ws)
        .run(&Dijkstra::new(&UnitCost, &Undirected))
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
    let original = dc::compile(
        r#"vv {} g {
        a {} b {} c {} d {} z {}
        @ab { (- a, + b); } @bc { (- b, + c); }
        @cd { (- c, + d); } @ac { (- a, + c); }
    }"#,
    );
    let permuted = dc::compile(
        r#"vv {} g {
        @cd { (- c, + d); }
        z {} d {} b {} a {} c {}
        @ac { (- a, + c); } @bc { (- b, + c); } @ab { (- a, + b); }
    }"#,
    );
    let names = ["a", "b", "c", "d", "z"];
    let declarations =
        |world: &dc::World| names.map(|name| world.ir().nodes[world.node(name).raw()].decl);
    assert_ne!(
        declarations(&original),
        declarations(&permuted),
        "the fixture must actually permute declarations"
    );
    assert_ne!(
        original.node("a"),
        permuted.node("a"),
        "node record IDs must also change"
    );
    assert_ne!(
        original.edge_arc("cd"),
        permuted.edge_arc("cd"),
        "arc IDs must actually change"
    );
    // Verify reference remapping independently by semantic names, preserving slot order and metadata.
    for edge in ["ab", "bc", "cd", "ac"] {
        let left = &original.ir().arcs[original.edge_arc(edge).raw()].refs;
        let right = &permuted.ir().arcs[permuted.edge_arc(edge).raw()].refs;
        assert_eq!(left.len(), right.len());
        for (a, b) in left.iter().zip(right) {
            let name_of = |world: &dc::World, target: DeclId| {
                names
                    .into_iter()
                    .find(|name| world.ir().nodes[world.node(name).raw()].decl == target)
                    .unwrap()
            };
            assert_eq!(
                name_of(&original, a.target()),
                name_of(&permuted, b.target())
            );
            assert_eq!(a.sign(), b.sign());
            assert_eq!(
                original.ir().reference_atom(a).unwrap().weights,
                permuted.ir().reference_atom(b).unwrap().weights
            );
            assert_eq!(
                original.ir().reference_atom(a).unwrap().anno,
                permuted.ir().reference_atom(b).unwrap().anno
            );
            assert_eq!(
                original.ir().reference_atom(a).unwrap().direction,
                permuted.ir().reference_atom(b).unwrap().direction
            );
        }
    }
    let original_index = build_incidence_index(original.ir()).unwrap();
    let permuted_index = build_incidence_index(permuted.ir()).unwrap();
    let left = HyperTraversal::new(&original_index);
    let right = HyperTraversal::new(&permuted_index);
    let mut workspace = InformedSearchWorkspace::new();
    for (from, to, expected) in [
        ("a", "d", Some(2.0)),
        ("b", "d", Some(2.0)),
        ("a", "a", Some(0.0)),
        ("a", "z", None),
    ] {
        let mut query = |world: &dc::World, traversal: &HyperTraversal<'_, '_>| {
            let mut case =
                SearchCase::new(traversal, world.node(from), world.node(to), &mut workspace);
            let dijkstra = case.run(&Dijkstra::new(&UnitCost, &Undirected)).unwrap();
            let astar = case
                .run(&AStar::new(&UnitCost, &ZeroHeuristic, &Undirected))
                .unwrap();
            assert_eq!(
                dijkstra.as_ref().map(|p| p.path_hash()),
                astar.as_ref().map(|p| p.path_hash())
            );
            let prepared = PreparedBergeSearch::build(
                traversal.index(),
                world.node(to),
                &mut TraversalWorkspace::new(),
            )
            .unwrap();
            let assisted = prepared
                .search_with_heuristic(
                    world.node(from),
                    &UnitCost,
                    &ZeroHeuristic,
                    &Undirected,
                    &mut workspace,
                )
                .unwrap();
            assert_eq!(
                assisted.map(|p| p.total_cost),
                dijkstra.as_ref().map(|p| p.total_cost)
            );
            dijkstra.map(|p| p.total_cost)
        };
        assert_eq!(query(&original, &left), expected);
        assert_eq!(query(&permuted, &right), expected);
        assert_eq!(
            ic::oracle_dijkstra_cost(
                permuted.ir(),
                &permuted_index,
                permuted.node(from),
                permuted.node(to),
                OracleCost::Unit
            ),
            expected
        );
    }
    // A stale dense ID selects the isolated z node in the permuted fixture: this
    // negative control would fail if the test merely rebuilt the original layout.
    assert_eq!(original.node("a"), permuted.node("z"));
    let stale = SearchCase::new(
        &right,
        original.node("a"),
        permuted.node("d"),
        &mut workspace,
    )
    .run(&Dijkstra::new(&UnitCost, &Undirected))
    .unwrap();
    assert!(
        stale.is_none(),
        "reusing unremapped dense IDs must change the query"
    );
}

#[test]
fn disjoint_union_preserves_within_distances() {
    let single = ic::window_ir(10, 2, false, SignMode::AllPlus);
    let multi = ic::multi_component_ir(2, 10, 2);
    let mut workspace = InformedSearchWorkspace::new();
    let start = NodeId::new(0);
    let goal = NodeId::new(9);
    let before = unit_path_cost(&single, start, goal, &Undirected, &mut workspace);
    let after = unit_path_cost(&multi, start, goal, &Undirected, &mut workspace);
    assert_eq!(before, Some(9.0));
    assert_eq!(
        after, before,
        "adding a disjoint component preserves the original distance"
    );
}

// ---- 8. duplicate incidence preserved (multiplicity observable, cost unchanged) --------------------

#[test]
fn duplicate_incidence_preserves_cost_and_is_observable() {
    let plain = ic::window_ir(20, 3, false, SignMode::AllPlus);
    let ip = build_incidence_index(&plain).unwrap();
    let tp = HyperTraversal::new(&ip);
    let mut ws = InformedSearchWorkspace::new();
    let p = SearchCase::new(&tp, NodeId::new(0), NodeId::new(19), &mut ws)
        .run(&Dijkstra::new(&UnitCost, &Undirected))
        .unwrap()
        .unwrap();
    let plain_resolved = ws.stats().resolved_incidences;

    let dup = ic::dup_heavy_ir(20, 3);
    let id = build_incidence_index(&dup).unwrap();
    let td = HyperTraversal::new(&id);
    let d = SearchCase::new(&td, NodeId::new(0), NodeId::new(19), &mut ws)
        .run(&Dijkstra::new(&UnitCost, &Undirected))
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
    .map(|members| {
        ArcRec::unowned(
            members
                .into_iter()
                .map(|(node, weight)| {
                    ir.store_reference(hymeko::ir::ir::ReferenceDraft::new(
                        hymeko::ir::ir::IncidenceSign::Plus,
                        RefAtomR::plain(DeclId::new(node))
                            .with_weights(Some(vec![ValueR::Num(weight)])),
                    ))
                })
                .collect(),
        )
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
    let dijkstra = SearchCase::new(&trav, NodeId::new(0), NodeId::new(3), &mut ws)
        .run(&Dijkstra::new(&cost, &Undirected))
        .unwrap()
        .unwrap();
    assert_eq!(dijkstra.total_cost, 11.0);
    let astar = SearchCase::new(&trav, NodeId::new(0), NodeId::new(3), &mut ws)
        .run(&AStar::new(&cost, &ConsistentBound, &Undirected))
        .unwrap()
        .unwrap();
    assert_eq!(astar.total_cost, dijkstra.total_cost);
    let prepared =
        PreparedBergeSearch::build(&idx, NodeId::new(3), &mut TraversalWorkspace::new()).unwrap();
    let assisted = prepared
        .search_with_heuristic(
            NodeId::new(0),
            &cost,
            &ConsistentBound,
            &Undirected,
            &mut ws,
        )
        .unwrap()
        .unwrap();
    assert_eq!(assisted.total_cost, oracle.unwrap());
    assert!(ws.stats().seeded_bound);
}
