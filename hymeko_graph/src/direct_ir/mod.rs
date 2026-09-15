//! Direct, zero-copy traversal of the canonical HyMeKo IR as a hypergraph.
//!
//! # Why this exists
//!
//! `hymeko_core::ir::Ir` **is** the signed-incidence hypergraph: `ArcRec.refs` is the
//! row orientation of the sparse operator `B`, and the borrowed
//! [`hymeko::ir::incidence::IncidenceIndex`] is its `Bᵀ` access permutation
//! (`node → (HyperArcId, ref_slot)`). This module walks that structure **in place** —
//! no dense view, no `Ir → SignedGraph` adapter, no pairwise flattening, no materialised
//! star or clique, no second hypergraph model. It is the standard HyMeKo graph path.
//!
//! Contrast with the rest of the crate: [`crate::signed_graph::SignedGraph`] and
//! [`crate::traversal::Csr`] are *pairwise* (a hypergraph reaches them only by star
//! expansion); [`crate::hyper_operator::SignedHyperIncidence`] is a *numeric* `f32`
//! operator. Neither is a topological traversal of the canonical `Ir`.
//!
//! # The alternating walk and its complexity contract
//!
//! Search alternates `NodeId → incident arcs → HyperArcId → arc members → NodeId`. Both a
//! `visited_nodes` **and** a `visited_arcs` set are mandatory: a `k`-member hyperarc must be
//! expanded **at most once per search**, otherwise it is re-expanded from each of its `k`
//! members and the walk degrades to `O(k²)`. With both sets the cost is
//! `O(nodes + arcs + incidences)` over the visited component. See [`workspace::TraversalWorkspace`].
//!
//! # Topology semantics (fixed, not configurable)
//!
//! `+`, `−` and `~` incidences all connect; a weight never gates reachability; a duplicate
//! incidence does not create a second visited node (but its multiplicity stays observable
//! through the index); a self-incidence terminates. Signs and weights are never dropped from
//! returned Berge steps — each step resolves back to a full
//! [`hymeko::ir::incidence::IncidenceRef`] via its `(arc, slot)` coordinate.
//!
//! # Identity
//!
//! Public inputs and outputs are canonical typed IDs — [`hymeko::common::ids::NodeId`],
//! [`hymeko::common::ids::HyperArcId`], [`hymeko::common::ids::DeclId`]. No context-free dense
//! `u32` vertex id ever leaves this module. Internally the walk runs in `DeclId` space (the
//! incidence key and ref-target space) and maps to `NodeId` at the boundary.

pub mod batch;
pub mod berge_enum;
pub mod components;
pub mod dual;
pub mod informed;
pub mod s_connect;
pub mod traversal;
pub mod workspace;

use hymeko::common::ids::{DeclId, HyperArcId, NodeId};
use hymeko::ir::ir::{DeclKind, Ir};

pub use batch::{batch_reachable_counts, par_batch_reachable_counts};
pub use berge_enum::{
    BergeCycle, EnumBudget, EnumOutcome, enumerate_berge_cycles, enumerate_berge_paths,
};
pub use components::{ComponentLabeling, connected_components};
pub use dual::{DualReach, dual_reachability};
pub use informed::{
    BergeCost, BergeHeuristic, DirectedConsumerProducer, DirectedProducerConsumer,
    IncidenceWeightCost, InformedSearchError, InformedSearchWorkspace, MetricHeuristic,
    MetricStepCost, OrientedMaterialisationDigest, PathHash, PositionProvider, SearchStats,
    SignCompatible, TransitionPolicy, Undirected, UnitCost, WeightedBergePath, ZeroHeuristic,
    astar_berge_path, dijkstra_berge_path,
};
pub use s_connect::{SComponentLabeling, s_connected_components};
pub use traversal::{BergeIncidence, BergePath, BergeStep, HyperTraversal, NodeSet};
pub use workspace::TraversalWorkspace;

/// A traversal over the canonical `Ir` could not complete. Ids-only, bounded, deterministic;
/// no attacker-controlled source text is ever embedded in an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectIrError {
    /// The start id is not a valid node of this `Ir` (out of range).
    InvalidStart {
        /// The offending node id.
        node: NodeId,
    },
    /// The start hyperarc id is out of range (dual traversal).
    InvalidStartArc {
        /// The offending arc id.
        arc: HyperArcId,
    },
    /// An arc reference targets a declaration that is not a node, where the topological
    /// contract requires one. The record is malformed for hypergraph traversal.
    TargetNotANode {
        /// The arc whose reference is ill-typed.
        arc: HyperArcId,
        /// The reference slot within the arc.
        slot: u32,
        /// The non-node target declaration.
        target: DeclId,
    },
    /// A predecessor chain was internally inconsistent during path reconstruction — a bug
    /// tripwire (a discovered node with no recorded predecessor arc), never expected in
    /// well-formed operation.
    PredecessorInconsistency {
        /// The node whose predecessor was missing.
        node: DeclId,
    },
    /// A bounded enumeration exceeded its declared budget without a natural stop; the caller's
    /// budget decides the cap, so this reports which limit bound the result.
    BudgetExceeded {
        /// Which budget dimension bound the run.
        which: BudgetKind,
    },
}

/// Which bounded-enumeration budget bound a result (for [`DirectIrError::BudgetExceeded`] and
/// [`berge_enum::EnumOutcome`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetKind {
    /// The maximum alternating depth (number of hyperedges) was reached.
    Depth,
    /// The maximum number of emitted results was reached.
    Results,
    /// The frontier / expansion budget was reached.
    Expansions,
}

impl core::fmt::Display for DirectIrError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidStart { node } => write!(f, "invalid start node id {}", node.raw()),
            Self::InvalidStartArc { arc } => write!(f, "invalid start arc id {}", arc.raw()),
            Self::TargetNotANode { arc, slot, target } => write!(
                f,
                "arc {} slot {slot} references non-node declaration {}",
                arc.raw(),
                target.raw()
            ),
            Self::PredecessorInconsistency { node } => {
                write!(
                    f,
                    "predecessor chain inconsistent at declaration {}",
                    node.raw()
                )
            }
            Self::BudgetExceeded { which } => write!(f, "enumeration budget exceeded: {which:?}"),
        }
    }
}

impl std::error::Error for DirectIrError {}

/// Resolve a start [`NodeId`] to its backing [`DeclId`], validating range.
///
/// # Errors
/// [`DirectIrError::InvalidStart`] if `start` is out of range for `ir.nodes`.
#[inline]
pub(crate) fn node_to_decl(ir: &Ir, start: NodeId) -> Result<DeclId, DirectIrError> {
    ir.nodes
        .get(start.raw())
        .map(|rec| rec.decl)
        .ok_or(DirectIrError::InvalidStart { node: start })
}

/// Map a node [`DeclId`] back to its [`NodeId`]. The traversal only ever marks node-kind decls,
/// so this is total on discovered decls; a `None` here would be an internal invariant break.
#[inline]
pub(crate) fn decl_to_node(ir: &Ir, decl: DeclId) -> Option<NodeId> {
    ir.as_node(decl)
}

/// True iff `decl` is a node-kind declaration.
#[inline]
pub(crate) fn is_node(ir: &Ir, decl: DeclId) -> bool {
    matches!(ir.decl_kind(decl), DeclKind::Node)
}
