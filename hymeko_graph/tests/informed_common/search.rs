//! Shared test execution: borrow the caller's traversal and scratch, preserving
//! reuse and statistics. Only public Graph entry points implement the algorithms.

use hymeko::common::ids::NodeId;
use hymeko_graph::direct_ir::{
    BergeCost, BergeHeuristic, HyperTraversal, InformedSearchError, InformedSearchWorkspace,
    SearchStats, TransitionPolicy, WeightedBergePath, astar_berge_path, dijkstra_berge_path,
};

pub type SearchResult = Result<Option<WeightedBergePath>, InformedSearchError>;

pub trait SearchRunner {
    fn run(
        &self,
        traversal: &HyperTraversal<'_, '_>,
        start: NodeId,
        goal: NodeId,
        workspace: &mut InformedSearchWorkspace,
    ) -> SearchResult;
}

pub struct Dijkstra<'a, C, P> {
    cost: &'a C,
    policy: &'a P,
}
pub struct AStar<'a, C, H, P> {
    cost: &'a C,
    heuristic: &'a H,
    policy: &'a P,
}

impl<'a, C: BergeCost, P: TransitionPolicy> Dijkstra<'a, C, P> {
    pub fn new(cost: &'a C, policy: &'a P) -> Self {
        Self { cost, policy }
    }
}

impl<'a, C: BergeCost, H: BergeHeuristic, P: TransitionPolicy> AStar<'a, C, H, P> {
    pub fn new(cost: &'a C, heuristic: &'a H, policy: &'a P) -> Self {
        Self {
            cost,
            heuristic,
            policy,
        }
    }
}

impl<C: BergeCost, P: TransitionPolicy> SearchRunner for Dijkstra<'_, C, P> {
    fn run(
        &self,
        traversal: &HyperTraversal<'_, '_>,
        start: NodeId,
        goal: NodeId,
        workspace: &mut InformedSearchWorkspace,
    ) -> SearchResult {
        dijkstra_berge_path(traversal, start, goal, self.cost, self.policy, workspace)
    }
}

impl<C: BergeCost, H: BergeHeuristic, P: TransitionPolicy> SearchRunner for AStar<'_, C, H, P> {
    fn run(
        &self,
        traversal: &HyperTraversal<'_, '_>,
        start: NodeId,
        goal: NodeId,
        workspace: &mut InformedSearchWorkspace,
    ) -> SearchResult {
        astar_berge_path(
            traversal,
            start,
            goal,
            self.cost,
            self.heuristic,
            self.policy,
            workspace,
        )
    }
}

pub struct SearchCase<'a, 'index, 'ir, 'scratch> {
    traversal: &'a HyperTraversal<'index, 'ir>,
    start: NodeId,
    goal: NodeId,
    workspace: &'scratch mut InformedSearchWorkspace,
}

impl<'a, 'index, 'ir, 'scratch> SearchCase<'a, 'index, 'ir, 'scratch> {
    pub fn new(
        traversal: &'a HyperTraversal<'index, 'ir>,
        start: NodeId,
        goal: NodeId,
        workspace: &'scratch mut InformedSearchWorkspace,
    ) -> Self {
        Self {
            traversal,
            start,
            goal,
            workspace,
        }
    }
    pub fn run(&mut self, strategy: &impl SearchRunner) -> SearchResult {
        strategy.run(self.traversal, self.start, self.goal, self.workspace)
    }
    pub fn stats(&self) -> SearchStats {
        self.workspace.stats()
    }
}
