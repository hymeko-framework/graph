//! The Berge search over the direct-Ir incidence space against petgraph, the conventional adjacency-list
//! graph search (`algo::astar`, `algo::dijkstra`, `visit::Bfs`), on the informed bench's workloads.
//!
//! Run with `cd benchmarks && cargo bench --bench search_bench`. Single host, `median_us` (median of 9
//! after warm-up), so wall deltas are PROVISIONAL. Every query's cost is checked against the Berge
//! Dijkstra cost. Work is frontier pushes on both sides: the Berge `SearchStats::pushes`, and
//! petgraph A*'s estimate calls (one per push, the start included).
//!
//! The grid is 2-uniform, so petgraph holds it as the same undirected graph. A hypergraph goes to
//! petgraph as its star expansion (incidence graph): a vertex per declaration and per arc, an edge per
//! incidence each way, costing 1 into an arc and 0 out of it, so its shortest paths are Berge paths.

use std::cell::Cell;

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    HyperTraversal, InformedSearchWorkspace, MetricHeuristic, PositionProvider, TraversalWorkspace,
    Undirected, UnitCost, astar_berge_path, dijkstra_berge_path,
};
use petgraph::graph::{DiGraph, NodeIndex, UnGraph};
use petgraph::visit::{Bfs, IntoNeighbors, Visitable};

#[path = "../../hymeko_graph/tests/informed_common/mod.rs"]
mod ic;

use ic::{SignMode, median_us};

struct Pos2(Vec<[f64; 2]>);
impl PositionProvider for Pos2 {
    fn position(&self, node: DeclId) -> Option<&[f64]> {
        self.0.get(node.raw()).map(|a| a.as_slice())
    }
}

/// The IR's 2-uniform arcs as an undirected petgraph graph over decl ids, unit edge cost.
fn petgraph_of_edges(ir: &Ir) -> UnGraph<(), f64> {
    let mut g = UnGraph::with_capacity(ir.decl_nodes.len(), ir.arcs.len());
    for _ in 0..ir.decl_nodes.len() {
        g.add_node(());
    }
    for arc in &ir.arcs {
        let [a, b] = arc.refs.as_slice() else {
            panic!("the grid's arcs are 2-uniform")
        };
        g.add_edge(
            NodeIndex::new(a.target().raw()),
            NodeIndex::new(b.target().raw()),
            1.0,
        );
    }
    g
}

/// The IR's star expansion: decl ids first, then one vertex per arc; into an arc costs 1, out costs 0.
fn petgraph_star(ir: &Ir) -> DiGraph<(), f64> {
    let n = ir.decl_nodes.len();
    let nnz: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
    let mut g = DiGraph::with_capacity(n + ir.arcs.len(), 2 * nnz);
    for _ in 0..n + ir.arcs.len() {
        g.add_node(());
    }
    for (i, arc) in ir.arcs.iter().enumerate() {
        let hub = NodeIndex::new(n + i);
        for r in &arc.refs {
            let v = NodeIndex::new(r.target().raw());
            g.add_edge(v, hub, 1.0);
            g.add_edge(hub, v, 0.0);
        }
    }
    g
}

/// Breadth-first from `start` until `goal` is dequeued; returns the vertices dequeued.
fn bfs_until<G>(g: G, start: G::NodeId, goal: G::NodeId) -> usize
where
    G: IntoNeighbors + Visitable,
    G::NodeId: PartialEq,
{
    let mut bfs = Bfs::new(g, start);
    let mut dequeued = 0;
    while let Some(v) = bfs.next(g) {
        dequeued += 1;
        if v == goal {
            return dequeued;
        }
    }
    panic!("goal unreachable")
}

/// petgraph A* returning the cost and the pushes (one estimate per push, the start included).
fn petgraph_astar_counted(
    g: &UnGraph<(), f64>,
    start: NodeIndex,
    goal: NodeIndex,
    h: impl Fn(NodeIndex) -> f64,
) -> (f64, usize) {
    let pushes = Cell::new(0usize);
    let (cost, _) = petgraph::algo::astar(
        g,
        start,
        |v| v == goal,
        |e| *e.weight(),
        |v| {
            pushes.set(pushes.get() + 1);
            h(v)
        },
    )
    .expect("reachable");
    (cost, pushes.get())
}

fn grid() {
    println!(
        "\n-- grid 200x200, goal d cells right of start, unit cost, Euclidean heuristic; push = frontier insertions --"
    );
    println!(
        "{:>5} | {:>9} {:>6} | {:>9} {:>6} | {:>9} {:>6} | {:>9} {:>7} | {:>9} {:>7}",
        "d",
        "B.A* us",
        "push",
        "pg.A* us",
        "push",
        "B.Dij us",
        "push",
        "pg.Dij us",
        "reached",
        "pg.BFS us",
        "deq"
    );
    let side = 200usize;
    let ir = ic::grid_ir(side, side);
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let positions = ic::grid_positions(side, side);
    let pos = Pos2(positions.clone());
    let pg = petgraph_of_edges(&ir);
    let (_, index_us, _) = median_us(9, || {
        let _ = build_incidence_index(&ir).unwrap();
    });
    let (_, build_us, _) = median_us(9, || {
        let _ = petgraph_of_edges(&ir);
    });
    let rc = |r: usize, c: usize| NodeId::new(r * side + c);
    let vertex = |n: NodeId| NodeIndex::new(ir.nodes[n.raw()].decl.raw());
    let (sr, sc) = (side / 2, side / 4);
    let start = rc(sr, sc);
    let ps = vertex(start);
    let mut ws = InformedSearchWorkspace::new();
    for &d in &[20usize, 40, 80] {
        let goal = rc(sr, sc + d);
        let pgoal = vertex(goal);
        let euc = MetricHeuristic::new(&pos, ir.nodes[goal.raw()].decl);
        let (gx, gy) = ((sc + d) as f64, sr as f64);
        let l2 = |v: NodeIndex| {
            let p = positions[v.index()];
            (p[0] - gx).hypot(p[1] - gy)
        };

        let dij = dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws)
            .unwrap()
            .unwrap();
        let dij_push = ws.stats().pushes;
        let a = astar_berge_path(&trav, start, goal, &UnitCost, &euc, &Undirected, &mut ws)
            .unwrap()
            .unwrap();
        let a_push = ws.stats().pushes;
        let (pa_cost, pa_push) = petgraph_astar_counted(&pg, ps, pgoal, l2);
        let pd = petgraph::algo::dijkstra(&pg, ps, Some(pgoal), |e| *e.weight());
        let deq = bfs_until(&pg, ps, pgoal);
        for cost in [a.total_cost, pa_cost, pd[&pgoal]] {
            assert!(
                (cost - dij.total_cost).abs() < 1e-9,
                "cost != Berge Dijkstra"
            );
        }

        let (_, a_us, _) = median_us(9, || {
            let _ = astar_berge_path(&trav, start, goal, &UnitCost, &euc, &Undirected, &mut ws)
                .unwrap();
        });
        let (_, pa_us, _) = median_us(9, || {
            let _ = petgraph::algo::astar(&pg, ps, |v| v == pgoal, |e| *e.weight(), l2);
        });
        let (_, dij_us, _) = median_us(9, || {
            let _ =
                dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws).unwrap();
        });
        let (_, pd_us, _) = median_us(9, || {
            let _ = petgraph::algo::dijkstra(&pg, ps, Some(pgoal), |e| *e.weight());
        });
        let (_, pb_us, _) = median_us(9, || {
            let _ = bfs_until(&pg, ps, pgoal);
        });
        println!(
            "{:>5} | {:>9.1} {:>6} | {:>9.1} {:>6} | {:>9.1} {:>6} | {:>9.1} {:>7} | {:>9.1} {:>7}",
            d,
            a_us,
            a_push,
            pa_us,
            pa_push,
            dij_us,
            dij_push,
            pd_us,
            pd.len(),
            pb_us,
            deq
        );
    }
    println!(
        "one-off builds: Berge incidence index {index_us:.1} us, petgraph graph {build_us:.1} us; n = {}",
        ir.decl_nodes.len()
    );
}

fn windows() {
    println!(
        "\n-- windows (arity-5 arcs, unit cost): Berge vs petgraph on the star expansion (reached/deq count arc vertices too) --"
    );
    println!(
        "{:>7} {:>7} | {:>9} {:>9} | {:>9} | {:>9} {:>7} | {:>9} {:>7} | {:>9} {:>7}",
        "m",
        "nnz",
        "index us",
        "star us",
        "B.BFS us",
        "B.Dij us",
        "push",
        "pg.Dij us",
        "reached",
        "pg.BFS us",
        "deq"
    );
    for &m in &[1_000usize, 10_000, 100_000] {
        let ir = ic::window_ir(m, 5, false, SignMode::AllPlus);
        let idx = build_incidence_index(&ir).unwrap();
        let trav = HyperTraversal::new(&idx);
        let nnz: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        let star = petgraph_star(&ir);
        let (_, index_us, _) = median_us(9, || {
            let _ = build_incidence_index(&ir).unwrap();
        });
        let (_, star_us, _) = median_us(9, || {
            let _ = petgraph_star(&ir);
        });
        let (start, goal) = (NodeId::new(0), NodeId::new(m - 1));
        let ps = NodeIndex::new(ir.nodes[start.raw()].decl.raw());
        let pgoal = NodeIndex::new(ir.nodes[goal.raw()].decl.raw());
        let mut bws = TraversalWorkspace::new();
        let mut ws = InformedSearchWorkspace::new();

        let dij = dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws)
            .unwrap()
            .unwrap();
        let dij_push = ws.stats().pushes;
        let pd = petgraph::algo::dijkstra(&star, ps, Some(pgoal), |e| *e.weight());
        assert!(
            (pd[&pgoal] - dij.total_cost).abs() < 1e-9,
            "star-expansion cost != Berge Dijkstra"
        );
        let deq = bfs_until(&star, ps, pgoal);

        let (_, bfs_us, _) = median_us(9, || {
            let _ = trav.berge_shortest_path(&mut bws, start, goal).unwrap();
        });
        let (_, dij_us, _) = median_us(9, || {
            let _ =
                dijkstra_berge_path(&trav, start, goal, &UnitCost, &Undirected, &mut ws).unwrap();
        });
        let (_, pd_us, _) = median_us(9, || {
            let _ = petgraph::algo::dijkstra(&star, ps, Some(pgoal), |e| *e.weight());
        });
        let (_, pb_us, _) = median_us(9, || {
            let _ = bfs_until(&star, ps, pgoal);
        });
        println!(
            "{:>7} {:>7} | {:>9.1} {:>9.1} | {:>9.1} | {:>9.1} {:>7} | {:>9.1} {:>7} | {:>9.1} {:>7}",
            m,
            nnz,
            index_us,
            star_us,
            bfs_us,
            dij_us,
            dij_push,
            pd_us,
            pd.len(),
            pb_us,
            deq
        );
    }
}

fn main() {
    println!("\n=== Berge search vs petgraph (us; median-of-9 warm; single-host PROVISIONAL) ===");
    grid();
    windows();
    println!();
}
