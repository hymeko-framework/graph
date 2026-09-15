//! Caller-owned scratch memory for direct-`Ir` traversal.
//!
//! One [`TraversalWorkspace`] is allocated once and reused across many queries on `Ir`s of the
//! same shape; a **warm** query does no heap allocation. Visited state is
//! *generation-marked* -- each query bumps a counter instead of clearing `O(n)` marks, so the
//! only linear clear is the rare generation wrap (exercised by a test).
//!
//! # Layout (data-oriented)
//! - `node_gen[d]` / `arc_gen[a]`: a decl/arc is "seen this query" iff its stamp `== gen`.
//!   Node marks are indexed in `DeclId` space (the incidence key space), arc marks in
//!   `HyperArcId` space.
//! - `frontier` / `discovered`: node-space FIFO and discovery buffer (primal traversal).
//! - `arc_frontier` / `arc_discovered`: arc-space FIFO and discovery buffer (dual traversal).
//! - predecessor arrays (`pred_node`/`pred_arc`/`pred_entry_slot`/`pred_slot`): allocated lazily, only for path
//!   queries that reconstruct an alternating walk.

use std::collections::VecDeque;

use hymeko::common::ids::{DeclId, HyperArcId};

/// Reusable scratch for the alternating hypergraph walk. See the module docs for the layout and
/// the zero-allocation-on-warm-query contract.
#[derive(Debug, Default)]
pub struct TraversalWorkspace {
    node_gen: Vec<u32>,
    arc_gen: Vec<u32>,
    stamp: u32,
    frontier: VecDeque<DeclId>,
    discovered: Vec<DeclId>,
    arc_frontier: VecDeque<HyperArcId>,
    arc_discovered: Vec<HyperArcId>,
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

    /// Ensure the visited-mark arrays cover `n_decls` decls and `n_arcs` arcs. Idempotent; grows
    /// only. New slots are stamped `0` (never equal to a live `gen`, which is `>= 1`).
    pub fn ensure_capacity(&mut self, n_decls: usize, n_arcs: usize) {
        if self.node_gen.len() < n_decls {
            self.node_gen.resize(n_decls, 0);
        }
        if self.arc_gen.len() < n_arcs {
            self.arc_gen.resize(n_arcs, 0);
        }
        // A grow can invalidate the lazily-sized predecessor arrays.
        if self.pred_ready && self.pred_node.len() < n_decls {
            self.pred_ready = false;
        }
    }

    /// Begin a new query: advance the generation (clearing all marks in `O(1)` amortised), and
    /// empty the frontiers and discovery buffers. Handles generation wrap by clearing the stamps
    /// once when the counter saturates -- after which no stale stamp can alias a live generation.
    pub fn begin(&mut self) {
        if self.stamp == u32::MAX {
            self.node_gen.iter_mut().for_each(|g| *g = 0);
            self.arc_gen.iter_mut().for_each(|g| *g = 0);
            self.stamp = 1;
        } else {
            self.stamp += 1;
        }
        self.frontier.clear();
        self.discovered.clear();
        self.arc_frontier.clear();
        self.arc_discovered.clear();
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
        self.node_gen.get(d.raw()).is_some_and(|&g| g == self.stamp)
    }

    /// Mark a node seen and append it to the discovery buffer. Precondition (debug-checked):
    /// `d` is in range and not already seen this query.
    #[inline]
    pub(crate) fn discover_node(&mut self, d: DeclId) {
        debug_assert!(d.raw() < self.node_gen.len(), "node decl out of capacity");
        debug_assert!(!self.node_seen(d), "double-discover of a node");
        self.node_gen[d.raw()] = self.stamp;
        self.discovered.push(d);
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
        &self.discovered
    }

    /// Number of nodes discovered so far this query (component range markers).
    #[inline]
    pub(crate) fn discovered_len(&self) -> usize {
        self.discovered.len()
    }

    // ---- arc-space (dual) ----

    #[inline]
    pub(crate) fn arc_seen(&self, a: HyperArcId) -> bool {
        self.arc_gen.get(a.raw()).is_some_and(|&g| g == self.stamp)
    }

    /// Mark an arc expanded/reached. Precondition (debug-checked): not already marked this query.
    #[inline]
    pub(crate) fn mark_arc(&mut self, a: HyperArcId) {
        debug_assert!(a.raw() < self.arc_gen.len(), "arc out of capacity");
        debug_assert!(!self.arc_seen(a), "double-mark of an arc");
        self.arc_gen[a.raw()] = self.stamp;
    }

    /// Mark an arc reached (dual traversal) and record it in the arc discovery buffer.
    #[inline]
    pub(crate) fn discover_arc(&mut self, a: HyperArcId) {
        self.mark_arc(a);
        self.arc_discovered.push(a);
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
        &self.arc_discovered
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

    /// Current generation stamp (for tests exercising wrap).
    #[cfg(test)]
    pub(crate) fn generation(&self) -> u32 {
        self.stamp
    }

    /// Force the generation near saturation (test-only, to exercise wrap in [`begin`]).
    #[cfg(test)]
    pub(crate) fn force_generation(&mut self, g: u32) {
        self.stamp = g;
    }
}

#[cfg(test)]
mod tests {
    use super::TraversalWorkspace;
    use hymeko::common::ids::{DeclId, HyperArcId};

    #[test]
    fn generation_advance_clears_node_marks() {
        let mut ws = TraversalWorkspace::new();
        ws.ensure_capacity(4, 2);
        ws.begin();
        let d = DeclId::new(2);
        ws.discover_node(d);
        assert!(ws.node_seen(d));
        ws.begin();
        assert!(!ws.node_seen(d), "a new generation clears the mark");
    }

    #[test]
    fn generation_wrap_resets_stale_stamps() {
        let mut ws = TraversalWorkspace::new();
        ws.ensure_capacity(4, 2);
        ws.begin(); // stamp = 1
        let d = DeclId::new(1);
        ws.discover_node(d); // node_gen[1] = 1
        // Saturate the counter so the next begin() must wrap.
        ws.force_generation(u32::MAX);
        assert!(
            !ws.node_seen(d),
            "stale stamp 1 must not alias generation u32::MAX"
        );
        ws.begin(); // wraps: clears stamps, stamp = 1
        assert_eq!(ws.generation(), 1);
        assert!(!ws.node_seen(d), "wrap cleared the stale stamp");
        ws.discover_node(d);
        assert!(ws.node_seen(d), "workspace remains usable after a wrap");
    }

    #[test]
    fn arc_marks_are_generation_scoped() {
        let mut ws = TraversalWorkspace::new();
        ws.ensure_capacity(2, 3);
        ws.begin();
        let a = HyperArcId::new(2);
        assert!(!ws.arc_seen(a));
        ws.discover_arc(a);
        assert!(ws.arc_seen(a));
        assert_eq!(ws.arc_discovered(), &[a]);
        ws.begin();
        assert!(!ws.arc_seen(a), "arc marks reset with the generation");
    }
}
