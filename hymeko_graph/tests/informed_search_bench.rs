//! Benchmark (mission §4/§11): BFS baseline vs Dijkstra vs A*(metric) over the direct-Ir incidence space.
//!
//! Prints a table (run with `-- --nocapture`); single host, `median_us` (median of 9 after warm-up), so
//! fine wall deltas are PROVISIONAL. The load-bearing comparison is **expanded states** (host-independent)
//! plus the correctness cross-check (A* cost == Dijkstra cost) done inline. Not a perf-gate *assertion* —
//! the §12 gate verdicts are computed and reported here.

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    BergeHeuristic, HyperTraversal, InformedSearchWorkspace, MetricHeuristic, PositionProvider,
    TraversalWorkspace, Undirected, UnitCost, astar_berge_path, dijkstra_berge_path,
};

#[path = "informed_common/mod.rs"]
mod ic;

use ic::{SignMode, median_us};

struct Pos2(Vec<[f64; 2]>);
impl PositionProvider for Pos2 {
    fn position(&self, node: DeclId) -> Option<&[f64]> {
        self.0.get(node.raw()).map(|a| a.as_slice())
    }
}

/// Manhattan (L1) heuristic to a fixed goal, over grid positions. On a 4-connected unit-cost grid it
/// equals the true remaining cost (admissible + consistent, C1) — the tight heuristic that prunes hardest.
struct ManhattanH {
    pos: Vec<[f64; 2]>,
    goal: [f64; 2],
}
impl BergeHeuristic for ManhattanH {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    fn h(&self, _ir: &Ir, node: DeclId) -> f64 {
        match self.pos.get(node.raw()) {
            Some(p) => (p[0] - self.goal[0]).abs() + (p[1] - self.goal[1]).abs(),
            None => 0.0,
        }
    }
}

#[test]
fn bench_informed_berge_search() {
    println!(
        "\n=== informed Berge search benchmark (us; median-of-9 warm; single-host PROVISIONAL) ==="
    );

    // ---- 1. branching grid, near start/goal in a large grid: A* prunes a disk to a cone ------------
    // Dijkstra settles every node with g <= C* (a Manhattan disk around start); A* with a tight L1
    // heuristic settles only nodes on/near optimal start->goal paths. Unit cost; both heuristics
    // (Manhattan tight, Euclidean loose) are admissible+consistent with unit cost on a grid.
    println!(
        "\n-- grid 200x200, goal d cells right of start, unit cost: expanded-state reduction is the headline --"
    );
    println!(
        "{:>5} | {:>10} {:>9} | {:>9} {:>9} {:>7} | {:>9} {:>9} {:>7}",
        "d", "Dijk us", "Dijk exp", "A*L1 us", "A*L1 exp", "L1 x", "A*L2 us", "A*L2 exp", "L2 x"
    );
    let side = 200usize;
    let ir = ic::grid_ir(side, side);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let positions = ic::grid_positions(side, side);
    let pos = Pos2(positions.clone());
    let rc = |r: usize, c: usize| NodeId::new(r * side + c);
    let sr = side / 2;
    let sc = side / 4;
    let start = rc(sr, sc);
    let mut ws = InformedSearchWorkspace::new();
    for &d in &[20usize, 40, 80] {
        let goal = rc(sr, sc + d);
        let goal_decl = ir.nodes[goal.raw()].decl;
        let man = ManhattanH {
            pos: positions.clone(),
            goal: [(sc + d) as f64, sr as f64],
        };
        let euc = MetricHeuristic::new(&pos, goal_decl);

        let dij = dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws)
            .unwrap()
            .unwrap();
        let dij_exp = ws.stats().expanded_nodes;
        let a_man = astar_berge_path(&trav, start, goal, &UnitCost, &man, &Undirected, &mut ws)
            .unwrap()
            .unwrap();
        let man_exp = ws.stats().expanded_nodes;
        let a_euc = astar_berge_path(&trav, start, goal, &UnitCost, &euc, &Undirected, &mut ws)
            .unwrap()
            .unwrap();
        let euc_exp = ws.stats().expanded_nodes;
        assert!(
            (dij.total_cost - a_man.total_cost).abs() < 1e-9
                && (dij.total_cost - a_euc.total_cost).abs() < 1e-9,
            "grid A* cost != Dijkstra"
        );

        let (_, dij_us, _) = median_us(9, || {
            let _ =
                dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws).unwrap();
        });
        let (_, man_us, _) = median_us(9, || {
            let _ = astar_berge_path(&trav, start, goal, &UnitCost, &man, &Undirected, &mut ws)
                .unwrap();
        });
        let (_, euc_us, _) = median_us(9, || {
            let _ = astar_berge_path(&trav, start, goal, &UnitCost, &euc, &Undirected, &mut ws)
                .unwrap();
        });
        println!(
            "{:>5} | {:>10.1} {:>9} | {:>9.1} {:>9} {:>6.1}x | {:>9.1} {:>9} {:>6.1}x",
            d,
            dij_us,
            dij_exp,
            man_us,
            man_exp,
            dij_exp as f64 / man_exp.max(1) as f64,
            euc_us,
            euc_exp,
            dij_exp as f64 / euc_exp.max(1) as f64
        );
    }

    // ---- 2. window (unit cost): baseline BFS vs Dijkstra latency (uninformative-heuristic overhead) --
    println!("\n-- window (arity 5, unit): Dijkstra-vs-BFS baseline; A*(Zero) overhead check --");
    println!(
        "{:>9} {:>8} | {:>10} | {:>10} | {:>10}",
        "m", "nnz", "BFS us", "Dijk us", "cost"
    );
    for &m in &[1_000usize, 10_000, 100_000] {
        let ir = ic::window_ir(m, 5, false, SignMode::AllPlus);
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let nnz: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        let (start, goal) = (NodeId::new(0), NodeId::new(m - 1));
        let mut bws = TraversalWorkspace::new();
        let mut ws = InformedSearchWorkspace::new();
        let cost = dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws)
            .unwrap()
            .unwrap()
            .total_cost;
        let (_, bfs_us, _) = median_us(9, || {
            let _ = trav.berge_shortest_path(&mut bws, start, goal).unwrap();
        });
        let (_, dij_us, _) = median_us(9, || {
            let _ =
                dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws).unwrap();
        });
        println!(
            "{:>9} {:>8} | {:>10.1} | {:>10.1} | {:>10.0}",
            m, nnz, bfs_us, dij_us, cost
        );
    }

    // ---- 3. untouched-op regression guard: warm reachable_nodes (must track the graph baseline) ------
    println!("\n-- untouched direct_ir op (regression guard): warm reachable_nodes --");
    {
        let ir = ic::window_ir(200_000, 5, false, SignMode::AllPlus);
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let mut bws = TraversalWorkspace::new();
        let _ = trav
            .reachable_nodes(&mut bws, NodeId::new(0))
            .unwrap()
            .len();
        let (_, us, _) = median_us(9, || {
            let _ = trav
                .reachable_nodes(&mut bws, NodeId::new(0))
                .unwrap()
                .len();
        });
        println!(
            "reachable_nodes m=200000 arity5: {us:.1} us (compare to graph perf report baseline)"
        );
    }
    println!();
}
