//! Requested Rust heap bytes, separate from allocator overhead and OS residency.
//! Compiled only under the explicit evidence-heap feature; no custom allocation policy.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

#[derive(Default)]
struct Counters {
    live: AtomicUsize,
    peak: AtomicUsize,
}
impl Counters {
    fn add(&self, size: usize) {
        let live = self.live.fetch_add(size, Relaxed) + size;
        self.peak.fetch_max(live, Relaxed);
    }
    fn remove(&self, size: usize) {
        self.live.fetch_sub(size, Relaxed);
    }
    fn resize(&self, old: usize, new: usize) {
        if new >= old {
            self.add(new - old);
        } else {
            self.remove(old - new);
        }
    }
}
static COUNTERS: Counters = Counters {
    live: AtomicUsize::new(0),
    peak: AtomicUsize::new(0),
};

pub struct ObservedSystem;
// SAFETY: all pointers/layouts pass unchanged to System, including realloc.
// The observer uses only atomics, never allocation, locks or panicking checks.
unsafe impl GlobalAlloc for ObservedSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            COUNTERS.add(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            COUNTERS.add(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(ptr, layout);
        }
        COUNTERS.remove(layout.size());
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(ptr, layout, new_size) };
        if !next.is_null() {
            COUNTERS.resize(layout.size(), new_size);
        }
        next
    }
}

pub fn live() -> usize {
    COUNTERS.live.load(Relaxed)
}
pub fn peak() -> usize {
    COUNTERS.peak.load(Relaxed).max(live())
}
pub fn reset_peak() {
    COUNTERS.peak.store(live(), Relaxed);
}

#[test]
fn counter_preserves_live_ownership_across_growth_shrink_and_release() {
    let c = Counters::default();
    c.add(32);
    c.add(96);
    assert_eq!(c.live.load(Relaxed), 128);
    c.resize(96, 32);
    assert_eq!(c.live.load(Relaxed), 64);
    assert_eq!(c.peak.load(Relaxed), 128);
    c.resize(32, 80);
    assert_eq!(c.live.load(Relaxed), 112);
    c.remove(80);
    c.remove(32);
    assert_eq!(c.live.load(Relaxed), 0);
}
