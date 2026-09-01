//! The alternating walk engine and the two load-bearing queries built on it.
//!
//! [`HyperTraversal::expand`] is the **single** place the `node -> arc -> node` step and the
//! visited-node AND visited-arc discipline live; [`reachable_nodes`](HyperTraversal::reachable_nodes)
//! and [`berge_shortest_path`](HyperTraversal::berge_shortest_path) -- and the sibling modules
//! [`super::components`], [`super::dual`], [`super::berge_enum`], [`super::s_connect`] -- drive it
//! rather than re-implementing the step.

use hymeko::common::ids::{DeclId, HyperArcId, NodeId};
use hymeko::ir::incidence::IncidenceIndex;
use hymeko::ir::ir::Ir;

use super::workspace::TraversalWorkspace;
use super::{DirectIrError, decl_to_node, is_node, node_to_decl};

/// A direct, zero-copy hypergraph traversal borrowing an [`IncidenceIndex`] (which already borrows
/// its `Ir`). One structured facade -- no separate `&Ir + &IncidenceIndex` pairing, no
/// builder/factory alias trio for the same operation.
#[derive(Clone, Copy)]
pub struct HyperTraversal<'a, 'ir> {
    index: &'a IncidenceIndex<'ir>,
}

impl<'a, 'ir> HyperTraversal<'a, 'ir> {
    /// Borrow an incidence index for traversal.
    #[must_use]
    pub fn new(index: &'a IncidenceIndex<'ir>) -> Self {
        Self { index }
    }

    /// The borrowed canonical `Ir`.
    #[inline]
    pub fn ir(&self) -> &'ir Ir {
        self.index.ir()
    }

    /// The borrowed incidence index.
    #[inline]
    pub fn index(&self) -> &'a IncidenceIndex<'ir> {
        self.index
    }

    /// Expand one node: for each *unseen* incident arc, mark it (so a `k`-member arc is expanded
    /// once per search), then discover its still-unseen node members. This is the only alternating
    /// step; every algorithm calls it.
    ///
    /// # Preconditions
    /// `node` is a valid, already-discovered node decl; the workspace is sized for this `Ir`
    /// (`ensure_capacity`). With `record_pred`, predecessor arrays are armed (`arm_predecessors`).
    ///
    /// # Errors
    /// [`DirectIrError::TargetNotANode`] if an arc reference targets a non-node declaration.
    pub(crate) fn expand(
        &self,
        ws: &mut TraversalWorkspace,
        node: DeclId,
        record_pred: bool,
    ) -> Result<(), DirectIrError> {
        let ir = self.index.ir();
        for &arc in self.index.incident_arcs(node) {
            if ws.arc_seen(arc) {
                continue;
            }
            ws.mark_arc(arc);
            // `arc` comes from the index, which was built over `ir.arcs`; it is in range.
            debug_assert!(
                arc.raw() < ir.arcs.len(),
                "arc from index out of ir.arcs range"
            );
            let refs = &ir.arcs[arc.raw()].refs;
            for (slot, sref) in refs.iter().enumerate() {
                let target = sref.target();
                if !is_node(ir, target) {
                    return Err(DirectIrError::TargetNotANode {
                        arc,
                        slot: slot as u32,
                        target,
                    });
                }
                // Already-seen covers the entry node, duplicate incidences, and self-incidence,
                // so none creates a second visited node -- yet multiplicity stays observable via
                // the index (`degree` / `incident_slots`).
                if ws.node_seen(target) {
                    continue;
                }
                ws.discover_node(target);
                if record_pred {
                    ws.set_pred(target, node, arc, slot as u32);
                }
                ws.push_frontier(target);
            }
        }
        Ok(())
    }

    /// Every node reachable from `start` through any incidence (`+`, `-` or `~`; weight does not
    /// gate reachability). `O(nodes + arcs + incidences)` over the visited component; a warm
    /// workspace allocates nothing.
    ///
    /// # Errors
    /// [`DirectIrError::InvalidStart`] if `start` is out of range;
    /// [`DirectIrError::TargetNotANode`] on a malformed arc reference.
    pub fn reachable_nodes<'w>(
        &self,
        ws: &'w mut TraversalWorkspace,
        start: NodeId,
    ) -> Result<NodeSet<'w, 'ir>, DirectIrError> {
        let ir = self.index.ir();
        let start_decl = node_to_decl(ir, start)?;
        ws.ensure_capacity(ir.decl_nodes.len(), ir.arcs.len());
        ws.begin();
        ws.discover_node(start_decl);
        ws.push_frontier(start_decl);
        while let Some(n) = ws.pop_frontier() {
            self.expand(ws, n, false)?;
        }
        Ok(NodeSet {
            ir,
            decls: ws.discovered(),
        })
    }

    /// A shortest Berge path from `start` to `goal` -- fewest hyperedges -- as an alternating
    /// sequence of steps, or `None` if `goal` is unreachable. Signs and weights are preserved:
    /// each step resolves back to a full `IncidenceRef` via its `(arc, slot)` coordinate.
    ///
    /// # Errors
    /// [`DirectIrError::InvalidStart`] if either endpoint is out of range;
    /// [`DirectIrError::TargetNotANode`] on a malformed arc reference;
    /// [`DirectIrError::PredecessorInconsistency`] if reconstruction hits a broken chain.
    pub fn berge_shortest_path(
        &self,
        ws: &mut TraversalWorkspace,
        start: NodeId,
        goal: NodeId,
    ) -> Result<Option<BergePath>, DirectIrError> {
        let ir = self.index.ir();
        let start_decl = node_to_decl(ir, start)?;
        let goal_decl = node_to_decl(ir, goal)?;
        ws.ensure_capacity(ir.decl_nodes.len(), ir.arcs.len());
        ws.arm_predecessors(ir.decl_nodes.len());
        ws.begin();
        if start_decl == goal_decl {
            return Ok(Some(BergePath {
                start,
                steps: Vec::new(),
            }));
        }
        ws.discover_node(start_decl);
        ws.push_frontier(start_decl);
        while let Some(n) = ws.pop_frontier() {
            self.expand(ws, n, true)?;
            if ws.node_seen(goal_decl) {
                return self
                    .reconstruct(ws, ir, start, start_decl, goal_decl)
                    .map(Some);
            }
        }
        Ok(None)
    }

    /// Walk predecessors from `goal_decl` back to `start_decl`, emitting alternating steps.
    fn reconstruct(
        &self,
        ws: &TraversalWorkspace,
        ir: &'ir Ir,
        start: NodeId,
        start_decl: DeclId,
        goal_decl: DeclId,
    ) -> Result<BergePath, DirectIrError> {
        let mut steps_rev: Vec<BergeStep> = Vec::new();
        let mut cur = goal_decl;
        while cur != start_decl {
            let (pred_node, pred_arc, pred_slot) = ws.pred(cur);
            if pred_arc.is_none() {
                return Err(DirectIrError::PredecessorInconsistency { node: cur });
            }
            let node_id = decl_to_node(ir, cur)
                .ok_or(DirectIrError::PredecessorInconsistency { node: cur })?;
            let iref = self.index.resolve(pred_arc, pred_slot);
            steps_rev.push(BergeStep {
                arc: pred_arc,
                slot: pred_slot,
                sign: iref.sign,
                node: node_id,
            });
            cur = pred_node;
        }
        steps_rev.reverse();
        Ok(BergePath {
            start,
            steps: steps_rev,
        })
    }
}

/// A set of reachable nodes, borrowing the workspace discovery buffer (no allocation to obtain).
/// Members are canonical [`NodeId`]s; the raw [`DeclId`] slice is available for index queries.
#[derive(Clone, Copy)]
pub struct NodeSet<'w, 'ir> {
    ir: &'ir Ir,
    decls: &'w [DeclId],
}

impl<'w, 'ir> NodeSet<'w, 'ir> {
    /// Number of reachable nodes (including the start).
    #[must_use]
    pub fn len(&self) -> usize {
        self.decls.len()
    }

    /// Whether the set is empty (never, for a valid start -- the start is always present).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }

    /// The reachable nodes as canonical [`NodeId`]s, in discovery order.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        let ir = self.ir;
        self.decls.iter().filter_map(move |&d| ir.as_node(d))
    }

    /// The reachable nodes as backing [`DeclId`]s (for further incidence queries).
    #[must_use]
    pub fn decls(&self) -> &'w [DeclId] {
        self.decls
    }
}

/// One alternating step of a Berge path: the `node` reached via `arc` at reference `slot`, with the
/// incidence `sign`. The full incidence (weights, annotation, target) resolves from `(arc, slot)`
/// through [`IncidenceIndex::resolve`]; nothing is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BergeStep {
    /// The hyperedge (canonical arc) traversed into this step's node.
    pub arc: HyperArcId,
    /// The reference slot of `node` within `arc` (its exact incidence coordinate).
    pub slot: u32,
    /// The incidence sign of `node` in `arc` (`+1` / `-1` / `0`).
    pub sign: i8,
    /// The node reached by this step.
    pub node: NodeId,
}

/// A Berge path: a start node followed by alternating `(arc, node)` steps. Its length in
/// hyperedges is `steps.len()`; the node sequence is `start` then each `step.node`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BergePath {
    /// The origin node.
    pub start: NodeId,
    /// The alternating steps, in order from `start` to the goal.
    pub steps: Vec<BergeStep>,
}

impl BergePath {
    /// Path length in hyperedges (Berge distance).
    #[must_use]
    pub fn hyperedge_len(&self) -> usize {
        self.steps.len()
    }

    /// The node sequence, `start` first.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::once(self.start).chain(self.steps.iter().map(|s| s.node))
    }

    /// The hyperedges traversed, in order.
    pub fn arcs(&self) -> impl Iterator<Item = HyperArcId> + '_ {
        self.steps.iter().map(|s| s.arc)
    }
}
