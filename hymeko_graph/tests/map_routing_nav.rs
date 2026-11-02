//! P3.1 — map/routing on the direct canonical-Ir path: coordinates + weights live in the `Ir`
//! (`DeclNode.anno.value`, incidence weights); a test-side `PositionProvider` reads them back. The
//! discrete and combined bounds use the library's `HopHeuristic` and `MaxHeuristic`.
//! Optimality is anchored on the frozen linear-scan Dijkstra oracle; admissibility and
//! consistency are checked exhaustively on a small map. No parallel map object; no second engine.

use hymeko_graph::direct_ir::{HopHeuristic, MaxHeuristic};
use ic::MapPositions;
use ic::search::{AStar, Dijkstra, SearchCase};
use std::collections::{HashMap, HashSet};

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::{IncidenceIndex, build_incidence_index};
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    BergeHeuristic, HyperTraversal, IncidenceWeightCost, InformedSearchWorkspace, MetricHeuristic,
    Undirected,
};

#[path = "informed_common/mod.rs"]
mod ic;

use ic::OracleCost;

// ---- coordinates read straight from the canonical Ir -----------------------------------------------

// ---- discrete Berge-hop admissible heuristic (min remaining hops * global min transition cost) ------

// ---- combined admissible heuristic: max(metric, discrete) ------------------------------------------

fn small_map() -> (Ir, usize, usize) {
    let (w, h) = (16usize, 16usize);
    let blocked = ic::map_obstacles(w, h);
    (ic::map_grid_ir(w, h, &blocked), w, h)
}

fn cost() -> IncidenceWeightCost {
    IncidenceWeightCost { base: 0.0 }
}

#[test]
fn hop_heuristic_handles_zero_missing_disconnected_and_invalid_bounds() {
    let ir = ic::multi_component_ir(2, 3, 2);
    let index = build_incidence_index(&ir).unwrap();
    let hops = ic::hop_distances_from(&ir, &index, NodeId::new(0));
    let heuristic = HopHeuristic::new(&hops, 0.5);
    for (raw, expected) in [(0, 0.0), (1, 0.5), (2, 1.0), (3, 0.0), (99, 0.0)] {
        assert_eq!(heuristic.h(&ir, DeclId::new(raw)), expected);
    }
    assert_eq!(HopHeuristic::new(&hops, 0.0).h(&ir, DeclId::new(2)), 0.0);
    assert_eq!(HopHeuristic::new(&[], 1.0).h(&ir, DeclId::new(0)), 0.0);
    for invalid in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(std::panic::catch_unwind(|| HopHeuristic::new(&hops, invalid)).is_err());
    }
    assert!(std::panic::catch_unwind(|| HopHeuristic::new(&[2], f64::MAX)).is_err());
}

#[test]
fn max_heuristic_propagates_both_guarantees_and_keeps_zero_tiebreak() {
    struct Bound<const A: bool, const C: bool>(f64);
    impl<const A: bool, const C: bool> BergeHeuristic for Bound<A, C> {
        const ADMISSIBLE: bool = A;
        const CONSISTENT: bool = C;
        fn h(&self, _: &Ir, _: DeclId) -> f64 {
            self.0
        }
        fn tiebreak(&self, _: &Ir, _: DeclId) -> u32 {
            7
        }
    }
    fn guarantees<H: BergeHeuristic>(_: &H) -> (bool, bool) {
        (H::ADMISSIBLE, H::CONSISTENT)
    }
    let a = Bound::<true, true>(2.0);
    let b = Bound::<true, true>(3.0);
    let inconsistent = Bound::<true, false>(4.0);
    let inadmissible = Bound::<false, true>(5.0);
    let max = MaxHeuristic::new(&a, &b);
    assert_eq!(guarantees(&max), (true, true));
    assert_eq!(
        guarantees(&MaxHeuristic::new(&a, &inconsistent)),
        (true, false)
    );
    assert_eq!(
        guarantees(&MaxHeuristic::new(&inconsistent, &a)),
        (true, false)
    );
    assert_eq!(
        guarantees(&MaxHeuristic::new(&a, &inadmissible)),
        (false, true)
    );
    let ir = Ir::default();
    assert_eq!(max.h(&ir, DeclId::new(0)), 3.0);
    assert_eq!(MaxHeuristic::new(&b, &a).h(&ir, DeclId::new(0)), 3.0);
    assert_eq!(max.tiebreak(&ir, DeclId::new(0)), 0);
}

// ---- admissibility + consistency, checked exhaustively on the small map (§5, §12) ------------------

#[test]
fn metric_and_discrete_admissible_and_consistent() {
    let (ir, w, h) = small_map();
    let idx = build_incidence_index(&ir).unwrap();
    let goal = NodeId::new(w * h - 1);
    let provider = MapPositions::from_ir(&ir);
    let metric = MetricHeuristic::new(&provider, ir.nodes[goal.raw()].decl);
    let hops = ic::hop_distances_from(&ir, &idx, goal);
    let discrete = HopHeuristic::new(&hops, ic::map_min_edge_cost(&ir));
    let combined = MaxHeuristic::new(&metric, &discrete);
    // h(goal) == 0, h >= 0, and admissibility h(n) <= oracle_dist(n, goal) for every reachable n.
    assert_eq!(metric.h(&ir, ir.nodes[goal.raw()].decl), 0.0);
    assert_eq!(discrete.h(&ir, ir.nodes[goal.raw()].decl), 0.0);
    assert_eq!(combined.h(&ir, ir.nodes[goal.raw()].decl), 0.0);
    for n in 0..(w * h) {
        let start = NodeId::new(n);
        let od = ic::oracle_dijkstra_cost(&ir, &idx, start, goal, OracleCost::Weight { base: 0.0 });
        let d = ir.nodes[start.raw()].decl;
        let (hm, hd) = (metric.h(&ir, d), discrete.h(&ir, d));
        assert!(hm >= 0.0 && hd >= 0.0, "non-negative");
        if let Some(dist) = od {
            assert!(
                combined.h(&ir, d) <= dist + 1e-9,
                "combined admissible at {n}"
            );
            assert!(
                hm <= dist + 1e-9,
                "metric admissible at {n}: {hm} <= {dist}"
            );
            assert!(
                hd <= dist + 1e-9,
                "discrete admissible at {n}: {hd} <= {dist}"
            );
        }
    }
    // consistency: h(u) <= cost(u,v) + h(v) for every legal Berge step (edge) of the grid.
    for a in &ir.arcs {
        for i in 0..a.refs.len() {
            for j in 0..a.refs.len() {
                if i == j {
                    continue;
                }
                let (u, v) = (a.refs[i].target(), a.refs[j].target());
                let c = ic::first_num_weight_pub(&ir, &a.refs[j]);
                for hh in [
                    metric.h(&ir, u) - metric.h(&ir, v),
                    discrete.h(&ir, u) - discrete.h(&ir, v),
                    combined.h(&ir, u) - combined.h(&ir, v),
                ] {
                    assert!(
                        hh <= c + 1e-9,
                        "consistency u={} v={} : {hh} <= {c}",
                        u.raw(),
                        v.raw()
                    );
                }
            }
        }
    }
}

// ---- optimal-cost parity: A*(each heuristic) == Dijkstra == oracle (§5, §12) ------------------------

fn dijkstra_oracle(ir: &Ir, idx: &IncidenceIndex<'_>, s: NodeId, g: NodeId) -> Option<f64> {
    ic::oracle_dijkstra_cost(ir, idx, s, g, OracleCost::Weight { base: 0.0 })
}

#[test]
fn astar_metric_discrete_combined_all_optimal() {
    let (ir, w, h) = small_map();
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let provider = MapPositions::from_ir(&ir);
    for &g in &[w * h - 1, w * h - 2, (h / 2) * w + w / 2] {
        let goal = NodeId::new(g);
        let metric = MetricHeuristic::new(&provider, ir.nodes[goal.raw()].decl);
        let hops = ic::hop_distances_from(&ir, &idx, goal);
        let discrete = HopHeuristic::new(&hops, ic::map_min_edge_cost(&ir));
        let combined = MaxHeuristic::new(&metric, &discrete);
        for s in [0usize, 5, w + 3, (h - 1) * w, w * h / 2] {
            let start = NodeId::new(s);
            let oracle = dijkstra_oracle(&ir, &idx, start, goal);
            let mut case = SearchCase::new(&trav, start, goal, &mut ws);
            let dij = case.run(&Dijkstra::new(&cost(), &Undirected)).unwrap();
            let am = case
                .run(&AStar::new(&cost(), &metric, &Undirected))
                .unwrap();
            let ad = case
                .run(&AStar::new(&cost(), &discrete, &Undirected))
                .unwrap();
            let ac = case
                .run(&AStar::new(&cost(), &combined, &Undirected))
                .unwrap();
            let q = |p: &Option<hymeko_graph::direct_ir::WeightedBergePath>| {
                p.as_ref().map(|x| (x.total_cost * 1e6) as i64)
            };
            let base = oracle.map(|x| (x * 1e6) as i64);
            assert_eq!(q(&dij), base, "dijkstra {s}->{g}");
            assert_eq!(q(&am), base, "A*(metric) {s}->{g}");
            assert_eq!(q(&ad), base, "A*(discrete) {s}->{g}");
            assert_eq!(q(&ac), base, "A*(combined) {s}->{g}");
            // every returned step is a valid alternating Berge step (nodes present in arcs)
            if let Some(p) = &am {
                assert!(
                    p.nodes().count() == p.hyperedge_len() + 1,
                    "path step count"
                );
            }
        }
    }
}

// ---- blocked region -> unreachable, typed None (§12) ----------------------------------------------

#[test]
fn blocked_region_is_unreachable() {
    let (w, h) = (16usize, 16usize);
    // fully wall off node (w*h-1)'s row-1 & col-1 neighbours by blocking the whole last row & column
    let mut blocked = HashSet::new();
    let corner = w * h - 1;
    for c in 0..w {
        if (h - 1) * w + c != corner {
            blocked.insert((h - 1) * w + c);
        }
    }
    for r in 0..h {
        if r * w + (w - 1) != corner {
            blocked.insert(r * w + (w - 1));
        }
    }
    let ir = ic::map_grid_ir(w, h, &blocked);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let p = SearchCase::new(&trav, NodeId::new(0), NodeId::new(corner), &mut ws)
        .run(&Dijkstra::new(&cost(), &Undirected))
        .unwrap();
    assert!(p.is_none(), "walled-off corner is unreachable (typed None)");
}

// ---- transport hyperedge: discrete stays admissible+optimal where metric would not (§4, §5) --------

#[test]
fn transport_multiway_discrete_optimal() {
    let (w, h) = (16usize, 16usize);
    let blocked = HashSet::new();
    let ir = ic::map_grid_with_transport_ir(w, h, &blocked, 4, 0.5); // fare 0.5 < euclidean between stations
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let goal = NodeId::new(w * h - 1);
    let hops = ic::hop_distances_from(&ir, &idx, goal);
    let discrete = HopHeuristic::new(&hops, ic::map_min_edge_cost(&ir));
    for s in [0usize, 5, w * h / 2] {
        let start = NodeId::new(s);
        let oracle = dijkstra_oracle(&ir, &idx, start, goal);
        let ad = SearchCase::new(&trav, start, goal, &mut ws)
            .run(&AStar::new(&cost(), &discrete, &Undirected))
            .unwrap();
        let q = ad.as_ref().map(|x| (x.total_cost * 1e6) as i64);
        assert_eq!(
            q,
            oracle.map(|x| (x * 1e6) as i64),
            "A*(discrete) optimal with a multiway transport edge"
        );
    }
}

// ---- measurement: warm A* per heuristic + reuse + discrete-prep break-even (--nocapture) -----------

#[test]
#[ignore = "performance benchmark; opt in with --ignored --exact"]
fn measure_map_routing() {
    println!("\n# P3.1 map routing (release medians; warm = post-index, reused workspace)");
    println!(
        "{:<14} {:>6} {:>8} {:>9} {:>11} {:>11} {:>11} {:>11} {:>11}",
        "map", "V", "E", "nnz", "dijkstra", "A*_metric", "A*_disc", "A*_comb", "disc_prep"
    );
    for (w, h) in [(32usize, 32usize), (128, 128)] {
        let blocked = ic::map_obstacles(w, h);
        let ir = ic::map_grid_ir(w, h, &blocked);
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let v = ir.decl_nodes.len();
        let e = ir.arcs.len();
        let nnz: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        let (start, goal) = (NodeId::new(0), NodeId::new(v - 1));
        let provider = MapPositions::from_ir(&ir);
        let metric = MetricHeuristic::new(&provider, ir.nodes[goal.raw()].decl);
        let (prep_us, _, _) = ic::median_us(9, || {
            let _ = ic::hop_distances_from(&ir, &idx, goal);
        });
        let hops = ic::hop_distances_from(&ir, &idx, goal);
        let discrete = HopHeuristic::new(&hops, ic::map_min_edge_cost(&ir));
        let combined = MaxHeuristic::new(&metric, &discrete);
        // ONE reused workspace across all timed queries (zero-alloc warm target)
        let mut ws = InformedSearchWorkspace::new();
        let (dij, _, _) = ic::median_us(20, || {
            let _ = SearchCase::new(&trav, start, goal, &mut ws)
                .run(&Dijkstra::new(&cost(), &Undirected))
                .unwrap();
        });
        let (mtr, _, _) = ic::median_us(20, || {
            let _ = SearchCase::new(&trav, start, goal, &mut ws)
                .run(&AStar::new(&cost(), &metric, &Undirected))
                .unwrap();
        });
        let (dsc, _, _) = ic::median_us(20, || {
            let _ = SearchCase::new(&trav, start, goal, &mut ws)
                .run(&AStar::new(&cost(), &discrete, &Undirected))
                .unwrap();
        });
        let (cmb, _, _) = ic::median_us(20, || {
            let _ = SearchCase::new(&trav, start, goal, &mut ws)
                .run(&AStar::new(&cost(), &combined, &Undirected))
                .unwrap();
        });
        println!(
            "{:<14} {v:>6} {e:>8} {nnz:>9} {dij:>11.2} {mtr:>11.2} {dsc:>11.2} {cmb:>11.2} {prep_us:>11.2}",
            format!("grid{w}x{h}")
        );
    }
    let _ = HashMap::<u8, u8>::new();
}
