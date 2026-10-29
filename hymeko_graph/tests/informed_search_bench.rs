//! Benchmark (mission §4/§11): BFS baseline vs Dijkstra vs A*(metric) over the direct-Ir incidence space.
//!
//! Prints a table (run with `-- --nocapture`); single host, `median_us` (median of 9 after warm-up), so
//! fine wall deltas are PROVISIONAL. The load-bearing comparison is **expanded states** (host-independent)
//! plus the correctness cross-check (A* cost == Dijkstra cost) done inline. Not a perf-gate *assertion* —
//! the §12 gate verdicts are computed and reported here.
//!
//! Sections 1b and 2b run the same queries through the classical plain-graph search of this crate
//! (`traversal_heuristic::astar` and `bidirectional_bfs` on a `Csr`), on the 2-section of the same IR.
//! Their work measure is frontier pushes (the classical A* reports no expansions; it calls `h` once per
//! push), and both reset or allocate O(n) state per query, which the reset floor row isolates.

use std::cell::Cell;

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    BergeHeuristic, HyperTraversal, InformedSearchWorkspace, MetricHeuristic, PositionProvider,
    TraversalWorkspace, Undirected, UnitCost, astar_berge_path, dijkstra_berge_path,
};
use hymeko_graph::{
    AstarScratch, Csr, Heuristic, SignedGraph, ZeroHeuristic, astar, bidirectional_bfs,
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

/// `bidirectional_bfs` keeps u8 distances with 255 as the unreached mark, so each half stays below it.
const BI_HALF: u8 = u8::MAX - 1;

/// The 2-section (clique expansion) of the IR's arcs as a plain graph over decl ids: the classical
/// representation, which the Berge search reads in place instead. On the 2-uniform grid it is the grid.
fn two_section_csr(ir: &Ir) -> Csr {
    let (mut us, mut vs) = (Vec::new(), Vec::new());
    for arc in &ir.arcs {
        for (i, a) in arc.refs.iter().enumerate() {
            for b in &arc.refs[i + 1..] {
                us.push(a.target().raw() as u32);
                vs.push(b.target().raw() as u32);
            }
        }
    }
    let signs = vec![1i8; us.len()];
    Csr::from_graph(&SignedGraph::from_parts(
        ir.decl_nodes.len() as u32,
        &us,
        &vs,
        &signs,
    ))
}

/// Classical A* on the CSR, returning the hop count and the frontier pushes (`h` calls).
fn csr_astar_counted<H: Heuristic>(
    csr: &Csr,
    scratch: &mut AstarScratch,
    start: u32,
    goal: u32,
    h: &H,
) -> (usize, usize) {
    let calls = Cell::new(0usize);
    let counted = |v: u32| {
        calls.set(calls.get() + 1);
        h.h(v)
    };
    let path = astar(csr, scratch, start, goal, &counted, u32::MAX).expect("reachable");
    (path.len() - 1, calls.get())
}

#[test]
#[ignore = "performance benchmark; opt in with --ignored --exact"]
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

    // ---- 1b. the same grid and queries as a plain CSR graph: the classical A* and bi-BFS ------------
    println!(
        "\n-- the same grid as a plain CSR graph: classical A* (traversal_heuristic::astar), bi-BFS; push = frontier insertions --"
    );
    println!(
        "{:>5} | {:>9} {:>9} | {:>9} {:>9} | {:>9} {:>9} | {:>9} | {:>10} {:>10}",
        "d",
        "A*0 us",
        "A*0 push",
        "A*L1 us",
        "A*L1 push",
        "A*L2 us",
        "A*L2 push",
        "biBFS us",
        "B.Dij push",
        "B.L1 push"
    );
    let csr = two_section_csr(&ir);
    let (_, build_us, _) = median_us(9, || {
        let _ = two_section_csr(&ir);
    });
    let (_, index_us, _) = median_us(9, || {
        let _ = build_incidence_index(&ir).unwrap();
    });
    let mut scratch = AstarScratch::with_capacity(csr.n_nodes);
    let vertex = |n: NodeId| ir.nodes[n.raw()].decl.raw() as u32;
    let s0 = vertex(start);
    for &d in &[20usize, 40, 80] {
        let goal = rc(sr, sc + d);
        let g0 = vertex(goal);
        let (gx, gy) = ((sc + d) as f64, sr as f64);
        let l1 = |v: u32| {
            let p = positions[v as usize];
            (p[0] - gx).abs() + (p[1] - gy).abs()
        };
        let l2 = |v: u32| {
            let p = positions[v as usize];
            (p[0] - gx).hypot(p[1] - gy)
        };

        let dij = dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws)
            .unwrap()
            .unwrap();
        let berge_dij_push = ws.stats().pushes;
        let man = ManhattanH {
            pos: positions.clone(),
            goal: [gx, gy],
        };
        let _ = astar_berge_path(&trav, start, goal, &UnitCost, &man, &Undirected, &mut ws)
            .unwrap()
            .unwrap();
        let berge_l1_push = ws.stats().pushes;

        let (zero_hops, zero_push) = csr_astar_counted(&csr, &mut scratch, s0, g0, &ZeroHeuristic);
        let (l1_hops, l1_push) = csr_astar_counted(&csr, &mut scratch, s0, g0, &l1);
        let (l2_hops, l2_push) = csr_astar_counted(&csr, &mut scratch, s0, g0, &l2);
        let bi_hops = bidirectional_bfs(&csr, s0, g0, BI_HALF).expect("reachable") as usize;
        let berge = dij.total_cost as usize;
        assert!(
            zero_hops == berge && l1_hops == berge && l2_hops == berge && bi_hops == berge,
            "CSR search cost != Berge Dijkstra"
        );

        let (_, zero_us, _) = median_us(9, || {
            let _ = astar(&csr, &mut scratch, s0, g0, &ZeroHeuristic, u32::MAX);
        });
        let (_, l1_us, _) = median_us(9, || {
            let _ = astar(&csr, &mut scratch, s0, g0, &l1, u32::MAX);
        });
        let (_, l2_us, _) = median_us(9, || {
            let _ = astar(&csr, &mut scratch, s0, g0, &l2, u32::MAX);
        });
        let (_, bi_us, _) = median_us(9, || {
            let _ = bidirectional_bfs(&csr, s0, g0, BI_HALF);
        });
        println!(
            "{:>5} | {:>9.1} {:>9} | {:>9.1} {:>9} | {:>9.1} {:>9} | {:>9.1} | {:>10} {:>10}",
            d,
            zero_us,
            zero_push,
            l1_us,
            l1_push,
            l2_us,
            l2_push,
            bi_us,
            berge_dij_push,
            berge_l1_push
        );
    }
    let (_, floor_us, _) = median_us(9, || {
        let _ = astar(&csr, &mut scratch, s0, s0, &ZeroHeuristic, u32::MAX);
    });
    println!(
        "CSR A* reset floor (start == goal: the O(n) scratch reset alone): {floor_us:.1} us; one-off builds: 2-section CSR {build_us:.1} us, Berge incidence index {index_us:.1} us; n = {}",
        csr.n_nodes
    );

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

    // ---- 2b. the same windows as their 2-section plain graph: classical A*(Zero) and bi-BFS ----------
    println!(
        "\n-- the same windows as a plain CSR graph (2-section: an arity-5 arc becomes 10 edges) --"
    );
    println!(
        "{:>9} {:>9} | {:>10} {:>10} | {:>10} {:>9} | {:>10} | {:>10}",
        "m", "csr nnz", "csr bld us", "index us", "A*0 us", "A*0 push", "biBFS us", "B.Dij push"
    );
    for &m in &[1_000usize, 10_000, 100_000] {
        let ir = ic::window_ir(m, 5, false, SignMode::AllPlus);
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let mut ws = InformedSearchWorkspace::new();
        let (start, goal) = (NodeId::new(0), NodeId::new(m - 1));
        let berge = dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws)
            .unwrap()
            .unwrap()
            .total_cost as usize;
        let berge_push = ws.stats().pushes;

        let csr = two_section_csr(&ir);
        let (_, build_us, _) = median_us(9, || {
            let _ = two_section_csr(&ir);
        });
        let (_, index_us, _) = median_us(9, || {
            let _ = build_incidence_index(&ir).unwrap();
        });
        let mut scratch = AstarScratch::with_capacity(csr.n_nodes);
        let s0 = ir.nodes[start.raw()].decl.raw() as u32;
        let g0 = ir.nodes[goal.raw()].decl.raw() as u32;
        let (hops, push) = csr_astar_counted(&csr, &mut scratch, s0, g0, &ZeroHeuristic);
        assert_eq!(hops, berge, "2-section hop count != Berge Dijkstra");
        let (_, a0_us, _) = median_us(9, || {
            let _ = astar(&csr, &mut scratch, s0, g0, &ZeroHeuristic, u32::MAX);
        });
        let bi = if hops <= 2 * BI_HALF as usize {
            assert_eq!(bidirectional_bfs(&csr, s0, g0, BI_HALF), Some(hops as u32));
            let (_, us, _) = median_us(9, || {
                let _ = bidirectional_bfs(&csr, s0, g0, BI_HALF);
            });
            format!("{us:.1}")
        } else {
            "u8 depth".to_string()
        };
        println!(
            "{:>9} {:>9} | {:>10.1} {:>10.1} | {:>10.1} {:>9} | {:>10} | {:>10}",
            m,
            csr.col_idx.len(),
            build_us,
            index_us,
            a0_us,
            push,
            bi,
            berge_push
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
