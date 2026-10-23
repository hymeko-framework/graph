//! Goal-bound Berge certificates and entropy ordering on the existing A* engine.
use super::*;
use crate::direct_ir::TraversalWorkspace;
use hymeko::tensor::ShannonEntropy;

/// Reusable preparation for queries to one goal on one immutable canonical IR.
///
/// Undirected BFS gives a lower bound under any transition policy that only
/// removes transitions. Entropy orders equal-f candidates and never prunes.
/// Recorded incidence links offer a candidate route whose forward legality and
/// cost are rechecked on every query before it supplies an upper bound. Per-arc
/// relaxed distances can then reject entire expansions without visiting members.
/// Preparation is O(V + E + I) plus O(V log V) ranking, amortized across queries;
/// it is intentionally explicit, not imposed on one-shot searches.
pub struct PreparedBergeSearch<'a, 'ir> {
    traversal: HyperTraversal<'a, 'ir>,
    goal: NodeId,
    hops: Vec<u32>,
    priorities: Vec<u32>,
    max_hops: u32,
    toward_goal: Vec<GoalLink>,
    arc_hops: Vec<u32>,
}

/// Two exact incidence slots in one canonical hyperarc. The exit target is
/// borrowed from the IR, so no duplicate node or topology table is necessary.
#[derive(Clone, Copy)]
struct GoalLink {
    arc: HyperArcId,
    from_slot: u32,
    to_slot: u32,
}

impl GoalLink {
    const NONE: Self = Self {
        arc: HyperArcId::NONE,
        from_slot: 0,
        to_slot: 0,
    };
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
        workspace.arm_predecessors(ir.decl_nodes.len());
        let mut hops = vec![u32::MAX; ir.decl_nodes.len()];
        let mut toward_goal = vec![GoalLink::NONE; ir.decl_nodes.len()];
        hops[goal_decl.raw()] = 0;
        workspace.discover_node(goal_decl);
        workspace.push_frontier(goal_decl);
        while let Some(node) = workspace.pop_frontier() {
            let first = workspace.discovered_len();
            traversal.expand(workspace, node, true)?;
            for &discovered in &workspace.discovered()[first..] {
                // Preparation checked node count; a simple BFS distance is smaller.
                hops[discovered.raw()] = hops[node.raw()] + 1;
                let (_, arc, toward_slot, discovered_slot) = workspace.pred(discovered);
                // Reverse the undirected discovery step. A later query checks
                // its own direction and cost before accepting this as a route.
                toward_goal[discovered.raw()] = GoalLink {
                    arc,
                    from_slot: discovered_slot,
                    to_slot: toward_slot,
                };
            }
        }
        let priorities = Self::entropy_order(index);
        let max_hops = hops
            .iter()
            .copied()
            .filter(|&n| n != u32::MAX)
            .max()
            .unwrap_or(0);
        let arc_hops = ir
            .arcs
            .iter()
            .map(|arc| {
                arc.refs
                    .iter()
                    .filter_map(|r| hops.get(r.target().raw()).copied())
                    .min()
                    .unwrap_or(u32::MAX)
            })
            .collect();
        Ok(Self {
            traversal,
            goal,
            hops,
            priorities,
            max_hops,
            toward_goal,
            arc_hops,
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
        self.search_with_heuristic(start, cost, &ZeroHeuristic, policy, workspace)
    }

    /// A* with `max(caller_bound, relaxed_hops * minimum_step_cost)` and a
    /// validated Berge-route incumbent. The caller's heuristic participates in
    /// f, not just ordering, and must be admissible and consistent for this
    /// same goal, cost model and policy. Entropy supplies the equal-f tie-break;
    /// the caller heuristic's optional tie-break is not used by this facade.
    ///
    /// A direction-rejected candidate route disables only the initial upper
    /// bound; it never proves that the requested directed goal is unreachable.
    /// Equal-cost candidates remain eligible. Preparation is explicit and must
    /// be included when evaluating one-shot or changing-topology workloads.
    ///
    /// # Errors
    /// As [`astar_berge_path`], plus [`InformedSearchError::BadStepBound`] for an
    /// invalid cost certificate. Legal candidate-route costs are validated by
    /// the same engine helper as expanded transitions.
    pub fn search_with_heuristic<C: BergeCost, H: BergeHeuristic, P: TransitionPolicy>(
        &self,
        start: NodeId,
        cost: &C,
        heuristic: &H,
        policy: &P,
        workspace: &mut InformedSearchWorkspace,
    ) -> Result<Option<WeightedBergePath>, InformedSearchError> {
        let start_decl = node_to_decl(self.traversal.ir(), start)?;
        let minimum = cost.minimum_step_cost();
        if !minimum.is_finite()
            || minimum < 0.0
            || !(f64::from(self.max_hops) * minimum).is_finite()
        {
            return Err(InformedSearchError::BadStepBound {
                value_bits: minimum.to_bits(),
            });
        }
        // Share bound composition with the existing heuristic contract.
        let hops = HopHeuristic {
            hops: &self.hops,
            min_step_cost: minimum,
        };
        let ordered = OrderedBounds {
            bound: MaxHeuristic::new(&hops, heuristic),
            priorities: &self.priorities,
        };
        let engine = InformedSearch::new(&self.traversal, cost, &ordered, policy)?;
        if self.relaxed_hops(start).is_none() {
            workspace.ensure(
                self.traversal.ir().decl_nodes.len(),
                self.traversal.ir().arcs.len(),
            );
            workspace.begin();
            return Ok(None);
        }
        let bounds = self.bounds(start_decl, minimum, &engine)?;
        engine.run(start, self.goal, workspace, bounds)
    }

    fn bounds<C: BergeCost, H: BergeHeuristic, P: TransitionPolicy>(
        &self,
        start: DeclId,
        minimum: f64,
        engine: &InformedSearch<'_, '_, '_, C, H, P>,
    ) -> Result<BergeBounds<'_>, InformedSearchError> {
        let mut bounds = BergeBounds {
            arc_hops: &self.arc_hops,
            minimum_step: minimum,
            ..BergeBounds::default()
        };
        let index = self.traversal.index();
        let goal = self.traversal.ir().nodes[self.goal.raw()].decl;
        let mut node = start;
        let mut upper = 0.0;
        for _ in 0..self.hops[start.raw()] {
            let link = self.toward_goal[node.raw()];
            if link.arc.is_none() {
                return Err(InformedSearchError::PredecessorInconsistency { node });
            }
            let from = index.resolve(link.arc, link.from_slot);
            let to = index.resolve(link.arc, link.to_slot);
            bounds.witness_steps += 1;
            if !engine.policy.allows(&from, &to) {
                return Ok(bounds);
            }
            upper += engine.step_cost(node, &to)?;
            if !upper.is_finite() {
                // An overflowing optional route is not a finite certificate.
                // A different finite route may still exist; let A* find it.
                return Ok(bounds);
            }
            node = to.target;
        }
        if node != goal {
            return Err(InformedSearchError::PredecessorInconsistency { node });
        }
        bounds.upper = upper;
        Ok(bounds)
    }
}
struct OrderedBounds<'a, H> {
    bound: H,
    priorities: &'a [u32],
}
impl<H: BergeHeuristic> BergeHeuristic for OrderedBounds<'_, H> {
    const ADMISSIBLE: bool = H::ADMISSIBLE;
    const CONSISTENT: bool = H::CONSISTENT;
    fn h(&self, ir: &Ir, node: DeclId) -> f64 {
        self.bound.h(ir, node)
    }
    fn tiebreak(&self, _: &Ir, node: DeclId) -> u32 {
        self.priorities[node.raw()]
    }
}
