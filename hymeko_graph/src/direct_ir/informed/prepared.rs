//! Goal-bound structural lower bounds and entropy ordering on the existing engine.
use super::*;
use crate::direct_ir::TraversalWorkspace;
use hymeko::tensor::ShannonEntropy;

/// Reusable preparation for queries to one goal on one immutable canonical IR.
///
/// Undirected BFS gives a lower bound under any transition policy that only
/// removes transitions. Entropy orders equal-f candidates and never prunes.
/// Preparation is O(V + E + I) plus O(V log V) ranking, amortized across queries;
/// it is intentionally explicit, not imposed on one-shot searches.
pub struct PreparedBergeSearch<'a, 'ir> {
    traversal: HyperTraversal<'a, 'ir>,
    goal: NodeId,
    hops: Vec<u32>,
    priorities: Vec<u32>,
    max_hops: u32,
}
impl<'a, 'ir> PreparedBergeSearch<'a, 'ir> {
    /// Build relaxed distances using the existing alternating BFS expansion.
    /// Structural data stay bound to `index`, preventing reuse on another IR.
    pub fn build(
        index: &'a IncidenceIndex<'ir>,
        goal: NodeId,
        workspace: &mut TraversalWorkspace,
    ) -> Result<Self, InformedSearchError> {
        let traversal = HyperTraversal::new(index);
        let ir = traversal.ir();
        let goal_decl = node_to_decl(ir, goal)?;
        if ir.nodes.len() >= u32::MAX as usize {
            return Err(InformedSearchError::PreparationTooLarge {
                nodes: ir.nodes.len(),
            });
        }
        workspace.ensure_capacity(ir.decl_nodes.len(), ir.arcs.len());
        workspace.begin();
        let mut hops = vec![u32::MAX; ir.decl_nodes.len()];
        hops[goal_decl.raw()] = 0;
        workspace.discover_node(goal_decl);
        workspace.push_frontier(goal_decl);
        while let Some(node) = workspace.pop_frontier() {
            let first = workspace.discovered_len();
            traversal.expand(workspace, node, false)?;
            for &discovered in &workspace.discovered()[first..] {
                // Preparation checked node count; a simple BFS distance is smaller.
                hops[discovered.raw()] = hops[node.raw()] + 1;
            }
        }
        let priorities = Self::entropy_order(index);
        let max_hops = hops
            .iter()
            .copied()
            .filter(|&n| n != u32::MAX)
            .max()
            .unwrap_or(0);
        Ok(Self {
            traversal,
            goal,
            hops,
            priorities,
            max_hops,
        })
    }
    fn entropy_order(index: &IncidenceIndex<'_>) -> Vec<u32> {
        let ir = index.ir();
        let mut ranked: Vec<_> = ir
            .nodes
            .iter()
            .map(|node| {
                let entropy = ShannonEntropy::from_counts(
                    index
                        .incident_arcs(node.decl)
                        .iter()
                        .map(|arc| ir.arcs[arc.raw()].refs.len().saturating_sub(1)),
                )
                .bits();
                (entropy, node.decl)
            })
            .collect();
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.raw().cmp(&b.1.raw())));
        let mut priorities = vec![0; ir.decl_nodes.len()];
        for (rank, (_, node)) in ranked.into_iter().enumerate() {
            priorities[node.raw()] = rank as u32;
        }
        priorities
    }
    /// Exact relaxed hop distance; `None` means disconnected or an invalid node.
    pub fn relaxed_hops(&self, node: NodeId) -> Option<u32> {
        let declaration = self.traversal.ir().nodes.get(node.raw())?.decl;
        self.hops
            .get(declaration.raw())
            .copied()
            .filter(|&h| h != u32::MAX)
    }
    /// Deterministic entropy rank (lower entropy first). This value never enters f.
    pub fn entropy_priority(&self, node: NodeId) -> Option<u32> {
        let declaration = self.traversal.ir().nodes.get(node.raw())?.decl;
        self.priorities.get(declaration.raw()).copied()
    }
    /// Search through the canonical A* engine with goal-bound data and typed policy.
    ///
    /// A disconnected start is rejected before frontier work. A custom cost's
    /// `minimum_step_cost` must satisfy its trait contract. The default zero
    /// remains conservative for arbitrary nonnegative weighted costs.
    pub fn search<C: BergeCost, P: TransitionPolicy>(
        &self,
        start: NodeId,
        cost: &C,
        policy: &P,
        workspace: &mut InformedSearchWorkspace,
    ) -> Result<Option<WeightedBergePath>, InformedSearchError> {
        node_to_decl(self.traversal.ir(), start)?;
        let minimum = cost.minimum_step_cost();
        if !minimum.is_finite()
            || minimum < 0.0
            || !(f64::from(self.max_hops) * minimum).is_finite()
        {
            return Err(InformedSearchError::BadStepBound {
                value_bits: minimum.to_bits(),
            });
        }
        if self.relaxed_hops(start).is_none() {
            workspace.ensure(
                self.traversal.ir().decl_nodes.len(),
                self.traversal.ir().arcs.len(),
            );
            workspace.begin();
            return Ok(None);
        }
        // HopHeuristic already owns bound evaluation; the adapter adds ordering.
        // Construction already certifies the table's topology and maximum. Avoid
        // HopHeuristic::new's full table validation on every prepared query.
        let heuristic = OrderedHops {
            hops: HopHeuristic {
                hops: &self.hops,
                min_step_cost: minimum,
            },
            priorities: &self.priorities,
        };
        astar_berge_path(
            &self.traversal,
            start,
            self.goal,
            cost,
            &heuristic,
            policy,
            workspace,
        )
    }
}
struct OrderedHops<'a> {
    hops: HopHeuristic<'a>,
    priorities: &'a [u32],
}
impl BergeHeuristic for OrderedHops<'_> {
    const ADMISSIBLE: bool = true;
    const CONSISTENT: bool = true;
    fn h(&self, ir: &Ir, node: DeclId) -> f64 {
        self.hops.h(ir, node)
    }
    fn tiebreak(&self, _: &Ir, node: DeclId) -> u32 {
        self.priorities[node.raw()]
    }
}
