//! P3.2 §3 — heuristic diagnosis with the engine work counters (`SearchStats`, incl. the added
//! `heuristic_evals`/`reopened`). Dijkstra vs Euclidean/Manhattan/discrete A*, time + counters together,
//! on a uniform grid and a serpentine corridor. Manhattan is admissible only where every step covers one
//! coordinate unit at cost >= its per-coordinate minimum (proven on the small fixture). All variants stay
//! optimal; the counters make "prunes fewer states" vs "weak heuristic" distinguishable.

use std::collections::HashSet;

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    BergeHeuristic, HyperTraversal, IncidenceWeightCost, InformedSearchWorkspace, MetricHeuristic,
    PositionProvider, Undirected, astar_berge_path, dijkstra_berge_path,
};

#[path = "informed_common/mod.rs"]
mod ic;
use ic::OracleCost;

struct IrPos {
    p: Vec<Option<[f64; 2]>>,
}
impl IrPos {
    fn new(ir: &Ir) -> Self {
        Self {
            p: (0..ir.decl_nodes.len())
                .map(|d| ic::ir_position(ir, DeclId::new(d)))
                .collect(),
        }
    }
}
impl PositionProvider for IrPos {
    fn position(&self, n: DeclId) -> Option<&[f64]> {
        self.p
            .get(n.raw())
            .and_then(|o| o.as_ref())
            .map(|a| a.as_slice())
    }
}

struct Discrete {
    hops: Vec<u32>,
    minc: f64,
}
impl BergeHeuristic for Discrete {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    fn h(&self, _ir: &Ir, n: DeclId) -> f64 {
        match self.hops.get(n.raw()).copied() {
            Some(u32::MAX) | None => 0.0,
            Some(k) => k as f64 * self.minc,
        }
    }
}

/// Manhattan lower bound scaled by the per-coordinate minimum step cost.
struct Manhattan<'a> {
    pos: &'a IrPos,
    goal: [f64; 2],
    min_coord_step: f64,
}
impl BergeHeuristic for Manhattan<'_> {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    fn h(&self, _ir: &Ir, n: DeclId) -> f64 {
        match self.pos.position(n) {
            Some(p) => {
                ((p[0] - self.goal[0]).abs() + (p[1] - self.goal[1]).abs()) * self.min_coord_step
            }
            None => 0.0,
        }
    }
}

fn cost() -> IncidenceWeightCost {
    IncidenceWeightCost { base: 0.0 }
}

// ---- Manhattan admissibility + consistency, exhaustive on the small fixture (§3, §9) --------------

#[test]
fn manhattan_admissible_and_consistent() {
    let ir = ic::map_grid_ir(16, 16, &HashSet::new());
    let idx = build_incidence_index(&ir).unwrap();
    let goal = NodeId::new(16 * 16 - 1);
    let pos = IrPos::new(&ir);
    let gp = pos.position(ir.nodes[goal.raw()].decl).unwrap();
    let man = Manhattan {
        pos: &pos,
        goal: [gp[0], gp[1]],
        min_coord_step: 1.0,
    };
    for n in 0..(16 * 16) {
        let d = ir.nodes[n].decl;
        let od = ic::oracle_dijkstra_cost(
            &ir,
            &idx,
            NodeId::new(n),
            goal,
            OracleCost::Weight { base: 0.0 },
        );
        if let Some(dist) = od {
            assert!(man.h(&ir, d) <= dist + 1e-9, "manhattan admissible at {n}");
        }
    }
    for a in &ir.arcs {
        for i in 0..a.refs.len() {
            for j in 0..a.refs.len() {
                if i == j {
                    continue;
                }
                let (u, v) = (a.refs[i].target(), a.refs[j].target());
                let c = ic::first_num_weight_pub(&a.refs[j]);
                assert!(
                    man.h(&ir, u) - man.h(&ir, v) <= c + 1e-9,
                    "manhattan consistent"
                );
            }
        }
    }
}

// ---- counters do not change the result; internal consistency (§2, §9) -----------------------------

#[test]
fn instrumented_search_matches_oracle() {
    let ir = ic::map_grid_ir(16, 16, &ic::map_obstacles(16, 16));
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let pos = IrPos::new(&ir);
    let goal = NodeId::new(16 * 16 - 1);
    let metric = MetricHeuristic::new(&pos, ir.nodes[goal.raw()].decl);
    for s in [0usize, 7, 100] {
        let oracle = ic::oracle_dijkstra_cost(
            &ir,
            &idx,
            NodeId::new(s),
            goal,
            OracleCost::Weight { base: 0.0 },
        );
        let p = astar_berge_path(
            &trav,
            NodeId::new(s),
            goal,
            &cost(),
            &metric,
            &Undirected,
            &mut ws,
        )
        .unwrap();
        assert_eq!(
            p.map(|x| (x.total_cost * 1e6) as i64),
            oracle.map(|x| (x * 1e6) as i64)
        );
        let st = ws.stats();
        assert_eq!(st.pops, st.expanded_nodes + st.stale_pops, "pop accounting");
        assert_eq!(st.reopened, 0, "no reopen with a consistent heuristic");
    }
}

// ---- the diagnosis table (release; --nocapture) ---------------------------------------------------

#[test]
#[ignore = "performance benchmark; opt in with --ignored --exact"]
fn measure_heuristic_diagnosis() {
    println!("\n# P3.2 heuristic diagnosis: time (us) + work counters");
    println!(
        "{:<12} {:<10} {:>9} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7} {:>7}",
        "workload", "algo", "us", "exp_n", "exp_arc", "res_inc", "push", "pop", "stale", "h_eval"
    );
    for (name, ir) in [
        (
            "grid128",
            ic::map_grid_ir(128, 128, &ic::map_obstacles(128, 128)),
        ),
        ("corridor64", ic::corridor_map_ir(64, 64)),
    ] {
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let v = ir.decl_nodes.len();
        let (start, goal) = (NodeId::new(0), NodeId::new(v - 1));
        let pos = IrPos::new(&ir);
        let g = pos.position(ir.nodes[goal.raw()].decl).unwrap();
        let gp = [g[0], g[1]];
        let metric = MetricHeuristic::new(&pos, ir.nodes[goal.raw()].decl);
        let man = Manhattan {
            pos: &pos,
            goal: gp,
            min_coord_step: 1.0,
        };
        let hops = ic::hop_distances_from(&ir, &idx, goal);
        let disc = Discrete {
            hops,
            minc: ic::map_min_edge_cost(&ir),
        };
        let mut ws = InformedSearchWorkspace::new();
        let emit = |algo: &str, us: f64, ws: &InformedSearchWorkspace| {
            let st = ws.stats();
            println!(
                "{name:<12} {algo:<10} {us:>9.2} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7} {:>7}",
                st.expanded_nodes,
                st.expanded_arcs,
                st.resolved_incidences,
                st.pushes,
                st.pops,
                st.stale_pops,
                st.heuristic_evals
            );
        };
        let (u1, _, _) = ic::median_us(15, || {
            let _ = dijkstra_berge_path(&trav, start, goal, &cost(), &Undirected, &mut ws).unwrap();
        });
        emit("dijkstra", u1, &ws);
        let (u2, _, _) = ic::median_us(15, || {
            let _ = astar_berge_path(&trav, start, goal, &cost(), &metric, &Undirected, &mut ws)
                .unwrap();
        });
        emit("euclidean", u2, &ws);
        let (u3, _, _) = ic::median_us(15, || {
            let _ =
                astar_berge_path(&trav, start, goal, &cost(), &man, &Undirected, &mut ws).unwrap();
        });
        emit("manhattan", u3, &ws);
        let (u4, _, _) = ic::median_us(15, || {
            let _ =
                astar_berge_path(&trav, start, goal, &cost(), &disc, &Undirected, &mut ws).unwrap();
        });
        emit("discrete", u4, &ws);
    }
}

// ---- counters reset correctly under workspace reuse (§9 mandatory) --------------------------------

#[test]
fn counters_reset_under_reuse() {
    let ir = ic::map_grid_ir(16, 16, &ic::map_obstacles(16, 16));
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let mut ws = InformedSearchWorkspace::new();
    let (s, g) = (NodeId::new(0), NodeId::new(16 * 16 - 1));
    let _ = dijkstra_berge_path(&trav, s, g, &cost(), &Undirected, &mut ws).unwrap();
    let first = ws.stats();
    // a second identical query on the reused workspace must produce IDENTICAL stats (a fresh reset),
    // not accumulated ones — this is what an omitted counter reset would break.
    let _ = dijkstra_berge_path(&trav, s, g, &cost(), &Undirected, &mut ws).unwrap();
    let second = ws.stats();
    assert_eq!(
        first.expanded_nodes, second.expanded_nodes,
        "expanded reset under reuse"
    );
    assert_eq!(first.pushes, second.pushes, "pushes reset under reuse");
    assert_eq!(first.pops, second.pops, "pops reset under reuse");
    assert_eq!(
        first.heuristic_evals, second.heuristic_evals,
        "h_eval reset under reuse"
    );
}
