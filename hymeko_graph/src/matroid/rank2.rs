//! The **rank-2 graphic matroid**: ground elements are canonical `HyperArcId`s that resolve to exactly
//! two node incidences. A multiway hyperarc is a typed error, never silently reinterpreted as a pairwise
//! edge. Rank = `N − C`; the greedy weighted basis is Kruskal. Uses the shared
//! [`GraphicMatroidWorkspace`] DSU (no second union-find).

use hymeko::common::ids::{HyperArcId, NodeId};
use hymeko::ir::ir::{ArcRec, Ir, ValueR};

use super::{GraphicMatroidWorkspace, MatroidError, MatroidRef};
use crate::direct_ir::decl_to_node;

fn arc_weight(ir: &Ir, arc: &ArcRec) -> Option<f64> {
    arc.refs.iter().find_map(|r| {
        let w = ir.reference_atom(r).expect("model-local reference").weights;
        w.and_then(|ws| {
            ws.iter().find_map(|v| match v {
                ValueR::Num(n) => Some(*n),
                _ => None,
            })
        })
    })
}

/// The two node endpoints of a rank-2 hyperarc, or a typed error if it is not a clean pairwise edge.
fn rank2_endpoints(ir: &Ir, arc_id: HyperArcId) -> Result<(NodeId, NodeId), MatroidError> {
    let arc = ir
        .arcs
        .get(arc_id.raw())
        .ok_or(MatroidError::UnknownArc { arc: arc_id })?;
    let mut ends: [Option<NodeId>; 2] = [None, None];
    let mut count = 0usize;
    for (slot, sref) in arc.refs.iter().enumerate() {
        let t = sref.target();
        match decl_to_node(ir, t) {
            Some(nid) => {
                if count < 2 {
                    ends[count] = Some(nid);
                }
                count += 1;
            }
            None => {
                // a reference that targets neither a valid node makes this not a clean rank-2 edge
                return Err(MatroidError::DanglingTarget {
                    arc: arc_id,
                    slot: slot as u32,
                });
            }
        }
    }
    if count != 2 {
        return Err(MatroidError::WrongCardinality {
            arc: arc_id,
            node_incidences: count,
        });
    }
    let (u, v) = (ends[0].unwrap(), ends[1].unwrap());
    if u == v {
        return Err(MatroidError::SelfLoop { arc: arc_id });
    }
    Ok((u, v))
}

/// Rank-2 graphic-matroid **spanning forest**: the maximal independent set of hyperarcs (a forest over
/// the node set). Returns the basis size (= the matroid **rank** = `N − C`), writing the chosen canonical
/// `HyperArcId`s into `out`. Every hyperarc must be a rank-2 edge (else a typed error).
///
/// # Errors
/// [`MatroidError`] on a multiway/dangling/self-loop hyperarc or a `u32` overflow.
pub fn rank2_spanning_forest(
    m: MatroidRef<'_, '_>,
    ws: &mut GraphicMatroidWorkspace,
    out: &mut Vec<HyperArcId>,
) -> Result<usize, MatroidError> {
    let ir = m.ir();
    ws.begin(ir.nodes.len())?;
    out.clear();
    for ai in 0..ir.arcs.len() {
        let arc = HyperArcId::new(ai);
        let (u, v) = rank2_endpoints(ir, arc)?;
        if ws.union(u.raw() as u32, v.raw() as u32) {
            out.push(arc);
        }
    }
    Ok(out.len())
}

/// The rank of the rank-2 graphic matroid (`N − C`) without retaining the basis.
///
/// # Errors
/// As [`rank2_spanning_forest`].
pub fn rank2_rank(
    m: MatroidRef<'_, '_>,
    ws: &mut GraphicMatroidWorkspace,
) -> Result<usize, MatroidError> {
    let ir = m.ir();
    ws.begin(ir.nodes.len())?;
    let mut rank = 0usize;
    for ai in 0..ir.arcs.len() {
        let (u, v) = rank2_endpoints(ir, HyperArcId::new(ai))?;
        if ws.union(u.raw() as u32, v.raw() as u32) {
            rank += 1;
        }
    }
    Ok(rank)
}

/// The minimum- (`minimize=true`) or maximum-weight rank-2 spanning forest — the graphic-matroid greedy
/// algorithm (= Kruskal). Returns the total selected weight; `out` gets the canonical `HyperArcId`s.
///
/// # Errors
/// [`MatroidError::NonFiniteWeight`] for a non-finite weight; otherwise as [`rank2_spanning_forest`].
pub fn rank2_weighted_basis(
    m: MatroidRef<'_, '_>,
    minimize: bool,
    ws: &mut GraphicMatroidWorkspace,
    out: &mut Vec<HyperArcId>,
) -> Result<f64, MatroidError> {
    let ir = m.ir();
    ws.begin(ir.nodes.len())?;
    out.clear();
    // collect (weight, arc) for every rank-2 edge, validating cardinality up front.
    let mut edges: Vec<(f64, u32, u32, u32)> = Vec::with_capacity(ir.arcs.len());
    for ai in 0..ir.arcs.len() {
        let arc = HyperArcId::new(ai);
        let (u, v) = rank2_endpoints(ir, arc)?;
        let w = arc_weight(ir, &ir.arcs[ai]).unwrap_or(1.0);
        if !w.is_finite() {
            return Err(MatroidError::NonFiniteWeight { arc });
        }
        edges.push((w, ai as u32, u.raw() as u32, v.raw() as u32));
    }
    // greedy: sort by weight (stable, deterministic tie-break by arc index), then union.
    edges.sort_by(|a, b| {
        let o = a.0.total_cmp(&b.0);
        if minimize {
            o.then(a.1.cmp(&b.1))
        } else {
            o.reverse().then(a.1.cmp(&b.1))
        }
    });
    let mut total = 0.0;
    for (w, ai, u, v) in edges {
        if ws.union(u, v) {
            out.push(HyperArcId::new(ai as usize));
            total += w;
        }
    }
    Ok(total)
}

/// An **incremental**, append-only rank-2 independence stream: feed hyperarcs one at a time into a
/// caller-owned workspace; each call returns `true` if the hyperarc is independent of those already
/// accepted (joins the forest) or `false` if it closes a cycle. Deletion is **not** supported.
pub struct IncrementalRank2<'ws, 'ir> {
    ws: &'ws mut GraphicMatroidWorkspace,
    n_nodes: usize,
    snapshot: Option<&'ir Ir>,
    accepted: usize,
}

impl<'ws, 'ir> IncrementalRank2<'ws, 'ir> {
    /// Begin an incremental session over an `n_nodes`-node universe. The first offered arc binds the
    /// session to that canonical `Ir` snapshot; later offers from another snapshot are rejected.
    ///
    /// # Errors
    /// [`MatroidError::Overflow`] if `n_nodes` exceeds `u32`.
    pub fn begin(
        ws: &'ws mut GraphicMatroidWorkspace,
        n_nodes: usize,
    ) -> Result<Self, MatroidError> {
        ws.begin(n_nodes)?;
        Ok(Self {
            ws,
            n_nodes,
            snapshot: None,
            accepted: 0,
        })
    }
    /// Offer one rank-2 hyperarc; `Ok(true)` = independent (accepted), `Ok(false)` = closes a cycle.
    ///
    /// # Errors
    /// As [`rank2_spanning_forest`] for a non-rank-2 hyperarc.
    pub fn offer(&mut self, ir: &'ir Ir, arc: HyperArcId) -> Result<bool, MatroidError> {
        if ir.nodes.len() != self.n_nodes {
            return Err(MatroidError::NodeUniverseMismatch {
                expected: self.n_nodes,
                actual: ir.nodes.len(),
            });
        }
        match self.snapshot {
            None => self.snapshot = Some(ir),
            Some(snapshot) if !std::ptr::eq(snapshot, ir) => {
                return Err(MatroidError::ForeignSnapshot);
            }
            Some(_) => {}
        }
        let (u, v) = rank2_endpoints(ir, arc)?;
        let joined = self.ws.union(u.raw() as u32, v.raw() as u32);
        if joined {
            self.accepted += 1;
        }
        Ok(joined)
    }
    /// The number of accepted (independent) hyperarcs so far — the current forest rank.
    #[must_use]
    pub fn accepted(&self) -> usize {
        self.accepted
    }
}
