//! P3.3 §6 — the O(1), zero-allocation facade proof for [`MatroidRef`]. Two independent lines of
//! evidence: (1) *structural* — the facade is exactly one machine word (a single thin reference), is `Copy`,
//! and carries no per-entity state; (2) *empirical* — with a counting global allocator, constructing the
//! facade and reading its accessors performs **zero** heap allocations, and that cost is **independent of
//! graph size** (a 4-node graph and a ~250k-incidence graph both cost 0 allocations for the facade). The
//! index build itself is Θ(nnz) and is deliberately excluded from the facade measurement — the O(1) claim
//! is about the borrowed view, not the derived index.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::{GraphicMatroidWorkspace, MatroidRef, rank2_rank};

#[path = "informed_common/mod.rs"]
mod ic;
use ic::SignMode;

// A THREAD-LOCAL allocation counter: the global-allocator hook runs on whichever thread allocates, so a
// per-thread `Cell` attributes each allocation to its thread. `cargo test` runs tests concurrently, so a
// process-global counter would be polluted by sibling tests' allocations — the thread-local one makes each
// measurement see only its own thread. Const-initialized `Cell<usize>` needs no lazy init and no
// destructor, so reading/writing it inside the allocator is itself allocation-free (no reentrancy).
thread_local! {
    static TL_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = TL_ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static GA: Counting = Counting;

fn allocs() -> usize {
    TL_ALLOCS.with(|c| c.get())
}

fn assert_copy<T: Copy>() {}

// ---- (1) structural proof -------------------------------------------------------------------------

#[test]
fn facade_is_one_word_and_copy() {
    // one thin reference → exactly one machine word, no more (the P3.4 provenance-safe facade).
    assert_eq!(
        std::mem::size_of::<MatroidRef>(),
        std::mem::size_of::<usize>(),
        "MatroidRef must be exactly one reference wide (holds only &IncidenceIndex; the Ir travels through it)"
    );
    // Copy (and therefore Clone) — a facade, not an owner.
    assert_copy::<MatroidRef>();
    // a bit-for-bit copy is a valid independent handle (Copy semantics).
    let ir = ic::grid_ir(3, 3);
    let idx = build_incidence_index(&ir).unwrap();
    let a = MatroidRef::new(&idx);
    let b = a; // Copy
    assert!(
        std::ptr::eq(a.incidence().ir(), b.incidence().ir()),
        "the copy borrows the same Ir"
    );
    assert!(
        std::ptr::eq(a.incidence(), b.incidence()),
        "the copy borrows the same index"
    );
}

// ---- (2) empirical proof: 0 allocations, independent of graph size --------------------------------

#[test]
fn facade_construction_is_zero_alloc_and_size_independent() {
    // tiny graph
    let small = ic::grid_ir(2, 2);
    let small_idx = build_incidence_index(&small).unwrap();
    // large graph (~250k incidences): a Steiner triple system on 511 points.
    let big = ic::steiner_sts_ir(9); // 2^9 - 1 = 511 points
    let big_idx = build_incidence_index(&big).unwrap();

    // measure ONLY the facade construction + accessor reads, after the index already exists.
    let a0 = allocs();
    let mref_small = MatroidRef::new(&small_idx);
    std::hint::black_box(mref_small.incidence().ir());
    std::hint::black_box(mref_small.incidence());
    let a1 = allocs();

    let mref_big = MatroidRef::new(&big_idx);
    std::hint::black_box(mref_big.incidence().ir());
    std::hint::black_box(mref_big.incidence());
    let a2 = allocs();

    assert_eq!(
        a1 - a0,
        0,
        "facade construction on a tiny graph allocates nothing"
    );
    assert_eq!(
        a2 - a1,
        0,
        "facade construction on a 511-point STS allocates nothing (size-independent)"
    );
}

#[test]
fn facade_copies_are_free_and_stateless() {
    // copying the facade N times allocates nothing and each copy is an equivalent handle.
    let ir = ic::window_ir(64, 2, false, SignMode::AllPlus);
    let idx = build_incidence_index(&ir).unwrap();
    let base = MatroidRef::new(&idx);
    let a0 = allocs();
    let mut acc = 0usize;
    for _ in 0..1000 {
        let c = base; // Copy
        acc += std::hint::black_box(c.incidence().ir().nodes.len());
    }
    let a1 = allocs();
    assert_eq!(a1 - a0, 0, "1000 facade copies allocate nothing");
    assert_eq!(
        acc,
        1000 * ir.nodes.len(),
        "each copy is a live handle to the same Ir"
    );
}

#[test]
fn rank_computation_is_not_o1_but_facade_is() {
    // guard against over-claiming: the *facade* is O(1)/0-alloc, but a rank COMPUTATION touches the
    // ground set (Θ(arcs)). This test documents that boundary rather than asserting rank is free.
    let ir = ic::grid_ir(16, 16);
    let idx = build_incidence_index(&ir).unwrap();
    let m = MatroidRef::new(&idx);
    let mut ws = GraphicMatroidWorkspace::new();
    // warm the workspace so the first-use resize is not counted.
    let _ = rank2_rank(m, &mut ws).unwrap();
    let a0 = allocs();
    let r = rank2_rank(m, &mut ws).unwrap();
    let a1 = allocs();
    assert_eq!(r, 16 * 16 - 1, "connected grid rank = N - 1");
    // a warmed rank pass over an already-sized workspace does not grow the DSU (0 new allocations),
    // but it is NOT O(1): it iterates every arc. The facade is the O(1) part; the pass is Θ(arcs).
    assert_eq!(
        a1 - a0,
        0,
        "a warmed rank pass reuses the workspace (no new allocation)"
    );
}
