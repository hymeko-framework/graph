//! P3.2 §4 — workspace-reuse + batch matrix with a bench-local counting global allocator. Measured
//! finding (honest): a reused `InformedSearchWorkspace` is a ~4x allocation reduction vs a fresh one per
//! query (21 -> ~5 allocs/query on grid128), but is NOT zero-alloc — the returned path Vec, the
//! reconstruction, and heap growth for a larger frontier on a varied goal still allocate. A true
//! zero-alloc warm path would need engine-side path-buffer reuse (a candidate optimization, not done).
//! The counter uses relaxed atomics only in the allocator hook (measurement), never in the search engine.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::direct_ir::{
    HyperTraversal, IncidenceWeightCost, InformedSearchWorkspace, Undirected, dijkstra_berge_path,
};

#[path = "informed_common/mod.rs"]
mod ic;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(l.size(), Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static GA: Counting = Counting;

fn snap() -> (usize, usize) {
    (
        ALLOCS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    )
}

fn cost() -> IncidenceWeightCost {
    IncidenceWeightCost { base: 0.0 }
}

/// Measured structural claim (HONEST): a reused workspace does NOT reach zero allocations per query —
/// the returned path Vec + reconstruction, and heap growth for a larger frontier on a varied goal, still
/// allocate. But reuse is a large reduction versus a fresh workspace per query. Reaching a true zero-alloc
/// warm path would need engine-side path-buffer reuse (a candidate optimization, not done here).
#[test]
#[ignore = "comparative allocation profile; opt in with --ignored --exact"]
fn reused_workspace_reduces_allocation_vs_fresh() {
    let ir = ic::map_grid_ir(64, 64, &ic::map_obstacles(64, 64));
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let v = ir.decl_nodes.len();
    let goals: Vec<usize> = (0..256).map(|i| (i * 7 + 13) % v).collect();
    // fresh: a new workspace per query
    let (f0, _) = snap();
    for &g in &goals {
        let mut ws = InformedSearchWorkspace::new();
        let p = dijkstra_berge_path(
            &trav,
            NodeId::new(0),
            NodeId::new(g),
            &cost(),
            &Undirected,
            &mut ws,
        )
        .unwrap();
        drop(p);
    }
    let (f1, _) = snap();
    let fresh = f1 - f0;
    // reused: one warmed workspace across the same 256 goals
    let mut ws = InformedSearchWorkspace::new();
    for _ in 0..2 {
        let _ = dijkstra_berge_path(
            &trav,
            NodeId::new(0),
            NodeId::new(v - 1),
            &cost(),
            &Undirected,
            &mut ws,
        )
        .unwrap();
    }
    let (a0, _) = snap();
    for &g in &goals {
        let p = dijkstra_berge_path(
            &trav,
            NodeId::new(0),
            NodeId::new(g),
            &cost(),
            &Undirected,
            &mut ws,
        )
        .unwrap();
        drop(p);
    }
    let (a1, _) = snap();
    let reused = a1 - a0;
    assert!(
        reused * 2 < fresh,
        "reuse must materially cut allocations: reused {reused} vs fresh {fresh} over 256 queries"
    );
}

// ---- the workspace-reuse + batch table (release; --nocapture) -------------------------------------

#[test]
#[ignore = "performance benchmark; opt in with --ignored --exact"]
fn measure_workspace_reuse_and_batch() {
    let ir = ic::map_grid_ir(128, 128, &ic::map_obstacles(128, 128));
    let idx = build_incidence_index(&ir).unwrap();
    let trav = HyperTraversal::new(&idx);
    let v = ir.decl_nodes.len();
    let goals: Vec<usize> = (0..4096).map(|i| (i * 31 + 7) % v).collect();

    println!("\n# P3.2 workspace-reuse + batch (release; grid128; allocations & us per query)");
    println!(
        "{:<26} {:>10} {:>12} {:>14}",
        "mode", "queries", "us/query", "allocs/query"
    );

    // (1) fresh workspace per query
    {
        let (a0, _) = snap();
        let (us, _, _) = ic::median_us(50, || {
            let mut ws = InformedSearchWorkspace::new();
            let _ = dijkstra_berge_path(
                &trav,
                NodeId::new(0),
                NodeId::new(v - 1),
                &cost(),
                &Undirected,
                &mut ws,
            )
            .unwrap();
        });
        // count allocs over a fixed count
        let n = 64;
        let (b0, _) = snap();
        for _ in 0..n {
            let mut ws = InformedSearchWorkspace::new();
            let _ = dijkstra_berge_path(
                &trav,
                NodeId::new(0),
                NodeId::new(v - 1),
                &cost(),
                &Undirected,
                &mut ws,
            )
            .unwrap();
        }
        let (b1, _) = snap();
        let _ = a0;
        println!(
            "{:<26} {:>10} {:>12.2} {:>14.2}",
            "fresh-workspace/query",
            1,
            us,
            (b1 - b0) as f64 / n as f64
        );
    }
    // (2) one reused workspace, repeated same goal
    {
        let mut ws = InformedSearchWorkspace::new();
        for _ in 0..2 {
            let _ = dijkstra_berge_path(
                &trav,
                NodeId::new(0),
                NodeId::new(v - 1),
                &cost(),
                &Undirected,
                &mut ws,
            )
            .unwrap();
        }
        let (us, _, _) = ic::median_us(50, || {
            let _ = dijkstra_berge_path(
                &trav,
                NodeId::new(0),
                NodeId::new(v - 1),
                &cost(),
                &Undirected,
                &mut ws,
            )
            .unwrap();
        });
        let n = 64;
        let (b0, _) = snap();
        for _ in 0..n {
            let p = dijkstra_berge_path(
                &trav,
                NodeId::new(0),
                NodeId::new(v - 1),
                &cost(),
                &Undirected,
                &mut ws,
            )
            .unwrap();
            drop(p);
        }
        let (b1, _) = snap();
        println!(
            "{:<26} {:>10} {:>12.2} {:>14.2}",
            "reused/same-goal",
            1,
            us,
            (b1 - b0) as f64 / n as f64
        );
    }
    // (3) reused workspace, varied goals, batch 1/16/256
    for &batch in &[1usize, 16, 256] {
        let mut ws = InformedSearchWorkspace::new();
        for _ in 0..2 {
            let _ = dijkstra_berge_path(
                &trav,
                NodeId::new(0),
                NodeId::new(v - 1),
                &cost(),
                &Undirected,
                &mut ws,
            )
            .unwrap();
        }
        let (us, _, _) = ic::median_us(10, || {
            for i in 0..batch {
                let g = goals[i % goals.len()];
                let _ = dijkstra_berge_path(
                    &trav,
                    NodeId::new(0),
                    NodeId::new(g),
                    &cost(),
                    &Undirected,
                    &mut ws,
                )
                .unwrap();
            }
        });
        let (b0, _) = snap();
        for i in 0..batch {
            let g = goals[i % goals.len()];
            let p = dijkstra_berge_path(
                &trav,
                NodeId::new(0),
                NodeId::new(g),
                &cost(),
                &Undirected,
                &mut ws,
            )
            .unwrap();
            drop(p);
        }
        let (b1, _) = snap();
        println!(
            "{:<26} {:>10} {:>12.2} {:>14.2}",
            format!("reused/varied batch={batch}"),
            batch,
            us / batch as f64,
            (b1 - b0) as f64 / batch as f64
        );
    }
}
