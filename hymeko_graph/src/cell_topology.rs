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

use core::marker::PhantomData;

use rayon::prelude::*;

/// Cells one parallel task contracts. A fixed block: the rows are independent, so the partition
/// decides only which thread computes a row, never its bits.
const CELLS_PER_TASK: usize = 256;

/// Incidence-features below which a gather runs on the calling thread: fewer than this many
/// multiply-adds cost less than waking the pool. Rows are independent either way, so the choice
/// decides only which thread computes a row, never its bits.
const PARALLEL_WORK: usize = 1 << 15;

/// Gather tasks per pool thread ([`RankIncidence::with_cofaces`]): the lower cells are blocked by
/// their incidence count into this many blocks a thread, so a skewed degree still balances.
const GATHER_TASKS_PER_THREAD: usize = 4;

/// Incidences below which a gather block is not split further.
const GATHER_MIN_INCIDENCES: usize = 64;

/// **The value type of an incidence view**: its signs, scales and features. `f32` is the storage
/// and the default -- every recorded kernel result is `f32` -- and `f64` serves a consumer whose
/// solver runs in double precision (plan 2026-10-20-rank-incidence-scalar). A kernel performs the
/// same operations in the same order at either type.
pub trait IncidenceScalar:
    Copy
    + Send
    + Sync
    + core::fmt::Debug
    + PartialEq
    + core::ops::Add<Output = Self>
    + core::ops::Mul<Output = Self>
    + core::ops::Div<Output = Self>
    + core::ops::AddAssign
    + core::ops::MulAssign
    + core::iter::Sum
{
    /// Zero.
    const ZERO: Self;
    /// One.
    const ONE: Self;
    /// A count as a value: an arity.
    fn from_count(n: usize) -> Self;
    /// `+1` for `bit == 0`, `-1` for `bit == 1`, built from the bits without a branch.
    fn unit_sign(bit: u64) -> Self;
}

impl IncidenceScalar for f32 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    fn from_count(n: usize) -> Self {
        n as f32
    }
    #[inline]
    fn unit_sign(bit: u64) -> Self {
        f32::from_bits(0x3f80_0000 | ((bit as u32) << 31))
    }
}

impl IncidenceScalar for f64 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    fn from_count(n: usize) -> Self {
        n as f64
    }
    #[inline]
    fn unit_sign(bit: u64) -> Self {
        f64::from_bits(0x3ff0_0000_0000_0000 | (bit << 63))
    }
}

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
enum SignsRef<'a, F = f32> {
    Unit { negative: &'a [u64], len: usize },
    Values(&'a [F]),
}

/// A borrowed sign table: one sign per incidence, in member order, held as one bit each when every
/// sign is `±1` and as an `F` each otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Signs<'a, F = f32>(SignsRef<'a, F>);

impl<'a, F: IncidenceScalar> Signs<'a, F> {
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
    pub fn get(self, at: usize) -> F {
        assert!(at < self.len(), "sign {at} of {}", self.len());
        match self.0 {
            SignsRef::Unit { negative, .. } => UnitBits(negative).sign(at),
            SignsRef::Values(values) => values[at],
        }
    }

    /// Every sign, in member order.
    pub fn iter(self) -> impl ExactSizeIterator<Item = F> + 'a {
        (0..self.len()).map(move |at| self.get(at))
    }

    /// The signs as values.
    #[must_use]
    pub fn to_vec(self) -> Vec<F> {
        self.iter().collect()
    }

    /// The value table, when the signs are held as one.
    #[must_use]
    pub const fn as_values(self) -> Option<&'a [F]> {
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
trait SignLookup<F>: Copy + Send + Sync {
    fn sign(self, at: usize) -> F;
}

impl<F: IncidenceScalar> SignLookup<F> for &[F] {
    #[inline]
    fn sign(self, at: usize) -> F {
        self[at]
    }
}

#[derive(Clone, Copy)]
struct UnitBits<'a>(&'a [u64]);

impl<F: IncidenceScalar> SignLookup<F> for UnitBits<'_> {
    /// `±1` built from the stored bit: `1`'s bits with the sign bit set for `-1`, no branch.
    #[inline]
    fn sign(self, at: usize) -> F {
        F::unit_sign((self.0[at / 64] >> (at % 64)) & 1)
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

/// A kernel's row width: `d` features, or exactly one, which the compiler then addresses as a
/// scalar instead of a slice. The arithmetic and its order are the same for both.
trait Width: Copy + Send + Sync {
    fn d(self) -> usize;

    /// `row = Σ w · x[c]` over `terms`, summed in their order from `+0`: what zeroing `row` and
    /// accumulating each term into it computes, bit for bit.
    fn gather<F: IncidenceScalar>(
        self,
        row: &mut [F],
        x: &[F],
        terms: impl Iterator<Item = (F, usize)>,
    );
}

/// One feature per row.
#[derive(Clone, Copy)]
struct One;

impl Width for One {
    #[inline]
    fn d(self) -> usize {
        1
    }

    /// The sum is held in a local, not in `row`: a store into `row` before every term's bounds
    /// check would chain each addition through memory.
    #[inline]
    fn gather<F: IncidenceScalar>(
        self,
        row: &mut [F],
        x: &[F],
        terms: impl Iterator<Item = (F, usize)>,
    ) {
        let mut sum = F::ZERO;
        for (w, c) in terms {
            sum += w * x[c];
        }
        row[0] = sum;
    }
}

/// `d` features per row.
#[derive(Clone, Copy)]
struct Features(usize);

impl Width for Features {
    #[inline]
    fn d(self) -> usize {
        self.0
    }

    #[inline]
    fn gather<F: IncidenceScalar>(
        self,
        row: &mut [F],
        x: &[F],
        terms: impl Iterator<Item = (F, usize)>,
    ) {
        let d = self.0;
        row.fill(F::ZERO);
        for (w, c) in terms {
            accumulate(row, w, &x[c * d..c * d + d]);
        }
    }
}

/// Run `$body` with `$w` bound to the [`Width`] of `$d`, monomorphised per form.
macro_rules! with_width {
    ($d:expr, |$w:ident| $body:expr) => {
        if $d == 1 {
            let $w = One;
            $body
        } else {
            let $w = Features($d);
            $body
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
            cofaces: None,
        })
    }

    /// The borrowed incidence of `rank` at any scalar `F`, with the given scales, for a rank whose
    /// every sign is `±1`: held as bits, those read as `±1` at every scalar, so no table is
    /// converted. `None` when the rank holds real-valued signs -- an `f32` table, which only the
    /// `f32` view of [`Self::incidence`] reads.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`], or [`CellTopologyError::ShapeMismatch`] when a scale does
    /// not have one entry per lower cell.
    pub fn unit_incidence<'a, F: IncidenceScalar>(
        &'a self,
        rank: usize,
        to_cell: &'a [F],
        to_lower: &'a [F],
    ) -> Result<Option<RankIncidence<'a, F>>, CellTopologyError> {
        let cells = self.rank(rank)?;
        let n_lower = self.n_cells(rank - 1)?;
        if to_cell.len() != n_lower || to_lower.len() != n_lower {
            return Err(CellTopologyError::ShapeMismatch { rank });
        }
        let SignTable::Unit { negative, len } = &cells.signs else {
            return Ok(None);
        };
        Ok(Some(RankIncidence {
            n_lower,
            arity: cells.arity,
            members: &cells.members,
            signs: Signs(SignsRef::Unit {
                negative,
                len: *len,
            }),
            to_cell,
            to_lower,
            cofaces: None,
        }))
    }

    /// The reverse incidence of `rank`: for every lower cell, the cells that contain it.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`].
    pub fn cofaces(&self, rank: usize) -> Result<RankCofaces, CellTopologyError> {
        self.reverse_incidence(rank)
    }

    /// The reverse incidence of `rank` by position: for every lower cell, the positions in the
    /// member table that name it -- what [`RankIncidence::with_cofaces`] gathers over.
    ///
    /// # Errors
    /// [`CellTopologyError::NoSuchRank`].
    pub fn coface_positions(
        &self,
        rank: usize,
    ) -> Result<RankCofaces<Position>, CellTopologyError> {
        self.reverse_incidence(rank)
    }

    fn reverse_incidence<L: CofaceLabel>(
        &self,
        rank: usize,
    ) -> Result<RankCofaces<L>, CellTopologyError> {
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
pub struct RankIncidence<'a, F = f32> {
    n_lower: usize,
    arity: usize,
    members: &'a [u32],
    signs: Signs<'a, F>,
    to_cell: &'a [F],
    to_lower: &'a [F],
    /// The rank's reverse incidence by position, when the caller holds it: the scatters then run
    /// as gathers ([`Self::with_cofaces`]).
    cofaces: Option<&'a RankCofaces<Position>>,
}

impl<'a, F: IncidenceScalar> RankIncidence<'a, F> {
    /// A rank view over tables the caller holds.
    ///
    /// # Errors
    /// [`CellTopologyError`] with `rank: 1`, since the view has no rank of its own, for any
    /// violated invariant.
    pub fn new(
        n_lower: usize,
        arity: usize,
        members: &'a [u32],
        signs: &'a [F],
        to_cell: &'a [F],
        to_lower: &'a [F],
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
            cofaces: None,
        })
    }

    /// The same incidence with other signs, such as signs a model learns.
    ///
    /// # Errors
    /// [`CellTopologyError::ShapeMismatch`] when `signs` is not one per incidence.
    pub fn with_signs(self, signs: &'a [F]) -> Result<Self, CellTopologyError> {
        if signs.len() != self.members.len() {
            return Err(CellTopologyError::ShapeMismatch { rank: 1 });
        }
        Ok(Self {
            signs: Signs(SignsRef::Values(signs)),
            ..self
        })
    }

    /// The same incidence holding its reverse incidence by position, `cofaces` (from
    /// [`CellTopology::coface_positions`] of the same rank): [`Self::cell_to_lower`] and
    /// [`Self::lower_to_cell_adjoint`] then run as gathers, each lower cell over its positions, in
    /// parallel -- once the scatter holds [`PARALLEL_WORK`] incidence-features; below, the serial
    /// scatter is the cheaper walk. The bits are the serial scatters': both add into a lower cell
    /// in ascending position.
    ///
    /// # Errors
    /// [`CellTopologyError::ShapeMismatch`] when `cofaces` is not this rank's: another number of
    /// lower cells or of positions, a row not strictly ascending, or a position naming another
    /// lower cell. Together these make the listing a permutation of this rank's positions.
    pub fn with_cofaces(
        self,
        cofaces: &'a RankCofaces<Position>,
    ) -> Result<Self, CellTopologyError> {
        let mismatch = Err(CellTopologyError::ShapeMismatch { rank: 1 });
        if cofaces.n_lower() != self.n_lower || cofaces.parts().1.len() != self.members.len() {
            return mismatch;
        }
        for v in 0..self.n_lower {
            let row = cofaces.of(v);
            let ascending = row.windows(2).all(|pair| pair[0] < pair[1]);
            let names_v = row.iter().all(|&at| {
                self.members
                    .get(at as usize)
                    .is_some_and(|&m| m as usize == v)
            });
            if !(ascending && names_v) {
                return mismatch;
            }
        }
        Ok(Self {
            cofaces: Some(cofaces),
            ..self
        })
    }

    /// Every member of cell `c` in slot order, as `(position, lower cell)`.
    #[inline]
    fn members_of(&self, c: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
        let first = c * self.arity;
        self.members[first..first + self.arity]
            .iter()
            .enumerate()
            .map(move |(slot, &v)| (first + slot, v as usize))
    }

    /// Every incidence of lower cell `v` in ascending position, as `(position, cell)`.
    #[inline]
    fn cofaces_of<'c>(
        &self,
        cofaces: &'c RankCofaces<Position>,
        v: usize,
    ) -> impl Iterator<Item = (usize, usize)> + 'c {
        // `arity` fits `u32`: every position is a `u32` below `members.len()`.
        let arity = self.arity as u32;
        cofaces
            .of(v)
            .iter()
            .map(move |&at| (at as usize, (at / arity) as usize))
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
    pub const fn signs(&self) -> Signs<'a, F> {
        self.signs
    }

    fn inv_arity(&self) -> F {
        F::ONE / F::from_count(self.arity)
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
    pub fn lower_to_cell(&self, x: &[F], d: usize, out: &mut [F]) {
        assert_eq!(x.len(), self.n_lower * d, "x must be (n_lower, d)");
        assert_eq!(out.len(), self.n_cells() * d, "out must be (n_cells, d)");
        with_sign_lookup!(self.signs, |s| with_width!(d, |w| self
            .lower_to_cell_by(s, x, w, out)));
    }

    fn lower_to_cell_by<S: SignLookup<F>, W: Width>(&self, s: S, x: &[F], w: W, out: &mut [F]) {
        let inv_k = self.inv_arity();
        let work = out.len() * self.arity;
        self.per_row(w, out, work, |c, row| {
            let terms = self
                .members_of(c)
                .map(|(at, v)| (s.sign(at) * self.to_cell[v] * inv_k, v));
            w.gather(row, x, terms);
        })
    }

    /// The adjoint of [`Self::lower_to_cell`]: `(n_cells, d)` → `(n_lower, d)`, written into
    /// `out`. A scatter, serial -- or, on a view holding its cofaces ([`Self::with_cofaces`]), a
    /// parallel gather with the same bits.
    ///
    /// # Panics
    /// When `grad_h.len() != n_cells * d` or `out.len() != n_lower * d`.
    pub fn lower_to_cell_adjoint(&self, grad_h: &[F], d: usize, out: &mut [F]) {
        assert_eq!(
            grad_h.len(),
            self.n_cells() * d,
            "grad_h must be (n_cells, d)"
        );
        assert_eq!(out.len(), self.n_lower * d, "out must be (n_lower, d)");
        with_sign_lookup!(self.signs, |s| with_width!(d, |w| self
            .lower_to_cell_adjoint_by(s, grad_h, w, out)));
    }

    fn lower_to_cell_adjoint_by<S: SignLookup<F>, W: Width>(
        &self,
        s: S,
        grad_h: &[F],
        w: W,
        grad_x: &mut [F],
    ) {
        let (inv_k, d) = (self.inv_arity(), w.d());
        let work = self.members.len() * d;
        if let Some(cofaces) = self.cofaces.filter(|_| work >= PARALLEL_WORK) {
            self.per_lower(w, cofaces, grad_x, |v, row| {
                let terms = self
                    .cofaces_of(cofaces, v)
                    .map(|(at, c)| (s.sign(at) * self.to_cell[v] * inv_k, c));
                w.gather(row, grad_h, terms);
            });
            return;
        }
        grad_x.fill(F::ZERO);
        self.for_each_incidence(s, |c, v, sign| {
            let weight = sign * self.to_cell[v] * inv_k;
            accumulate(
                &mut grad_x[v * d..v * d + d],
                weight,
                &grad_h[c * d..c * d + d],
            );
        });
    }

    /// Every incidence in member order as `(cell, lower cell, sign)`, walked cell by cell so the
    /// cell index is counted, not divided out of the position. The walk of the two serial kernels:
    /// their order is the contract's, unchanged.
    fn for_each_incidence<S: SignLookup<F>>(&self, s: S, mut visit: impl FnMut(usize, usize, F)) {
        // `arity >= 1`: both constructors refuse arity zero.
        for (c, members) in self.members.chunks_exact(self.arity).enumerate() {
            let first = c * self.arity;
            for (i, &member) in members.iter().enumerate() {
                visit(c, member as usize, s.sign(first + i));
            }
        }
    }

    /// Signed sum of each lower cell's cofaces, scaled after summing: `(n_cells, d)` →
    /// `(n_lower, d)`, written into `out`. A scatter, serial -- or, on a view holding its cofaces
    /// ([`Self::with_cofaces`]), a parallel gather with the same bits.
    ///
    /// # Panics
    /// When `h.len() != n_cells * d` or `out.len() != n_lower * d`.
    pub fn cell_to_lower(&self, h: &[F], d: usize, out: &mut [F]) {
        assert_eq!(h.len(), self.n_cells() * d, "h must be (n_cells, d)");
        assert_eq!(out.len(), self.n_lower * d, "out must be (n_lower, d)");
        with_sign_lookup!(self.signs, |s| with_width!(d, |w| self
            .cell_to_lower_by(s, h, w, out)));
    }

    fn cell_to_lower_by<S: SignLookup<F>, W: Width>(&self, s: S, h: &[F], w: W, y: &mut [F]) {
        let d = w.d();
        let work = self.members.len() * d;
        if let Some(cofaces) = self.cofaces.filter(|_| work >= PARALLEL_WORK) {
            self.per_lower(w, cofaces, y, |v, row| {
                let terms = self.cofaces_of(cofaces, v).map(|(at, c)| (s.sign(at), c));
                w.gather(row, h, terms);
                let scale = self.to_lower[v];
                for value in row {
                    *value *= scale;
                }
            });
            return;
        }
        y.fill(F::ZERO);
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
    pub fn cell_to_lower_adjoint(&self, grad_y: &[F], d: usize, out: &mut [F]) {
        assert_eq!(
            grad_y.len(),
            self.n_lower * d,
            "grad_y must be (n_lower, d)"
        );
        assert_eq!(out.len(), self.n_cells() * d, "out must be (n_cells, d)");
        with_sign_lookup!(self.signs, |s| with_width!(d, |w| self
            .cell_to_lower_adjoint_by(s, grad_y, w, out)));
    }

    fn cell_to_lower_adjoint_by<S: SignLookup<F>, W: Width>(
        &self,
        s: S,
        grad_y: &[F],
        w: W,
        out: &mut [F],
    ) {
        let work = out.len() * self.arity;
        self.per_row(w, out, work, |c, row| {
            let terms = self
                .members_of(c)
                .map(|(at, v)| (s.sign(at) * self.to_lower[v], v));
            w.gather(row, grad_y, terms);
        })
    }

    /// Fills the `(rows, d)` field `field` -- cells or lower cells: row `r` is written whole by
    /// `row(r, ..)` alone, which [`Width::gather`] does, over fixed blocks of [`CELLS_PER_TASK`] rows -- in parallel
    /// once `work` (the multiply-adds the rows take) reaches [`PARALLEL_WORK`], on the calling
    /// thread below. The task that computes a row is the first to touch it.
    fn per_row<W: Width>(
        &self,
        w: W,
        field: &mut [F],
        work: usize,
        row: impl Fn(usize, &mut [F]) + Sync,
    ) {
        let d = w.d();
        if d == 0 {
            return;
        }
        let task =
            |(task, block): (usize, &mut [F])| fill_rows(task * CELLS_PER_TASK, block, d, &row);
        if work < PARALLEL_WORK {
            field
                .chunks_mut(CELLS_PER_TASK * d)
                .enumerate()
                .for_each(task);
        } else {
            field
                .par_chunks_mut(CELLS_PER_TASK * d)
                .enumerate()
                .for_each(task);
        }
    }

    /// Fills the `(n_lower, d)` field `field` by gathering over `cofaces`: row `v` is written whole
    /// by `row(v, ..)` alone, the rows in contiguous blocks of equal incidence
    /// count, [`GATHER_TASKS_PER_THREAD`] a pool thread, in parallel -- so a hub's block holds few
    /// rows and a sparse one many. A row is never split: it sums in its own order under any
    /// partition, and the pool's size decides only the partition.
    fn per_lower<W: Width>(
        &self,
        w: W,
        cofaces: &RankCofaces<Position>,
        field: &mut [F],
        row: impl Fn(usize, &mut [F]) + Sync,
    ) {
        let d = w.d();
        if d == 0 {
            return;
        }
        let (offsets, _) = cofaces.parts();
        let tasks = GATHER_TASKS_PER_THREAD * rayon::current_num_threads();
        let per_task = self
            .members
            .len()
            .div_ceil(tasks)
            .max(GATHER_MIN_INCIDENCES);
        let mut blocks = Vec::new();
        let (mut first, mut rest) = (0, field);
        while first < self.n_lower {
            // The first row past `first` whose incidences start at or beyond the target.
            let target = offsets[first] as usize + per_task;
            let reached = offsets[first + 1..].partition_point(|&o| (o as usize) < target);
            let end = (first + 1 + reached).min(self.n_lower);
            let (block, tail) = core::mem::take(&mut rest).split_at_mut((end - first) * d);
            blocks.push((first, block));
            (first, rest) = (end, tail);
        }
        blocks
            .into_par_iter()
            .for_each(|(first, block)| fill_rows(first, block, d, &row));
    }

    /// The gradient of [`Self::lower_to_cell`] with respect to the signs, one per incidence,
    /// written into `out`.
    ///
    /// # Panics
    /// When `x` or `grad_h` does not have the forward's shape, or `out` does not hold one value
    /// per incidence.
    pub fn lower_to_cell_sign_gradient(&self, x: &[F], grad_h: &[F], d: usize, out: &mut [F]) {
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
    pub fn cell_to_lower_sign_gradient(&self, h: &[F], grad_y: &[F], d: usize, out: &mut [F]) {
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

/// Rows `first..` of a field, `d` wide, in `block`: each written whole by `row`.
#[inline]
fn fill_rows<F: IncidenceScalar>(
    first: usize,
    block: &mut [F],
    d: usize,
    row: &impl Fn(usize, &mut [F]),
) {
    for (offset, out) in block.chunks_mut(d).enumerate() {
        row(first + offset, out);
    }
}

/// `out += w · x`, element by element.
#[inline]
fn accumulate<F: IncidenceScalar>(out: &mut [F], w: F, x: &[F]) {
    for (o, &xj) in out.iter_mut().zip(x) {
        *o += w * xj;
    }
}

/// `Σ a[j] · b[j]`, summed in index order.
#[inline]
fn dot<F: IncidenceScalar>(a: &[F], b: &[F]) -> F {
    a.iter().zip(b).map(|(&x, &y)| x * y).sum()
}

/// The reverse incidence of one rank: for every lower cell, its incidences in ascending order,
/// labelled by `L` -- the ids of the cells that contain it ([`Cell`], the default), or the
/// member-table positions that name it ([`Position`]). A cell that names a member twice is listed
/// twice.
///
/// # Invariants
/// `offsets.len() == n_lower + 1`, non-decreasing, `offsets[n_lower] == cells.len()`, and every
/// row sorted ascending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankCofaces<L = Cell> {
    offsets: Vec<u32>,
    cells: Vec<u32>,
    label: PhantomData<L>,
}

/// What a reverse incidence lists for each lower cell: the [`Cell`] containing it, or the
/// [`Position`] in the member table that names it.
pub trait CofaceLabel: Copy {
    /// The label of slot `slot` of cell `cell`, cells of `arity` members.
    fn label(cell: usize, slot: usize, arity: usize) -> u32;
}

/// Lists cell ids: [`CellTopology::cofaces`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell;

impl CofaceLabel for Cell {
    #[inline]
    fn label(cell: usize, _: usize, _: usize) -> u32 {
        cell as u32
    }
}

/// Lists member-table positions, `cell · arity + slot`: [`CellTopology::coface_positions`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position;

impl CofaceLabel for Position {
    #[inline]
    fn label(cell: usize, slot: usize, arity: usize) -> u32 {
        (cell * arity + slot) as u32
    }
}

impl<L: CofaceLabel> RankCofaces<L> {
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
            for (at, &member) in chunk.iter().enumerate() {
                let slot = &mut cursor[member as usize];
                cells[*slot as usize] = L::label(cell, at, arity);
                *slot += 1;
            }
        }
        Self {
            offsets,
            cells,
            label: PhantomData,
        }
    }

    /// Lower cells covered.
    #[must_use]
    pub fn n_lower(&self) -> usize {
        self.offsets.len() - 1
    }

    /// The labels of lower cell `v`, ascending: the cells containing it, or the positions naming
    /// it.
    ///
    /// # Panics
    /// When `v >= n_lower`.
    #[must_use]
    pub fn of(&self, v: usize) -> &[u32] {
        &self.cells[self.offsets[v] as usize..self.offsets[v + 1] as usize]
    }

    /// The flat offsets and labels, for a caller that walks the CSR itself.
    #[must_use]
    pub fn parts(&self) -> (&[u32], &[u32]) {
        (&self.offsets, &self.cells)
    }
}
