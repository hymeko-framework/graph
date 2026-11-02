//! The typed failure of a matroid operation, shared by both matroids. A hyperarc is never silently
//! misread as a pairwise edge, and a non-node target is never silently skipped.

use std::fmt;

use hymeko::common::ids::HyperArcId;

/// Typed failure of a matroid operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatroidError {
    /// Invalid paired-column dimensions or bounded frontier search controls.
    InvalidFrontierRequest,
    /// Invalid prefix, realizable columns or bounds for required-column search.
    InvalidReconstructionRequest,
    /// Invalid or numerically refused bounded latent proposal request.
    InvalidLatentRequest,
    /// A binary column has a coefficient that is not an absent/unit or 0/1 scalar.
    NonBinaryIncidence {
        /// Offending column.
        arc: HyperArcId,
        /// Offending reference position.
        slot: u32,
    },
    /// A selected column occurs more than once in the ground set.
    DuplicateGroundElement {
        /// Repeated canonical column.
        arc: HyperArcId,
    },
    /// Sparse binary elimination exceeded an explicit caller budget.
    BinaryWorkspaceBudget,
    /// A graded-cell Levi coordinate is outside its borrowed topology.
    CellIncidenceOutOfRange {
        /// Upper-cell rank, starting at one.
        rank: usize,
        /// Upper-cell position within its rank.
        cell: usize,
        /// Incidence position within the cell.
        slot: usize,
    },
    /// A canonical hyperarc ID is outside the bound snapshot's arc table.
    UnknownArc {
        /// The out-of-range hyperarc ID.
        arc: HyperArcId,
    },
    /// An incremental session was offered an element from a different canonical `Ir` snapshot.
    ForeignSnapshot,
    /// A rank-2 incremental session's declared node universe differs from the offered snapshot.
    NodeUniverseMismatch {
        /// Node count supplied when the session began.
        expected: usize,
        /// Node count in the offered canonical snapshot.
        actual: usize,
    },
    /// A hyperarc does not resolve to exactly two node incidences (rank-2 only; e.g. a multiway edge).
    WrongCardinality {
        /// The offending hyperarc.
        arc: HyperArcId,
        /// How many node incidences it actually has (≠ 2).
        node_incidences: usize,
    },
    /// A hyperarc reference targets a declaration that is not a valid node.
    DanglingTarget {
        /// The offending hyperarc.
        arc: HyperArcId,
        /// The reference slot within the hyperarc whose target is not a node.
        slot: u32,
    },
    /// A hyperarc's two node incidences are the same node (self-loop) — rejected by the rank-2 policy.
    SelfLoop {
        /// The offending hyperarc.
        arc: HyperArcId,
    },
    /// A weighted operation encountered a non-finite weight.
    NonFiniteWeight {
        /// The offending hyperarc.
        arc: HyperArcId,
    },
    /// The internal `u32` DSU coordinate space is too small for the vertex universe (`N` for rank-2,
    /// `N + A` for Levi).
    Overflow,
}

impl fmt::Display for MatroidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFrontierRequest => write!(f, "invalid binary frontier request"),
            Self::InvalidReconstructionRequest => {
                write!(f, "invalid bounded binary reconstruction request")
            }
            Self::InvalidLatentRequest => {
                write!(f, "invalid bounded binary latent proposal request")
            }
            Self::NonBinaryIncidence { arc, slot } => write!(
                f,
                "hyperarc {} slot {slot} is not a binary coefficient",
                arc.raw()
            ),
            Self::DuplicateGroundElement { arc } => {
                write!(f, "duplicate binary ground element {}", arc.raw())
            }
            Self::BinaryWorkspaceBudget => write!(f, "binary matroid workspace budget exceeded"),
            Self::CellIncidenceOutOfRange { rank, cell, slot } => write!(
                f,
                "invalid cell incidence at rank {rank}, cell {cell}, slot {slot}"
            ),
            Self::UnknownArc { arc } => {
                write!(f, "unknown hyperarc {} in the bound snapshot", arc.raw())
            }
            Self::ForeignSnapshot => {
                write!(
                    f,
                    "incremental matroid session cannot mix canonical Ir snapshots"
                )
            }
            Self::NodeUniverseMismatch { expected, actual } => write!(
                f,
                "incremental rank-2 session expects {expected} nodes, but the offered snapshot has {actual}"
            ),
            Self::WrongCardinality {
                arc,
                node_incidences,
            } => write!(
                f,
                "hyperarc {} has {node_incidences} node incidences, not the 2 a rank-2 edge requires",
                arc.raw()
            ),
            Self::DanglingTarget { arc, slot } => {
                write!(
                    f,
                    "hyperarc {} slot {slot} targets a non-node declaration",
                    arc.raw()
                )
            }
            Self::SelfLoop { arc } => write!(f, "hyperarc {} is a self-loop", arc.raw()),
            Self::NonFiniteWeight { arc } => {
                write!(f, "hyperarc {} has a non-finite weight", arc.raw())
            }
            Self::Overflow => write!(f, "vertex universe exceeds the u32 DSU coordinate"),
        }
    }
}
impl std::error::Error for MatroidError {}
