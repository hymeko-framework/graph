//! **Graded signed cell topology, and the one owner of its width-`d` contractions.**
//!
//! A [`CellTopology`] is rank 0, the vertices, plus any number of **ranks** `k ≥ 1`. Each rank is
//! a uniform-arity table of signed incidences into rank `k − 1`: a hyperedge is a rank-1 cell over
//! vertices, a face a rank-2 cell over hyperedges, a hierarchy block a cell over its members.
//! Arity may differ between ranks and is uniform within one.
//!
//! Degrees, [scale policies](ScalePolicy) and the [reverse incidence](RankCofaces) are derived
//! read-only views, never a second source of truth.
//!
//! # The contraction contract
//!
//! [`RankIncidence`] is the borrowed form every kernel reads. Its `to_cell` and `to_lower` scales
//! are separate because the signed hypergraph convolution uses `D^{-1/2}` in one direction and
//! `D^{-1/2} / D` in the other. For a rank with arity `k`, sign `σ` and scales `s↑`, `s↓`:
//!
//! ```text
//! lower_to_cell:  h[c]  = Σ_i σ[c,i] · s↑[v] · (1/k) · x[v]          v = members[c,i]
//! cell_to_lower:  y[v]  = s↓[v] · Σ_{(c,i): members[c,i] = v} σ[c,i] · h[c]
//! ```
//!
//! Each field row is `d` wide. **The accumulation order is part of the contract**: cell-major,
//! members in declaration order, the per-lower scale applied after summing. The per-term weight is
//! formed as `(σ · s↑) · (1/k)` before it multiplies the field. [`crate::hyper_operator`]
//! documented this order first; its operator is now the `d = 1`, single-scale case of these
//! kernels.
//!
//! # Parallel kernels keep that order
//!
//! A kernel whose output is cell-major (`lower_to_cell`, `cell_to_lower_adjoint`) writes each cell
//! row from that cell's members alone, so it runs over fixed blocks of cells in parallel and every
//! row is summed exactly as the serial loop sums it: the result is the same bits on any number of
//! threads. The two kernels that scatter onto lower cells stay serial, because their rows collect
//! from many cells and a parallel sum would reorder them.
//!
//! # Signs are stored by what they are
//!
//! A rank whose every sign is exactly `±1` keeps **one bit per incidence** ([`Signs`]); a rank with
//! a real-valued sign (a learned one, a weight) keeps an `f32` each. [`CellTopology::with_rank`]
//! chooses, the kernels read either through the same order, and `±1` round-trips exactly, so the
//! form changes the bytes held, never a result.

use rayon::prelude::*;

/// Cells one parallel task contracts. A fixed block: the rows are independent, so the partition
/// decides only which thread computes a row, never its bits.
const CELLS_PER_TASK: usize = 256;

/// A rank's incidence signs, stored by what they are.
#[derive(Clone, Debug, PartialEq)]
enum SignTable {
    /// Every sign is exactly `±1`: one bit per incidence, set for `-1`.
    Unit { negative: Vec<u64>, len: usize },
    /// A real value per incidence.
    Values(Vec<f32>),
}

impl SignTable {
    /// The bit form when every sign is exactly `+1` or `-1`, the `f32` form otherwise.
    ///
    /// One pass, a word at a time and without a branch per sign: a sign is `±1` exactly when its
    /// bits without the sign bit are `1.0`'s, and its sign bit is the bit stored.
    fn from_values(signs: Vec<f32>) -> Self {
        const MAGNITUDE: u32 = 0x7fff_ffff;
        const ONE: u32 = 0x3f80_0000;
        let mut negative = vec![0u64; signs.len().div_ceil(64)];
        for (word, chunk) in negative.iter_mut().zip(signs.chunks(64)) {
            let mut unit = true;
            let mut bits = 0u64;
            for (i, s) in chunk.iter().enumerate() {
                let raw = s.to_bits();
                unit &= raw & MAGNITUDE == ONE;
                bits |= u64::from(raw >> 31) << i;
            }
            if !unit {
                return Self::Values(signs);
            }
            *word = bits;
        }
        Self::Unit {
            negative,
            len: signs.len(),
        }
    }

    fn view(&self) -> Signs<'_> {
        Signs(match self {
            Self::Unit { negative, len } => SignsRef::Unit {
                negative,
                len: *len,
            },
            Self::Values(values) => SignsRef::Values(values),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SignsRef<'a> {
    Unit { negative: &'a [u64], len: usize },
    Values(&'a [f32]),
}

/// A borrowed sign table: one sign per incidence, in member order, held as one bit each when every
/// sign is `±1` and as an `f32` each otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Signs<'a>(SignsRef<'a>);

impl<'a> Signs<'a> {
    /// Incidences.
    #[must_use]
    pub const fn len(self) -> usize {
        match self.0 {
            SignsRef::Unit { len, .. } => len,
            SignsRef::Values(values) => values.len(),
        }
    }

    /// Whether there is no incidence.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The sign of incidence `at`.
    ///
    /// # Panics
    /// When `at >= self.len()`.
    #[must_use]
    pub fn get(self, at: usize) -> f32 {
        assert!(at < self.len(), "sign {at} of {}", self.len());
        match self.0 {
            SignsRef::Unit { negative, .. } => UnitBits(negative).sign(at),
            SignsRef::Values(values) => values[at],
        }
    }

    /// Every sign, in member order.
    pub fn iter(self) -> impl ExactSizeIterator<Item = f32> + 'a {
        (0..self.len()).map(move |at| self.get(at))
    }

    /// The signs as `f32` values.
    #[must_use]
    pub fn to_vec(self) -> Vec<f32> {
        self.iter().collect()
    }

    /// The `f32` table, when the signs are held as one.
    #[must_use]
    pub const fn as_values(self) -> Option<&'a [f32]> {
        match self.0 {
            SignsRef::Unit { .. } => None,
            SignsRef::Values(values) => Some(values),
        }
    }

    /// Whether every sign is `±1` and held as one bit.
    #[must_use]
    pub const fn is_unit(self) -> bool {
        matches!(self.0, SignsRef::Unit { .. })
    }

    /// Bytes the table holds.
    #[must_use]
    pub const fn stored_bytes(self) -> usize {
        match self.0 {
            SignsRef::Unit { negative, .. } => std::mem::size_of_val(negative),
            SignsRef::Values(values) => std::mem::size_of_val(values),
        }
    }
}

/// How a kernel reads a sign; chosen once per call, so the inner loop does not branch on the form.
trait SignLookup: Copy + Send + Sync {
    fn sign(self, at: usize) -> f32;
}

impl SignLookup for &[f32] {
    #[inline]
    fn sign(self, at: usize) -> f32 {
        self[at]
    }
}

#[derive(Clone, Copy)]
struct UnitBits<'a>(&'a [u64]);

impl SignLookup for UnitBits<'_> {
    /// `±1.0` built from the stored bit: `1.0`'s bits with the sign bit set for `-1`, no branch.
    #[inline]
    fn sign(self, at: usize) -> f32 {
        let bit = ((self.0[at / 64] >> (at % 64)) & 1) as u32;
        f32::from_bits(0x3f80_0000 | (bit << 31))
    }
}

/// Run `$body` with `$s` bound to the sign lookup of `$signs`, monomorphised per form.
macro_rules! with_sign_lookup {
    ($signs:expr, |$s:ident| $body:expr) => {
        match $signs.0 {
            SignsRef::Unit { negative, .. } => {
                let $s = UnitBits(negative);
                $body
            }
            SignsRef::Values(values) => {
                let $s = values;
                $body
            }
        }
    };
}

/// Why a cell topology is not well formed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellTopologyError {
    /// A rank declared arity zero.
    ZeroArity {
        /// The rank being added.
        rank: usize,
    },
    /// `members.len()` is not a multiple of the arity, or `signs.len()` differs from it.
    ShapeMismatch {
        /// The rank being added.
        rank: usize,
    },
    /// A member names a lower cell that does not exist.
    MemberOutOfRange {
        /// The rank being added.
        rank: usize,
        /// Position in the flat member table.
        at: usize,
        /// The offending lower-cell index.
        member: u32,
    },
    /// A rank was requested that the topology does not have.
    NoSuchRank {
        /// The requested rank.
        rank: usize,
    },
}

impl core::fmt::Display for CellTopologyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroArity { rank } => write!(f, "rank {rank} has arity zero"),
            Self::ShapeMismatch { rank } => {
                write!(
                    f,
                    "rank {rank}: members/signs length must be n_cells * arity"
                )
            }
            Self::MemberOutOfRange { rank, at, member } => write!(
                f,
                "rank {rank}: member {member} at position {at} names no lower cell"
            ),
            Self::NoSuchRank { rank } => write!(f, "the topology has no rank {rank}"),
        }
    }
}

impl std::error::Error for CellTopologyError {}

/// One rank `k ≥ 1`: a uniform-arity signed incidence into rank `k − 1`.
#[derive(Clone, Debug, PartialEq)]
pub struct CellRank {
    arity: usize,
    members: Vec<u32>,
    signs: SignTable,
}

impl CellRank {
    /// Members per cell.
    #[must_use]
    pub const fn arity(&self) -> usize {
        self.arity
    }

    /// Cells in this rank.
    #[must_use]
    pub fn n_cells(&self) -> usize {
        self.members.len() / self.arity
    }

    /// Flat `n_cells * arity` lower-cell indices.
    #[must_use]
    pub fn members(&self) -> &[u32] {
        &self.members
    }

    /// Flat `n_cells * arity` incidence signs.
    #[must_use]
    pub fn signs(&self) -> Signs<'_> {
        self.signs.view()
    }

    /// Bytes the rank holds: its member table and its sign table.
    #[must_use]
    pub fn stored_bytes(&self) -> usize {
        std::mem::size_of_val(self.members.as_slice()) + self.signs().stored_bytes()
    }
}

/// Graded signed cells: `n_vertices` rank-0 cells and ranks `1..=max_rank()`.
#[derive(Clone, Debug, PartialEq)]
pub struct CellTopology {
    n_vertices: usize,
    ranks: Vec<CellRank>,
}

impl CellTopology {
    /// Rank 0 alone: `n_vertices` vertices and no higher cell.
    #[must_use]
    pub const fn vertices(n_vertices: usize) -> Self {
        Self {
            n_vertices,
            ranks: Vec::new(),
        }
    }

    /// Add the next rank, whose `members` index cells of the current top rank.
    ///
    /// # Errors
    /// [`CellTopologyError`] for zero arity, a ragged or sign-mismatched table, or a member out of
    /// range. Every member is checked.
    pub fn with_rank(
        mut self,
        arity: usize,
        members: Vec<u32>,
        signs: Vec<f32>,
    ) -> Result<Self, CellTopologyError> {
        let rank = self.ranks.len() + 1;
        if arity == 0 {
            return Err(CellTopologyError::ZeroArity { rank });
        }
        if !members.len().is_multiple_of(arity) || signs.len() != members.len() {
            return Err(CellTopologyError::ShapeMismatch { rank });
        }
        let n_lower = self.n_cells(rank - 1)?;
        if let Some((at, &member)) = members
            .iter()
            .enumerate()
            .find(|&(_, &m)| m as usize >= n_lower)
        {
            return Err(CellTopologyError::MemberOutOfRange { rank, at, member });
        }
        self.ranks.push(CellRank {
            arity,
            members,
            signs: SignTable::from_values(signs),
        });
        Ok(self)
    }

    /// The highest rank present; 0 for vertices alone.
    #[must_use]
    pub fn max_rank(&self) -> usize {
        self.ranks.len()
    }

    /// Cells of `rank`; rank 0 is the vertex count.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`] above [`Self::max_rank`].
    pub fn n_cells(&self, rank: usize) -> Result<usize, CellTopologyError> {
        match rank {
            0 => Ok(self.n_vertices),
            k => self.rank(k).map(CellRank::n_cells),
        }
    }

    /// Rank `k ≥ 1`.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`] for rank 0 or above [`Self::max_rank`].
    pub fn rank(&self, rank: usize) -> Result<&CellRank, CellTopologyError> {
        rank.checked_sub(1)
            .and_then(|index| self.ranks.get(index))
            .ok_or(CellTopologyError::NoSuchRank { rank })
    }

    /// Per lower cell, how many incidences of `rank` name it. Duplicate members count every time.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`].
    pub fn degrees(&self, rank: usize) -> Result<Vec<f32>, CellTopologyError> {
        let cells = self.rank(rank)?;
        let mut degrees = vec![0.0f32; self.n_cells(rank - 1)?];
        for &member in &cells.members {
            degrees[member as usize] += 1.0;
        }
        Ok(degrees)
    }

    /// The two scale vectors of `rank`, over its lower cells, from their degrees.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`].
    pub fn scales(
        &self,
        rank: usize,
        to_cell: ScalePolicy,
        to_lower: ScalePolicy,
    ) -> Result<RankScales, CellTopologyError> {
        let degrees = self.degrees(rank)?;
        Ok(RankScales {
            to_cell: to_cell.apply(&degrees),
            to_lower: to_lower.apply(&degrees),
        })
    }

    /// The borrowed incidence of `rank` with the given scales.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`], or [`CellTopologyError::ShapeMismatch`] when a scale does
    /// not have one entry per lower cell.
    pub fn incidence<'a>(
        &'a self,
        rank: usize,
        scales: &'a RankScales,
    ) -> Result<RankIncidence<'a>, CellTopologyError> {
        let cells = self.rank(rank)?;
        let n_lower = self.n_cells(rank - 1)?;
        if scales.to_cell.len() != n_lower || scales.to_lower.len() != n_lower {
            return Err(CellTopologyError::ShapeMismatch { rank });
        }
        Ok(RankIncidence {
            n_lower,
            arity: cells.arity,
            members: &cells.members,
            signs: cells.signs(),
            to_cell: &scales.to_cell,
            to_lower: &scales.to_lower,
        })
    }

    /// The reverse incidence of `rank`: for every lower cell, the cells that contain it.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`].
    pub fn cofaces(&self, rank: usize) -> Result<RankCofaces, CellTopologyError> {
        let cells = self.rank(rank)?;
        Ok(RankCofaces::build(
            &cells.members,
            cells.arity,
            self.n_cells(rank - 1)?,
        ))
    }
}

/// How a per-lower-cell scale derives from the degree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalePolicy {
    /// `1` everywhere.
    Unit,
    /// `max(d, 1)^{-1/2}`.
    InvSqrtDegree,
    /// `max(d, 1)^{-1/2} / max(d, 1)`, the edge→vertex scale of the signed convolution.
    InvSqrtDegreeOverDegree,
}

impl ScalePolicy {
    /// The scale of every lower cell with these degrees.
    ///
    /// Written as the signed convolution's recorded formula: the degree is clamped to at least one,
    /// and the over-degree form divides the inverse square root by the clamped degree. It is not
    /// `d^{-3/2}` computed directly, which would round differently.
    #[must_use]
    pub fn apply(self, degrees: &[f32]) -> Vec<f32> {
        degrees
            .iter()
            .map(|&degree| {
                let clamped = degree.max(1.0);
                match self {
                    Self::Unit => 1.0,
                    Self::InvSqrtDegree => clamped.powf(-0.5),
                    Self::InvSqrtDegreeOverDegree => clamped.powf(-0.5) / clamped,
                }
            })
            .collect()
    }
}

/// The two scale vectors of one rank, owned.
#[derive(Clone, Debug, PartialEq)]
pub struct RankScales {
    /// Applied to each term of [`RankIncidence::lower_to_cell`].
    pub to_cell: Vec<f32>,
    /// Applied after summing in [`RankIncidence::cell_to_lower`].
    pub to_lower: Vec<f32>,
}

/// One rank's signed incidence and scales, borrowed: what every contraction kernel reads.
///
/// # Invariants
/// `members.len() == signs.len()` is a multiple of `arity`; every member is `< n_lower`;
/// `to_cell.len() == to_lower.len() == n_lower`. [`CellTopology::incidence`] guarantees them;
/// [`Self::new`] checks them for a caller that holds the tables itself.
#[derive(Clone, Copy, Debug)]
pub struct RankIncidence<'a> {
    n_lower: usize,
    arity: usize,
    members: &'a [u32],
    signs: Signs<'a>,
    to_cell: &'a [f32],
    to_lower: &'a [f32],
}

impl<'a> RankIncidence<'a> {
    /// A rank view over tables the caller holds.
    ///
    /// # Errors
    /// [`CellTopologyError`] with `rank: 1`, since the view has no rank of its own, for any
    /// violated invariant.
    pub fn new(
        n_lower: usize,
        arity: usize,
        members: &'a [u32],
        signs: &'a [f32],
        to_cell: &'a [f32],
        to_lower: &'a [f32],
    ) -> Result<Self, CellTopologyError> {
        let rank = 1;
        if arity == 0 {
            return Err(CellTopologyError::ZeroArity { rank });
        }
        if !members.len().is_multiple_of(arity)
            || signs.len() != members.len()
            || to_cell.len() != n_lower
            || to_lower.len() != n_lower
        {
            return Err(CellTopologyError::ShapeMismatch { rank });
        }
        if let Some((at, &member)) = members
            .iter()
            .enumerate()
            .find(|&(_, &m)| m as usize >= n_lower)
        {
            return Err(CellTopologyError::MemberOutOfRange { rank, at, member });
        }
        Ok(Self {
            n_lower,
            arity,
            members,
            signs: Signs(SignsRef::Values(signs)),
            to_cell,
            to_lower,
        })
    }

    /// The same incidence with other signs, such as signs a model learns.
    ///
    /// # Errors
    /// [`CellTopologyError::ShapeMismatch`] when `signs` is not one per incidence.
    pub fn with_signs(self, signs: &'a [f32]) -> Result<Self, CellTopologyError> {
        if signs.len() != self.members.len() {
            return Err(CellTopologyError::ShapeMismatch { rank: 1 });
        }
        Ok(Self {
            signs: Signs(SignsRef::Values(signs)),
            ..self
        })
    }

    /// Lower cells.
    #[must_use]
    pub const fn n_lower(&self) -> usize {
        self.n_lower
    }

    /// Cells.
    #[must_use]
    pub fn n_cells(&self) -> usize {
        self.members.len() / self.arity
    }

    /// Members per cell.
    #[must_use]
    pub const fn arity(&self) -> usize {
        self.arity
    }

    /// Flat member table.
    #[must_use]
    pub const fn members(&self) -> &'a [u32] {
        self.members
    }

    /// Flat sign table.
    #[must_use]
    pub const fn signs(&self) -> Signs<'a> {
        self.signs
    }

    fn inv_arity(&self) -> f32 {
        1.0 / self.arity as f32
    }

    /// Signed scaled mean of each cell's members, `d` features wide: `(n_lower, d)` →
    /// `(n_cells, d)`, written into `out`. Parallel over blocks of cells; each row is summed in
    /// member order, so the result is the same bits on any number of threads.
    ///
    /// Every kernel of a view writes into a buffer its caller owns: `out`'s previous contents are
    /// ignored, so a caller may reuse one buffer across calls.
    ///
    /// # Panics
    /// When `x.len() != n_lower * d` or `out.len() != n_cells * d`.
    pub fn lower_to_cell(&self, x: &[f32], d: usize, out: &mut [f32]) {
        assert_eq!(x.len(), self.n_lower * d, "x must be (n_lower, d)");
        assert_eq!(out.len(), self.n_cells() * d, "out must be (n_cells, d)");
        with_sign_lookup!(self.signs, |s| self.lower_to_cell_by(s, x, d, out));
    }

    fn lower_to_cell_by<S: SignLookup>(&self, s: S, x: &[f32], d: usize, out: &mut [f32]) {
        let inv_k = self.inv_arity();
        self.per_cell(d, out, |c, row| {
            for at in c * self.arity..(c + 1) * self.arity {
                let v = self.members[at] as usize;
                accumulate(
                    row,
                    s.sign(at) * self.to_cell[v] * inv_k,
                    &x[v * d..v * d + d],
                );
            }
        })
    }

    /// The adjoint of [`Self::lower_to_cell`]: `(n_cells, d)` → `(n_lower, d)`, written into
    /// `out`.
    ///
    /// # Panics
    /// When `grad_h.len() != n_cells * d` or `out.len() != n_lower * d`.
    pub fn lower_to_cell_adjoint(&self, grad_h: &[f32], d: usize, out: &mut [f32]) {
        assert_eq!(
            grad_h.len(),
            self.n_cells() * d,
            "grad_h must be (n_cells, d)"
        );
        assert_eq!(out.len(), self.n_lower * d, "out must be (n_lower, d)");
        with_sign_lookup!(self.signs, |s| {
            self.lower_to_cell_adjoint_by(s, grad_h, d, out);
        });
    }

    fn lower_to_cell_adjoint_by<S: SignLookup>(
        &self,
        s: S,
        grad_h: &[f32],
        d: usize,
        grad_x: &mut [f32],
    ) {
        let inv_k = self.inv_arity();
        grad_x.fill(0.0);
        self.for_each_incidence(s, |c, v, sign| {
            let w = sign * self.to_cell[v] * inv_k;
            accumulate(&mut grad_x[v * d..v * d + d], w, &grad_h[c * d..c * d + d]);
        });
    }

    /// Every incidence in member order as `(cell, lower cell, sign)`, walked cell by cell so the
    /// cell index is counted, not divided out of the position. The walk of the two serial kernels:
    /// their order is the contract's, unchanged.
    fn for_each_incidence<S: SignLookup>(&self, s: S, mut visit: impl FnMut(usize, usize, f32)) {
        // `arity >= 1`: both constructors refuse arity zero.
        for (c, members) in self.members.chunks_exact(self.arity).enumerate() {
            let first = c * self.arity;
            for (i, &member) in members.iter().enumerate() {
                visit(c, member as usize, s.sign(first + i));
            }
        }
    }

    /// Signed sum of each lower cell's cofaces, scaled after summing: `(n_cells, d)` →
    /// `(n_lower, d)`, written into `out`.
    ///
    /// # Panics
    /// When `h.len() != n_cells * d` or `out.len() != n_lower * d`.
    pub fn cell_to_lower(&self, h: &[f32], d: usize, out: &mut [f32]) {
        assert_eq!(h.len(), self.n_cells() * d, "h must be (n_cells, d)");
        assert_eq!(out.len(), self.n_lower * d, "out must be (n_lower, d)");
        with_sign_lookup!(self.signs, |s| self.cell_to_lower_by(s, h, d, out));
    }

    fn cell_to_lower_by<S: SignLookup>(&self, s: S, h: &[f32], d: usize, y: &mut [f32]) {
        y.fill(0.0);
        self.for_each_incidence(s, |c, v, sign| {
            accumulate(&mut y[v * d..v * d + d], sign, &h[c * d..c * d + d]);
        });
        for (row, &scale) in y.chunks_mut(d.max(1)).zip(self.to_lower) {
            for value in row {
                *value *= scale;
            }
        }
    }

    /// The adjoint of [`Self::cell_to_lower`]: `(n_lower, d)` → `(n_cells, d)`, written into
    /// `out`. Parallel over blocks of cells, each row summed in member order: the same bits on any
    /// number of threads.
    ///
    /// # Panics
    /// When `grad_y.len() != n_lower * d` or `out.len() != n_cells * d`.
    pub fn cell_to_lower_adjoint(&self, grad_y: &[f32], d: usize, out: &mut [f32]) {
        assert_eq!(
            grad_y.len(),
            self.n_lower * d,
            "grad_y must be (n_lower, d)"
        );
        assert_eq!(out.len(), self.n_cells() * d, "out must be (n_cells, d)");
        with_sign_lookup!(self.signs, |s| {
            self.cell_to_lower_adjoint_by(s, grad_y, d, out);
        });
    }

    fn cell_to_lower_adjoint_by<S: SignLookup>(
        &self,
        s: S,
        grad_y: &[f32],
        d: usize,
        out: &mut [f32],
    ) {
        self.per_cell(d, out, |c, row| {
            for at in c * self.arity..(c + 1) * self.arity {
                let v = self.members[at] as usize;
                accumulate(
                    row,
                    s.sign(at) * self.to_lower[v],
                    &grad_y[v * d..v * d + d],
                );
            }
        })
    }

    /// Fills the `(n_cells, d)` field `field`: row `c` is zeroed and then written by `row(c, ..)`
    /// alone, over fixed blocks of [`CELLS_PER_TASK`] cells in parallel. The task that computes a
    /// row is the first to touch it.
    fn per_cell(&self, d: usize, field: &mut [f32], row: impl Fn(usize, &mut [f32]) + Sync) {
        if d == 0 {
            return;
        }
        field
            .par_chunks_mut(CELLS_PER_TASK * d)
            .enumerate()
            .for_each(|(task, block)| {
                for (offset, out) in block.chunks_mut(d).enumerate() {
                    out.fill(0.0);
                    row(task * CELLS_PER_TASK + offset, out);
                }
            });
    }

    /// The gradient of [`Self::lower_to_cell`] with respect to the signs, one per incidence,
    /// written into `out`.
    ///
    /// # Panics
    /// When `x` or `grad_h` does not have the forward's shape, or `out` does not hold one value
    /// per incidence.
    pub fn lower_to_cell_sign_gradient(
        &self,
        x: &[f32],
        grad_h: &[f32],
        d: usize,
        out: &mut [f32],
    ) {
        assert_eq!(x.len(), self.n_lower * d, "x must be (n_lower, d)");
        assert_eq!(
            grad_h.len(),
            self.n_cells() * d,
            "grad_h must be (n_cells, d)"
        );
        assert_eq!(
            out.len(),
            self.members.len(),
            "out must be one per incidence"
        );
        let inv_k = self.inv_arity();
        for (at, (value, &member)) in out.iter_mut().zip(self.members).enumerate() {
            let (c, v) = (at / self.arity, member as usize);
            *value = dot(&grad_h[c * d..c * d + d], &x[v * d..v * d + d]) * self.to_cell[v] * inv_k;
        }
    }

    /// The gradient of [`Self::cell_to_lower`] with respect to the signs, one per incidence,
    /// written into `out`.
    ///
    /// # Panics
    /// When `h` or `grad_y` does not have the forward's shape, or `out` does not hold one value
    /// per incidence.
    pub fn cell_to_lower_sign_gradient(
        &self,
        h: &[f32],
        grad_y: &[f32],
        d: usize,
        out: &mut [f32],
    ) {
        assert_eq!(h.len(), self.n_cells() * d, "h must be (n_cells, d)");
        assert_eq!(
            grad_y.len(),
            self.n_lower * d,
            "grad_y must be (n_lower, d)"
        );
        assert_eq!(
            out.len(),
            self.members.len(),
            "out must be one per incidence"
        );
        for (at, (value, &member)) in out.iter_mut().zip(self.members).enumerate() {
            let (c, v) = (at / self.arity, member as usize);
            *value = dot(&h[c * d..c * d + d], &grad_y[v * d..v * d + d]) * self.to_lower[v];
        }
    }
}

/// `out += w · x`, element by element.
#[inline]
fn accumulate(out: &mut [f32], w: f32, x: &[f32]) {
    for (o, &xj) in out.iter_mut().zip(x) {
        *o += w * xj;
    }
}

/// `Σ a[j] · b[j]`, summed in index order.
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(&x, &y)| x * y).sum()
}

/// The reverse incidence of one rank: for every lower cell, the ascending ids of the cells that
/// contain it. A cell that names a member twice is listed twice.
///
/// # Invariants
/// `offsets.len() == n_lower + 1`, non-decreasing, `offsets[n_lower] == cells.len()`, and every
/// row sorted ascending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankCofaces {
    offsets: Vec<u32>,
    cells: Vec<u32>,
}

impl RankCofaces {
    /// Counting sort over a flat member table: two passes and a prefix sum.
    fn build(members: &[u32], arity: usize, n_lower: usize) -> Self {
        let mut offsets = vec![0u32; n_lower + 1];
        for &member in members {
            offsets[member as usize + 1] += 1;
        }
        for v in 0..n_lower {
            offsets[v + 1] += offsets[v];
        }
        let mut cells = vec![0u32; members.len()];
        let mut cursor = offsets.clone();
        for (cell, chunk) in members.chunks_exact(arity).enumerate() {
            for &member in chunk {
                let slot = &mut cursor[member as usize];
                cells[*slot as usize] = cell as u32;
                *slot += 1;
            }
        }
        Self { offsets, cells }
    }

    /// Lower cells covered.
    #[must_use]
    pub fn n_lower(&self) -> usize {
        self.offsets.len() - 1
    }

    /// The cells containing lower cell `v`, ascending.
    ///
    /// # Panics
    /// When `v >= n_lower`.
    #[must_use]
    pub fn of(&self, v: usize) -> &[u32] {
        &self.cells[self.offsets[v] as usize..self.offsets[v + 1] as usize]
    }

    /// The flat offsets and cell ids, for a caller that walks the CSR itself.
    #[must_use]
    pub fn parts(&self) -> (&[u32], &[u32]) {
        (&self.offsets, &self.cells)
    }
}
