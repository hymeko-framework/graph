//! # `hymeko_graph` — generic signed-graph cycle / walk enumeration
//! with axiomatic pruning
//!
//! This crate lifts the Rust DFS, color-coding, reservoir and
//! open-walk enumerators that currently live inside `hymeko_py`
//! into a stand-alone library so they can be used outside the
//! Python / KAN pipeline.
//!
//! ## Why a separate crate
//!
//! The cycle / walk enumeration algorithms are valuable on their
//! own — `hymeko.enumerate_k_cycles_rs` already runs $20$–$50\times$
//! faster than serial Python on Slashdot-class graphs, and the
//! open-walk variant lands canonical-form-correct output bit-for-bit.
//! There is no reason to gate that on the rest of the HyMeKo
//! framework being installed.  The refactor also separates concerns:
//!
//! - `hymeko_graph` :  *generic* signed-graph algorithms —
//!   [`SignedGraph`], CSR construction, [`traversal`]'s DFS and
//!   BFS, k-cycle / k-walk enumeration, the
//!   [`pruner::CyclePruner`] trait below. Nothing domain-specific.
//! - `hymeko_pgraph` :  the P-graph reading — schema, the A1–A5
//!   axiom checker, and `FriedlerAxiomPruner`, which implements
//!   the trait below.
//!
//! The dependency runs one way, `hymeko_pgraph → hymeko_graph`,
//! and nothing here knows what a Material or an Operating Unit is.
//!
//! ## The pruning seam
//!
//! The pivot point of this crate is the [`pruner::CyclePruner`] trait:
//! during DFS, the enumerator consults a pruner at every partial
//! path to decide whether to continue, and at every closed cycle
//! to decide whether to emit. The stock pruners are the ones that
//! need no domain vocabulary:
//!
//! - [`balance::CartwrightHararyPruner`] — emit only balanced
//!   ($\prod s = +1$) cycles, or only unbalanced.
//! - [`balance::DavisWeakBalancePruner`] — exclude all-negative
//!   triangles (Davis 1967).
//! - [`balance::BipartiteOnlyPruner`] — for bipartite graphs (e.g.
//!   star-expansion of a hypergraph), prune odd-length cycles
//!   before they materialise.
//!
//! Anything that needs to know what the vertices *mean* belongs on
//! the other side of the trait. `hymeko_pgraph::FriedlerAxiomPruner`
//! is the worked example: it rejects M-M and O-O steps during the
//! DFS (A0), and requires a product node at emit time (A1).
//!
//! In every case the pruner kicks in *during* the DFS, not after
//! materialisation, so the enumeration cost is proportional to
//! the size of the *feasible* cycle set, not the full set.  On
//! P-graph-shaped instances this can be orders of magnitude
//! faster than enumerate-then-filter.
//!
//! ## The pairwise algorithms vs. the canonical-hypergraph path
//!
//! The [`balance`], [`cycle_enum`], [`topk_cycles`], [`traversal`], [`spine`] and
//! [`signed_tuples`] families are *pairwise* signed graphs — [`SignedGraph`]
//! is `edges: Vec<(u32, u32)>`. A hypergraph reaches those algorithms only by
//! being star-expanded first, which is why [`balance::BipartiteOnlyPruner`]
//! exists. They build on `rayon` + `dashmap` alone.
//!
//! [`direct_ir`], added 2026-09-01, is the hypergraph path: it traverses the
//! canonical `hymeko_core::ir::Ir` **in place** through the borrowed
//! `IncidenceIndex` — reachability, Berge shortest paths, connected components,
//! dual reachability (and bounded Berge enumeration, s-connected components and
//! batch reachability) — with no star/clique/CSR/`SignedGraph` materialisation
//! and canonical `NodeId`/`HyperArcId` output. This is the standard HyMeKo graph
//! path, and the reason the crate now depends on `hymeko_core`.
//!
//! `hotaru::hypergraph` is a separate, dependency-free general hypergraph
//! library over its **own** CSR model (reached from the IR by an optional copy);
//! it carries the wider suite — Levi / 2-section / s-line, transversals,
//! matchings, the acyclicity hierarchy, s-betweenness. `direct_ir` deliberately
//! does not reproduce that suite: it is the zero-copy canonical-IR core, upstream
//! of Nagare's (dormant) Berge traversal and parallel to Hotaru. The overlap and
//! a consolidation path are recorded in the 2026-09-01 direct-Ir plan/report.
//!
//! ## Status
//!
//! Complete. The cycle enumeration body an earlier note placed in
//! `hymeko_py/src/cycles.rs` now lives here, in [`cycle_enum`] and
//! [`topk_cycles`], taking the `&dyn CyclePruner` that note
//! anticipated.

// SIMD spine kernel (spine::fir_one_cycle_avx2) needs unsafe for
// `_mm256_*` intrinsics; the rest of the crate is `deny(unsafe_code)`
// and the unsafe is scoped via `#[allow]` only on that function.
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod balance;
pub mod color_coding;
pub mod community;
pub mod cycle_enum;
pub mod cycle_sampler;
pub mod direct_ir;
pub mod hyper_operator;
pub mod incidence;
pub mod path_closure;
pub mod pruner;
pub mod quadtree;
pub mod rand_lcg;
pub mod signed_graph;
pub mod signed_tuples;
pub mod spine;
pub mod topk_cycles;
pub mod topk_walks;
pub mod traversal;
pub mod traversal_heuristic;
pub mod unsigned_cycles;
pub mod vertex_filter;
pub mod walks_unsigned;

pub use balance::{BipartiteOnlyPruner, CartwrightHararyPruner, DavisWeakBalancePruner};
pub use cycle_enum::{enumerate_simple_cycles, enumerate_simple_cycles_noprune};
pub use direct_ir::{
    BergePath, BergeStep, ComponentLabeling, DirectIrError, DualReach, HyperTraversal, NodeSet,
    TraversalWorkspace, connected_components, dual_reachability,
};
pub use hyper_operator::{HyperIncidenceError, SignedHyperIncidence};
pub use incidence::{
    BuildOpts as IncidenceBuildOpts, IncidenceCoo, IncidenceOutput, IncidenceResult,
    build_edge_incidence, build_edge_incidence_vertex_adj,
};
pub use pruner::{
    CompositePruner, CountingPruner, CyclePruner, NoOpPruner, PrunerDecision, PrunerStats,
    StatsSnapshot,
};
pub use quadtree::{QuadtreeAnchors, QuadtreeConfig, build_quadtree, forman_kappa_4conn};
pub use signed_graph::{Sign, SignedGraph};
pub use signed_tuples::{
    EnumerationExecution, EnumerationRequest, MultiEdgeSignResolution, SignedTupleFamily,
    TupleEnumeration, enumerate_signed_tuples,
};
pub use spine::{
    CliffordFIR, FirBackward, GradRequest, SignedCycleFIR, clifford_fir_backward,
    clifford_fir_backward_with, clifford_fir_forward, fir_cycle_forward, fir_cycle_scatter_mean,
};
pub use topk_cycles::{
    EntropyGainScorer, HybridScorer, InverseDegreeScorer, TopKBuilder, TopKCycle, TopKCyclesBatch,
    UniformityHeuristic, UniformityState, degree_adaptive_m_v, enumerate_top_k_cycles,
    enumerate_top_k_cycles_bb, enumerate_top_k_cycles_noprune, enumerate_top_k_cycles_par,
    enumerate_top_k_cycles_par_batched, enumerate_top_k_cycles_par_bb,
    enumerate_top_k_cycles_par_bb_batched, enumerate_top_k_cycles_par_entropy,
    enumerate_top_k_cycles_par_entropy_batched, enumerate_top_k_cycles_par_noprune,
    enumerate_top_k_per_vertex_cycles, enumerate_top_k_per_vertex_cycles_adaptive,
    enumerate_top_k_per_vertex_cycles_noprune, enumerate_top_k_per_vertex_cycles_par,
    enumerate_top_k_per_vertex_cycles_par_adaptive,
    enumerate_top_k_per_vertex_cycles_par_adaptive_batched,
    enumerate_top_k_per_vertex_cycles_par_adaptive_starting,
    enumerate_top_k_per_vertex_cycles_par_adaptive_starting_batched,
    enumerate_top_k_per_vertex_cycles_par_adaptive_starting_bb_batched,
    enumerate_top_k_per_vertex_cycles_par_adaptive_starting_bb_global_batched,
    enumerate_top_k_per_vertex_cycles_par_batched, enumerate_top_k_per_vertex_cycles_par_noprune,
};
pub use topk_cycles::{WeightedSumScorer, tiered_m_v_by_degree};
pub use topk_walks::{
    TopKWalk, TopKWalksBatch, enumerate_top_k_walks, enumerate_top_k_walks_batch,
};
pub use traversal::{
    BFS_UNREACHED, BfsScratch, Csr, DfsScratch, bfs_distances, bidirectional_bfs, bs_clear, bs_get,
    bs_set, bs_words, bs_zero, count_connected_components, dfs_visit, dfs_visit_noprune,
    dfs_visit_pruned,
};
pub use traversal_heuristic::{
    AstarScratch, DegreeHeuristic, Heuristic, ZeroHeuristic, astar, best_first_dfs,
    enumerate_cycles_ordered, enumerate_cycles_ordered_noprune,
};
pub use vertex_filter::{AndFilter, DegreeFilter, NoFilter, TriangleFilter, VertexFilter};
