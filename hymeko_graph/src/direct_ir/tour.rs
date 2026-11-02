//! Metric Christofides over canonical hypergraph incidences.
//!
//! The objective is a closed walk visiting every node, charging a nonnegative
//! symmetric cost on every hyperarc traversal. The result may repeat nodes and
//! hyperarcs. It is NOT a simple Berge cycle or a pay-once hyperedge-cover tour.
//! Shortest-path metric closure, MST, exact odd-vertex matching, Euler shortcutting
//! and expansion give the classical 3/2 bound in exact arithmetic. Floating-point
//! arithmetic has its usual roundoff; no interval-certified bound is claimed.
//!
//! The initial exact matching backend uses bounded subset dynamic programming,
//! not polynomial-time blossom. Exceeding a budget returns an error, never a
//! greedy matching disguised as Christofides. Metric closure costs O(n^2) searches
//! and O(n^2) scalars; it stores no second graph or all-pairs path payloads.

use super::{
    BergeCost, BergePath, HyperTraversal, InformedSearchError, InformedSearchWorkspace, Undirected,
    dijkstra_berge_path,
};
use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::{IncidenceIndex, IncidenceRef};

/// Explicit resource limits, checked before the corresponding allocations.
#[derive(Debug, Clone, Copy)]
pub struct TourLimits {
    /// Maximum original vertices, bounding the quadratic distance table.
    pub max_nodes: usize,
    /// Maximum subset DP entries (2 raised to the number of odd vertices).
    pub max_matching_states: usize,
    /// Maximum number of returned original hyperarc traversals.
    pub max_walk_steps: usize,
}
impl Default for TourLimits {
    fn default() -> Self {
        Self {
            max_nodes: 256,
            max_matching_states: 1 << 20,
            max_walk_steps: 1_000_000,
        }
    }
}

/// A closed incidence walk plus the Hamiltonian order in the derived metric.
#[derive(Debug, Clone)]
pub struct BergeTour {
    /// All original nodes once, followed by the first node again.
    pub order: Vec<NodeId>,
    /// Expanded original incidences, potentially with repeats.
    pub walk: BergePath,
    /// Cost summed over every original hyperarc traversal in the walk.
    pub total_cost: f64,
    /// Minimum spanning tree cost in the shortest-path metric.
    pub tree_cost: f64,
    /// Exact minimum perfect matching cost on odd tree vertices.
    pub matching_cost: f64,
    /// Size of the exact matching domain, controlling subset-DP complexity.
    pub odd_vertices: usize,
}

/// Invalid domain, infeasible tour, exhausted budget, or failed search.
#[derive(Debug, Clone, PartialEq)]
pub enum TourError {
    /// No starting vertex exists.
    Empty,
    /// Cost array does not cover exactly the canonical arcs.
    CostDomain,
    /// A negative or nonfinite traversal weight.
    InvalidCost {
        /// Canonical arc position.
        arc: usize,
    },
    /// Two required vertices have no connecting path.
    Disconnected {
        /// Search origin.
        from: NodeId,
        /// Unreachable destination.
        to: NodeId,
    },
    /// An explicit resource cap was exceeded.
    Budget {
        /// Bounded resource name.
        resource: &'static str,
    },
    /// Finite input costs overflowed during accumulation.
    NonFiniteTotal,
    /// The shared search engine rejected the input or computation.
    Search(InformedSearchError),
}
impl std::fmt::Display for TourError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TourError {}

struct ArcCosts<'a>(&'a [f64]);
impl BergeCost for ArcCosts<'_> {
    fn step_cost(&self, _: DeclId, to: &IncidenceRef<'_>) -> f64 {
        self.0[to.arc.raw()]
    }
}

/// One owner of bounded Christofides construction and incidence expansion.
pub struct Christofides;
impl Christofides {
    /// Solve the all-node closed-walk problem with one cost per canonical arc.
    /// Costs ignore sign and port direction, explicitly selecting an undirected metric.
    ///
    /// # Errors
    /// Rejects empty/disconnected domains, invalid costs, arithmetic overflow,
    /// malformed search input, and each exceeded resource budget.
    pub fn solve(
        index: &IncidenceIndex<'_>,
        costs: &[f64],
        limits: TourLimits,
    ) -> Result<BergeTour, TourError> {
        let n = index.ir().nodes.len();
        if n == 0 {
            return Err(TourError::Empty);
        }
        if n > limits.max_nodes || n.checked_mul(n).is_none() {
            return Err(TourError::Budget {
                resource: "metric nodes",
            });
        }
        if costs.len() != index.ir().arcs.len() {
            return Err(TourError::CostDomain);
        }
        for (arc, &cost) in costs.iter().enumerate() {
            if !cost.is_finite() || cost < 0.0 {
                return Err(TourError::InvalidCost { arc });
            }
        }
        let cost = ArcCosts(costs);
        let traversal = HyperTraversal::new(index);
        let mut workspace = InformedSearchWorkspace::new();
        let mut metric = Metric {
            n,
            distances: vec![0.0; n * n],
        };
        for u in 0..n {
            for v in u + 1..n {
                let path = dijkstra_berge_path(
                    &traversal,
                    NodeId::new(u),
                    NodeId::new(v),
                    &cost,
                    &Undirected,
                    &mut workspace,
                )
                .map_err(TourError::Search)?
                .ok_or(TourError::Disconnected {
                    from: NodeId::new(u),
                    to: NodeId::new(v),
                })?;
                metric.distances[u * n + v] = path.total_cost;
                metric.distances[v * n + u] = path.total_cost;
            }
        }
        let (mut edges, tree_cost) = metric.spanning_tree();
        let mut degrees = vec![0usize; n];
        for &(u, v) in &edges {
            degrees[u] += 1;
            degrees[v] += 1;
        }
        let odd: Vec<_> = (0..n).filter(|&v| degrees[v] % 2 == 1).collect();
        let (pairs, matching_cost) =
            ExactMatching::solve(&metric, &odd, limits.max_matching_states)?;
        edges.extend(pairs);
        let order = Self::shortcut(n, &edges);
        let mut walk = BergePath {
            start: NodeId::new(0),
            steps: Vec::new(),
        };
        let mut total_cost = 0.0;
        for pair in order.windows(2) {
            let path = dijkstra_berge_path(
                &traversal,
                NodeId::new(pair[0]),
                NodeId::new(pair[1]),
                &cost,
                &Undirected,
                &mut workspace,
            )
            .map_err(TourError::Search)?
            .ok_or(TourError::Disconnected {
                from: NodeId::new(pair[0]),
                to: NodeId::new(pair[1]),
            })?;
            if path.steps.len() > limits.max_walk_steps.saturating_sub(walk.steps.len()) {
                return Err(TourError::Budget {
                    resource: "expanded walk steps",
                });
            }
            for step in &path.steps {
                total_cost += costs[step.arc.raw()];
            }
            walk.steps.extend(path.steps);
        }
        if !total_cost.is_finite() || !tree_cost.is_finite() || !matching_cost.is_finite() {
            return Err(TourError::NonFiniteTotal);
        }
        Ok(BergeTour {
            order: order.into_iter().map(NodeId::new).collect(),
            walk,
            total_cost,
            tree_cost,
            matching_cost,
            odd_vertices: odd.len(),
        })
    }

    fn shortcut(n: usize, edges: &[(usize, usize)]) -> Vec<usize> {
        let mut adjacency = vec![Vec::new(); n];
        for (id, &(u, v)) in edges.iter().enumerate() {
            adjacency[u].push((v, id));
            adjacency[v].push((u, id));
        }
        let mut used = vec![false; edges.len()];
        let mut stack = vec![0];
        let mut circuit = Vec::with_capacity(edges.len() + 1);
        while let Some(&u) = stack.last() {
            if let Some((v, id)) = adjacency[u].pop() {
                if !used[id] {
                    used[id] = true;
                    stack.push(v);
                }
            } else {
                circuit.push(stack.pop().unwrap());
            }
        }
        let mut seen = vec![false; n];
        let mut order = Vec::with_capacity(n + 1);
        for u in circuit.into_iter().rev() {
            if !seen[u] {
                seen[u] = true;
                order.push(u);
            }
        }
        order.push(0);
        order
    }
}

/// A derived distance table, not an alternate IR or adjacency representation.
struct Metric {
    n: usize,
    distances: Vec<f64>,
}
impl Metric {
    fn at(&self, u: usize, v: usize) -> f64 {
        self.distances[u * self.n + v]
    }
    fn spanning_tree(&self) -> (Vec<(usize, usize)>, f64) {
        // Dense Prim consumes metric scalars directly. The existing rank-2
        // matroid forest consumes original rank-2 IR arcs and is not this domain.
        let mut best = vec![f64::INFINITY; self.n];
        let mut parent = vec![0; self.n];
        let mut used = vec![false; self.n];
        best[0] = 0.0;
        let mut edges = Vec::with_capacity(self.n - 1);
        let mut total = 0.0;
        for _ in 0..self.n {
            let u = (0..self.n)
                .filter(|&u| !used[u])
                .min_by(|&u, &v| best[u].total_cmp(&best[v]).then(u.cmp(&v)))
                .unwrap();
            used[u] = true;
            if u != 0 {
                edges.push((parent[u], u));
                total += best[u];
            }
            for v in 0..self.n {
                let value = self.at(u, v);
                if !used[v] && value < best[v] {
                    best[v] = value;
                    parent[v] = u;
                }
            }
        }
        (edges, total)
    }
}

struct ExactMatching<'a> {
    metric: &'a Metric,
    odd: &'a [usize],
    memo: Vec<f64>,
    choice: Vec<usize>,
}
impl<'a> ExactMatching<'a> {
    fn solve(
        metric: &'a Metric,
        odd: &'a [usize],
        cap: usize,
    ) -> Result<(Vec<(usize, usize)>, f64), TourError> {
        let states = u32::try_from(odd.len())
            .ok()
            .and_then(|k| 1usize.checked_shl(k))
            .filter(|&states| states <= cap)
            .ok_or(TourError::Budget {
                resource: "matching states",
            })?;
        let mut solver = Self {
            metric,
            odd,
            memo: vec![f64::NAN; states],
            choice: vec![0; states],
        };
        solver.memo[0] = 0.0;
        let mut mask = states - 1;
        let cost = solver.minimum(mask);
        if !cost.is_finite() {
            return Err(TourError::NonFiniteTotal);
        }
        let mut pairs = Vec::with_capacity(odd.len() / 2);
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            let j = solver.choice[mask];
            pairs.push((odd[i], odd[j]));
            mask ^= (1 << i) | (1 << j);
        }
        Ok((pairs, cost))
    }
    fn minimum(&mut self, mask: usize) -> f64 {
        if !self.memo[mask].is_nan() {
            return self.memo[mask];
        }
        let i = mask.trailing_zeros() as usize;
        let rest = mask ^ (1 << i);
        let mut candidates = rest;
        let mut best = f64::INFINITY;
        while candidates != 0 {
            let j = candidates.trailing_zeros() as usize;
            candidates &= candidates - 1;
            let value = self.metric.at(self.odd[i], self.odd[j]) + self.minimum(rest ^ (1 << j));
            if value < best {
                best = value;
                self.choice[mask] = j;
            }
        }
        self.memo[mask] = best;
        best
    }
}
