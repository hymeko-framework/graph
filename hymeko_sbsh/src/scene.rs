//! The SBSH scene — the canonical IR, plus the spatial partition it does not carry.
//!
//! # What is borrowed and what is added
//!
//! [`hymeko::ir::incidence::IncidenceIndex`] already gives the signed-incidence hypergraph:
//! declarations as vertices, hyperarcs as edges, `ArcRec.refs` as the incidence operator, and a
//! `+` / `-` / `~` sign on every incidence. This module adds one thing to it — a **laminar
//! partition of the positioned declarations**, built by an orthtree over their coordinates — and
//! derives the width certificate from the result.
//!
//! Nothing here copies the IR. The scene holds the coordinates it was given and the tree it
//! built; every structural question is answered by asking the borrowed index.
//!
//! # Which declarations take part
//!
//! Exactly those the [`PositionProvider`] positions. An unpositioned declaration is not an error
//! and is not silently placed at the origin — it is simply not in the spatial partition, and
//! [`SbshScene::point_of`] returns `None` for it. A scene where nothing is positioned is an error
//! rather than an empty tree, because an empty root box has no meaning.
//!
//! # The primal graph, and why the width is taken over it
//!
//! Bounded-width solving is stated over the **primal** (Gaifman) graph of the hypergraph: each
//! hyperarc contributes a clique on its members. [`SbshScene::primal_edges`] builds exactly that,
//! restricted to positioned declarations, and [`SbshScene::width`] certifies it. A `k`-member arc
//! contributes `k*(k-1)/2` edges, so a scene with very wide arcs has a large primal graph by
//! construction — that is a property of the model, not of this code, and the certificate is what
//! reports it.

use std::collections::BTreeSet;

use hymeko::common::ids::DeclId;
use hymeko::ir::incidence::IncidenceIndex;
use hymeko_graph::direct_ir::PositionProvider;

use crate::orthtree::{CountSplit, OrthConfig, OrthTree, build_orthtree, point_bbox};
use crate::width::certificate_width_of;

/// A scene could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneError {
    /// No declaration in the IR carried a position, so there is no point set to partition.
    NoPositionedDeclarations,
    /// Positioned declarations disagreed on dimension; a scene has one ambient dimension.
    RaggedDimension { expected: usize, found: usize },
    /// The provider returned a zero-dimensional coordinate.
    ZeroDimension,
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPositionedDeclarations => {
                write!(
                    f,
                    "no declaration carried a position; there is nothing to partition"
                )
            }
            Self::RaggedDimension { expected, found } => {
                write!(
                    f,
                    "ragged position dimension: expected {expected}, found {found}"
                )
            }
            Self::ZeroDimension => write!(f, "positions must have at least one coordinate"),
        }
    }
}

impl std::error::Error for SceneError {}

/// A Spatial Bounded-width Signed Hypergraph over a borrowed IR.
#[derive(Clone, Debug)]
pub struct SbshScene {
    dim: usize,
    /// The positioned declarations, in point order: point `i` belongs to `decls[i]`.
    decls: Vec<DeclId>,
    // No reverse MAP: `decls` is built by enumerating declarations in ascending `DeclId` order,
    // so it is already sorted and `point_index` binary-searches it. A `BTreeMap` here would be a
    // second copy of an ordering the vector already has.
    /// Flat `decls.len() * dim` coordinate buffer.
    points: Vec<f64>,
    tree: OrthTree,
}

impl SbshScene {
    /// Build the spatial partition over every declaration the provider positions.
    ///
    /// `max_per_cell` is the subdivision criterion: a cell holding more than this many
    /// declarations splits, subject to `cfg.max_depth`.
    ///
    /// # Errors
    /// [`SceneError::NoPositionedDeclarations`] when the provider positions nothing;
    /// [`SceneError::RaggedDimension`] when two declarations disagree on dimension.
    ///
    /// # Postconditions
    /// The cell family is laminar; `point_of` is defined exactly on the positioned declarations.
    pub fn build<P: PositionProvider>(
        index: &IncidenceIndex<'_>,
        positions: &P,
        max_per_cell: usize,
        cfg: &OrthConfig,
    ) -> Result<Self, SceneError> {
        let ir = index.ir();
        let mut decls = Vec::new();
        let mut points: Vec<f64> = Vec::new();
        let mut dim = 0usize;

        for raw in 0..ir.decl_nodes.len() {
            let d = DeclId::new(raw);
            let Some(p) = positions.position(d) else {
                continue;
            };
            if dim == 0 {
                dim = p.len();
                if dim == 0 {
                    return Err(SceneError::ZeroDimension);
                }
            } else if p.len() != dim {
                return Err(SceneError::RaggedDimension {
                    expected: dim,
                    found: p.len(),
                });
            }
            decls.push(d);
            points.extend_from_slice(p);
        }
        if decls.is_empty() {
            return Err(SceneError::NoPositionedDeclarations);
        }

        // 5% pad keeps points off the exclusive-high face of the root box.
        let (lo, hi) = point_bbox(&points, dim, 0.05);
        let ids: Vec<u32> = (0..decls.len() as u32).collect();
        let tree = build_orthtree(
            dim,
            &lo,
            &hi,
            &points,
            &ids,
            &CountSplit { max_per_cell },
            cfg,
        );

        Ok(Self {
            dim,
            decls,
            points,
            tree,
        })
    }

    /// Ambient dimension of the scene.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The positioned declarations, in point order.
    pub fn decls(&self) -> &[DeclId] {
        &self.decls
    }

    /// The orthtree carrying the partition.
    pub fn tree(&self) -> &OrthTree {
        &self.tree
    }

    /// The flat coordinate buffer, `decls().len() * dim()`.
    pub fn points(&self) -> &[f64] {
        &self.points
    }

    /// The point index of a declaration, or `None` if it is not in the partition.
    ///
    /// `decls` is sorted by construction, so this is a binary search over it.
    pub fn point_index(&self, decl: DeclId) -> Option<u32> {
        self.decls
            .binary_search_by_key(&decl.raw(), |d| d.raw())
            .ok()
            .map(|i| i as u32)
    }

    /// The coordinate of a declaration, or `None` if it is not in the partition.
    pub fn point_of(&self, decl: DeclId) -> Option<&[f64]> {
        let i = self.point_index(decl)? as usize;
        Some(&self.points[i * self.dim..(i + 1) * self.dim])
    }

    /// The finest patch (leaf cell) containing a declaration.
    pub fn patch_of(&self, decl: DeclId) -> Option<usize> {
        let p = self.point_of(decl)?;
        self.tree.leaf_of(p)
    }

    /// The partition `P`: the declarations of each leaf cell, leaves in creation order. Every
    /// positioned declaration appears in exactly one block.
    pub fn patches(&self) -> Vec<Vec<DeclId>> {
        self.tree
            .leaves()
            .into_iter()
            .map(|c| {
                self.tree.members[c]
                    .iter()
                    .map(|&m| self.decls[m as usize])
                    .collect()
            })
            .collect()
    }

    /// Declarations whose position lies in the query box `[qlo, qhi)`, found by descending only
    /// into overlapping cells.
    ///
    /// # Preconditions
    /// `qlo.len() == qhi.len() == self.dim()`.
    pub fn objects_in_region(&self, qlo: &[f64], qhi: &[f64]) -> Vec<DeclId> {
        debug_assert_eq!(qlo.len(), self.dim);
        debug_assert_eq!(qhi.len(), self.dim);
        self.tree
            .objects_in_region(&self.points, qlo, qhi)
            .into_iter()
            .map(|m| self.decls[m as usize])
            .collect()
    }

    /// The primal (Gaifman) adjacency over positioned declarations, built **in place** from the
    /// IR: every hyperarc makes its positioned members mutually adjacent.
    ///
    /// No edge list is materialised. A `k`-member arc would flatten to `k*(k-1)/2` pairs, and
    /// `direct_ir` names exactly that -- "no pairwise flattening, no materialised star or clique"
    /// -- as what traversing the canonical IR must not do. The elimination game needs adjacency,
    /// so adjacency is what this builds.
    ///
    /// Signs are **not** consulted: width is a property of the unsigned adjacency.
    pub fn primal_adjacency(&self, index: &IncidenceIndex<'_>) -> Vec<BTreeSet<u32>> {
        let ir = index.ir();
        let mut adj = vec![BTreeSet::new(); self.decls.len()];
        let mut members: Vec<u32> = Vec::new();
        for arc in &ir.arcs {
            members.clear();
            for sref in &arc.refs {
                if let Some(i) = self.point_index(sref.target()) {
                    members.push(i);
                }
            }
            members.sort_unstable();
            members.dedup();
            for a in 0..members.len() {
                for b in (a + 1)..members.len() {
                    adj[members[a] as usize].insert(members[b]);
                    adj[members[b] as usize].insert(members[a]);
                }
            }
        }
        adj
    }

    /// The width certificate of this scene: a treewidth upper bound on the primal graph.
    ///
    /// A small width says a bounded-width direct solve over the scene is cheap; a large one says
    /// to route an iterative solve instead. The spatial partition contributes a width-1 laminar
    /// backbone, so whatever this reports above 1 comes from the semantic hyperarcs.
    pub fn width(&self, index: &IncidenceIndex<'_>) -> usize {
        certificate_width_of(&self.primal_adjacency(index))
    }
}
