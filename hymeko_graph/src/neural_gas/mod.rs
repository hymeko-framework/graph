//! Bounded batch neural-gas graph induction over 3-D points.
//!
//! This implements the deterministic batch/update/growth portion of Siow et al.,
//! Biomimetics 2024, 9:560, equations 16--28 and 30--35
//! (<https://doi.org/10.3390/biomimetics9090560>). Initialization and scheduling
//! belong to the caller. Random inactive-node deletion, color features and the
//! complete Fast MS-DBL-GNG procedure are not implemented by this contract.
//!
//! Winners use Euclidean distance with lower-index ties. Neighbors come from the
//! **pre-batch** graph. Winner and neighbor deltas are normalized separately.
//! New edges replace old ones; isolated nodes are removed in stable index order.
//! Growth uses a linearly interpolated error quantile and strict `>` selection.
//! The growth count is fixed before insertion; parent choices are recomputed.
//! Connectivity uses the existing [`crate::SignedGraph`] and CSR implementation.
//! Learned edges are statistical connections, not collision-free robot passages.
//!
//! ```
//! use hymeko_graph::{SignedGraph, neural_gas::{BatchGas, BatchGasConfig}};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let seed = SignedGraph { n_nodes: 2, edges: vec![(0, 1)], signs: vec![1] };
//! let config = BatchGasConfig { max_nodes: 3, ..Default::default() };
//! let mut map = BatchGas::new(&[[0., 0., 0.], [4., 0., 0.]], &seed, config)?;
//! let receipt = map.step(&[[0., 0., 0.], [2., 0., 0.]])?;
//! assert_eq!(receipt.evaluated_pairs, 4);
//! assert_eq!(receipt.insertion_parents, vec![[0, 1]]);
//! assert_eq!(map.graph().edges, vec![(0, 2), (1, 2)]);
//! # Ok(()) }
//! ```
//!
//! APPROVED-CORE-EDIT: independent BatchGas extension, user 2026-10-10.

mod batch;

use crate::SignedGraph;
use std::{collections::BTreeSet, error::Error, fmt};

const MAX_NODES: usize = 4096;
const MAX_POINTS: usize = 65536;
const MAX_EDGES: usize = 262144;
const MAX_PAIRS: usize = 16_777_216;
const MAX_COORDINATE: f64 = 1_000_000.0;

/// Parameters of one synchronous update and its subsequent graph growth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatchGasConfig {
    /// First-winner step size, in [0,1]. The two step sizes must sum to <= 1.
    pub winner_rate: f64,
    /// Pre-batch neighbor step size, in [0,1].
    pub neighbour_rate: f64,
    /// Positive normalization floor, in (0,1].
    pub epsilon: f64,
    /// Multiplier on accumulated errors after the batch, in [0,1].
    pub error_discount: f64,
    /// Multiplier on both parent errors when splitting an edge, in [0,1].
    pub growth_discount: f64,
    /// Linear quantile in [0,1]; count errors strictly above this threshold.
    pub growth_quantile: f64,
    /// Total prototype cap, in 2..=4096. Never exceeded during insertion.
    pub max_nodes: usize,
    /// Per-call distance-pair budget, in 1..=16_777_216.
    pub max_distance_evaluations: usize,
}

impl Default for BatchGasConfig {
    fn default() -> Self {
        Self {
            winner_rate: 0.5,
            neighbour_rate: 0.1,
            epsilon: 1e-4,
            error_discount: 0.5,
            growth_discount: 0.5,
            growth_quantile: 0.85,
            max_nodes: 2000,
            max_distance_evaluations: MAX_PAIRS,
        }
    }
}

impl BatchGasConfig {
    fn validate(self) -> Result<(), GasError> {
        let rates = [
            self.winner_rate,
            self.neighbour_rate,
            self.error_discount,
            self.growth_discount,
            self.growth_quantile,
        ];
        if rates
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self.winner_rate + self.neighbour_rate > 1.0
            || !self.epsilon.is_finite()
            || !(0.0..=1.0).contains(&self.epsilon)
            || self.epsilon == 0.0
            || !(2..=MAX_NODES).contains(&self.max_nodes)
            || !(1..=MAX_PAIRS).contains(&self.max_distance_evaluations)
        {
            return Err(GasError::InvalidConfig);
        }
        Ok(())
    }
}

/// Admission or numerical failure. `step` leaves the entire state unchanged on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GasError {
    /// Non-finite/out-of-range parameters or inconsistent resource limits.
    InvalidConfig,
    /// Empty/oversized points, or a coordinate not finite and bounded by 1e6.
    InvalidPoints,
    /// Graph count, signs, endpoint, self-edge, duplicate or edge cap violation.
    InvalidGraph,
    /// The full requested batch exceeds the distance-evaluation budget.
    BudgetExceeded {
        /// Number of point/prototype pairs required, before any state mutation.
        required: usize,
        /// Configured pair limit.
        limit: usize,
    },
    /// An intermediate result is not finite or escaped the admitted numeric range.
    NumericalFailure,
}
impl fmt::Display for GasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "batch neural gas: {self:?}")
    }
}
impl Error for GasError {}

/// Structural and work receipt. Removal preserves old relative index order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepReceipt {
    /// Exact point/prototype pairs evaluated; squared distances rank the winners.
    pub evaluated_pairs: usize,
    /// Prototype count on entry.
    pub nodes_before: usize,
    /// Prototype count after removal and growth.
    pub nodes_after: usize,
    /// Final undirected edge count.
    pub edges_after: usize,
    /// Number of isolated old prototypes removed.
    pub removed: usize,
    /// Number of inserted midpoint prototypes.
    pub inserted: usize,
    /// Old index -> retained index, or None for a removed prototype.
    pub old_to_new: Vec<Option<u32>>,
    /// Parent indices for each insertion, in insertion order. These refer to the
    /// post-removal graph and may include an earlier insertion from this step.
    pub insertion_parents: Vec<[u32; 2]>,
}

/// Transient algorithm state, using Graph's native connectivity owner.
///
/// No parser, semantic IR, traversal or spatial-index implementation lives here.
#[derive(Clone, Debug)]
pub struct BatchGas {
    positions: Vec<[f64; 3]>,
    graph: SignedGraph,
    errors: Vec<f64>,
    config: BatchGasConfig,
}

impl BatchGas {
    /// Admit 2..=max_nodes finite prototypes and a simple undirected graph.
    /// Only positive signs are accepted; endpoints are normalized and sorted.
    /// The initial edge cap is 262144; duplicate undirected edges are rejected.
    /// Isolated seeds are allowed and are removed if still isolated after a step.
    /// No allocation proportional to an unvalidated count occurs.
    pub fn new(
        positions: &[[f64; 3]],
        graph: &SignedGraph,
        config: BatchGasConfig,
    ) -> Result<Self, GasError> {
        config.validate()?;
        validate_points(positions, config.max_nodes)?;
        if positions.len() < 2
            || graph.n_nodes as usize != positions.len()
            || graph.edges.len() > MAX_EDGES
            || graph.edges.len() != graph.signs.len()
            || graph.signs.iter().any(|s| *s != 1)
        {
            return Err(GasError::InvalidGraph);
        }
        let mut edges = BTreeSet::new();
        for &(a, b) in &graph.edges {
            if a == b
                || a >= graph.n_nodes
                || b >= graph.n_nodes
                || !edges.insert((a.min(b), a.max(b)))
            {
                return Err(GasError::InvalidGraph);
            }
        }
        Ok(Self {
            positions: positions.to_vec(),
            errors: vec![0.0; positions.len()],
            graph: SignedGraph {
                n_nodes: graph.n_nodes,
                edges: edges.into_iter().collect(),
                signs: vec![1; graph.edges.len()],
            },
            config,
        })
    }

    /// Apply one full batch, then remove isolated nodes, discount errors and grow.
    ///
    /// Inputs: 1..=65536 points with finite coordinates of magnitude <= 1e6.
    /// The configured distance budget is checked before calculation/mutation.
    /// Memory is O(nodes + old edges + batch size), never a dense distance matrix.
    /// Errors are atomic: positions, connectivity and accumulated errors all stay
    /// unchanged. Every successful graph has >=2 nodes and no isolated vertices.
    pub fn step(&mut self, points: &[[f64; 3]]) -> Result<StepReceipt, GasError> {
        validate_points(points, MAX_POINTS)?;
        let required = points.len() * self.positions.len(); // Both factors already capped.
        if required > self.config.max_distance_evaluations {
            return Err(GasError::BudgetExceeded {
                required,
                limit: self.config.max_distance_evaluations,
            });
        }
        let (next, receipt) = batch::Batch::collect(self, points).finish(self, required)?;
        *self = next;
        Ok(receipt)
    }

    /// Current prototype positions. Dense indices correspond to graph vertices.
    pub fn positions(&self) -> &[[f64; 3]] {
        &self.positions
    }
    /// Existing native graph representation; no independent traversal facade.
    pub fn graph(&self) -> &SignedGraph {
        &self.graph
    }
    /// Current accumulated, discounted errors, for verification and monitoring.
    pub fn errors(&self) -> &[f64] {
        &self.errors
    }
}

fn validate_points(points: &[[f64; 3]], limit: usize) -> Result<(), GasError> {
    if points.is_empty()
        || points.len() > limit
        || points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
    {
        return Err(GasError::InvalidPoints);
    }
    Ok(())
}
