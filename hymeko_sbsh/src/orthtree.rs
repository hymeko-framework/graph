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
//! # The coordinate type is a parameter
//!
//! Coordinates are any [`Coordinate`]: `f64` (the default, and the scene tier's), `f32` (a
//! consumer whose point buffers are single precision), or an unsigned integer (a **lattice** tree,
//! whose cells stay on the integer grid, as a pixel raster's must). The structure and the split
//! are written once over the trait; `f64` stays the default type argument, so `OrthTree`,
//! `SplitRule` and `build_orthtree` over `f64` buffers read exactly as before. Only the bounding
//! box needs a coordinate that can be halved, so [`point_bbox`] asks for [`RealCoordinate`].
//!
//! # The laminar invariant
//!
//! Children nest inside their parent and are pairwise disjoint, so the cell family is *laminar*.
//! That is what makes the spatial side of an SBSH an alpha-acyclic hypertree of width 1, and it
//! is checkable rather than assumed: see [`OrthTree::verify_laminar`].

/// The coordinate arithmetic the orthtree needs: ordering, the midpoint a cell splits at, and the
/// extremes of a bounding box.
///
/// Implemented for `f32` and `f64`, whose midpoint is `lo * ½ + hi * ½`, so a tree over coordinates
/// both types represent exactly splits at the same places; and for `u32`, `u64` and `usize`, whose
/// midpoint is the floor `lo + (hi - lo) / 2`, so every cell of a lattice tree has integer corners.
pub trait Coordinate:
    Copy
    + PartialOrd
    + core::fmt::Debug
    + core::ops::Add<Output = Self>
    + core::ops::Sub<Output = Self>
    + core::ops::Mul<Output = Self>
{
    /// Zero.
    const ZERO: Self;
    /// Where the cell `[lo, hi)` splits on one axis.
    ///
    /// # Preconditions
    /// `lo < hi`.
    ///
    /// # Postconditions
    /// `lo <= mid <= hi`. `mid == lo` or `mid == hi` means the axis cannot be halved: a unit
    /// lattice side (always `mid == lo`), or two adjacent floats.
    fn midpoint(lo: Self, hi: Self) -> Self;
    /// The smaller of two coordinates.
    fn min(self, other: Self) -> Self;
    /// The larger of two coordinates.
    fn max(self, other: Self) -> Self;
}

/// A coordinate that can be halved: what widening a degenerate bounding-box axis needs.
/// Implemented for `f32` and `f64`.
pub trait RealCoordinate: Coordinate {
    /// One half: the midpoint weight, and the half-width a degenerate axis is widened by.
    const HALF: Self;
}

macro_rules! real_coordinate {
    ($($t:ty),*) => {$(
        impl Coordinate for $t {
            const ZERO: Self = 0.0;
            fn midpoint(lo: Self, hi: Self) -> Self {
                lo * Self::HALF + hi * Self::HALF
            }
            fn min(self, other: Self) -> Self {
                <$t>::min(self, other)
            }
            fn max(self, other: Self) -> Self {
                <$t>::max(self, other)
            }
        }

        impl RealCoordinate for $t {
            const HALF: Self = 0.5;
        }
    )*};
}

macro_rules! lattice_coordinate {
    ($($t:ty),*) => {$(
        impl Coordinate for $t {
            const ZERO: Self = 0;
            fn midpoint(lo: Self, hi: Self) -> Self {
                debug_assert!(lo < hi);
                // `hi - lo` cannot underflow under the precondition, and `lo + (hi - lo) / 2` never
                // exceeds `hi`, so the floor midpoint is overflow-free where `(lo + hi) / 2` is not.
                lo + (hi - lo) / 2
            }
            fn min(self, other: Self) -> Self {
                Ord::min(self, other)
            }
            fn max(self, other: Self) -> Self {
                Ord::max(self, other)
            }
        }
    )*};
}

real_coordinate!(f32, f64);
lattice_coordinate!(u32, u64, usize);

/// How a cell decides to subdivide.
///
/// `should_split` sees the cell box `[lo, hi)` (each `dim` long), its `level`, and the indices of
/// the points currently inside it. The build ANDs this with the universal `level < max_depth`
/// cap, so a rule need only encode its *content* criterion.
pub trait SplitRule<T: Coordinate = f64> {
    /// True iff the cell `[lo, hi)` at depth `level` holding `members` should subdivide,
    /// subject to the build's `max_depth` cap.
    fn should_split(&self, lo: &[T], hi: &[T], level: u32, members: &[u32]) -> bool;
}

/// Subdivide while a cell holds more than `max_per_cell` points — the scene rule.
#[derive(Clone, Copy, Debug)]
pub struct CountSplit {
    /// A cell with strictly more than this many members subdivides.
    pub max_per_cell: usize,
}

impl<T: Coordinate> SplitRule<T> for CountSplit {
    fn should_split(&self, _lo: &[T], _hi: &[T], _level: u32, members: &[u32]) -> bool {
        members.len() > self.max_per_cell
    }
}

/// Orthtree build options.
#[derive(Clone, Copy, Debug)]
pub struct OrthConfig {
    /// Maximum subdivision depth. The root is level 0.
    pub max_depth: u32,
    /// Drop orthants containing no points. On for point trees (the tree stays sparse); off keeps
    /// every orthant of positive volume at every internal cell (the full `2^dim` fan-out unless
    /// an axis cannot be halved).
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

/// A laminar family of cells: one root, every cell's children nested inside it and pairwise
/// disjoint, so the cells holding a point form one chain from the root. A family states its
/// structure; the queries every such family answers the same way are provided.
pub trait Laminar {
    /// What a cell holds.
    type Point: ?Sized;

    /// The root cell, `None` for an empty family.
    fn root(&self) -> Option<usize>;

    /// Cell `cell`'s children.
    fn children_of(&self, cell: usize) -> &[u32];

    /// Whether cell `cell` holds `point`.
    fn holds(&self, cell: usize, point: &Self::Point) -> bool;

    /// The cells holding `point`, root first, written into `path` (cleared first): each cell the
    /// child of the one before that holds the point, until no child does. Laminarity makes that
    /// child unique. Empty when the root does not hold the point.
    ///
    /// The point-location query of a hierarchical partition: a point a sparse family has no leaf
    /// for still has its chain down to the deepest cell that holds it.
    fn path_into(&self, point: &Self::Point, path: &mut Vec<usize>) {
        path.clear();
        let Some(mut cell) = self.root().filter(|&root| self.holds(root, point)) else {
            return;
        };
        loop {
            path.push(cell);
            match self
                .children_of(cell)
                .iter()
                .map(|&child| child as usize)
                .find(|&child| self.holds(child, point))
            {
                Some(child) => cell = child,
                None => return,
            }
        }
    }

    /// [`Self::path_into`] into a new vector.
    fn path(&self, point: &Self::Point) -> Vec<usize> {
        let mut path = Vec::new();
        self.path_into(point, &mut path);
        path
    }
}

/// A built N-dimensional orthtree. Leaves are the finest patches of the SBSH partition.
#[derive(Clone, Debug)]
pub struct OrthTree<T: Coordinate = f64> {
    /// Ambient dimension (2 = quadtree, 3 = octree, ...).
    pub dim: usize,
    /// Inclusive-low corner of each cell, flat `n_cells * dim`.
    pub lo: Vec<T>,
    /// Exclusive-high corner of each cell, flat `n_cells * dim`.
    pub hi: Vec<T>,
    /// Depth of each cell (root = 0).
    pub level: Vec<u32>,
    /// Parent cell index, or `-1` for the root.
    pub parent: Vec<i32>,
    /// Child cell indices; empty means leaf.
    pub children: Vec<Vec<u32>>,
    /// Point indices inside each cell.
    pub members: Vec<Vec<u32>>,
}

/// A cell holds a point inside its half-open box `[lo, hi)`, its far face included where that face
/// is the root's: a point on the root's outer boundary is held, by the root and by every cell
/// that shares that face.
impl<T: Coordinate> Laminar for OrthTree<T> {
    type Point = [T];

    fn root(&self) -> Option<usize> {
        (self.n_cells() > 0).then_some(0)
    }

    fn children_of(&self, cell: usize) -> &[u32] {
        &self.children[cell]
    }

    fn holds(&self, cell: usize, point: &[T]) -> bool {
        debug_assert_eq!(point.len(), self.dim);
        let (lo, hi, far) = (self.cell_lo(cell), self.cell_hi(cell), self.cell_hi(0));
        (0..self.dim).all(|d| {
            point[d] >= lo[d] && (point[d] < hi[d] || (hi[d] == far[d] && point[d] == hi[d]))
        })
    }
}

impl<T: Coordinate> OrthTree<T> {
    /// Number of cells, internal and leaf.
    pub fn n_cells(&self) -> usize {
        self.level.len()
    }

    /// Cell `c`'s inclusive-low corner.
    pub fn cell_lo(&self, c: usize) -> &[T] {
        &self.lo[c * self.dim..(c + 1) * self.dim]
    }

    /// Cell `c`'s exclusive-high corner.
    pub fn cell_hi(&self, c: usize) -> &[T] {
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
    pub fn contains(&self, c: usize, point: &[T], inclusive_far: bool) -> bool {
        let (lo, hi) = (self.cell_lo(c), self.cell_hi(c));
        (0..self.dim).all(|d| {
            point[d] >= lo[d] && (point[d] < hi[d] || (inclusive_far && point[d] <= hi[d]))
        })
    }

    /// The leaf holding `point`: the last cell of its [`Laminar::path`] when that cell is a leaf.
    /// `None` if the point lies outside the root box, or in an orthant a sparse tree dropped.
    ///
    /// # Preconditions
    /// `point.len() == self.dim`.
    pub fn leaf_of(&self, point: &[T]) -> Option<usize> {
        debug_assert_eq!(point.len(), self.dim);
        self.path(point)
            .last()
            .copied()
            .filter(|&c| self.is_leaf(c))
    }

    /// Does cell `c`'s box intersect the query box? Half-open, `[lo, hi)` against `[qlo, qhi)`,
    /// for point ranges; `closed` takes both boxes closed, so touching counts, on a cell's low
    /// face and its high face alike.
    fn box_intersects(&self, c: usize, qlo: &[T], qhi: &[T], closed: bool) -> bool {
        let (lo, hi) = (self.cell_lo(c), self.cell_hi(c));
        (0..self.dim).all(|d| {
            let (below, above) = (lo[d] < qhi[d], qlo[d] < hi[d]);
            (below || (closed && lo[d] == qhi[d])) && (above || (closed && qlo[d] == hi[d]))
        })
    }

    /// Range query pruned by the tree: point indices whose position lies in `[qlo, qhi)`, found
    /// by descending only into cells whose box overlaps the query. Returns sorted indices.
    ///
    /// # Preconditions
    /// `points.len() == n_points * dim`; `qlo.len() == qhi.len() == dim`; the tree was built over
    /// these same `points`.
    pub fn objects_in_region(&self, points: &[T], qlo: &[T], qhi: &[T]) -> Vec<u32> {
        let mut out = Vec::new();
        self.visit_leaves(
            |c| self.box_intersects(c, qlo, qhi, false),
            |c| {
                for &m in &self.members[c] {
                    let p = &points[m as usize * self.dim..(m as usize + 1) * self.dim];
                    if (0..self.dim).all(|d| p[d] >= qlo[d] && p[d] < qhi[d]) {
                        out.push(m);
                    }
                }
            },
        );
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The pruned descent the region queries share: visit every leaf whose cell and ancestors all
    /// pass `enters`, skipping a subtree as soon as its cell fails. An empty tree visits nothing.
    fn visit_leaves(&self, enters: impl Fn(usize) -> bool, mut visit: impl FnMut(usize)) {
        let mut stack: Vec<usize> = self.root().into_iter().collect();
        while let Some(c) = stack.pop() {
            if !enters(c) {
                continue;
            }
            if self.is_leaf(c) {
                visit(c);
            } else {
                stack.extend(self.children[c].iter().map(|&ch| ch as usize));
            }
        }
    }

    /// The leaves whose closed box meets the closed query box `[qlo, qhi]`, ascending: the cells
    /// a conservative test has to look at. Cells are taken closed here, not half-open as for
    /// point location, because a broad phase must not depend on which side of a face a touching
    /// object is assigned to: a box that only touches a leaf's face, low or high, meets it.
    ///
    /// # Preconditions
    /// `qlo.len() == qhi.len() == self.dim`; `qlo[d] <= qhi[d]`.
    pub fn leaves_meeting(&self, qlo: &[T], qhi: &[T]) -> Vec<usize> {
        debug_assert!(qlo.len() == self.dim && qhi.len() == self.dim);
        let mut out = Vec::new();
        self.visit_leaves(|c| self.box_intersects(c, qlo, qhi, true), |c| out.push(c));
        out.sort_unstable();
        out
    }

    /// The face adjacency of the leaves: every unordered pair `(a, b)`, `a < b`, whose boxes share
    /// a `(dim - 1)`-dimensional face patch of positive measure. On one axis one box ends where
    /// the other begins, and on every other axis their open intervals overlap. Boxes that meet
    /// only along an edge or at a corner are not adjacent, and a dropped orthant is a gap, not a
    /// neighbour. Sorted ascending: the cell graph of the partition, the routes between patches.
    ///
    /// Shared faces compare exactly: a child copies its parent's corners and siblings share the
    /// parent's midpoint, so the two sides of a face hold the same coordinate value.
    pub fn face_adjacency(&self) -> Vec<(u32, u32)> {
        let dim = self.dim;
        let mut pairs = Vec::new();
        let (mut qlo, mut qhi) = (Vec::with_capacity(dim), Vec::with_capacity(dim));
        for a in self.leaves() {
            let (alo, ahi) = (self.cell_lo(a), self.cell_hi(a));
            for d in 0..dim {
                // Only the high face: the pair's other side is the neighbour's low face, so each
                // pair is found once, from the box that ends there.
                qlo.clear();
                qlo.extend_from_slice(alo);
                qlo[d] = ahi[d];
                qhi.clear();
                qhi.extend_from_slice(ahi);
                for b in self.leaves_meeting(&qlo, &qhi) {
                    let (blo, bhi) = (self.cell_lo(b), self.cell_hi(b));
                    if b != a
                        && blo[d] == ahi[d]
                        && (0..dim).all(|e| e == d || alo[e].max(blo[e]) < ahi[e].min(bhi[e]))
                    {
                        pairs.push((Ord::min(a, b) as u32, Ord::max(a, b) as u32));
                    }
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        pairs
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
fn build_cell<T: Coordinate>(
    dim: usize,
    lo: Vec<T>,
    hi: Vec<T>,
    level: u32,
    members: Vec<u32>,
    parent: i32,
    points: &[T],
    rule: &dyn SplitRule<T>,
    cfg: &OrthConfig,
    tree: &mut OrthTree<T>,
) -> u32 {
    let me = tree.level.len() as u32;
    tree.lo.extend_from_slice(&lo);
    tree.hi.extend_from_slice(&hi);
    tree.level.push(level);
    tree.parent.push(parent);
    tree.children.push(Vec::new());
    tree.members.push(members);
    let members = &tree.members[me as usize];

    if level >= cfg.max_depth || !rule.should_split(&lo, &hi, level, members) {
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
    let mid: Vec<T> = (0..dim).map(|d| T::midpoint(lo[d], hi[d])).collect();
    if (0..dim).all(|d| mid[d] <= lo[d] || mid[d] >= hi[d]) {
        return me;
    }
    let mut bins = vec![Vec::new(); 1usize << dim];
    for &m in members {
        let p = &points[m as usize * dim..(m as usize + 1) * dim];
        if (0..dim).all(|d| p[d] >= lo[d] && p[d] < hi[d]) {
            let orthant = (0..dim).fold(0, |bits, d| bits | ((usize::from(p[d] >= mid[d])) << d));
            bins[orthant].push(m);
        }
    }
    let mut kids = Vec::new();
    for (orthant, child_members) in bins.into_iter().enumerate() {
        if cfg.drop_empty && child_members.is_empty() {
            continue;
        }
        let mut clo = vec![T::ZERO; dim];
        let mut chi = vec![T::ZERO; dim];
        for d in 0..dim {
            if orthant & (1 << d) != 0 {
                clo[d] = mid[d];
                chi[d] = hi[d];
            } else {
                clo[d] = lo[d];
                chi[d] = mid[d];
            }
        }
        // An axis that cannot be halved (`mid` on a face) leaves half its orthants without volume.
        // They hold no points either, so a cell only splits along the axes that can be halved.
        if (0..dim).any(|d| clo[d] >= chi[d]) {
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
pub fn build_orthtree<T: Coordinate>(
    dim: usize,
    root_lo: &[T],
    root_hi: &[T],
    points: &[T],
    member_ids: &[u32],
    rule: &dyn SplitRule<T>,
    cfg: &OrthConfig,
) -> OrthTree<T> {
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
pub fn point_bbox<T: RealCoordinate>(points: &[T], dim: usize, pad: T) -> (Vec<T>, Vec<T>) {
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
        if span <= T::ZERO {
            lo[d] = lo[d] - T::HALF;
            hi[d] = hi[d] + T::HALF;
        } else {
            lo[d] = lo[d] - pad * span;
            hi[d] = hi[d] + pad * span;
        }
    }
    (lo, hi)
}
