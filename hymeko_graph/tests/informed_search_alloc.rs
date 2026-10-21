//! Warm-query zero-allocation proof for `direct_ir::informed`, in its own single-test binary.
//!
//! A global counting allocator counts every thread, and Rust runs a binary's tests in parallel, so this
//! is deliberately the ONLY test in this binary (mirrors `direct_ir_alloc.rs`). The measured window is an
//! **unreachable** warm query: it drives the full Dijkstra/A* loop over the start's component but returns
//! `Ok(None)`, so no result `Vec` is built — isolating the search + workspace from the one unavoidable
//! owned-result allocation a *successful* query makes.

#[path = "informed_common/search.rs"]
mod search;
use hymeko::ir::ir::IrConstruction;
use search::{AStar, Dijkstra, SearchCase};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use hymeko::common::ids::DeclId;
use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::{AnnoR, ArcRec, Ir, SignedRefR};
use hymeko_graph::direct_ir::{
    HyperTraversal, InformedSearchWorkspace, MetricHeuristic, MetricStepCost, PositionProvider,
    Undirected, UnitCost, ZeroHeuristic,
};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

struct LinePos(Vec<[f64; 1]>);
impl PositionProvider for LinePos {
    fn position(&self, node: DeclId) -> Option<&[f64]> {
        self.0.get(node.raw()).map(|a| a.as_slice())
    }
}

/// Two disjoint window components; a `0 -> other-component` query is unreachable but exercises the full
/// search over component 0.
fn two_components(m: usize, k: usize) -> Ir {
    let mut ir = Ir::default();
    for _ in 0..(2 * m) {
        IrConstruction::new(&mut ir)
            .append_node(
                hymeko::common::ids::SymId::new(0),
                AnnoR::default(),
                Vec::new(),
            )
            .unwrap();
    }
    for c in 0..2 {
        let base = c * m;
        for i in 0..(m - k + 1) {
            let refs: Vec<SignedRefR> = (i..i + k)
                .map(|j| {
                    SignedRefR::plain(DeclId::new(base + j), hymeko::ir::ir::IncidenceSign::Plus)
                })
                .collect();
            ir.arcs.push(ArcRec::unowned(refs));
        }
    }
    ir
}

#[test]
fn warm_informed_query_allocates_nothing() {
    let m = 500;
    let ir = two_components(m, 4);
    let index = build_incidence_index(&ir).expect("index");
    let trav = HyperTraversal::new(&index);
    let pos = LinePos(ic_line(2 * m));
    let cost = MetricStepCost(&pos);
    let goal_decl = ir.nodes[NodeId::new(2 * m - 1).raw()].decl;
    let heur = MetricHeuristic::new(&pos, goal_decl);
    let mut ws = InformedSearchWorkspace::new();

    // start in component 0, goal in component 1 -> unreachable (Ok(None)); no result Vec allocated.
    let (start, goal) = (NodeId::new(0), NodeId::new(2 * m - 1));

    // Warm-up sizes the workspace arrays and grows the heap to its peak.
    for _ in 0..3 {
        assert!(
            SearchCase::new(&trav, start, goal, &mut ws)
                .run(&Dijkstra::new(&UnitCost, &Undirected))
                .unwrap()
                .is_none()
        );
        assert!(
            SearchCase::new(&trav, start, goal, &mut ws)
                .run(&AStar::new(&cost, &heur, &Undirected))
                .unwrap()
                .is_none()
        );
        assert!(
            SearchCase::new(&trav, start, goal, &mut ws)
                .run(&AStar::new(&UnitCost, &ZeroHeuristic, &Undirected))
                .unwrap()
                .is_none()
        );
    }

    assert!(
        SearchCase::new(&trav, start, goal, &mut ws)
            .stats()
            .expanded_nodes
            > 0
    );
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..64 {
        assert!(
            SearchCase::new(&trav, start, goal, &mut ws)
                .run(&Dijkstra::new(&UnitCost, &Undirected))
                .unwrap()
                .is_none()
        );
        assert!(
            SearchCase::new(&trav, start, goal, &mut ws)
                .run(&AStar::new(&cost, &heur, &Undirected))
                .unwrap()
                .is_none()
        );
    }
    let allocs = ALLOCS.load(Ordering::Relaxed) - before;
    assert_eq!(
        allocs, 0,
        "warm informed search must allocate nothing (got {allocs})"
    );
}

fn ic_line(m: usize) -> Vec<[f64; 1]> {
    (0..m).map(|i| [i as f64]).collect()
}
