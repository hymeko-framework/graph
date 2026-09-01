//! Connected components of the canonical `Ir` hypergraph, by direct traversal.
//!
//! One generation covers the whole labeling: each still-unseen node seeds a flood that reuses the
//! shared [`HyperTraversal::expand`] step, and the nodes it discovers form one component. Because
//! the visited marks persist across floods (no `begin` between them), every node is touched exactly
//! once -- total cost `O(nodes + arcs + incidences)`.

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::IncidenceIndex;

use super::DirectIrError;
use super::traversal::HyperTraversal;
use super::workspace::TraversalWorkspace;

/// A labeling of every node into its connected component. Component ids are dense `0..count`.
#[derive(Debug, Clone)]
pub struct ComponentLabeling {
    label_of: Vec<u32>,
    count: u32,
}

impl ComponentLabeling {
    /// Number of connected components.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The component id of `node`, or `None` if the id is out of range.
    #[must_use]
    pub fn component_of(&self, node: NodeId) -> Option<u32> {
        self.label_of.get(node.raw()).copied()
    }

    /// Whether two nodes lie in the same component (`None` if either id is out of range).
    #[must_use]
    pub fn same_component(&self, a: NodeId, b: NodeId) -> Option<bool> {
        Some(self.component_of(a)? == self.component_of(b)?)
    }

    /// The per-node label slice, indexed by `NodeId`.
    #[must_use]
    pub fn labels(&self) -> &[u32] {
        &self.label_of
    }
}

/// Partition every node of the `Ir` into connected components (`+`/`-`/`~` all connect).
///
/// # Errors
/// [`DirectIrError::TargetNotANode`] on a malformed arc reference.
pub fn connected_components(
    index: &IncidenceIndex<'_>,
    ws: &mut TraversalWorkspace,
) -> Result<ComponentLabeling, DirectIrError> {
    let ir = index.ir();
    let trav = HyperTraversal::new(index);
    ws.ensure_capacity(ir.decl_nodes.len(), ir.arcs.len());
    ws.begin();

    let mut label_of = vec![0u32; ir.nodes.len()];
    let mut count = 0u32;

    for nid in 0..ir.nodes.len() {
        let decl = ir.nodes[nid].decl;
        if ws.node_seen(decl) {
            continue;
        }
        let comp = count;
        count += 1;
        let lo = ws.discovered_len();
        ws.discover_node(decl);
        ws.push_frontier(decl);
        while let Some(n) = ws.pop_frontier() {
            trav.expand(ws, n, false)?;
        }
        let hi = ws.discovered_len();
        for &d in &ws.discovered()[lo..hi] {
            if let Some(node) = ir.as_node(d) {
                label_of[node.raw()] = comp;
            }
        }
    }

    Ok(ComponentLabeling { label_of, count })
}
