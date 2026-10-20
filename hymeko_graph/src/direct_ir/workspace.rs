//! Caller-owned scratch memory for direct-`Ir` traversal.
//!
//! Visitation uses typed sparse sets: inverse positions are live only when the
//! dense discovery buffer points back to the same ID. Clearing a query resets
//! lengths in O(1), without a generation counter or a wraparound sweep.
//! Preparation reserves the full declaration/arc domains. Compared with u32
//! stamps, full-width positions and arc discovery require more memory.
//! Predecessor arrays are still allocated lazily for path queries.

use hymeko::common::ids::{DeclId, DeclTag, HyperArcId, HyperArcTag, Id};
use std::collections::VecDeque;

/// Query membership and discovery order, owned together to prevent stale aliases.
/// Only canonical IDs are stored, so clearing the dense Vec runs no destructors.
pub(super) struct SparseVisited<Tag> {
    positions: Vec<usize>,
    dense: Vec<Id<Tag>>,
}
impl<Tag> Default for SparseVisited<Tag> {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            dense: Vec::new(),
        }
    }
}
impl<Tag> std::fmt::Debug for SparseVisited<Tag> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(&self.dense).finish()
    }
}
impl<Tag> SparseVisited<Tag> {
    pub(super) fn ensure(&mut self, domain: usize) {
        if self.positions.len() < domain {
            self.positions.resize(domain, 0);
        }
        if self.dense.capacity() < domain {
            self.dense.reserve(domain - self.dense.len());
        }
    }
    pub(super) fn clear(&mut self) {
        self.dense.clear();
    }
    #[inline]
    pub(super) fn contains(&self, id: Id<Tag>) -> bool {
        self.positions
            .get(id.raw())
            .and_then(|&at| self.dense.get(at))
            == Some(&id)
    }
    /// Returns true only on first discovery. ID must fit the prepared domain.
    #[inline]
    pub(super) fn insert(&mut self, id: Id<Tag>) -> bool {
        if self.contains(id) {
            return false;
        }
        self.positions[id.raw()] = self.dense.len();
        self.dense.push(id);
        true
    }
    pub(super) fn as_slice(&self) -> &[Id<Tag>] {
        &self.dense
    }
}

/// Reusable scratch for the alternating hypergraph walk. See the module docs for the layout and
/// the zero-allocation-on-warm-query contract.
#[derive(Debug, Default)]
pub struct TraversalWorkspace {
    nodes: SparseVisited<DeclTag>,
    arcs: SparseVisited<HyperArcTag>,
    frontier: VecDeque<DeclId>,
    arc_frontier: VecDeque<HyperArcId>,
    // Lazily sized (path queries only).
    pred_node: Vec<DeclId>,
    pred_arc: Vec<HyperArcId>,
    pred_slot: Vec<u32>,
    /// The slot the predecessor occupied in `pred_arc` -- the step's ENTRY coordinate, recorded
    /// because the expansion is the last place it is known.
    pred_entry_slot: Vec<u32>,
    pred_ready: bool,
}

impl TraversalWorkspace {
    /// A fresh, empty workspace. Grows on first use.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepare the full ID domains and FIFO capacity; grows only. Warm queries
    /// within these domains need no additional visitation/frontier allocation.
    pub fn ensure_capacity(&mut self, n_decls: usize, n_arcs: usize) {
        self.nodes.ensure(n_decls);
        self.arcs.ensure(n_arcs);
        if self.frontier.capacity() < n_decls {
            self.frontier.reserve(n_decls - self.frontier.len());
        }
        if self.arc_frontier.capacity() < n_arcs {
            self.arc_frontier.reserve(n_arcs - self.arc_frontier.len());
        }
        if self.pred_ready && self.pred_node.len() < n_decls {
            self.pred_ready = false;
        }
    }

    /// Begin a new query with O(1) logical clears; stale inverse positions cannot
    /// alias live discovery entries. No query counter can wrap.
    pub fn begin(&mut self) {
        self.nodes.clear();
        self.arcs.clear();
        self.frontier.clear();
        self.arc_frontier.clear();
    }

    /// Prepare predecessor arrays for a path query. Cheap when already sized.
    pub(crate) fn arm_predecessors(&mut self, n_decls: usize) {
        if !self.pred_ready || self.pred_node.len() < n_decls {
            self.pred_node.resize(n_decls, DeclId::NONE);
            self.pred_arc.resize(n_decls, HyperArcId::NONE);
            self.pred_slot.resize(n_decls, 0);
            self.pred_entry_slot.resize(n_decls, 0);
            self.pred_ready = true;
        }
    }

    // ---- node-space (primal) ----

    #[inline]
    pub(crate) fn node_seen(&self, d: DeclId) -> bool {
        self.nodes.contains(d)
    }

    /// Mark a node seen and append it to the discovery buffer. Precondition (debug-checked):
    /// `d` is in range and not already seen this query.
    #[inline]
    pub(crate) fn discover_node(&mut self, d: DeclId) {
        debug_assert!(!self.node_seen(d), "double-discover of a node");
        self.nodes.insert(d);
    }

    #[inline]
    pub(crate) fn push_frontier(&mut self, d: DeclId) {
        self.frontier.push_back(d);
    }

    #[inline]
    pub(crate) fn pop_frontier(&mut self) -> Option<DeclId> {
        self.frontier.pop_front()
    }

    /// The nodes discovered by the current query, in discovery order (seeds first per layer).
    #[inline]
    pub(crate) fn discovered(&self) -> &[DeclId] {
        self.nodes.as_slice()
    }

    /// Number of nodes discovered so far this query (component range markers).
    #[inline]
    pub(crate) fn discovered_len(&self) -> usize {
        self.nodes.as_slice().len()
    }

    // ---- arc-space (dual) ----

    #[inline]
    pub(crate) fn arc_seen(&self, a: HyperArcId) -> bool {
        self.arcs.contains(a)
    }

    /// Mark an arc expanded/reached. Precondition (debug-checked): not already marked this query.
    #[inline]
    pub(crate) fn mark_arc(&mut self, a: HyperArcId) {
        debug_assert!(!self.arc_seen(a), "double-mark of an arc");
        self.arcs.insert(a);
    }

    /// Mark an arc reached (dual traversal) and record it in the arc discovery buffer.
    #[inline]
    pub(crate) fn discover_arc(&mut self, a: HyperArcId) {
        self.mark_arc(a);
    }

    #[inline]
    pub(crate) fn push_arc_frontier(&mut self, a: HyperArcId) {
        self.arc_frontier.push_back(a);
    }

    #[inline]
    pub(crate) fn pop_arc_frontier(&mut self) -> Option<HyperArcId> {
        self.arc_frontier.pop_front()
    }

    /// The arcs reached by the current dual query, in discovery order.
    #[inline]
    pub(crate) fn arc_discovered(&self) -> &[HyperArcId] {
        self.arcs.as_slice()
    }

    // ---- predecessors (path reconstruction) ----

    #[inline]
    pub(crate) fn set_pred(
        &mut self,
        node: DeclId,
        pred_node: DeclId,
        arc: HyperArcId,
        entry_slot: u32,
        slot: u32,
    ) {
        let i = node.raw();
        self.pred_node[i] = pred_node;
        self.pred_arc[i] = arc;
        self.pred_entry_slot[i] = entry_slot;
        self.pred_slot[i] = slot;
    }

    #[inline]
    pub(crate) fn pred(&self, node: DeclId) -> (DeclId, HyperArcId, u32, u32) {
        let i = node.raw();
        (
            self.pred_node[i],
            self.pred_arc[i],
            self.pred_entry_slot[i],
            self.pred_slot[i],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_membership_matches_an_independent_boolean_oracle() {
        let mut visited = SparseVisited::<DeclTag>::default();
        for domain in [1, 7, 32, 9, 65] {
            visited.ensure(domain);
            let capacities = (visited.positions.capacity(), visited.dense.capacity());
            for round in 0..37 {
                visited.clear();
                let mut oracle = vec![false; domain];
                let mut order = Vec::new();
                for step in 0..domain * 3 {
                    let raw = (step * 17 + round * 13) % domain;
                    let id = DeclId::new(raw);
                    assert_eq!(visited.insert(id), !oracle[raw]);
                    if !oracle[raw] {
                        order.push(id);
                    }
                    oracle[raw] = true;
                    for (raw, &expected) in oracle.iter().enumerate() {
                        assert_eq!(visited.contains(DeclId::new(raw)), expected);
                    }
                    assert!(!visited.contains(DeclId::NONE));
                    assert_eq!(visited.as_slice(), order);
                }
                assert_eq!(
                    capacities,
                    (visited.positions.capacity(), visited.dense.capacity())
                );
            }
        }
        assert!(!format!("{visited:?}").is_empty());
    }

    #[test]
    fn begin_leaves_inverse_storage_untouched_and_reuses_discovery_order() {
        let mut ws = TraversalWorkspace::new();
        ws.ensure_capacity(8, 5);
        ws.begin();
        ws.discover_node(DeclId::new(7));
        ws.discover_node(DeclId::new(1));
        ws.mark_arc(HyperArcId::new(4));
        ws.discover_arc(HyperArcId::new(2));
        let before = (ws.nodes.positions.clone(), ws.arcs.positions.clone());
        ws.begin();
        assert_eq!(
            before,
            (ws.nodes.positions.clone(), ws.arcs.positions.clone())
        );
        assert!(!ws.node_seen(DeclId::new(1)));
        assert!(!ws.arc_seen(HyperArcId::new(4)));
        ws.discover_node(DeclId::new(1));
        ws.discover_arc(HyperArcId::new(2));
        assert!(
            !ws.node_seen(DeclId::new(7)),
            "stale position 0 must not alias"
        );
        assert!(!ws.arc_seen(HyperArcId::new(4)));
        assert_eq!(ws.discovered(), &[DeclId::new(1)]);
        assert_eq!(ws.arc_discovered(), &[HyperArcId::new(2)]);
        ws.arm_predecessors(8);
        ws.ensure_capacity(16, 10);
        assert!(!ws.pred_ready);
        ws.arm_predecessors(16);
        ws.discover_node(DeclId::new(15));
        ws.discover_arc(HyperArcId::new(9));
        assert!(ws.node_seen(DeclId::new(15)));
        assert!(ws.arc_seen(HyperArcId::new(9)));
    }
}
