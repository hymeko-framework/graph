//! One cycle walk and one parallel fold; policies own selection, not traversal.

use super::{
    BoundedScorer, CyclePruner, DIST_INF, HeapEntry, MAX_INLINE_K, PrunerDecision, SignedGraph,
    TopKCycle, TopKCyclesBatch, UniformityHeuristic, UniformityState, atomic_min_f64,
    bfs_distances_capped, csr_sign_of, heap_capacity_hint,
};
use rayon::prelude::*;
use std::collections::{BinaryHeap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

type Heap = BinaryHeap<HeapEntry>;

/// The CSR is derived once per call from the existing SignedGraph owner.
pub(super) struct CycleSearch<'a, P> {
    row_ptr: Vec<u32>,
    col_idx: Vec<u32>,
    signs: Vec<i8>,
    n: usize,
    k: usize,
    pruner: &'a P,
}

struct WalkScratch {
    visited: Vec<bool>,
    path: Vec<u32>,
    dist: Vec<u8>,
    bfs_a: Vec<u32>,
    bfs_b: Vec<u32>,
}

impl WalkScratch {
    fn new(n: usize, k: usize) -> Self {
        Self {
            visited: vec![false; n],
            path: Vec::with_capacity(k),
            dist: vec![DIST_INF; n],
            bfs_a: Vec::new(),
            bfs_b: Vec::new(),
        }
    }
}

impl<'a, P: CyclePruner> CycleSearch<'a, P> {
    pub(super) fn new(graph: &SignedGraph, k: usize, pruner: &'a P) -> Self {
        let (row_ptr, col_idx, signs) = graph.build_csr_with_signs();
        Self {
            row_ptr,
            col_idx,
            signs,
            n: graph.n_nodes as usize,
            k,
            pruner,
        }
    }

    fn root<S: Selection>(&self, start: u32, walk: &mut WalkScratch, selection: &mut S) {
        bfs_distances_capped(
            &self.row_ptr,
            &self.col_idx,
            start,
            self.k,
            &mut walk.dist,
            &mut walk.bfs_a,
            &mut walk.bfs_b,
        );
        walk.path.clear();
        walk.path.push(start);
        walk.visited[start as usize] = true;
        self.descend(start, walk, selection, 0);
        walk.visited[start as usize] = false;
    }

    fn descend<S: Selection>(
        &self,
        start: u32,
        walk: &mut WalkScratch,
        selection: &mut S,
        negatives: usize,
    ) {
        if walk.path.len() == self.k {
            self.close(start, &walk.path, selection);
            return;
        }
        // Every root has one vertex and recursion only appends before entering.
        let tail = *walk.path.last().expect("nonempty cycle prefix");
        let first = self.row_ptr[tail as usize] as usize;
        let end = self.row_ptr[tail as usize + 1] as usize;
        let remaining = (self.k - walk.path.len()) as u8;
        // A recursion level retains its original threshold even when a deeper
        // visit changes the heap or the cross-worker fullness state.
        let threshold = selection.threshold(start);
        for slot in first..end {
            let next = self.col_idx[slot];
            if next < start || walk.visited[next as usize] {
                continue;
            }
            let distance = walk.dist[next as usize];
            if distance == DIST_INF || distance > remaining {
                continue;
            }
            if self.pruner.extend_ok(&walk.path, next) == PrunerDecision::Reject {
                continue;
            }
            let extension = Extension {
                path: &walk.path,
                next,
                k: self.k,
                negatives: negatives + usize::from(self.signs[slot] < 0),
            };
            if !selection.allows(&extension, threshold) {
                continue;
            }
            let negatives = extension.negatives;
            walk.path.push(next);
            walk.visited[next as usize] = true;
            self.descend(start, walk, selection, negatives);
            walk.path.pop();
            walk.visited[next as usize] = false;
        }
    }

    fn close<S: Selection>(&self, start: u32, path: &[u32], selection: &mut S) {
        // Only a full prefix reaches here, and public callers reject k < 3.
        let last = path[self.k - 1];
        let Some(closing) = csr_sign_of(&self.row_ptr, &self.col_idx, &self.signs, last, start)
        else {
            return;
        };
        if path[1] >= last {
            return;
        }
        debug_assert!(self.k <= MAX_INLINE_K, "k_len exceeds MAX_INLINE_K");
        let mut signs = [0i8; MAX_INLINE_K];
        for j in 0..self.k - 1 {
            // Interior pairs were reached through the CSR adjacency itself.
            signs[j] = csr_sign_of(
                &self.row_ptr,
                &self.col_idx,
                &self.signs,
                path[j],
                path[j + 1],
            )
            .expect("interior edge present");
        }
        signs[self.k - 1] = closing;
        if self.pruner.emit_ok(path, &signs[..self.k]) == PrunerDecision::Accept {
            selection.emit(path, &signs[..self.k]);
        }
    }

    fn parallel<I, S, Init, Empty, Merge>(
        &self,
        starts: I,
        init: Init,
        empty: Empty,
        merge: Merge,
    ) -> S::Heaps
    where
        P: Sync,
        I: ParallelIterator<Item = u32>,
        S: Selection + Send,
        S::Heaps: Send,
        Init: Fn() -> S + Sync + Send,
        Empty: Fn() -> S::Heaps + Sync + Send,
        Merge: Fn(S::Heaps, S::Heaps) -> S::Heaps + Sync + Send,
    {
        starts
            .fold(
                || (init(), WalkScratch::new(self.n, self.k)),
                |(mut selection, mut walk), start| {
                    self.root(start, &mut walk, &mut selection);
                    (selection, walk)
                },
            )
            // Drop walk buffers and heuristic state before reducing, just as the
            // previous implementations did. Reduction allocates only result heaps.
            .map(|(selection, _)| selection.into_heaps())
            .reduce(empty, merge)
    }

    pub(super) fn global_serial<R: ScoringRule>(&self, cap: usize, rule: R) -> Vec<TopKCycle> {
        let mut selection = Global {
            heap: new_heap(cap),
            cap,
            rule,
        };
        let mut walk = WalkScratch::new(self.n, self.k);
        for start in 0..self.n as u32 {
            self.root(start, &mut walk, &mut selection);
        }
        collect_global(self.k, selection.heap)
    }

    pub(super) fn global<O, R, Init>(&self, cap: usize, init: Init) -> O
    where
        P: Sync,
        O: CycleOutput,
        R: ScoringRule + Send,
        Init: Fn() -> R + Sync + Send,
    {
        let heap = self.parallel(
            (0..self.n as u32).into_par_iter(),
            || Global {
                heap: new_heap(cap),
                cap,
                rule: init(),
            },
            || new_heap(cap),
            |mut a, b| {
                merge_heap(&mut a, b, cap);
                a
            },
        );
        collect_global(self.k, heap)
    }

    pub(super) fn vertices_serial<R: ScoringRule>(&self, caps: &[u32], rule: R) -> Vec<TopKCycle> {
        let mut selection = PerVertex {
            heaps: caps
                .iter()
                .map(|&cap| Heap::with_capacity((cap as usize).saturating_add(1)))
                .collect(),
            caps,
            rule,
            bound: RootBound,
        };
        let mut walk = WalkScratch::new(self.n, self.k);
        for start in 0..self.n as u32 {
            self.root(start, &mut walk, &mut selection);
        }
        collect_union(self.k, selection.heaps)
    }

    pub(super) fn vertices<O, R, B, Init>(
        &self,
        caps: &[u32],
        starts: &[u32],
        init: Init,
        bound: B,
    ) -> O
    where
        P: Sync,
        O: CycleOutput,
        R: ScoringRule + Send,
        B: VertexBound + Copy + Sync + Send,
        Init: Fn() -> R + Sync + Send,
    {
        let heaps = self.parallel(
            starts.par_iter().copied(),
            || PerVertex {
                heaps: vec![Heap::new(); self.n],
                caps,
                rule: init(),
                bound,
            },
            || vec![Heap::new(); self.n],
            |mut a, b| {
                for ((av, bv), &cap) in a.iter_mut().zip(b).zip(caps) {
                    merge_heap(av, bv, cap as usize);
                }
                a
            },
        );
        collect_union(self.k, heaps)
    }
}

fn new_heap(cap: usize) -> Heap {
    Heap::with_capacity(heap_capacity_hint(cap))
}

fn merge_heap(a: &mut Heap, b: Heap, cap: usize) {
    if cap == 0 {
        return;
    }
    for entry in b {
        if a.len() < cap {
            a.push(entry);
        } else if a.peek().is_none_or(|min| entry.cmp_preference(min).is_gt()) {
            a.pop();
            a.push(entry);
        }
    }
}

/// Construct entries only after admission. Stateful rules see rollback before
/// the new heap entry is inserted, then update after successful insertion.
fn retain(
    heap: &mut Heap,
    cap: usize,
    score: f64,
    path: &[u32],
    signs: &[i8],
    evict: impl FnOnce(&HeapEntry),
) -> bool {
    if cap == 0 {
        return false;
    }
    if heap.len() >= cap {
        if heap
            .peek()
            .is_some_and(|min| !min.cmp_preference_vs_slice(score, path).is_lt())
        {
            return false;
        }
        if let Some(entry) = heap.pop() {
            evict(&entry);
        }
    }
    heap.push(HeapEntry::from_slices(score, path, signs));
    true
}

fn full_threshold(heap: &Heap, cap: usize) -> f64 {
    if cap > 0 && heap.len() == cap {
        heap.peek().map_or(f64::NEG_INFINITY, |entry| entry.score)
    } else {
        f64::NEG_INFINITY
    }
}

pub(super) struct Extension<'a> {
    path: &'a [u32],
    next: u32,
    k: usize,
    negatives: usize,
}

pub(super) trait ScoringRule {
    fn score(&self, path: &[u32], signs: &[i8]) -> f64;
    fn allows(&self, extension: &Extension<'_>, threshold: Option<f64>) -> bool;
    fn evicted(&mut self, _entry: &HeapEntry) {}
    fn inserted(&mut self, _path: &[u32], _signs: &[i8]) {}
}

pub(super) struct Plain<'a, S>(pub(super) &'a S);
impl<S: Fn(&[u32], &[i8]) -> f64> ScoringRule for Plain<'_, S> {
    fn score(&self, path: &[u32], signs: &[i8]) -> f64 {
        (self.0)(path, signs)
    }
    fn allows(&self, _extension: &Extension<'_>, _threshold: Option<f64>) -> bool {
        true
    }
}

pub(super) struct Bounded<'a, S>(pub(super) &'a S);
impl<S: BoundedScorer> ScoringRule for Bounded<'_, S> {
    fn score(&self, path: &[u32], signs: &[i8]) -> f64 {
        self.0.score(path, signs)
    }
    fn allows(&self, e: &Extension<'_>, threshold: Option<f64>) -> bool {
        // None means the global fullness gate is off: do not call upper_bound.
        threshold.is_none_or(|limit| {
            let bound = self.0.upper_bound(e.negatives, e.k - e.path.len() - 1, e.k);
            // Preserve the original NaN comparison behavior as well.
            !matches!(
                bound.partial_cmp(&limit),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            )
        })
    }
}

pub(super) struct Heuristic<'a, H> {
    heuristic: &'a H,
    state: UniformityState,
}
impl<'a, H> Heuristic<'a, H> {
    pub(super) fn new(heuristic: &'a H, n: usize) -> Self {
        Self {
            heuristic,
            state: UniformityState::new(n),
        }
    }
}
impl<H: UniformityHeuristic> ScoringRule for Heuristic<'_, H> {
    fn score(&self, path: &[u32], signs: &[i8]) -> f64 {
        self.heuristic.score(path, signs, &self.state)
    }
    fn allows(&self, e: &Extension<'_>, threshold: Option<f64>) -> bool {
        let mut prefix = [0u32; MAX_INLINE_K];
        let len = e.path.len() + 1;
        prefix[..e.path.len()].copy_from_slice(e.path);
        prefix[e.path.len()] = e.next;
        let bound = self
            .heuristic
            .upper_bound(&prefix[..len], e.k - len, e.k, &self.state);
        !matches!(
            bound.partial_cmp(&threshold.unwrap_or(f64::NEG_INFINITY)),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        )
    }
    fn evicted(&mut self, entry: &HeapEntry) {
        self.heuristic
            .rollback(entry.vertices(), entry.signs_slice(), &mut self.state);
    }
    fn inserted(&mut self, path: &[u32], signs: &[i8]) {
        self.heuristic.update(path, signs, &mut self.state);
    }
}

trait Selection {
    type Heaps;
    fn threshold(&self, start: u32) -> Option<f64>;
    fn allows(&self, extension: &Extension<'_>, threshold: Option<f64>) -> bool;
    fn emit(&mut self, path: &[u32], signs: &[i8]);
    fn into_heaps(self) -> Self::Heaps;
}

struct Global<R> {
    heap: Heap,
    cap: usize,
    rule: R,
}
impl<R: ScoringRule> Selection for Global<R> {
    type Heaps = Heap;
    fn threshold(&self, _start: u32) -> Option<f64> {
        Some(full_threshold(&self.heap, self.cap))
    }
    fn allows(&self, e: &Extension<'_>, threshold: Option<f64>) -> bool {
        self.rule.allows(e, threshold)
    }
    fn emit(&mut self, path: &[u32], signs: &[i8]) {
        let score = self.rule.score(path, signs);
        if retain(&mut self.heap, self.cap, score, path, signs, |e| {
            self.rule.evicted(e)
        }) {
            self.rule.inserted(path, signs);
        }
    }
    fn into_heaps(self) -> Heap {
        self.heap
    }
}

struct PerVertex<'a, R, B> {
    heaps: Vec<Heap>,
    caps: &'a [u32],
    rule: R,
    bound: B,
}
impl<R: ScoringRule, B: VertexBound> Selection for PerVertex<'_, R, B> {
    type Heaps = Vec<Heap>;
    fn threshold(&self, start: u32) -> Option<f64> {
        self.bound.threshold(start, self.caps, &self.heaps)
    }
    fn allows(&self, e: &Extension<'_>, threshold: Option<f64>) -> bool {
        self.rule.allows(e, threshold)
    }
    fn emit(&mut self, path: &[u32], signs: &[i8]) {
        let score = self.rule.score(path, signs);
        for &vertex in path {
            let cap = self.caps[vertex as usize] as usize;
            let heap = &mut self.heaps[vertex as usize];
            let was_full = heap.len() == cap;
            if retain(heap, cap, score, path, signs, |_| {}) {
                self.bound.retained(vertex, heap, cap, was_full);
            }
        }
    }
    fn into_heaps(self) -> Vec<Heap> {
        self.heaps
    }
}

pub(super) trait VertexBound {
    fn threshold(&self, start: u32, caps: &[u32], heaps: &[Heap]) -> Option<f64>;
    fn retained(&self, _vertex: u32, _heap: &Heap, _cap: usize, _was_full: bool) {}
}

#[derive(Clone, Copy)]
pub(super) struct RootBound;
impl VertexBound for RootBound {
    fn threshold(&self, start: u32, caps: &[u32], heaps: &[Heap]) -> Option<f64> {
        Some(full_threshold(
            &heaps[start as usize],
            caps[start as usize] as usize,
        ))
    }
}

/// One call owns the atomic state; every Rayon fold task borrows it.
pub(super) struct SharedBound {
    min: AtomicU64,
    full: AtomicUsize,
    seen: Vec<AtomicBool>,
    gate_count: usize,
}
impl SharedBound {
    pub(super) fn new(caps: &[u32], gate: f64) -> Self {
        let active = caps.iter().filter(|&&cap| cap > 0).count();
        Self {
            min: AtomicU64::new(f64::INFINITY.to_bits()),
            full: AtomicUsize::new(0),
            seen: (0..caps.len()).map(|_| AtomicBool::new(false)).collect(),
            gate_count: (gate.clamp(0.0, 1.0) * active as f64).ceil() as usize,
        }
    }
}
impl VertexBound for &SharedBound {
    fn threshold(&self, _start: u32, _caps: &[u32], _heaps: &[Heap]) -> Option<f64> {
        let full = self.full.load(Ordering::Relaxed);
        if full > 0 && full >= self.gate_count {
            Some(f64::from_bits(self.min.load(Ordering::Relaxed)))
        } else {
            None
        }
    }
    fn retained(&self, vertex: u32, heap: &Heap, cap: usize, was_full: bool) {
        if heap.len() != cap {
            return;
        }
        if !was_full && !self.seen[vertex as usize].swap(true, Ordering::Relaxed) {
            self.full.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(entry) = heap.peek() {
            atomic_min_f64(&self.min, entry.score);
        }
    }
}

pub(super) trait CycleOutput {
    fn with_capacity(k: usize, capacity: usize) -> Self;
    fn for_union(k: usize, capacity: usize) -> Self;
    fn push(&mut self, entry: HeapEntry);
    fn finish(&mut self);
}
impl CycleOutput for Vec<TopKCycle> {
    fn with_capacity(_k: usize, capacity: usize) -> Self {
        Vec::with_capacity(capacity)
    }
    fn for_union(_k: usize, _capacity: usize) -> Self {
        Vec::new()
    }
    fn push(&mut self, entry: HeapEntry) {
        Vec::push(
            self,
            (
                entry.score,
                entry.vertices().to_vec(),
                entry.signs_slice().to_vec(),
            ),
        );
    }
    fn finish(&mut self) {
        self.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    }
}
impl CycleOutput for TopKCyclesBatch {
    fn with_capacity(k: usize, capacity: usize) -> Self {
        Self::with_capacity(k, capacity)
    }
    fn for_union(k: usize, capacity: usize) -> Self {
        Self::with_capacity(k, capacity)
    }
    fn push(&mut self, entry: HeapEntry) {
        self.push(entry.score, entry.vertices(), entry.signs_slice());
    }
    fn finish(&mut self) {
        self.sort_by_score_desc();
    }
}
fn collect_global<O: CycleOutput>(k: usize, heap: Heap) -> O {
    let mut output = O::with_capacity(k, heap.len());
    for entry in heap {
        output.push(entry);
    }
    output.finish();
    output
}
fn collect_union<O: CycleOutput>(k: usize, heaps: Vec<Heap>) -> O {
    let mut output = O::for_union(k, heaps.iter().map(Heap::len).sum());
    let mut seen = HashSet::new();
    for entry in heaps.into_iter().flatten() {
        let slice = entry.vertices();
        let mut canonical = [0u32; MAX_INLINE_K];
        canonical[..slice.len()].copy_from_slice(slice);
        canonical[..slice.len()].sort_unstable();
        if seen.insert(canonical) {
            output.push(entry);
        }
    }
    output.finish();
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct AdmissionTrace(Mutex<Vec<char>>);
    impl UniformityHeuristic for AdmissionTrace {
        fn score(&self, vs: &[u32], _: &[i8], state: &UniformityState) -> f64 {
            assert_eq!(
                state.total,
                state.counts.iter().map(|&c| u64::from(c)).sum()
            );
            self.0.lock().unwrap().push('S');
            f64::from(vs[0])
        }
        fn update(&self, vs: &[u32], signs: &[i8], state: &mut UniformityState) {
            assert_eq!(state.total, 0, "eviction must roll back before insertion");
            self.0.lock().unwrap().push('U');
            super::super::EntropyGainScorer.update(vs, signs, state);
        }
        fn rollback(&self, vs: &[u32], signs: &[i8], state: &mut UniformityState) {
            assert_eq!(state.total, vs.len() as u64);
            assert!(vs.iter().all(|&v| state.counts[v as usize] == 1));
            self.0.lock().unwrap().push('R');
            super::super::EntropyGainScorer.rollback(vs, signs, state);
        }
        fn upper_bound(&self, _: &[u32], _: usize, _: usize, _: &UniformityState) -> f64 {
            f64::INFINITY
        }
    }

    #[test]
    fn stateful_admission_rolls_back_exactly_once_and_skips_rejected_updates() {
        let trace = AdmissionTrace::default();
        let mut selection = Global {
            heap: new_heap(1),
            cap: 1,
            rule: Heuristic::new(&trace, 6),
        };
        let first = [0, 1, 2];
        let second = [3, 4, 5];
        let signs = [1, -1, 1];
        selection.emit(&first, &signs);
        selection.emit(&second, &signs);
        selection.emit(&first, &signs);
        assert_eq!(*trace.0.lock().unwrap(), ['S', 'U', 'S', 'R', 'U', 'S']);
        assert_eq!(selection.rule.state.counts, [0, 0, 0, 1, 1, 1]);
        assert_eq!(selection.heap.peek().unwrap().vertices(), second);
    }

    #[test]
    fn shared_bound_counts_distinct_full_vertices_and_workspace_reuses_buffers() {
        let caps = [2, 0, 1];
        let bound = SharedBound::new(&caps, 1.0);
        let mut heaps = vec![Heap::new(); 3];
        assert_eq!((&bound).threshold(0, &caps, &heaps), None);
        heaps[2].push(HeapEntry::from_slices(2.0, &[0, 1, 2], &[1, 1, 1]));
        for _ in 0..3 {
            (&bound).retained(2, &heaps[2], 1, false);
        }
        assert_eq!(bound.full.load(Ordering::Relaxed), 1);
        assert_eq!((&bound).threshold(0, &caps, &heaps), None);
        heaps[0].push(HeapEntry::from_slices(1.0, &[0, 1, 2], &[1, 1, 1]));
        heaps[0].push(HeapEntry::from_slices(3.0, &[0, 1, 2], &[1, 1, 1]));
        (&bound).retained(0, &heaps[0], 2, false);
        assert_eq!((&bound).threshold(0, &caps, &heaps), Some(1.0));

        let graph = super::super::tests::build_three_triangles();
        let search = CycleSearch::new(&graph, 3, &super::super::NoOpPruner);
        let mut walk = WalkScratch::new(search.n, 3);
        let mut selection = Global {
            heap: new_heap(2),
            cap: 2,
            rule: Plain(&super::super::scorers::balance),
        };
        search.root(0, &mut walk, &mut selection);
        let pointers = (
            walk.path.as_ptr(),
            walk.visited.as_ptr(),
            walk.dist.as_ptr(),
        );
        let capacities = (
            walk.path.capacity(),
            walk.visited.capacity(),
            walk.dist.capacity(),
        );
        search.root(0, &mut walk, &mut selection);
        assert_eq!(
            pointers,
            (
                walk.path.as_ptr(),
                walk.visited.as_ptr(),
                walk.dist.as_ptr()
            )
        );
        assert_eq!(
            capacities,
            (
                walk.path.capacity(),
                walk.visited.capacity(),
                walk.dist.capacity()
            )
        );
        assert!(walk.visited.iter().all(|&seen| !seen));
        assert_eq!(walk.path, [0]);
    }
}
