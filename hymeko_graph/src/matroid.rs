//! Zero-copy **rank-2 graphic matroid** over the canonical `hymeko_core::ir::Ir` and its sparse
//! `IncidenceIndex`. This is NOT a second graph model: [`MatroidRef`] is a borrowed, `Copy`, two-word
//! facade over the existing objects (O(1), zero-allocation construction, no graph-sized loop, cannot
//! outlive the snapshot), and [`GraphicMatroidWorkspace`] is caller-owned DSU *scratch* (parent / rank /
//! generation / scratch), never canonical topology. Ground elements are canonical `HyperArcId`s that
//! resolve to exactly two valid node incidences; a multiway hyperarc is a typed error, never silently
//! reinterpreted as a pairwise edge.
//!
//! # Complexity (honest)
//! Facade creation is O(1). Rank / spanning-forest / weighted-basis computation is Θ(ground set) — never
//! O(1). Workspace preparation is measured separately. The basis is written into a caller-owned buffer.
//! No claim is made that B, Bᵀ, BᵀB or a complete matroid basis is computed in O(1).
//!
//! # Semantic boundary
//! A rank-2 graphic basis preserves **incidence-graph connectivity only**. It does NOT preserve
//! shortest-path cost, hyperedge cardinality after removal, signs/weights as an operator, T/S-norm
//! propagation, BᵀB, or message passing. It must never replace the canonical hypergraph.

use std::fmt;

use hymeko::common::ids::{HyperArcId, NodeId};
use hymeko::ir::incidence::IncidenceIndex;
use hymeko::ir::ir::{Ir, SignedRefR, ValueR};

use crate::direct_ir::decl_to_node;

/// A borrowed, `Copy`, O(1) facade over the canonical `Ir` + its incidence index. Two references, no
/// topology; cannot outlive the borrowed snapshot; never serialized.
#[derive(Clone, Copy)]
pub struct MatroidRef<'a, 'ir> {
    ir: &'a Ir,
    incidence: &'a IncidenceIndex<'ir>,
}

impl<'a, 'ir> MatroidRef<'a, 'ir> {
    /// Borrow the canonical objects. O(1), zero allocation, no loop, no clone.
    #[inline]
    #[must_use]
    pub fn new(ir: &'a Ir, incidence: &'a IncidenceIndex<'ir>) -> Self {
        Self { ir, incidence }
    }
    /// The borrowed canonical `Ir` (the sole semantic model).
    #[inline]
    #[must_use]
    pub fn ir(&self) -> &'a Ir {
        self.ir
    }
    /// The borrowed sparse incidence index (the sole permitted derived view).
    #[inline]
    #[must_use]
    pub fn incidence(&self) -> &'a IncidenceIndex<'ir> {
        self.incidence
    }
}

/// Typed failure of a rank-2 matroid operation. A multiway hyperarc is rejected, never treated as an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatroidError {
    /// A hyperarc does not resolve to exactly two node incidences (e.g. a multiway transport edge).
    WrongCardinality {
        /// The offending hyperarc.
        arc: HyperArcId,
        /// How many node incidences it actually has (≠ 2).
        node_incidences: usize,
    },
    /// A hyperarc reference targets a declaration that is not a valid node.
    DanglingTarget {
        /// The offending hyperarc.
        arc: HyperArcId,
        /// The reference slot within the hyperarc whose target is not a node.
        slot: u32,
    },
    /// A hyperarc's two node incidences are the same node (self-loop), which the policy rejects.
    SelfLoop {
        /// The offending hyperarc.
        arc: HyperArcId,
    },
    /// A weighted operation encountered a non-finite weight.
    NonFiniteWeight {
        /// The offending hyperarc.
        arc: HyperArcId,
    },
    /// The snapshot has more nodes than fit the internal `u32` DSU coordinate.
    Overflow,
}

impl fmt::Display for MatroidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongCardinality {
                arc,
                node_incidences,
            } => write!(
                f,
                "hyperarc {} has {node_incidences} node incidences, not the 2 a rank-2 edge requires",
                arc.raw()
            ),
            Self::DanglingTarget { arc, slot } => {
                write!(
                    f,
                    "hyperarc {} slot {slot} targets a non-node declaration",
                    arc.raw()
                )
            }
            Self::SelfLoop { arc } => write!(f, "hyperarc {} is a self-loop", arc.raw()),
            Self::NonFiniteWeight { arc } => {
                write!(f, "hyperarc {} has a non-finite weight", arc.raw())
            }
            Self::Overflow => write!(f, "snapshot node count exceeds the u32 DSU coordinate"),
        }
    }
}
impl std::error::Error for MatroidError {}

/// Caller-owned reusable DSU scratch for the rank-2 graphic matroid. Path-halving find, union by rank,
/// generation-stamped lazy reset (no full clear between queries). Stores **no** canonical topology.
#[derive(Default)]
pub struct GraphicMatroidWorkspace {
    parent: Vec<u32>,
    rank: Vec<u32>,
    generation: Vec<u32>,
    cur_gen: u32,
    /// General-purpose scratch (e.g. sorted edge indices for a weighted basis).
    scratch: Vec<u32>,
}

impl GraphicMatroidWorkspace {
    /// A fresh, empty workspace. Grows lazily on first use to fit the snapshot's node count.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retained scratch bytes (for memory accounting; not topology).
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        (self.parent.capacity()
            + self.rank.capacity()
            + self.generation.capacity()
            + self.scratch.capacity())
            * std::mem::size_of::<u32>()
    }

    fn begin(&mut self, n: usize) -> Result<(), MatroidError> {
        if u32::try_from(n).is_err() {
            return Err(MatroidError::Overflow);
        }
        if self.parent.len() < n {
            self.parent.resize(n, 0);
            self.rank.resize(n, 0);
            self.generation.resize(n, 0);
        }
        self.cur_gen = self.cur_gen.wrapping_add(1);
        if self.cur_gen == 0 {
            self.generation.iter_mut().for_each(|g| *g = 0);
            self.cur_gen = 1;
        }
        self.scratch.clear();
        Ok(())
    }

    #[inline]
    fn touch(&mut self, x: u32) {
        let i = x as usize;
        if self.generation[i] != self.cur_gen {
            self.generation[i] = self.cur_gen;
            self.parent[i] = x;
            self.rank[i] = 0;
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        self.touch(x);
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            self.touch(p);
            let gp = self.parent[p as usize];
            self.touch(gp);
            self.parent[x as usize] = gp; // path halving
            x = gp;
        }
        x
    }

    /// Union `a` and `b`; returns `true` iff they were in different sets (i.e. this edge joins the forest).
    fn union(&mut self, a: u32, b: u32) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        let (ra, rb) = (ra as usize, rb as usize);
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => self.parent[ra] = rb as u32,
            std::cmp::Ordering::Greater => self.parent[rb] = ra as u32,
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra as u32;
                self.rank[ra] += 1;
            }
        }
        true
    }
}

fn arc_weight(arc: &hymeko::ir::ir::ArcRec) -> Option<f64> {
    arc.refs.iter().find_map(|r| {
        let w = match r {
            SignedRefR::Plus(x) | SignedRefR::Minus(x) | SignedRefR::Neutral(x) => {
                x.weights.as_deref()
            }
        };
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
    let arc = &ir.arcs[arc_id.raw()];
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
    let ir = m.ir;
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
    let ir = m.ir;
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
    let ir = m.ir;
    ws.begin(ir.nodes.len())?;
    out.clear();
    // collect (weight, arc) for every rank-2 edge, validating cardinality up front.
    let mut edges: Vec<(f64, u32, u32, u32)> = Vec::with_capacity(ir.arcs.len());
    for ai in 0..ir.arcs.len() {
        let arc = HyperArcId::new(ai);
        let (u, v) = rank2_endpoints(ir, arc)?;
        let w = arc_weight(&ir.arcs[ai]).unwrap_or(1.0);
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
pub struct IncrementalRank2<'ws> {
    ws: &'ws mut GraphicMatroidWorkspace,
    accepted: usize,
}

impl<'ws> IncrementalRank2<'ws> {
    /// Begin an incremental session over an `n_nodes`-node snapshot.
    ///
    /// # Errors
    /// [`MatroidError::Overflow`] if `n_nodes` exceeds `u32`.
    pub fn begin(
        ws: &'ws mut GraphicMatroidWorkspace,
        n_nodes: usize,
    ) -> Result<Self, MatroidError> {
        ws.begin(n_nodes)?;
        Ok(Self { ws, accepted: 0 })
    }
    /// Offer one rank-2 hyperarc; `Ok(true)` = independent (accepted), `Ok(false)` = closes a cycle.
    ///
    /// # Errors
    /// As [`rank2_spanning_forest`] for a non-rank-2 hyperarc.
    pub fn offer(&mut self, ir: &Ir, arc: HyperArcId) -> Result<bool, MatroidError> {
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
