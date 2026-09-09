//! # `hymeko_sbsh` — Spatial Bounded-width Signed Hypergraph
//!
//! ## What this crate adds, and what it deliberately does not
//!
//! The canonical [`hymeko::ir::ir::Ir`] **is already a signed-incidence hypergraph**: its
//! declarations are the vertices, its hyperarcs the edges, `ArcRec.refs` the incidence operator
//! `B`, and each incidence carries a `+` / `-` / `~` sign. In the algebraic vocabulary that makes
//! the IR a `(V, E, B, sigma)` — four of the five parts of a signed patch hypergraph.
//!
//! The missing part is `P`: a **partition of the vertices into patches**. SBSH supplies exactly
//! that, and nothing else:
//!
//! ```text
//!     Ir            (V, E, B, sigma)      the canonical signed-incidence hypergraph
//!     + SBSH        P                     a laminar spatial partition, from an N-D orthtree
//!     ------------------------------------------------------------------------------
//!     = the full structure, plus a cheap number that says whether solving over it is easy
//! ```
//!
//! The partition is **laminar** by construction: orthtree cells at different depths nest, and
//! cells at the same depth are disjoint. A laminar family is an alpha-acyclic hypertree of width
//! 1, so the spatial side contributes a width-1 backbone; all the difficulty lives in the
//! semantic hyperarcs grafted onto it. [`width::certificate_width`] is the one structural number
//! that says how much.
//!
//! ## Ownership, and why this is a separate crate
//!
//! Per `CORE.YAML`, `hymeko_graph` is the canonical owner of generic signed-graph and hypergraph
//! algorithms -- incidence, cycle and walk enumeration, CSR primitives -- and **no second copy may
//! exist under any name**. This crate therefore:
//!
//! - **reuses** [`hymeko_graph::direct_ir::PositionProvider`] as the IR-to-coordinate seam rather
//!   than declaring a second one;
//! - **reuses** `hymeko_graph::direct_ir` for anything that walks the IR;
//! - **does not** contain a hypergraph, an incidence structure, or a traversal.
//!
//! What it does contain is spatial: an N-dimensional orthtree over *points*, and a treewidth
//! upper bound. Note that [`hymeko_graph::quadtree`] is a **different concern** and is not
//! duplicated here -- it subdivides a raster by image variance and Forman-Ricci curvature, over
//! pixels. This crate subdivides a *point set* in `R^k`. Same shape of recursion, different input
//! and different criterion, which is why the split rule is a trait
//! ([`orthtree::SplitRule`]) rather than a mode flag: a caller that wants energy-driven
//! subdivision implements it, and no raster rule is restated here.
//!
//! ## Dimension
//!
//! One code path covers every dimension. A **quadtree** is `dim = 2` (`2^2 = 4` orthants); an
//! **octree** is `dim = 3` (`2^3 = 8`); higher dimensions work unchanged and are the reason the
//! type is not named after either. Coordinates are `f64` throughout, matching
//! [`hymeko_graph::direct_ir::PositionProvider`], so binding a scene to the IR needs no lossy
//! conversion at the boundary.

pub mod orthtree;
pub mod scene;
pub mod width;

pub use orthtree::{CountSplit, OrthConfig, OrthTree, SplitRule, build_orthtree, point_bbox};
pub use scene::{SbshScene, SceneError};
pub use width::{ElimCost, certificate_width, elimination_cost};
