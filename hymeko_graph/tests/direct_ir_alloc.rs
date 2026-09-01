//! Warm-query zero-allocation proof for `direct_ir`, in its own single-test binary.
//!
//! This file owns a counting global allocator. It is deliberately the ONLY test here: Rust runs
//! tests within a binary in parallel, and a global allocator counts every thread, so a second
//! (allocating) test in the same binary would corrupt the measurement. Isolated, the measured
//! window sees only this thread's allocations.

mod direct_ir_common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use hymeko::ir::incidence::build_incidence_index;

use hymeko_graph::direct_ir::{HyperTraversal, TraversalWorkspace};

use direct_ir_common::compile;

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const SRC: &str = r#"
vv {}
g {
    a {} b {} c {} d {} e {}
    @e0 { (+ a, - b, ~ c); }
    @e1 { (+ c, + d); }
    @e2 { (+ d, + e); }
}
"#;

#[test]
fn warm_reachable_query_allocates_nothing() {
    let w = compile(SRC);
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();
    let start = w.node("a");

    // Warm-up sizes the workspace buffers.
    let warm = trav.reachable_nodes(&mut ws, start).unwrap().len();
    assert!(warm >= 5);

    // Now every warm query must reuse those buffers without touching the allocator.
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..64 {
        let n = trav.reachable_nodes(&mut ws, start).unwrap().len();
        assert_eq!(n, warm);
    }
    let allocs = ALLOCS.load(Ordering::Relaxed) - before;
    assert_eq!(
        allocs, 0,
        "warm reachable_nodes must allocate nothing (got {allocs})"
    );
}
