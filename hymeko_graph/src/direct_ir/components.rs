//! Connected components of the canonical `Ir` hypergraph, by direct traversal.
//!
//! One query covers the whole labeling: each still-unseen node seeds a flood that reuses the
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
    /// Component labels from already validated numeric coordinates. Reuses the
    /// matroid union-find and avoids constructing a reverse incidence index.
    pub(crate) fn from_projection(
        projection: &hymeko::ir::incidence::IncidenceProjection<'_>,
        workspace: &mut crate::GraphicMatroidWorkspace,
    ) -> Result<Self, crate::MatroidError> {
        workspace.begin(projection.domain_len())?;
        for row in projection.rows() {
            let mut targets = row.iter_targets();
            if let Some(first) = targets.next() {
                for target in targets {
                    workspace.union(first as u32, target as u32);
                }
            }
        }
        let mut roots = vec![u32::MAX; projection.domain_len()];
        let mut label_of = Vec::with_capacity(projection.domain_len());
        let mut count = 0;
        for node in 0..projection.domain_len() {
            let root = workspace.find(node as u32) as usize;
            if roots[root] == u32::MAX {
                roots[root] = count;
                count += 1;
            }
            label_of.push(roots[root]);
        }
        Ok(Self { label_of, count })
    }
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

#[cfg(test)]
mod projection_tests {
    #[test]
    fn shared_union_find_labels_match_the_existing_incidence_traversal() {
        use super::*;
        use crate::ir_test_support::Case;
        use hymeko::ir::incidence::{IncidenceDomain, IncidenceProjection, build_incidence_index};
        use hymeko::ir::ir::IrBuilder;
        let ir = Case::unsigned(9, &[&[0, 1, 1], &[1, 4], &[2, 3], &[3, 5], &[], &[7]])
            .build()
            .unwrap();
        let projection = IncidenceProjection::build(&ir, IncidenceDomain::Nodes).unwrap();
        let got = ComponentLabeling::from_projection(
            &projection,
            &mut crate::GraphicMatroidWorkspace::new(),
        )
        .unwrap();
        let index = build_incidence_index(&ir).unwrap();
        let expected = connected_components(&index, &mut TraversalWorkspace::new()).unwrap();
        assert_eq!(got.labels(), expected.labels());
        assert_eq!(got.count(), 5);
    }
}
