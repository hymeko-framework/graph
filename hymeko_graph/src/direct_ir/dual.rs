//! Dual reachability: the arcs reachable from a start hyperarc in the dual hypergraph.
//!
//! In the dual, hyperedges become vertices and two hyperedges are adjacent iff they share a node.
//! The walk alternates `arc -> node -> arc` directly on the canonical `Ir` -- no dual hypergraph is
//! materialised. Both a visited-arc **and** a visited-node set are required: marking a node the
//! first time it is reached means its incident-arc list is scanned once, so a node shared by `m`
//! arcs is not rescanned `m` times. Cost `O(arcs + nodes + incidences)` over the reached dual
//! component.

use hymeko::common::ids::HyperArcId;
use hymeko::ir::incidence::IncidenceIndex;

use super::workspace::TraversalWorkspace;
use super::{DirectIrError, is_node};

/// The set of hyperarcs reachable in the dual, borrowing the workspace arc-discovery buffer.
#[derive(Clone, Copy)]
pub struct DualReach<'w> {
    arcs: &'w [HyperArcId],
}

impl<'w> DualReach<'w> {
    /// Number of reachable hyperarcs (including the start).
    #[must_use]
    pub fn len(&self) -> usize {
        self.arcs.len()
    }

    /// Whether the reached set is empty (never, for a valid start).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.arcs.is_empty()
    }

    /// The reachable hyperarcs, in discovery order.
    #[must_use]
    pub fn arcs(&self) -> &'w [HyperArcId] {
        self.arcs
    }
}

/// Every hyperarc reachable from `start` in the dual hypergraph (two arcs adjacent iff they share
/// a node). A warm workspace allocates nothing.
///
/// # Errors
/// [`DirectIrError::InvalidStartArc`] if `start` is out of range;
/// [`DirectIrError::TargetNotANode`] on a malformed arc reference.
pub fn dual_reachability<'w>(
    index: &IncidenceIndex<'_>,
    ws: &'w mut TraversalWorkspace,
    start: HyperArcId,
) -> Result<DualReach<'w>, DirectIrError> {
    let ir = index.ir();
    if start.raw() >= ir.arcs.len() {
        return Err(DirectIrError::InvalidStartArc { arc: start });
    }
    ws.ensure_capacity(ir.decl_nodes.len(), ir.arcs.len());
    ws.begin();
    ws.discover_arc(start);
    ws.push_arc_frontier(start);
    while let Some(a) = ws.pop_arc_frontier() {
        // `a` was reached through the index / start check, so it indexes `ir.arcs`.
        debug_assert!(a.raw() < ir.arcs.len(), "dual arc out of ir.arcs range");
        let refs = &ir.arcs[a.raw()].refs;
        for (slot, sref) in refs.iter().enumerate() {
            let node = sref.target();
            if !is_node(ir, node) {
                return Err(DirectIrError::TargetNotANode {
                    arc: a,
                    slot: slot as u32,
                    target: node,
                });
            }
            if ws.node_seen(node) {
                continue;
            }
            ws.discover_node(node);
            for &b in index.incident_arcs(node) {
                if ws.arc_seen(b) {
                    continue;
                }
                ws.discover_arc(b);
                ws.push_arc_frontier(b);
            }
        }
    }
    Ok(DualReach {
        arcs: ws.arc_discovered(),
    })
}
