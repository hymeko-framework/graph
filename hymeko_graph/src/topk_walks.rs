//! Top-$K$ signed-walk enumeration with admissible-upper-bound pruning.
//!
//! Walk analogue of [`crate::topk_cycles::enumerate_top_k_cycles`].
//! Differences from the cycle path:
//! - Open structure: no closure check at depth `k_len`; the walk has
//!   `k_len` edges and `k_len + 1` vertices.
//! - Canonical-form filter `walk[0] <= walk[walk_len]` halves the
//!   emitted count (matches the convention in `walks_unsigned.rs`).
//! - The `BoundedScorer` trait used here is the same one defined in
//!   [`crate::topk_cycles`]; the admissible upper bound's contract
//!   carries through identically because the score function takes
//!   only `(vs, signs)` and the bound takes only
//!   `(n_neg_so_far, k_remaining, k_len)`.
//!
//! Promoted to library API on 2026-06-03 to close the framework story
//! the Python reference at
//! [`hymeko_neuro::src::core::abb_walks::abb_enumerate_walks`] sketches.
//! The Python path stays the correctness specification; this Rust path
//! is the production runner that drops the per-walk Python object
//! allocation cost.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::signed_graph::SignedGraph;
use crate::topk_cycles::BoundedScorer;

/// Output of [`enumerate_top_k_walks`]: $K$ best walks in descending
/// score order, each as `(score, vertices, edge-signs)`. Vertex count
/// is `walk_len + 1`; sign count is `walk_len`.
pub type TopKWalk = (f64, Vec<u32>, Vec<i8>);

/// Structure-of-Arrays (SoA) output, mirroring
/// [`crate::topk_cycles::TopKCyclesBatch`]. Suitable for zero-copy
/// PyO3 conversion via `into_pyarray()`.
#[derive(Debug, Clone)]
pub struct TopKWalksBatch {
    /// Row-major flat walks, shape (N, walk_len + 1). Length =
    /// N * (walk_len + 1).
    pub walks: Vec<u32>,
    /// Row-major flat signs, shape (N, walk_len). Length = N * walk_len.
    pub signs: Vec<i8>,
    /// Per-walk scores. Length = N.
    pub scores: Vec<f64>,
    /// Walk length (number of edges).
    pub walk_len: usize,
}

impl TopKWalksBatch {
    /// Number of walks retained.
    pub fn len(&self) -> usize {
        self.scores.len()
    }
    /// True iff the batch holds no walks.
    pub fn is_empty(&self) -> bool {
        self.scores.is_empty()
    }
}

const MAX_INLINE_WALK_LEN: usize = 16;

/// Heap entry: `[walk_len + 1]` vertices + `[walk_len]` signs inline
/// to avoid per-entry `Vec` allocation. The heap is ordered such that
/// the least-preferred entry is at the top (next eviction candidate);
/// this matches the cycle pattern and lets us call `peek()` to compare
/// against the current top-$K$ threshold.
type HeapEntry = crate::ranked_path::RankedPath<{ MAX_INLINE_WALK_LEN + 1 }, MAX_INLINE_WALK_LEN>;

/// Enumerate the top-$K$ open simple walks of length `walk_len` by
/// `scorer.score`, with admissible-upper-bound DFS pruning.
///
/// Canonical form: emit `walk` only if `walk[0] <= walk[walk_len]`
/// (halves the count over both directions). Simple walks: no vertex
/// revisit within a walk.
///
/// Returns the surviving walks in descending score order.
pub fn enumerate_top_k_walks<S: BoundedScorer>(
    graph: &SignedGraph,
    walk_len: usize,
    top_k: usize,
    scorer: &S,
) -> Vec<TopKWalk> {
    if walk_len == 0 || top_k == 0 {
        return Vec::new();
    }
    if walk_len > MAX_INLINE_WALK_LEN {
        // Fall back to Vec-backed entries would change the API; for
        // the production HSiKAN range walk_len <= 5 we never hit this.
        // A future refactor could lift the inline cap if needed.
        return Vec::new();
    }
    WalkSearch::new(graph, walk_len, top_k, scorer).run()
}

/// SoA-output variant of [`enumerate_top_k_walks`] for zero-copy PyO3
/// conversion. Functionally equivalent (same walks, same order); the
/// difference is the return layout.
pub fn enumerate_top_k_walks_batch<S: BoundedScorer>(
    graph: &SignedGraph,
    walk_len: usize,
    top_k: usize,
    scorer: &S,
) -> TopKWalksBatch {
    let v = enumerate_top_k_walks(graph, walk_len, top_k, scorer);
    let n = v.len();
    let mut walks: Vec<u32> = Vec::with_capacity(n * (walk_len + 1));
    let mut signs: Vec<i8> = Vec::with_capacity(n * walk_len);
    let mut scores: Vec<f64> = Vec::with_capacity(n);
    for (sc, vs, ss) in v {
        walks.extend_from_slice(&vs);
        signs.extend_from_slice(&ss);
        scores.push(sc);
    }
    TopKWalksBatch {
        walks,
        signs,
        scores,
        walk_len,
    }
}

/// One invocation owns its CSR, selection limits and backtracking state.
/// The exhaustive test oracle remains separate from this bounded search.
struct WalkSearch<'a, S> {
    row_ptr: Vec<u32>,
    col_idx: Vec<u32>,
    signs_csr: Vec<i8>,
    walk_len: usize,
    top_k: usize,
    scorer: &'a S,
    visited: Vec<bool>,
    path: Vec<u32>,
    signs: Vec<i8>,
    heap: BinaryHeap<HeapEntry>,
}

impl<'a, S: BoundedScorer> WalkSearch<'a, S> {
    fn new(graph: &SignedGraph, walk_len: usize, top_k: usize, scorer: &'a S) -> Self {
        // Public entry validation precedes all allocation, as before.
        debug_assert!(walk_len > 0 && walk_len <= MAX_INLINE_WALK_LEN && top_k > 0);
        let (row_ptr, col_idx, signs_csr) = graph.build_csr_with_signs();
        Self {
            row_ptr,
            col_idx,
            signs_csr,
            walk_len,
            top_k,
            scorer,
            visited: vec![false; graph.n_nodes as usize],
            path: Vec::with_capacity(walk_len + 1),
            signs: Vec::with_capacity(walk_len),
            heap: BinaryHeap::with_capacity(top_k + 1),
        }
    }

    fn run(mut self) -> Vec<TopKWalk> {
        for start in 0..self.visited.len() as u32 {
            self.visit_root(start);
        }
        let mut out: Vec<_> = self
            .heap
            .into_iter()
            .map(|e| (e.score, e.vertices().to_vec(), e.signs_slice().to_vec()))
            .collect();
        out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        out
    }

    fn visit_root(&mut self, start: u32) {
        self.path.clear();
        self.signs.clear();
        self.path.push(start);
        self.visited[start as usize] = true;
        self.descend(start);
        self.visited[start as usize] = false;
    }

    fn descend(&mut self, start: u32) {
        if self.signs.len() == self.walk_len {
            self.offer_complete(start);
            return;
        }
        if self.dominated() {
            return;
        }
        // visit_root starts a nonempty path; recursion only pushes before entry.
        let tail = *self.path.last().expect("nonempty walk prefix");
        let first = self.row_ptr[tail as usize] as usize;
        let end = self.row_ptr[tail as usize + 1] as usize;
        for edge in first..end {
            let next = self.col_idx[edge];
            if self.visited[next as usize] {
                continue;
            }
            self.path.push(next);
            self.signs.push(self.signs_csr[edge]);
            self.visited[next as usize] = true;
            self.descend(start);
            self.path.pop();
            self.signs.pop();
            self.visited[next as usize] = false;
        }
    }

    fn dominated(&self) -> bool {
        if self.heap.len() < self.top_k {
            return false;
        }
        let negatives = self.signs.iter().filter(|&&s| s < 0).count();
        let remaining = self.walk_len - self.signs.len();
        let upper = self.scorer.upper_bound(negatives, remaining, self.walk_len);
        self.heap.peek().is_some_and(|worst| upper <= worst.score)
    }

    fn offer_complete(&mut self, start: u32) {
        // A complete walk has walk_len+1 vertices, and walk_len is nonzero.
        let last = *self.path.last().expect("complete walk has vertices");
        if start > last {
            return;
        }
        let score = self.scorer.score(&self.path, &self.signs);
        if self.heap.len() < self.top_k {
            self.heap
                .push(HeapEntry::from_slices(score, &self.path, &self.signs));
        } else if let Some(worst) = self.heap.peek() {
            let candidate = HeapEntry::from_slices(score, &self.path, &self.signs);
            if candidate.cmp_preference(worst) == Ordering::Greater {
                self.heap.pop();
                self.heap.push(candidate);
            }
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signed_graph::SignedGraph;
    use crate::topk_cycles::{BalanceScorer, FractionNegativeScorer};

    fn walk_observations<S: BoundedScorer>(scorer: &S) -> u64 {
        let mut hash = 0xcbf29ce484222325_u64;
        for graph in [
            toy_graph(),
            SignedGraph {
                n_nodes: 0,
                edges: vec![],
                signs: vec![],
            },
        ] {
            for length in [0, 1, 2, 3, 4, 16, 17] {
                for cap in [0, 1, 3, 20] {
                    let rows = enumerate_top_k_walks(&graph, length, cap, scorer);
                    let batch = enumerate_top_k_walks_batch(&graph, length, cap, scorer);
                    let encoded = format!("{rows:?}{batch:?}");
                    hash = encoded
                        .bytes()
                        .fold(hash, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
                }
            }
        }
        hash
    }

    #[test]
    fn frozen_walk_selection_and_batch_layout() {
        let observed = [
            walk_observations(&FractionNegativeScorer),
            walk_observations(&BalanceScorer),
        ];
        assert_eq!(observed, [13895120261424007879, 17559861434204650573]);
    }

    #[test]
    fn search_state_cleans_each_root_and_retains_its_buffers() {
        let graph = toy_graph();
        let mut search = WalkSearch::new(&graph, 3, 1, &FractionNegativeScorer);
        let pointers = (
            search.path.as_ptr(),
            search.signs.as_ptr(),
            search.visited.as_ptr(),
            search.heap.as_slice().as_ptr(),
        );
        let capacities = (
            search.path.capacity(),
            search.signs.capacity(),
            search.visited.capacity(),
            search.heap.capacity(),
        );
        for start in 0..graph.n_nodes {
            search.visit_root(start);
            assert_eq!(search.path, [start]);
            assert!(search.signs.is_empty());
            assert!(search.visited.iter().all(|&seen| !seen));
            assert!(search.heap.len() <= 1);
            assert_eq!(
                pointers,
                (
                    search.path.as_ptr(),
                    search.signs.as_ptr(),
                    search.visited.as_ptr(),
                    search.heap.as_slice().as_ptr()
                )
            );
            assert_eq!(
                capacities,
                (
                    search.path.capacity(),
                    search.signs.capacity(),
                    search.visited.capacity(),
                    search.heap.capacity()
                )
            );
        }
    }

    fn toy_graph() -> SignedGraph {
        // Triangle with mixed signs + one extra edge to a 4th vertex.
        // Vertices: 0..4. Edges: (0,1)+, (1,2)-, (0,2)+, (2,3)-.
        // ``signs`` are i8 in ±1 (Pos / Neg) per the canonical layout.
        let eu: [u32; 4] = [0, 1, 0, 2];
        let ev: [u32; 4] = [1, 2, 2, 3];
        let es: [i8; 4] = [1, -1, 1, -1];
        SignedGraph::from_parts(4, &eu, &ev, &es)
    }

    #[test]
    fn enumerate_top_k_walks_returns_at_most_k() {
        let g = toy_graph();
        let s = FractionNegativeScorer;
        let v = enumerate_top_k_walks(&g, 2, 3, &s);
        assert!(v.len() <= 3);
    }

    #[test]
    fn enumerate_top_k_walks_top_k_zero_is_empty() {
        let g = toy_graph();
        let s = BalanceScorer;
        let v = enumerate_top_k_walks(&g, 2, 0, &s);
        assert!(v.is_empty());
    }

    #[test]
    fn enumerate_top_k_walks_walk_len_zero_is_empty() {
        let g = toy_graph();
        let s = BalanceScorer;
        let v = enumerate_top_k_walks(&g, 0, 5, &s);
        assert!(v.is_empty());
    }

    #[test]
    fn enumerate_top_k_walks_descending_by_score() {
        let g = toy_graph();
        let s = FractionNegativeScorer;
        let v = enumerate_top_k_walks(&g, 2, 10, &s);
        for w in v.windows(2) {
            assert!(
                w[0].0 >= w[1].0,
                "scores not in descending order: {:?}",
                v.iter().map(|x| x.0).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn enumerate_top_k_walks_canonical_form_holds() {
        let g = toy_graph();
        let s = BalanceScorer;
        let v = enumerate_top_k_walks(&g, 3, 100, &s);
        for (_score, walk, signs) in &v {
            assert_eq!(signs.len(), 3);
            assert_eq!(walk.len(), 4);
            assert!(
                walk[0] <= walk[walk.len() - 1],
                "canonical form violated: walk={walk:?}"
            );
        }
    }

    #[test]
    fn enumerate_top_k_walks_no_vertex_revisit() {
        let g = toy_graph();
        let s = BalanceScorer;
        let v = enumerate_top_k_walks(&g, 3, 100, &s);
        for (_score, walk, _signs) in &v {
            let mut sorted = walk.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                walk.len(),
                "vertex revisited in walk: {walk:?}"
            );
        }
    }

    #[test]
    fn enumerate_top_k_walks_batch_shape_consistent() {
        let g = toy_graph();
        let s = FractionNegativeScorer;
        let batch = enumerate_top_k_walks_batch(&g, 2, 5, &s);
        let n = batch.len();
        assert_eq!(batch.walks.len(), n * (batch.walk_len + 1));
        assert_eq!(batch.signs.len(), n * batch.walk_len);
        assert_eq!(batch.scores.len(), n);
    }

    /// Brute reference: enumerate every simple walk of length k from
    /// every start, score it, filter canonical, sort, take K. Used to
    /// verify the heap-bounded ABB result matches the exhaustive top-K.
    fn brute_top_k<S: BoundedScorer>(
        g: &SignedGraph,
        walk_len: usize,
        top_k: usize,
        scorer: &S,
    ) -> Vec<TopKWalk> {
        let (row_ptr, col_idx, signs_csr) = g.build_csr_with_signs();
        let n = g.n_nodes as usize;
        let mut all: Vec<TopKWalk> = Vec::new();

        /// The immutable inputs of the recursion, bundled so `rec` takes a context and its
        /// mutable frontier rather than ten positional parameters (CLAUDE.md 6.5 #6).
        struct RecCtx<'a> {
            row_ptr: &'a [u32],
            col_idx: &'a [u32],
            signs_csr: &'a [i8],
            start: u32,
            walk_len: usize,
            scorer: &'a dyn crate::topk_cycles::BoundedScorer,
        }
        /// The mutable state the recursion threads through.
        struct RecState<'a> {
            path: &'a mut Vec<u32>,
            signs: &'a mut Vec<i8>,
            visited: &'a mut Vec<bool>,
            all: &'a mut Vec<TopKWalk>,
        }
        fn rec(ctx: &RecCtx<'_>, st: &mut RecState<'_>) {
            let (row_ptr, col_idx, signs_csr, start, walk_len, scorer) = (
                ctx.row_ptr,
                ctx.col_idx,
                ctx.signs_csr,
                ctx.start,
                ctx.walk_len,
                ctx.scorer,
            );
            let (path, signs, visited, all) = (
                &mut *st.path,
                &mut *st.signs,
                &mut *st.visited,
                &mut *st.all,
            );
            if signs.len() == walk_len {
                if start <= *path.last().unwrap() {
                    let sc = scorer.score(path, signs);
                    all.push((sc, path.clone(), signs.clone()));
                }
                return;
            }
            let tail = *path.last().unwrap();
            let s = row_ptr[tail as usize] as usize;
            let e = row_ptr[tail as usize + 1] as usize;
            for ei in s..e {
                let nxt = col_idx[ei];
                if visited[nxt as usize] {
                    continue;
                }
                let sign = signs_csr[ei];
                path.push(nxt);
                signs.push(sign);
                visited[nxt as usize] = true;
                rec(
                    ctx,
                    &mut RecState {
                        path,
                        signs,
                        visited,
                        all,
                    },
                );
                path.pop();
                signs.pop();
                visited[nxt as usize] = false;
            }
        }

        let mut visited = vec![false; n];
        let mut path = Vec::new();
        let mut signs = Vec::new();
        for start in 0..(n as u32) {
            path.clear();
            signs.clear();
            path.push(start);
            visited[start as usize] = true;
            let ctx = RecCtx {
                row_ptr: &row_ptr,
                col_idx: &col_idx,
                signs_csr: &signs_csr,
                start,
                walk_len,
                scorer,
            };
            rec(
                &ctx,
                &mut RecState {
                    path: &mut path,
                    signs: &mut signs,
                    visited: &mut visited,
                    all: &mut all,
                },
            );
            visited[start as usize] = false;
        }
        all.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        all.truncate(top_k);
        all
    }

    #[test]
    fn enumerate_top_k_walks_matches_brute_force_scores() {
        // The ABB top-K MUST produce the same score multiset as a
        // brute-force enumerate+sort+take_k. Tied-score walks may
        // differ row-by-row; the multiset equality is the contract.
        let g = toy_graph();
        let s = FractionNegativeScorer;
        for walk_len in [2, 3] {
            for k in [1, 3, 7, 100] {
                let abb = enumerate_top_k_walks(&g, walk_len, k, &s);
                let brute = brute_top_k(&g, walk_len, k, &s);
                let abb_scores: Vec<f64> = abb.iter().map(|x| x.0).collect();
                let brute_scores: Vec<f64> = brute.iter().map(|x| x.0).collect();
                assert_eq!(
                    abb_scores, brute_scores,
                    "ABB top-{k} (walk_len={walk_len}) scores differ from brute: {abb_scores:?} vs {brute_scores:?}"
                );
            }
        }
    }
}
