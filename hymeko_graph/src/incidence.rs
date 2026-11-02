//! Line-graph-style edge → cycle incidence assembly.
//!
//! Migrated from `hymeko_py/src/incidence.rs` (2026-05-22 CLAUDE.md
//! §6.5 anti-pattern #2 cleanup): the construction of the
//! per-query-edge → tuple-index sparse incidence matrix `M_e` (1/|N|
//! normalised) is pure CSR / sorted-merge arithmetic and belongs in
//! the algorithm crate, not the PyO3 binding.
//!
//! For each query edge (u, v), `M_e[query, t] = 1/|N(query)|` iff
//! cycle / hyperedge `t` shares an endpoint with the query (one of
//! `t`'s vertices == u or v).  Self-edges (k=2) are optionally
//! excluded by supplying a `self_map` from sorted-pair (min, max)
//! keys to the tuple index they identify.
//!
//! ## Public API
//!
//! Two entries with shared semantics:
//!
//! - [`build_edge_incidence_vertex_adj`] — original signature, returns
//!   [`IncidenceCoo`].  Bit-for-bit identical to the pre-2026-05-22
//!   behaviour.  Internally delegates to [`build_edge_incidence`] with
//!   [`BuildOpts::default()`].
//! - [`build_edge_incidence`] — Strategy entry; takes [`BuildOpts`]
//!   (parallel toggle, bitset threshold, output format).  See the
//!   2026-05-22 fast-path plan in
//!   `docs/plans/2026-05-22-incidence-vertex-adj-fast/`.

use rayon::prelude::*;

// ─────────────────────────────────────────────────────────────────────
// Bitset adjacency (plan 4.3 — replaces sorted-merge for low-T graphs)
// ─────────────────────────────────────────────────────────────────────

/// Per-vertex bitset adjacency: bit `t` of `row(v)` is set iff tuple
/// `t` is incident to vertex `v`.  Memory cost is
/// `n_vertices · ceil(n_tuples / 64) · 8` bytes; gate at
/// `BuildOpts::bitset_threshold` so the path is only taken when the
/// bitset fits in L3 cache.
struct BitsetAdj {
    n_words: usize,
    bits: Vec<u64>, // [n_vertices * n_words]
}

impl BitsetAdj {
    fn build(csr_row_ptr: &[u32], csr_col_idx: &[u32], n_tuples: usize) -> Self {
        let n_words = n_tuples.div_ceil(64).max(1);
        let n_vertices = csr_row_ptr.len().saturating_sub(1);
        let mut bits = vec![0u64; n_vertices * n_words];
        for v in 0..n_vertices {
            let lo = csr_row_ptr[v] as usize;
            let hi = csr_row_ptr[v + 1] as usize;
            for &t in &csr_col_idx[lo..hi] {
                let t = t as usize;
                bits[v * n_words + t / 64] |= 1u64 << (t % 64);
            }
        }
        Self { n_words, bits }
    }

    #[inline]
    fn row(&self, v: u32) -> &[u64] {
        let v = v as usize;
        &self.bits[v * self.n_words..(v + 1) * self.n_words]
    }
}

// ─────────────────────────────────────────────────────────────────────
// Sorted-pair self-edge map (plan 4.2 — replaces the prior HashMap)
// ─────────────────────────────────────────────────────────────────────

/// Compact, cache-friendly self-edge lookup.
///
/// Keys are encoded as `(lo as u64) << 32 | hi as u64` and stored
/// sorted in a contiguous `Vec<u64>` so the lookup is a single
/// `binary_search` over packed memory.  Replaces the per-query
/// `HashMap<(u32, u32), u32>` lookup, which suffered cache-line
/// ping-pong under Rayon parallel dispatch.
struct SelfMap {
    /// Sorted `(lo << 32) | hi` packed keys.
    keys: Vec<u64>,
    /// Parallel-indexed tuple ids.
    tuple_idx: Vec<u32>,
}

impl SelfMap {
    fn build(self_keys_u: &[u32], self_keys_v: &[u32], self_tuple_idx: &[u32]) -> Self {
        let n = self_keys_u.len();
        let mut pairs: Vec<(u64, u32)> = Vec::with_capacity(n);
        for i in 0..n {
            let lo = self_keys_u[i] as u64;
            let hi = self_keys_v[i] as u64;
            let key = (lo << 32) | hi;
            pairs.push((key, self_tuple_idx[i]));
        }
        pairs.sort_unstable_by_key(|p| p.0);
        let mut keys = Vec::with_capacity(n);
        let mut tuple_idx = Vec::with_capacity(n);
        for (k, t) in pairs {
            keys.push(k);
            tuple_idx.push(t);
        }
        Self { keys, tuple_idx }
    }

    #[inline]
    fn lookup(&self, k_lo: u32, k_hi: u32) -> Option<u32> {
        if self.keys.is_empty() {
            return None;
        }
        let key = ((k_lo as u64) << 32) | (k_hi as u64);
        match self.keys.binary_search(&key) {
            Ok(idx) => Some(self.tuple_idx[idx]),
            Err(_) => None,
        }
    }
}

/// COO triplet result of [`build_edge_incidence_vertex_adj`].
#[derive(Debug, Clone, Default)]
pub struct IncidenceCoo {
    /// Row indices = query-edge indices (length == `nnz`).
    pub rows: Vec<u32>,
    /// Column indices = tuple indices (length == `nnz`).
    pub cols: Vec<u32>,
    /// Values, all equal to `1 / |N(query)|` for the matching row.
    pub vals: Vec<f32>,
}

/// Output format selector for [`build_edge_incidence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncidenceOutput {
    /// Plain COO triplet (`rows`, `cols`, `vals`).  Row order is
    /// `0..E_query` in the serial path and *chunk* order in the
    /// parallel path (rows within a chunk stay ascending).
    Coo,
    /// CSR-ready: `row_ptr` of length `E_query + 1`, `cols` and `vals`
    /// grouped by row.  PyTorch's `sparse_csr_tensor` can consume this
    /// directly and skip the coalesce pass.
    Csr,
}

/// Dispatch knobs for [`build_edge_incidence`].
///
/// Serial and parallel execution share the same chunk walker. Adjacency is
/// either sorted CSR merging or an explicitly enabled bitset representation.
#[derive(Debug, Clone, Copy)]
pub struct BuildOpts {
    /// If `true`, run the per-edge work via Rayon `par_chunks(1024)`.
    /// Default `false` (matches the pre-2026-05-22 serial path).
    pub parallel: bool,
    /// Enable bitset adjacency when the nonempty tuple-ID domain is at most
    /// this size. `0` disables it. Memory depends on both vertices and tuples.
    pub bitset_threshold: u32,
    /// Output format; see [`IncidenceOutput`].
    pub output: IncidenceOutput,
}

impl Default for BuildOpts {
    fn default() -> Self {
        Self {
            parallel: false,
            bitset_threshold: 0,
            output: IncidenceOutput::Coo,
        }
    }
}

/// Discriminated return of [`build_edge_incidence`].
#[derive(Debug, Clone)]
pub enum IncidenceResult {
    /// COO triplet variant.
    Coo(IncidenceCoo),
    /// CSR variant.  `row_ptr.len() == E_query + 1`.
    Csr {
        /// `row_ptr[e]..row_ptr[e+1]` slice indexes `cols`/`vals` for
        /// query edge `e`.
        row_ptr: Vec<u32>,
        /// Column indices grouped by row.
        cols: Vec<u32>,
        /// Values grouped by row (all `1 / |N(e)|` for row `e`).
        vals: Vec<f32>,
    },
}

// ─────────────────────────────────────────────────────────────────────
// Per-edge work (shared serial + parallel)
// ─────────────────────────────────────────────────────────────────────
//
// The per-edge helpers write directly into the caller's output vectors
// to avoid the per-edge `Vec<u32>` allocation that an earlier draft of
// this module incurred via `std::mem::take(scratch)`.

/// The COO triplet buffers one edge writes into.
///
/// Bundled so the two `process_one_edge_*` helpers take a context rather than a flat parameter
/// list (CLAUDE.md 6.5 #6: a config struct, not an `#[allow]`). Purely internal -- no public
/// signature changes.
struct CooSink<'a> {
    rows: &'a mut Vec<u32>,
    cols: &'a mut Vec<u32>,
    vals: &'a mut Vec<f32>,
}

impl CooSink<'_> {
    /// Emit one complete triplet; the three parallel buffers grow together.
    #[inline]
    fn push(&mut self, row: u32, column: u32, value: f32) {
        self.rows.push(row);
        self.cols.push(column);
        self.vals.push(value);
    }
}

impl IncidenceCoo {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            rows: Vec::with_capacity(capacity),
            cols: Vec::with_capacity(capacity),
            vals: Vec::with_capacity(capacity),
        }
    }

    fn sink(&mut self) -> CooSink<'_> {
        CooSink {
            rows: &mut self.rows,
            cols: &mut self.cols,
            vals: &mut self.vals,
        }
    }

    fn append(&mut self, other: Self) {
        self.rows.extend(other.rows);
        self.cols.extend(other.cols);
        self.vals.extend(other.vals);
    }
}

/// Only adjacency arithmetic varies. Allocation, query order and output
/// assembly belong to the shared runner; scratch stays local to a chunk.
trait EdgeKernel: Sync {
    type Scratch;
    fn scratch(&self) -> Self::Scratch;
    fn process(
        &self,
        row: u32,
        u: u32,
        v: u32,
        scratch: &mut Self::Scratch,
        sink: &mut CooSink<'_>,
    );
}

impl EdgeKernel for CsrView<'_> {
    type Scratch = Vec<u32>;
    fn scratch(&self) -> Self::Scratch {
        Vec::with_capacity(64)
    }
    #[inline]
    fn process(
        &self,
        row: u32,
        u: u32,
        v: u32,
        scratch: &mut Self::Scratch,
        sink: &mut CooSink<'_>,
    ) {
        process_one_edge_into(row, u, v, self, scratch, sink);
    }
}

impl EdgeKernel for BitsetView<'_> {
    type Scratch = Vec<u64>;
    fn scratch(&self) -> Self::Scratch {
        Vec::with_capacity(self.adj.n_words)
    }
    #[inline]
    fn process(
        &self,
        row: u32,
        u: u32,
        v: u32,
        scratch: &mut Self::Scratch,
        sink: &mut CooSink<'_>,
    ) {
        process_one_edge_bitset_into(row, u, v, self, scratch, sink);
    }
}

/// The adjacency view `process_one_edge_into` reads.
struct CsrView<'a> {
    n_nodes: usize,
    row_ptr: &'a [u32],
    col_idx: &'a [u32],
    self_map: &'a SelfMap,
}

#[inline]
fn process_one_edge_into(
    ei: u32,
    u: u32,
    v: u32,
    csr: &CsrView<'_>,
    scratch: &mut Vec<u32>,
    sink: &mut CooSink<'_>,
) {
    let (n_nodes, csr_row_ptr, csr_col_idx, self_map) =
        (csr.n_nodes, csr.row_ptr, csr.col_idx, csr.self_map);
    if (u as usize) >= n_nodes || (v as usize) >= n_nodes {
        return;
    }

    let u_start = csr_row_ptr[u as usize] as usize;
    let u_end = csr_row_ptr[u as usize + 1] as usize;
    let v_start = csr_row_ptr[v as usize] as usize;
    let v_end = csr_row_ptr[v as usize + 1] as usize;

    let adj_u = &csr_col_idx[u_start..u_end];
    let adj_v = &csr_col_idx[v_start..v_end];

    scratch.clear();
    scratch.reserve(adj_u.len() + adj_v.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < adj_u.len() && j < adj_v.len() {
        let a = adj_u[i];
        let b = adj_v[j];
        if a < b {
            if scratch.last().copied() != Some(a) {
                scratch.push(a);
            }
            i += 1;
        } else if b < a {
            if scratch.last().copied() != Some(b) {
                scratch.push(b);
            }
            j += 1;
        } else {
            if scratch.last().copied() != Some(a) {
                scratch.push(a);
            }
            i += 1;
            j += 1;
        }
    }
    while i < adj_u.len() {
        let a = adj_u[i];
        if scratch.last().copied() != Some(a) {
            scratch.push(a);
        }
        i += 1;
    }
    while j < adj_v.len() {
        let b = adj_v[j];
        if scratch.last().copied() != Some(b) {
            scratch.push(b);
        }
        j += 1;
    }

    let (k_lo, k_hi) = if u < v { (u, v) } else { (v, u) };
    if let Some(self_t) = self_map.lookup(k_lo, k_hi)
        && let Ok(pos) = scratch.binary_search(&self_t)
    {
        scratch.remove(pos);
    }

    let n_adj = scratch.len();
    if n_adj == 0 {
        return;
    }
    let w = 1.0_f32 / n_adj as f32;
    for &t in scratch.iter() {
        sink.push(ei, t, w);
    }
}

/// The bitset adjacency view `process_one_edge_bitset_into` reads.
struct BitsetView<'a> {
    n_vertices: usize,
    adj: &'a BitsetAdj,
    self_map: &'a SelfMap,
}

#[inline]
fn process_one_edge_bitset_into(
    ei: u32,
    u: u32,
    v: u32,
    view: &BitsetView<'_>,
    word_scratch: &mut Vec<u64>,
    sink: &mut CooSink<'_>,
) {
    let (n_vertices, adj, self_map) = (view.n_vertices, view.adj, view.self_map);
    if (u as usize) >= n_vertices || (v as usize) >= n_vertices {
        return;
    }
    let rows_u = adj.row(u);
    let rows_v = adj.row(v);
    word_scratch.clear();
    word_scratch.reserve(adj.n_words);
    for w in 0..adj.n_words {
        word_scratch.push(rows_u[w] | rows_v[w]);
    }

    let (k_lo, k_hi) = if u < v { (u, v) } else { (v, u) };
    if let Some(self_t) = self_map.lookup(k_lo, k_hi) {
        let t = self_t as usize;
        word_scratch[t / 64] &= !(1u64 << (t % 64));
    }

    let n_adj: u32 = word_scratch.iter().map(|w| w.count_ones()).sum();
    if n_adj == 0 {
        return;
    }
    let w_val = 1.0_f32 / n_adj as f32;
    for (word_idx, &word) in word_scratch.iter().enumerate() {
        let mut remaining = word;
        let base = (word_idx * 64) as u32;
        while remaining != 0 {
            let bit = remaining.trailing_zeros();
            sink.push(ei, base + bit, w_val);
            remaining &= remaining - 1;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Entry points
// ─────────────────────────────────────────────────────────────────────

/// Build `M_e` in COO form via per-query sorted-merge over the CSR
/// vertex → tuple adjacency.  Original entry — bit-for-bit identical
/// to the pre-2026-05-22 behaviour.
///
/// # Postconditions
///
/// - `rows.len() == cols.len() == vals.len()`.
/// - All `vals[i] == 1 / |N(rows[i])|`.
/// - Rows appear in ascending `ei` order (serial dispatch).
#[allow(clippy::too_many_arguments)] // legacy flat-arg surface; new code uses BuildOpts
pub fn build_edge_incidence_vertex_adj(
    edges_u: &[u32],
    edges_v: &[u32],
    csr_row_ptr: &[u32],
    csr_col_idx: &[u32],
    self_keys_u: &[u32],
    self_keys_v: &[u32],
    self_tuple_idx: &[u32],
) -> Result<IncidenceCoo, &'static str> {
    let opts = BuildOpts::default();
    match build_edge_incidence(
        edges_u,
        edges_v,
        csr_row_ptr,
        csr_col_idx,
        self_keys_u,
        self_keys_v,
        self_tuple_idx,
        opts,
    )? {
        IncidenceResult::Coo(coo) => Ok(coo),
        IncidenceResult::Csr { .. } => {
            unreachable!("BuildOpts::default() requests Coo output")
        }
    }
}

/// Strategy entry — dispatch on [`BuildOpts`].
///
/// See [`build_edge_incidence_vertex_adj`] for the algorithm and the
/// 2026-05-22 fast-path plan for the optimisation axes.
///
/// # Errors
///
/// Returns `Err` if paired input arrays have inconsistent lengths.
#[allow(clippy::too_many_arguments)] // outer Strategy entry, axes are in BuildOpts
pub fn build_edge_incidence(
    edges_u: &[u32],
    edges_v: &[u32],
    csr_row_ptr: &[u32],
    csr_col_idx: &[u32],
    self_keys_u: &[u32],
    self_keys_v: &[u32],
    self_tuple_idx: &[u32],
    opts: BuildOpts,
) -> Result<IncidenceResult, &'static str> {
    if edges_u.len() != edges_v.len() {
        return Err("edges_u and edges_v must have the same length");
    }
    if self_keys_u.len() != self_keys_v.len() || self_keys_u.len() != self_tuple_idx.len() {
        return Err("self_keys_* and self_tuple_idx must have the same length");
    }

    let self_map = SelfMap::build(self_keys_u, self_keys_v, self_tuple_idx);

    let n_nodes = csr_row_ptr.len().saturating_sub(1);

    // Bitset-path dispatch: take the bitset path only when the caller
    // has opted in (`bitset_threshold > 0`) AND `n_tuples` fits within
    // the threshold.  `n_tuples = max(csr_col_idx) + 1` (single O(nnz)
    // pass, cheap relative to the per-query work).
    let n_tuples = if csr_col_idx.is_empty() {
        0usize
    } else {
        csr_col_idx.iter().copied().max().unwrap_or(0) as usize + 1
    };
    let use_bitset =
        opts.bitset_threshold > 0 && n_tuples > 0 && n_tuples <= opts.bitset_threshold as usize;

    let coo = if use_bitset {
        let adj = BitsetAdj::build(csr_row_ptr, csr_col_idx, n_tuples);
        let kernel = BitsetView {
            n_vertices: n_nodes,
            adj: &adj,
            self_map: &self_map,
        };
        assemble(edges_u, edges_v, &kernel, opts.parallel)
    } else {
        let kernel = CsrView {
            n_nodes,
            row_ptr: csr_row_ptr,
            col_idx: csr_col_idx,
            self_map: &self_map,
        };
        assemble(edges_u, edges_v, &kernel, opts.parallel)
    };

    Ok(match opts.output {
        IncidenceOutput::Coo => IncidenceResult::Coo(coo),
        IncidenceOutput::Csr => to_csr(coo, edges_u.len()),
    })
}

// ─────────────────────────────────────────────────────────────────────
// Serial path
// ─────────────────────────────────────────────────────────────────────

const PARALLEL_CHUNK_SIZE: usize = 1024;

fn assemble_chunk<K: EdgeKernel>(
    edges_u: &[u32],
    edges_v: &[u32],
    base: usize,
    kernel: &K,
) -> IncidenceCoo {
    let mut result = IncidenceCoo::with_capacity(edges_u.len() * 16);
    let mut scratch = kernel.scratch();
    let mut sink = result.sink();
    for (offset, (&u, &v)) in edges_u.iter().zip(edges_v).enumerate() {
        kernel.process((base + offset) as u32, u, v, &mut scratch, &mut sink);
    }
    result
}

fn assemble<K: EdgeKernel>(
    edges_u: &[u32],
    edges_v: &[u32],
    kernel: &K,
    parallel: bool,
) -> IncidenceCoo {
    if !parallel {
        return assemble_chunk(edges_u, edges_v, 0, kernel);
    }
    if edges_u.is_empty() {
        return IncidenceCoo::default();
    }
    // Keep indexed chunk order and the existing chunk size: concatenation
    // must reproduce the serial row/column order, including skipped rows.
    let chunks: Vec<_> = edges_u
        .chunks(PARALLEL_CHUNK_SIZE)
        .zip(edges_v.chunks(PARALLEL_CHUNK_SIZE))
        .enumerate()
        .collect();
    let partials: Vec<_> = chunks
        .into_par_iter()
        .map(|(chunk, (eu, ev))| assemble_chunk(eu, ev, chunk * PARALLEL_CHUNK_SIZE, kernel))
        .collect();
    let mut result = IncidenceCoo::with_capacity(partials.iter().map(|p| p.rows.len()).sum());
    for partial in partials {
        result.append(partial);
    }
    result
}

// COO to CSR conversion retains its existing row-order contract.
fn to_csr(coo: IncidenceCoo, e_query: usize) -> IncidenceResult {
    // Counting sort by row.  Rows are already in ascending order from
    // both serial and parallel paths, so we can skip the bucket sort
    // and just build `row_ptr` by counting.
    let nnz = coo.rows.len();
    let mut row_ptr: Vec<u32> = vec![0; e_query + 1];
    for &r in &coo.rows {
        row_ptr[r as usize + 1] += 1;
    }
    for i in 1..row_ptr.len() {
        row_ptr[i] += row_ptr[i - 1];
    }

    // If rows are already ascending, cols / vals stay in place.
    // Defensive check + sort if not (would only fire if a future
    // path changed the contract).
    let already_sorted = coo.rows.windows(2).all(|w| w[0] <= w[1]);
    if already_sorted {
        return IncidenceResult::Csr {
            row_ptr,
            cols: coo.cols,
            vals: coo.vals,
        };
    }

    // Permutation-based reorder: stable counting sort by row.
    let mut cur = row_ptr.clone();
    let mut cols_out: Vec<u32> = vec![0; nnz];
    let mut vals_out: Vec<f32> = vec![0.0; nnz];
    for i in 0..nnz {
        let r = coo.rows[i] as usize;
        let pos = cur[r] as usize;
        cols_out[pos] = coo.cols[i];
        vals_out[pos] = coo.vals[i];
        cur[r] += 1;
    }
    IncidenceResult::Csr {
        row_ptr,
        cols: cols_out,
        vals: vals_out,
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csr_conversion_moves_sorted_buffers_and_stably_groups_unsorted_rows() {
        let sorted = IncidenceCoo {
            rows: vec![0, 0, 2],
            cols: vec![3, 4, 8],
            vals: vec![0.5, 0.5, 1.0],
        };
        let pointers = (sorted.cols.as_ptr(), sorted.vals.as_ptr());
        let IncidenceResult::Csr {
            row_ptr,
            cols,
            vals,
        } = to_csr(sorted, 3)
        else {
            panic!("CSR result")
        };
        assert_eq!(row_ptr, [0, 2, 2, 3]);
        assert_eq!(pointers, (cols.as_ptr(), vals.as_ptr()));
        assert_eq!(cols, [3, 4, 8]);
        assert_eq!(vals, [0.5, 0.5, 1.0]);

        let unsorted = IncidenceCoo {
            rows: vec![1, 0, 1, 0],
            cols: vec![8, 3, 7, 2],
            vals: vec![0.25, 0.5, 0.75, 0.5],
        };
        let IncidenceResult::Csr {
            row_ptr,
            cols,
            vals,
        } = to_csr(unsorted, 3)
        else {
            panic!("CSR result")
        };
        assert_eq!(row_ptr, [0, 2, 4, 4]);
        assert_eq!(cols, [3, 2, 8, 7]);
        assert_eq!(vals, [0.5, 0.5, 0.25, 0.75]);
    }

    /// CSR for the trivial graph 0-1, 0-2, 1-2 (each vertex incident
    /// to two tuples, tuple ids 0,1,2 in the order edges appear).
    fn make_triangle_csr() -> (Vec<u32>, Vec<u32>) {
        let csr_row_ptr = vec![0u32, 2, 4, 6];
        let csr_col_idx = vec![0u32, 1, 0, 2, 1, 2];
        (csr_row_ptr, csr_col_idx)
    }

    #[test]
    fn rejects_mismatched_edges_length() {
        let err =
            build_edge_incidence_vertex_adj(&[0u32], &[0u32, 1u32], &[0u32], &[], &[], &[], &[]);
        assert!(err.is_err());
    }

    #[test]
    fn rejects_mismatched_self_arrays() {
        let err = build_edge_incidence_vertex_adj(
            &[],
            &[],
            &[0u32],
            &[],
            &[0u32, 1u32],
            &[1u32],
            &[0u32],
        );
        assert!(err.is_err());
    }

    #[test]
    fn ignores_query_with_oob_endpoint() {
        let (rp, ci) = make_triangle_csr();
        let out =
            build_edge_incidence_vertex_adj(&[9u32], &[0u32], &rp, &ci, &[], &[], &[]).unwrap();
        assert!(out.rows.is_empty());
    }

    #[test]
    fn triangle_query_produces_union_with_uniform_weights() {
        let (rp, ci) = make_triangle_csr();
        let out =
            build_edge_incidence_vertex_adj(&[0u32], &[1u32], &rp, &ci, &[], &[], &[]).unwrap();
        assert_eq!(out.rows, vec![0, 0, 0]);
        assert_eq!(out.cols, vec![0, 1, 2]);
        for &w in &out.vals {
            assert!((w - 1.0 / 3.0).abs() < 1e-6);
        }
    }

    #[test]
    fn self_edge_is_excluded_and_weight_renormalised() {
        let (rp, ci) = make_triangle_csr();
        let out =
            build_edge_incidence_vertex_adj(&[0u32], &[1u32], &rp, &ci, &[0u32], &[1u32], &[0u32])
                .unwrap();
        assert_eq!(out.rows, vec![0, 0]);
        assert_eq!(out.cols, vec![1, 2]);
        for &w in &out.vals {
            assert!((w - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn empty_adjacency_skips_row() {
        let rp = vec![0u32, 1, 1];
        let ci = vec![0u32];
        let out =
            build_edge_incidence_vertex_adj(&[1u32], &[1u32], &rp, &ci, &[], &[], &[]).unwrap();
        assert!(out.rows.is_empty());
    }

    // ---- BuildOpts dispatch tests ----

    #[test]
    fn parallel_matches_serial_on_triangle() {
        let (rp, ci) = make_triangle_csr();
        let opts = BuildOpts {
            parallel: true,
            ..BuildOpts::default()
        };
        let r = build_edge_incidence(&[0u32], &[1u32], &rp, &ci, &[], &[], &[], opts).unwrap();
        let IncidenceResult::Coo(coo) = r else {
            panic!("expected Coo")
        };
        assert_eq!(coo.rows, vec![0, 0, 0]);
        assert_eq!(coo.cols, vec![0, 1, 2]);
    }

    #[test]
    fn csr_output_round_trips_with_coo() {
        let (rp, ci) = make_triangle_csr();
        let opts = BuildOpts {
            output: IncidenceOutput::Csr,
            ..BuildOpts::default()
        };
        let r = build_edge_incidence(&[0u32, 1u32], &[1u32, 2u32], &rp, &ci, &[], &[], &[], opts)
            .unwrap();
        let IncidenceResult::Csr {
            row_ptr,
            cols,
            vals,
        } = r
        else {
            panic!("expected Csr")
        };
        // 2 query edges → row_ptr.len() == 3.
        assert_eq!(row_ptr.len(), 3);
        // Each query produces 3 entries (union over the triangle).
        assert_eq!(row_ptr, vec![0, 3, 6]);
        assert_eq!(cols.len(), 6);
        assert_eq!(vals.len(), 6);
    }
}
