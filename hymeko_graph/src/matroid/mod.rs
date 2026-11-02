//! Zero-copy **matroids over the canonical `hymeko_core::ir::Ir`** and its sparse `IncidenceIndex`.
//! [`CellLeviMatroid`] also borrows graded cell topology; it shares the same Levi
//! traversal and union-find engine, keeping only per-rank coordinate offsets.
//!
//! This is NOT a second graph model. [`MatroidRef`] is a borrowed, `Copy`, **one-word** facade holding
//! only `&IncidenceIndex`; the `Ir` is reached through `IncidenceIndex::ir()`, so a facade built from a
//! foreign `(Ir, index)` pair is *unrepresentable* — the invalid state cannot be constructed, rather than
//! being rejected at runtime. [`GraphicMatroidWorkspace`] is caller-owned union-find *scratch*
//! (parent / rank / generation / scratch), never canonical topology, and is **shared** by both matroids.
//!
//! The **rank-2 graphic matroid** ([`rank2_spanning_forest`], [`rank2_rank`],
//! [`rank2_weighted_basis`], [`IncrementalRank2`]) has ground = `HyperArcId`s resolving to exactly two
//! node incidences. (The implicit incidence/Levi graphic matroid over `(HyperArcId, slot)` coordinates
//! is added alongside it, reusing the same [`GraphicMatroidWorkspace`].)
//!
//! # Complexity (honest)
//! Facade creation is O(1), zero-allocation, one word. Every rank / basis computation is Θ(ground set) —
//! never O(1). No claim is made that B, Bᵀ, BᵀB, or a complete basis is computed in O(1).
//!
//! # Semantic boundary
//! A graphic basis (rank-2 or Levi) is a **connectivity certificate only**. It preserves neither
//! weighted shortest paths, incidence signs, hyperedge semantics, T/S-norm propagation, B/Bᵀ/BᵀB, nor
//! message passing. It must never replace the canonical hypergraph.

mod binary;
mod cell;
mod latent;
pub use binary::{
    BinaryBasis, BinaryCoordinate, BinaryFrontierBarrier, BinaryFrontierSearch, BinaryFrontierStep,
    BinaryMatroidWorkspace, binary_column, binary_frontier_barrier, binary_frontier_search,
    binary_weighted_basis,
};
pub use latent::{BinaryLatentProposals, binary_latent_proposals};
mod reconstruction;
pub use reconstruction::{BinaryReconstructionOptions, BinaryReconstructionSearch, binary_reconstruction_search};
mod error;
pub use cell::{CellLeviCoord, CellLeviMatroid, CellLeviSession};
mod facade;
mod levi;
mod rank2;
mod workspace;

pub use error::MatroidError;
pub use facade::MatroidRef;
pub use levi::{
    IncrementalLevi, LeviCoord, levi_components, levi_independent, levi_rank, levi_spanning_basis,
};
pub use rank2::{IncrementalRank2, rank2_rank, rank2_spanning_forest, rank2_weighted_basis};
pub use workspace::GraphicMatroidWorkspace;
