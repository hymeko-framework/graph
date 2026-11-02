//! Levi connectivity over every adjacent rank of an existing graded cell topology.
use super::levi::{LeviSession, LeviSource, LeviTraversal};
use super::{GraphicMatroidWorkspace, MatroidError};
use crate::cell_topology::CellTopology;

/// One original incidence connecting a cell to a member of the preceding rank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellLeviCoord {
    /// Upper-cell rank (at least one).
    pub rank: usize,
    /// Upper-cell index in that rank.
    pub cell: usize,
    /// Original member slot, including duplicate incidences.
    pub slot: usize,
}
/// Borrowed Levi matroid of a graded topology. Owns only per-rank offsets.
///
/// All cells, including isolated rank-zero cells, are vertices of the implicit
/// Levi graph. Every adjacent-rank incidence is a ground element. Signs and
/// weights do not change structural membership, including a zero coefficient.
/// The basis certifies connectivity, not equivalence of numerical operators.
pub struct CellLeviMatroid<'a> {
    topology: &'a CellTopology,
    offsets: Vec<usize>,
}
impl<'a> CellLeviMatroid<'a> {
    /// Prepare rank offsets in O(number of ranks), without copying topology.
    pub fn new(topology: &'a CellTopology) -> Result<Self, MatroidError> {
        let mut offsets = vec![0usize];
        for rank in 0..=topology.max_rank() {
            // The loop bounds guarantee a declared rank, including vertices.
            let count = topology.n_cells(rank).expect("rank bounded by max_rank");
            let total = offsets[rank]
                .checked_add(count)
                .ok_or(MatroidError::Overflow)?;
            u32::try_from(total).map_err(|_| MatroidError::Overflow)?;
            offsets.push(total);
        }
        Ok(Self { topology, offsets })
    }
    /// Number of cells across all ranks, including isolated cells.
    pub fn vertex_count(&self) -> usize {
        self.offsets[self.offsets.len() - 1]
    }
    /// Matroid rank; the existing shared Levi traversal owns the computation.
    pub fn rank(&self, workspace: &mut GraphicMatroidWorkspace) -> Result<usize, MatroidError> {
        Ok(LeviTraversal::forest(self, workspace, None)?.1)
    }
    /// Components of the implicit incidence graph across all ranks.
    pub fn components(
        &self,
        workspace: &mut GraphicMatroidWorkspace,
    ) -> Result<usize, MatroidError> {
        let (vertices, rank) = LeviTraversal::forest(self, workspace, None)?;
        Ok(vertices - rank)
    }
    /// Deterministic spanning basis in (rank, cell, slot) order.
    pub fn spanning_basis(
        &self,
        workspace: &mut GraphicMatroidWorkspace,
        output: &mut Vec<CellLeviCoord>,
    ) -> Result<usize, MatroidError> {
        Ok(LeviTraversal::forest(self, workspace, Some(output))?.1)
    }
    /// Whether these incidence coordinates are cycle-free. The sequence must
    /// represent a set: offering the same coordinate twice returns false.
    pub fn independent(
        &self,
        coordinates: &[CellLeviCoord],
        workspace: &mut GraphicMatroidWorkspace,
    ) -> Result<bool, MatroidError> {
        LeviTraversal::independent(self, coordinates, workspace)
    }
    /// Begin incremental selection, borrowing the same canonical topology view.
    pub fn incremental<'s, 'w>(
        &'s self,
        workspace: &'w mut GraphicMatroidWorkspace,
    ) -> Result<CellLeviSession<'s, 'a, 'w>, MatroidError> {
        Ok(CellLeviSession {
            inner: LeviSession::begin(self, workspace)?,
        })
    }
}
impl LeviSource for &CellLeviMatroid<'_> {
    type Coordinate = CellLeviCoord;
    fn universe(self) -> Result<usize, MatroidError> {
        Ok(self.vertex_count())
    }
    fn coordinates(&self) -> impl Iterator<Item = CellLeviCoord> {
        (1..=self.topology.max_rank()).flat_map(|rank| {
            // Every rank in this bounded range was validated at construction.
            let cells = self.topology.rank(rank).expect("rank bounded by max_rank");
            (0..cells.n_cells()).flat_map(move |cell| {
                (0..cells.arity()).map(move |slot| CellLeviCoord { rank, cell, slot })
            })
        })
    }
    fn endpoints(self, c: CellLeviCoord) -> Result<(u32, u32), MatroidError> {
        let error = MatroidError::CellIncidenceOutOfRange {
            rank: c.rank,
            cell: c.cell,
            slot: c.slot,
        };
        let rank = self.topology.rank(c.rank).map_err(|_| error)?;
        if c.cell >= rank.n_cells() || c.slot >= rank.arity() {
            return Err(error);
        }
        let lower = rank.members()[c.cell * rank.arity() + c.slot] as usize;
        // Immutable topology validates member IDs; constructor checked total u32 width.
        Ok((
            (self.offsets[c.rank - 1] + lower) as u32,
            (self.offsets[c.rank] + c.cell) as u32,
        ))
    }
}
/// Incremental Levi selection bound to one cell topology and caller workspace.
pub struct CellLeviSession<'s, 'a, 'w> {
    inner: LeviSession<'w, &'s CellLeviMatroid<'a>>,
}
impl CellLeviSession<'_, '_, '_> {
    /// Accept an incidence iff it joins two previously distinct components.
    pub fn offer(&mut self, coordinate: CellLeviCoord) -> Result<bool, MatroidError> {
        self.inner.offer(coordinate)
    }
    /// Current independent-set cardinality.
    pub fn accepted(&self) -> usize {
        self.inner.accepted()
    }
}
