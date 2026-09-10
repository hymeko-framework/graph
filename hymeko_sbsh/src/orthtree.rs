//! Generic **N-dimensional orthtree** — the spatial substrate of SBSH.
//!
//! One laminar `2^dim`-ary tree over a point set in `R^k`. A **quadtree** is `dim = 2`
//! (`2^2 = 4` orthants); an **octree** is `dim = 3` (`2^3 = 8`); higher dimensions are the same
//! code. The type is named for the general case because the general case is what is implemented.
//!
//! # Why the split rule is a trait
//!
//! The *structure* (nest, subdivide, prune) is independent of *why* a cell subdivides, so the
//! criterion is a Strategy ([`SplitRule`]) rather than a `kind: &str` or a mode flag. This crate
//! ships only [`CountSplit`], the point-density rule. A raster rule is **deliberately absent**:
//! `hymeko_graph::quadtree` is this workspace's owner of image subdivision (variance +
//! Forman-Ricci curvature over pixels), and restating it here would be the second copy that
//! `CORE.YAML` forbids. A caller who wants energy-driven subdivision implements the trait.
//!
//! # Layout
//!
//! Cell boxes are flat SoA buffers: cell `c`'s box is `lo[c*dim .. c*dim+dim]` (inclusive-low)
//! and `hi[c*dim .. c*dim+dim]` (exclusive-high). Cold connectivity (`parent`, `children`,
//! `members`) sits beside the hot coordinate arrays.
//!
//! # The laminar invariant
//!
//! Children nest inside their parent and are pairwise disjoint, so the cell family is *laminar*.
//! That is what makes the spatial side of an SBSH an alpha-acyclic hypertree of width 1, and it
//! is checkable rather than assumed: see [`OrthTree::verify_laminar`].

/// How a cell decides to subdivide.
///
/// `should_split` sees the cell box `[lo, hi)` (each `dim` long), its `level`, and the indices of
/// the points currently inside it. The build ANDs this with the universal `level < max_depth`
/// cap, so a rule need only encode its *content* criterion.
pub trait SplitRule {
    /// True iff the cell `[lo, hi)` at depth `level` holding `members` should subdivide,
    /// subject to the build's `max_depth` cap.
    fn should_split(&self, lo: &[f64], hi: &[f64], level: u32, members: &[u32]) -> bool;
}

/// Subdivide while a cell holds more than `max_per_cell` points — the scene rule.
#[derive(Clone, Copy, Debug)]
pub struct CountSplit {
    /// A cell with strictly more than this many members subdivides.
    pub max_per_cell: usize,
}

impl SplitRule for CountSplit {
    fn should_split(&self, _lo: &[f64], _hi: &[f64], _level: u32, members: &[u32]) -> bool {
        members.len() > self.max_per_cell
    }
}

/// Orthtree build options.
#[derive(Clone, Copy, Debug)]
pub struct OrthConfig {
    /// Maximum subdivision depth. The root is level 0.
    pub max_depth: u32,
    /// Drop orthants containing no points. On for point trees (the tree stays sparse); off keeps
    /// the full `2^dim` fan-out at every internal cell.
    pub drop_empty: bool,
}

impl Default for OrthConfig {
    fn default() -> Self {
        Self {
            max_depth: 8,
            drop_empty: true,
        }
    }
}

/// A built N-dimensional orthtree. Leaves are the finest patches of the SBSH partition.
#[derive(Clone, Debug)]
pub struct OrthTree {
    /// Ambient dimension (2 = quadtree, 3 = octree, ...).
    pub dim: usize,
    /// Inclusive-low corner of each cell, flat `n_cells * dim`.
    pub lo: Vec<f64>,
    /// Exclusive-high corner of each cell, flat `n_cells * dim`.
    pub hi: Vec<f64>,
    /// Depth of each cell (root = 0).
    pub level: Vec<u32>,
    /// Parent cell index, or `-1` for the root.
    pub parent: Vec<i32>,
    /// Child cell indices; empty means leaf.
    pub children: Vec<Vec<u32>>,
    /// Point indices inside each cell.
    pub members: Vec<Vec<u32>>,
}

impl OrthTree {
    /// Number of cells, internal and leaf.
    pub fn n_cells(&self) -> usize {
        self.level.len()
    }

    /// Cell `c`'s inclusive-low corner.
    pub fn cell_lo(&self, c: usize) -> &[f64] {
        &self.lo[c * self.dim..(c + 1) * self.dim]
    }

    /// Cell `c`'s exclusive-high corner.
    pub fn cell_hi(&self, c: usize) -> &[f64] {
        &self.hi[c * self.dim..(c + 1) * self.dim]
    }

    /// Is cell `c` a leaf?
    pub fn is_leaf(&self, c: usize) -> bool {
        self.children[c].is_empty()
    }

    /// Leaf cell indices, in creation (DFS) order.
    pub fn leaves(&self) -> Vec<usize> {
        (0..self.n_cells()).filter(|&c| self.is_leaf(c)).collect()
    }

    /// Does cell `c` contain `point` under half-open `[lo, hi)` semantics? With `inclusive_far`
    /// the exclusive-high face is treated as inclusive, so a point exactly on the outer boundary
    /// of the root is not lost.
    pub fn contains(&self, c: usize, point: &[f64], inclusive_far: bool) -> bool {
        let (lo, hi) = (self.cell_lo(c), self.cell_hi(c));
        (0..self.dim).all(|d| {
            point[d] >= lo[d] && (point[d] < hi[d] || (inclusive_far && point[d] <= hi[d]))
        })
    }

    /// Descend from the root to the leaf containing `point`, the root's outer face inclusive.
    /// `None` if the point lies outside the root box.
    ///
    /// # Preconditions
    /// `point.len() == self.dim`.
    pub fn leaf_of(&self, point: &[f64]) -> Option<usize> {
        debug_assert_eq!(point.len(), self.dim);
        if !self.contains(0, point, true) {
            return None;
        }
        let mut c = 0usize;
        while !self.is_leaf(c) {
            match self.children[c].iter().find(|&&ch| {
                let (lo, hi) = (self.cell_lo(ch as usize), self.cell_hi(ch as usize));
                (0..self.dim).all(|d| {
                    point[d] >= lo[d]
                        && (point[d] < hi[d] || (hi[d] == self.cell_hi(0)[d] && point[d] == hi[d]))
                })
            }) {
                Some(&ch) => c = ch as usize,
                None => return None, // no leaf in a dropped (empty) orthant
            }
        }
        Some(c)
    }

    /// Does cell `c`'s box overlap the query box `[qlo, qhi)`?
    fn box_intersects(&self, c: usize, qlo: &[f64], qhi: &[f64]) -> bool {
        let (lo, hi) = (self.cell_lo(c), self.cell_hi(c));
        (0..self.dim).all(|d| lo[d] < qhi[d] && qlo[d] < hi[d])
    }

    /// Range query pruned by the tree: point indices whose position lies in `[qlo, qhi)`, found
    /// by descending only into cells whose box overlaps the query. Returns sorted indices.
    ///
    /// # Preconditions
    /// `points.len() == n_points * dim`; `qlo.len() == qhi.len() == dim`; the tree was built over
    /// these same `points`.
    pub fn objects_in_region(&self, points: &[f64], qlo: &[f64], qhi: &[f64]) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack = vec![0usize];
        while let Some(c) = stack.pop() {
            if !self.box_intersects(c, qlo, qhi) {
                continue;
            }
            if self.is_leaf(c) {
                for &m in &self.members[c] {
                    let p = &points[m as usize * self.dim..(m as usize + 1) * self.dim];
                    if (0..self.dim).all(|d| p[d] >= qlo[d] && p[d] < qhi[d]) {
                        out.push(m);
                    }
                }
            } else {
                stack.extend(self.children[c].iter().map(|&ch| ch as usize));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Check the laminar invariant: every child nests inside its parent, and same-parent children
    /// are pairwise disjoint. Returns the first violation as `Err`.
    ///
    /// This is the property the width-1 spatial backbone rests on, so it is verifiable rather
    /// than asserted in prose.
    pub fn verify_laminar(&self) -> Result<(), String> {
        for c in 0..self.n_cells() {
            let (plo, phi) = (self.cell_lo(c), self.cell_hi(c));
            let kids = &self.children[c];
            for &k in kids {
                let (klo, khi) = (self.cell_lo(k as usize), self.cell_hi(k as usize));
                for d in 0..self.dim {
                    if klo[d] < plo[d] || khi[d] > phi[d] {
                        return Err(format!("cell {k} escapes parent {c} on axis {d}"));
                    }
                }
            }
            for (i, &a) in kids.iter().enumerate() {
                for &b in &kids[i + 1..] {
                    let (alo, ahi) = (self.cell_lo(a as usize), self.cell_hi(a as usize));
                    let (blo, bhi) = (self.cell_lo(b as usize), self.cell_hi(b as usize));
                    if (0..self.dim).all(|d| alo[d] < bhi[d] && blo[d] < ahi[d]) {
                        return Err(format!("siblings {a} and {b} overlap under {c}"));
                    }
                }
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn build_cell(
    dim: usize,
    lo: Vec<f64>,
    hi: Vec<f64>,
    level: u32,
    members: Vec<u32>,
    parent: i32,
    points: &[f64],
    rule: &dyn SplitRule,
    cfg: &OrthConfig,
    tree: &mut OrthTree,
) -> u32 {
    let me = tree.level.len() as u32;
    tree.lo.extend_from_slice(&lo);
    tree.hi.extend_from_slice(&hi);
    tree.level.push(level);
    tree.parent.push(parent);
    tree.children.push(Vec::new());
    tree.members.push(members.clone());

    if level >= cfg.max_depth || !rule.should_split(&lo, &hi, level, &members) {
        return me;
    }

    // Identical coordinates cannot be separated by further subdivision.
    if members.first().is_some_and(|&first| {
        members.iter().all(|&m| {
            points[m as usize * dim..(m as usize + 1) * dim]
                == points[first as usize * dim..(first as usize + 1) * dim]
        })
    }) {
        return me;
    }
    let mid: Vec<f64> = (0..dim).map(|d| lo[d] * 0.5 + hi[d] * 0.5).collect();
    if (0..dim).all(|d| mid[d] <= lo[d] || mid[d] >= hi[d]) {
        return me;
    }
    let mut bins = vec![Vec::new(); 1usize << dim];
    for &m in &members {
        let p = &points[m as usize * dim..(m as usize + 1) * dim];
        if (0..dim).all(|d| p[d] >= lo[d] && p[d] < hi[d]) {
            let orthant = (0..dim).fold(0, |bits, d| bits | ((usize::from(p[d] >= mid[d])) << d));
            bins[orthant].push(m);
        }
    }
    let mut kids = Vec::new();
    for (orthant, child_members) in bins.into_iter().enumerate() {
        let mut clo = vec![0.0f64; dim];
        let mut chi = vec![0.0f64; dim];
        for d in 0..dim {
            if orthant & (1 << d) != 0 {
                clo[d] = mid[d];
                chi[d] = hi[d];
            } else {
                clo[d] = lo[d];
                chi[d] = mid[d];
            }
        }
        if cfg.drop_empty && child_members.is_empty() {
            continue;
        }
        let child = build_cell(
            dim,
            clo,
            chi,
            level + 1,
            child_members,
            me as i32,
            points,
            rule,
            cfg,
            tree,
        );
        kids.push(child);
    }
    tree.children[me as usize] = kids;
    me
}

/// Build an N-dimensional orthtree over the root box `[root_lo, root_hi)`.
///
/// `points` is a flat `n_points * dim` coordinate buffer; `member_ids` are the point indices
/// seeded into the root — pass `0..n_points` for the whole set.
///
/// # Preconditions
/// `root_lo.len() == root_hi.len() == dim`; `root_lo[d] < root_hi[d]`;
/// `points.len() == n_points * dim`; every index in `member_ids` is `< n_points`.
///
/// # Postconditions
/// Cell 0 is the root (`parent == -1`, `level == 0`); the family is laminar
/// ([`OrthTree::verify_laminar`]).
pub fn build_orthtree(
    dim: usize,
    root_lo: &[f64],
    root_hi: &[f64],
    points: &[f64],
    member_ids: &[u32],
    rule: &dyn SplitRule,
    cfg: &OrthConfig,
) -> OrthTree {
    assert_eq!(root_lo.len(), dim, "root_lo must have `dim` coordinates");
    assert_eq!(root_hi.len(), dim, "root_hi must have `dim` coordinates");
    assert!(dim > 0, "an orthtree needs at least one dimension");
    debug_assert!((0..dim).all(|d| root_lo[d] < root_hi[d]));
    debug_assert!(points.len().is_multiple_of(dim));
    let mut tree = OrthTree {
        dim,
        lo: Vec::new(),
        hi: Vec::new(),
        level: Vec::new(),
        parent: Vec::new(),
        children: Vec::new(),
        members: Vec::new(),
    };
    build_cell(
        dim,
        root_lo.to_vec(),
        root_hi.to_vec(),
        0,
        member_ids.to_vec(),
        -1,
        points,
        rule,
        cfg,
        &mut tree,
    );
    tree
}

/// Axis-aligned bounding box `(lo, hi)` of a flat `n_points * dim` buffer, padded by `pad` of the
/// per-axis span. A non-zero pad keeps points off the exclusive-high face; a degenerate axis
/// (every point equal) is widened to a unit interval so the root box is never empty.
///
/// # Preconditions
/// `points.len() == n_points * dim` with `n_points >= 1`.
pub fn point_bbox(points: &[f64], dim: usize, pad: f64) -> (Vec<f64>, Vec<f64>) {
    assert!(dim > 0, "an orthtree needs at least one dimension");
    assert!(!points.is_empty() && points.len().is_multiple_of(dim));
    let n = points.len() / dim;
    let mut lo = points[..dim].to_vec();
    let mut hi = points[..dim].to_vec();
    for i in 1..n {
        for d in 0..dim {
            let p = points[i * dim + d];
            lo[d] = lo[d].min(p);
            hi[d] = hi[d].max(p);
        }
    }
    for d in 0..dim {
        let span = hi[d] - lo[d];
        if span <= 0.0 {
            lo[d] -= 0.5;
            hi[d] += 0.5;
        } else {
            lo[d] -= pad * span;
            hi[d] += pad * span;
        }
    }
    (lo, hi)
}
