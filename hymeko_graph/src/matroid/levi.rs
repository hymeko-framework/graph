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

/// `(vertex_count, rank)` over the implicit Levi graph — the shared inner loop.
fn levi_forest(ir: &Ir, ws: &mut GraphicMatroidWorkspace) -> Result<(usize, usize), MatroidError> {
    let n = ir.nodes.len();
    let a = ir.arcs.len();
    let vtx = n.checked_add(a).ok_or(MatroidError::Overflow)?;
    ws.begin(vtx)?;
    let mut rank = 0usize;
    for ai in 0..a {
        let arc = HyperArcId::new(ai);
        for slot in 0..ir.arcs[ai].refs.len() {
            let (nc, ac) = levi_endpoints(ir, n, arc, slot)?;
            if ws.union(nc, ac) {
                rank += 1;
            }
        }
    }
    Ok((vtx, rank))
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
    Ok(levi_forest(m.ir(), ws)?.1)
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
    let (vtx, rank) = levi_forest(m.ir(), ws)?;
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
    let ir = m.ir();
    let n = ir.nodes.len();
    let a = ir.arcs.len();
    let vtx = n.checked_add(a).ok_or(MatroidError::Overflow)?;
    ws.begin(vtx)?;
    out.clear();
    for ai in 0..a {
        let arc = HyperArcId::new(ai);
        for slot in 0..ir.arcs[ai].refs.len() {
            let (nc, ac) = levi_endpoints(ir, n, arc, slot)?;
            if ws.union(nc, ac) {
                out.push(LeviCoord {
                    arc,
                    slot: slot as u32,
                });
            }
        }
    }
    Ok(out.len())
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
    let ir = m.ir();
    let n = ir.nodes.len();
    let vtx = n.checked_add(ir.arcs.len()).ok_or(MatroidError::Overflow)?;
    ws.begin(vtx)?;
    for c in coords {
        let (nc, ac) = levi_endpoints(ir, n, c.arc, c.slot as usize)?;
        if !ws.union(nc, ac) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// An **incremental**, append-only Levi independence stream: feed incidence coordinates one at a time;
/// each call returns `true` if the coordinate is independent of those already accepted (joins the forest)
/// or `false` if it closes a cycle. Deletion is **not** supported.
pub struct IncrementalLevi<'ws, 'ir> {
    ws: &'ws mut GraphicMatroidWorkspace,
    snapshot: &'ir Ir,
    n_nodes: usize,
    accepted: usize,
}

impl<'ws, 'ir> IncrementalLevi<'ws, 'ir> {
    /// Begin an incremental session over the `N + A` vertex universe of `ir`.
    ///
    /// # Errors
    /// [`MatroidError::Overflow`] if `N + A` exceeds `u32`.
    pub fn begin(ws: &'ws mut GraphicMatroidWorkspace, ir: &'ir Ir) -> Result<Self, MatroidError> {
        let vtx = ir
            .nodes
            .len()
            .checked_add(ir.arcs.len())
            .ok_or(MatroidError::Overflow)?;
        ws.begin(vtx)?;
        Ok(Self {
            ws,
            snapshot: ir,
            n_nodes: ir.nodes.len(),
            accepted: 0,
        })
    }
    /// Offer one incidence coordinate; `Ok(true)` = independent (accepted), `Ok(false)` = closes a cycle.
    ///
    /// # Errors
    /// As [`levi_rank`] for a non-node / dangling coordinate.
    pub fn offer(&mut self, ir: &'ir Ir, arc: HyperArcId, slot: u32) -> Result<bool, MatroidError> {
        if !std::ptr::eq(self.snapshot, ir) {
            return Err(MatroidError::ForeignSnapshot);
        }
        let (nc, ac) = levi_endpoints(self.snapshot, self.n_nodes, arc, slot as usize)?;
        let joined = self.ws.union(nc, ac);
        if joined {
            self.accepted += 1;
        }
        Ok(joined)
    }
    /// The number of accepted (independent) coordinates so far — the current forest rank.
    #[must_use]
    pub fn accepted(&self) -> usize {
        self.accepted
    }
}
