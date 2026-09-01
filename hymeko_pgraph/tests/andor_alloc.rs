//! Warm-workspace-reuse proof for the AND/OR fixpoint, in its own single-test binary (a global counting
//! allocator counts every thread, so this is the only test here; mirrors the direct_ir alloc test).
//!
//! `propagate` allocates its owned result (`Availability`: one `Vec` + the producible `BTreeSet`,
//! `O(materials)`) — that is unavoidable. What must be allocation-free is the **fixpoint loop** over the
//! caller-owned workspace. We prove it by showing two consecutive warm calls allocate an **identical**
//! (and small, result-sized) count: if the loop touched the allocator, the counts would track the
//! iteration work, not just the result.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use hymeko::ir::incidence::build_incidence_index;
use hymeko::module_store::module_store::ModuleStore;
use hymeko::module_store::source_provider::MemProvider;
use hymeko::util::real_parser::RealParser;

use hymeko_pgraph::andor::{
    AndOrWorkspace, BooleanExact, PropagationConfig, UnitReliability, propagate,
};
use hymeko_pgraph::meta_resolve::lower_resolved;
use hymeko_pgraph::view::PGraphView;

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

const META: &str = include_str!("../data/meta_pgraph.hymeko");

/// A deterministic chain of `n` intermediates from one raw to a product (n worklist stages).
fn chain_src(n: usize) -> String {
    let mut b = String::from(
        "R {\n    @\"meta_pgraph.hymeko\";\n    using pgraph.raw as raw;\n    using pgraph.product as product;\n    using pgraph.intermediate as inter;\n    using pgraph.process as process;\n}\nbody {\n",
    );
    b.push_str("    M0: + <isa> raw {}\n");
    for i in 1..n {
        let role = if i == n - 1 { "product" } else { "inter" };
        b.push_str(&format!("    M{i}: + <isa> {role} {{}}\n"));
    }
    for i in 1..n {
        b.push_str(&format!("    @U{i} {{ (+M{i}, -M{}); }}\n", i - 1));
    }
    b.push_str("}\n");
    b
}

#[test]
fn warm_propagation_loop_is_allocation_free() {
    let src = chain_src(300);
    let provider = MemProvider::default()
        .with_file("m.hymeko", src.as_str())
        .with_file("meta_pgraph.hymeko", META);
    let mut ms = ModuleStore::new(provider, RealParser);
    let prog = ms
        .compile(std::path::Path::new("m.hymeko"))
        .expect("compile");
    let lowered = lower_resolved(&prog.ir, &ms.it).expect("lower");
    let index = build_incidence_index(&prog.ir).expect("index");
    let view = PGraphView::from_lowered(&index, &lowered);
    let cfg = PropagationConfig::boolean_exact(lowered.units.len(), lowered.materials.len());
    let mut ws = AndOrWorkspace::new();

    // Warm-up: sizes the workspace arrays and grows the queue/pending buffers to their peak.
    for _ in 0..3 {
        let a = propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg).unwrap();
        assert!(a.producible().len() >= 300);
    }

    // Two consecutive warm calls must allocate an identical, small (result-sized) count.
    let before = ALLOCS.load(Ordering::Relaxed);
    let a = propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg).unwrap();
    let mid = ALLOCS.load(Ordering::Relaxed);
    let b = propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg).unwrap();
    let after = ALLOCS.load(Ordering::Relaxed);
    let allocs_a = mid - before;
    let allocs_b = after - mid;

    assert_eq!(a.producible(), b.producible());
    assert_eq!(
        allocs_a, allocs_b,
        "warm calls must allocate identically (loop reuses the workspace)"
    );
    // The count is the owned result only (one Vec + the producible BTreeSet), NOT the ~300 fixpoint
    // iterations — a loop that touched the allocator would scale with the work, not the result.
    assert!(
        allocs_a < 300,
        "warm allocations {allocs_a} must be result-sized, not iteration-scaled"
    );
}
