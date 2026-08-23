//! # `hymeko_pgraph`
//!
//! P-graph framework over the canonical signed-incidence hypergraph IR
//! defined in [`hymeko_core::ir`]. P-graphs are a well-known
//! process-engineering formalism (Friedler, Tarján, Huang, Fan 1992)
//! for modelling synthesis problems as bipartite Material / Operating-Unit
//! graphs with five structural axioms (A1–A5). This crate embeds the
//! P-graph formalism into the HyMeKo IR so:
//!
//! 1. axiom-checking becomes a query on top of the existing IR
//!    (Phase 1, [`axioms`]);
//! 2. Maximal Structure Generation (MSG) and Solution Structure
//!    Generation (SSG) become constructive operations over the IR
//!    (Phase 2, [`msg`] / [`ssg`]);
//! 3. an entropy-guided Accelerated Branch-and-Bound (ABB) solves
//!    NAS / synthesis problems using the IR's signed-incidence
//!    spectral entropy (Phase 3, [`entropy`] / [`abb`]).
//!
//! ## Phase 0 (this crate, today)
//!
//! - [`schema::PNodeKind`]: bipartite type tag — Material vs.
//!   OperatingUnit.
//! - [`schema::PGraphSchema`]: per-decl `PNodeKind` map plus a directed
//!   edge set, with [`schema::PGraphSchema::is_bipartite_consistent`]
//!   gate enforced at construction.
//! - [`AxiomViolation`] enum (A1–A5) with **canonical Friedler 1992
//!   semantics** restored 2026-05-19; see [`axioms`] module docstring
//!   and `docs/plans/2026-05-19-pgraph-axiom-semantics-fix/`.
//!
//! Subsequent phases (axioms, MSG/SSG, entropy oracle, ABB) populate
//! the modules listed in the plan at
//! `docs/plans/plans_20260429/hymeko_pgraph_plan.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Clippy `-D warnings` staging (collapsible_ifs, tag scans, map iteration style).
#![allow(
    clippy::collapsible_if,
    clippy::for_kv_map,
    clippy::manual_contains,
    clippy::only_used_in_recursion
)]

pub mod abb;
pub mod axiom_extensions;
pub mod axioms;
pub mod builder;
pub mod cli;
pub mod dump;
pub mod lowering;
pub mod meta_resolve;
pub mod msg;
#[cfg(feature = "pgip")]
pub mod pgip_io;
pub mod reachability;
pub mod regime;
pub mod schema;
pub mod ssg;
pub mod ssg_dm;

pub use abb::{AbbOptions, AbbSolution, solve as abb_solve};
pub use axiom_extensions::{ExtensionAxiomBundle, ExtensionAxiomViolation};
pub use axioms::{AxiomBundle, AxiomTrace, AxiomViolation};
pub use builder::{BuilderError, MaterialKind, PgraphBuilder};
pub use cli::{
    CliError, load_pgraph, render_entities, render_graphviz, render_pgraph,
    render_solution, to_dot, to_friedler_dot,
};
pub use dump::{
    DumpAlgorithm, PgraphAnalysisJson, SsgAlgorithm,
    analyze_lowered_with_full_options,
    analyze_lowered_with_regime, analyze_lowered_with_regime_full,
    analyze_lowered_with_regime_topk,
    analyze_source, analyze_source_with_full_options,
    analyze_source_with_options, analyze_source_with_regime,
    analyze_source_with_regime_full,
    analyze_source_with_regime_topk,
};
pub use lowering::{LowerError, LoweredPGraph, lower};
pub use meta_resolve::{MetaResolveError, compile_sources, compile_to_lowered, lower_resolved};
pub use msg::{
    MaximalStructure, MaximalStructureOptions, maximal_structure, maximal_structure_with_options,
    maximal_structure_with_regime,
};
#[cfg(feature = "pgip")]
pub use pgip_io::{PgipError, read_pgip, write_pgip};
pub use regime::{Canonical, Composite, CostDominance, NoExcess, Regime};
pub use schema::{PGraphError, PGraphSchema, PNodeKind};
pub use ssg::{SolutionStructure, SsgOptions, enumerate as ssg_enumerate};
pub use ssg_dm::{
    SsgDmOptions, SsgDmResult, enumerate as ssg_dm_enumerate,
    enumerate_with_options as ssg_dm_enumerate_with_options,
};
