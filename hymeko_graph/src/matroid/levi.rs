//! The **implicit incidence/Levi graphic matroid**.
//!
//! The graphic matroid of the *implicit* bipartite incidence (Levi) graph of the canonical `Ir`:
//! - one vertex per canonical **node**, one vertex per canonical **hyperarc**;
//! - one graph **edge** per exact incidence coordinate `(HyperArcId, slot)`.
//!
//! Nothing is materialised — no Levi graph, star adjacency, pairwise graph, or second incidence table.
//! The canonical `Ir` (reached through the facade's `IncidenceIndex`) is iterated in place, and the
//! **shared** [`GraphicMatroidWorkspace`] DSU is reused over a combined dense coordinate space: node `n`
//! occupies coordinate `n`, hyperarc `a` occupies coordinate `N + a`.
//!
//! # Vertex universe (inclusion policy)
//! The universe is **all** `N` nodes and **all** `A` hyperarcs: `vertex_count = N + A`. An **isolated
//! node** and a **zero-incidence hyperarc** are therefore included, each as a singleton component. A
//! reference to a **non-node** (edge/arc) declaration, or a **dangling** target, is a typed
//! [`MatroidError::DanglingTarget`] — never silently skipped. Duplicate / parallel incidence coordinates
//! remain **distinct** ground elements; the second to be offered closes a cycle (dependent), it is not
//! deduplicated away. With `C` connected components:
//! ```text
//! rank = basis_size = vertex_count - C = (N + A) - C .
//! ```
//!
//! # Semantic boundary
//! A Levi spanning basis is a **connectivity certificate only**: it does not preserve weighted shortest
//! paths, incidence signs, hyperedge semantics, T/S-norm propagation, B/Bᵀ/BᵀB, or message passing.

use hymeko::common::ids::HyperArcId;
use hymeko::ir::ir::Ir;

use super::{GraphicMatroidWorkspace, MatroidError, MatroidRef};
use crate::direct_ir::decl_to_node;

/// An exact incidence coordinate — a ground element of the Levi matroid. Canonical and deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeviCoord {
    /// The hyperarc the incidence belongs to.
    pub arc: HyperArcId,
    /// The reference slot within that hyperarc.
    pub slot: u32,
}

/// The two DSU coordinates of one incidence: the target node's coordinate (`node.raw()`) and the
/// hyperarc's coordinate (`N + arc`). A non-node / dangling / out-of-range target is a typed error.
fn levi_endpoints(
    ir: &Ir,
    n_nodes: usize,
    arc: HyperArcId,
    slot: usize,
) -> Result<(u32, u32), MatroidError> {
    let ai = arc.raw();
    let refs = ir
        .arcs
        .get(ai)
        .map(|a| a.refs.as_slice())
        .filter(|r| slot < r.len())
        .ok_or(MatroidError::DanglingTarget {
            arc,
            slot: slot as u32,
        })?;
    match decl_to_node(ir, refs[slot].target()) {
        Some(nid) => Ok((nid.raw() as u32, (n_nodes + ai) as u32)),
        None => Err(MatroidError::DanglingTarget {
            arc,
            slot: slot as u32,
        }),
    }
}

/// Private common source for IR and graded-cell Levi incidences.
pub(super) trait LeviSource: Copy {
    type Coordinate: Copy;
    fn universe(self) -> Result<usize, MatroidError>;
    fn coordinates(&self) -> impl Iterator<Item = Self::Coordinate>;
    fn endpoints(self, coordinate: Self::Coordinate) -> Result<(u32, u32), MatroidError>;
}
#[derive(Clone, Copy)]
struct IrSource<'a>(&'a Ir);
impl LeviSource for IrSource<'_> {
    type Coordinate = LeviCoord;
    fn universe(self) -> Result<usize, MatroidError> {
        self.0
            .nodes
            .len()
            .checked_add(self.0.arcs.len())
            .ok_or(MatroidError::Overflow)
    }
    fn coordinates(&self) -> impl Iterator<Item = LeviCoord> {
        self.0.arcs.iter().enumerate().flat_map(|(arc, record)| {
            (0..record.refs.len()).map(move |slot| LeviCoord {
                arc: HyperArcId::new(arc),
                slot: slot as u32,
            })
        })
    }
    fn endpoints(self, c: LeviCoord) -> Result<(u32, u32), MatroidError> {
        levi_endpoints(self.0, self.0.nodes.len(), c.arc, c.slot as usize)
    }
}
/// The single incremental union step shared by every Levi source.
pub(super) struct LeviSession<'w, S: LeviSource> {
    ws: &'w mut GraphicMatroidWorkspace,
    pub(super) source: S,
    accepted: usize,
}
impl<'w, S: LeviSource> LeviSession<'w, S> {
    pub(super) fn begin(
        source: S,
        ws: &'w mut GraphicMatroidWorkspace,
    ) -> Result<Self, MatroidError> {
        ws.begin(source.universe()?)?;
        Ok(Self {
            source,
            ws,
            accepted: 0,
        })
    }
    pub(super) fn offer(&mut self, coordinate: S::Coordinate) -> Result<bool, MatroidError> {
        let (a, b) = self.source.endpoints(coordinate)?;
        let joined = self.ws.union(a, b);
        self.accepted += usize::from(joined);
        Ok(joined)
    }
    pub(super) fn accepted(&self) -> usize {
        self.accepted
    }
}
/// Batch operations consume the same source and incremental union owner.
pub(super) struct LeviTraversal;
impl LeviTraversal {
    pub(super) fn forest<S: LeviSource>(
        source: S,
        ws: &mut GraphicMatroidWorkspace,
        mut out: Option<&mut Vec<S::Coordinate>>,
    ) -> Result<(usize, usize), MatroidError> {
        let mut session = LeviSession::begin(source, ws)?;
        if let Some(output) = out.as_mut() {
            output.clear();
        }
        for coordinate in source.coordinates() {
            if session.offer(coordinate)?
                && let Some(output) = out.as_mut()
            {
                output.push(coordinate);
            }
        }
        Ok((source.universe()?, session.accepted()))
    }
    pub(super) fn independent<S: LeviSource>(
        source: S,
        coords: &[S::Coordinate],
        ws: &mut GraphicMatroidWorkspace,
    ) -> Result<bool, MatroidError> {
        let mut session = LeviSession::begin(source, ws)?;
        for &coordinate in coords {
            if !session.offer(coordinate)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// The Levi/incidence graphic-matroid **rank** = `(N + A) − C`, without retaining the basis.
///
/// # Errors
/// [`MatroidError::DanglingTarget`] for a non-node incidence, [`MatroidError::Overflow`] if `N + A`
/// exceeds `u32`.
pub fn levi_rank(
    m: MatroidRef<'_, '_>,
    ws: &mut GraphicMatroidWorkspace,
) -> Result<usize, MatroidError> {
    Ok(LeviTraversal::forest(IrSource(m.ir()), ws, None)?.1)
}

/// The number of connected components `C` of the implicit Levi graph over the full `N + A` vertex
/// universe (isolated nodes and zero-incidence hyperarcs counted as singletons).
///
/// # Errors
/// As [`levi_rank`].
pub fn levi_components(
    m: MatroidRef<'_, '_>,
    ws: &mut GraphicMatroidWorkspace,
) -> Result<usize, MatroidError> {
    let (vtx, rank) = LeviTraversal::forest(IrSource(m.ir()), ws, None)?;
    Ok(vtx - rank)
}

/// The Levi **spanning basis**: a maximal independent set of incidence coordinates (a spanning forest of
/// the implicit bipartite graph). Returns the basis size (= rank = `(N + A) − C`) and writes the chosen
/// canonical [`LeviCoord`]s into `out` in deterministic (arc, then slot) order.
///
/// # Errors
/// As [`levi_rank`].
pub fn levi_spanning_basis(
    m: MatroidRef<'_, '_>,
    ws: &mut GraphicMatroidWorkspace,
    out: &mut Vec<LeviCoord>,
) -> Result<usize, MatroidError> {
    Ok(LeviTraversal::forest(IrSource(m.ir()), ws, Some(out))?.1)
}

/// Is this set of incidence coordinates **independent** (acyclic) in the Levi matroid?
///
/// # Errors
/// As [`levi_rank`] (a bad / dangling coordinate is a typed error, not a silent skip).
pub fn levi_independent(
    m: MatroidRef<'_, '_>,
    coords: &[LeviCoord],
    ws: &mut GraphicMatroidWorkspace,
) -> Result<bool, MatroidError> {
    LeviTraversal::independent(IrSource(m.ir()), coords, ws)
}

/// An **incremental**, append-only Levi independence stream: feed incidence coordinates one at a time;
/// each call returns `true` if the coordinate is independent of those already accepted (joins the forest)
/// or `false` if it closes a cycle. Deletion is **not** supported.
pub struct IncrementalLevi<'ws, 'ir> {
    inner: LeviSession<'ws, IrSource<'ir>>,
}

impl<'ws, 'ir> IncrementalLevi<'ws, 'ir> {
    /// Begin an incremental session over the `N + A` vertex universe of `ir`.
    ///
    /// # Errors
    /// [`MatroidError::Overflow`] if `N + A` exceeds `u32`.
    pub fn begin(ws: &'ws mut GraphicMatroidWorkspace, ir: &'ir Ir) -> Result<Self, MatroidError> {
        Ok(Self {
            inner: LeviSession::begin(IrSource(ir), ws)?,
        })
    }
    /// Offer one incidence coordinate; `Ok(true)` = independent (accepted), `Ok(false)` = closes a cycle.
    ///
    /// # Errors
    /// As [`levi_rank`] for a non-node / dangling coordinate.
    pub fn offer(&mut self, ir: &'ir Ir, arc: HyperArcId, slot: u32) -> Result<bool, MatroidError> {
        if !std::ptr::eq(self.inner.source.0, ir) {
            return Err(MatroidError::ForeignSnapshot);
        }
        self.inner.offer(LeviCoord { arc, slot })
    }
    /// The number of accepted (independent) coordinates so far — the current forest rank.
    #[must_use]
    pub fn accepted(&self) -> usize {
        self.inner.accepted()
    }
}
