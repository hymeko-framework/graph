//! Stage P-engine Slice 1 (2026-05-19): programmatic P-graph
//! construction without a `.hymeko` source file or `.pgip` SQLite
//! file. Lets users build a P-graph fluently from code — necessary
//! for in-process Python use cases (HSiKAN architecture choice,
//! Gömb cortical-circuit choice, PyTorch `nn.Module` layers).
//!
//! Companion plan: [`docs/plans/2026-05-19-pgraph-engine/`].
//!
//! # Example
//!
//! ```
//! use hymeko_pgraph::builder::{PgraphBuilder, MaterialKind};
//! use hymeko_pgraph::abb::{AbbOptions, solve_with_options};
//! use hymeko_pgraph::maximal_structure;
//!
//! let mut b = PgraphBuilder::new();
//! b.add_material("toluene", MaterialKind::Raw).unwrap();
//! b.add_material("h2",      MaterialKind::Raw).unwrap();
//! b.add_material("mix",     MaterialKind::Intermediate).unwrap();
//! b.add_material("benzene", MaterialKind::Product).unwrap();
//! b.add_material("methane", MaterialKind::Intermediate).unwrap();
//! b.add_unit("Mixer",   &["toluene", "h2"], &["mix"],            100.0).unwrap();
//! b.add_unit("Reactor", &["mix"],            &["benzene", "methane"], 250.0).unwrap();
//! b.add_unit("Disposal",&["methane"],        &[],                50.0).unwrap();
//! b.add_unit("Direct",  &["toluene", "h2"], &["benzene"],         800.0).unwrap();
//!
//! let graph = b.build().unwrap();
//! let msg = maximal_structure(&graph);
//! let opts = AbbOptions::default();
//! let sol = solve_with_options(&graph, &msg, opts).unwrap();
//! // Canonical optimum: Mixer + Reactor = 350 (Methane vented; the
//! // Disposal sink reaches no product and is not in the maximal structure).
//! assert!((sol.cost - 350.0).abs() < 1e-9);
//! ```

use std::collections::{BTreeMap, BTreeSet};

use hymeko::common::ids::{DeclId, EdgeId};

use crate::lowering::LoweredPGraph;
use crate::schema::{PGraphSchema, PNodeKind};

/// Material kind, mirrors the `.pgip` schema's `materialTypes` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialKind {
    /// Type 0 — neither raw nor demanded.
    Intermediate,
    /// Type 1 — feedstock; free to use as ABB input.
    Raw,
    /// Type 2 — required product; ABB must reach it.
    Product,
}

/// A unit's contribution along a named cost dimension. Multiple
/// dimensions can coexist (Stage P-mo multi-objective ABB).
#[derive(Debug, Clone)]
struct UnitDef {
    inputs: Vec<String>,
    outputs: Vec<String>,
    cost: f64,
    multi_cost: BTreeMap<String, f64>,
}

/// Errors raised by [`PgraphBuilder::build`].
#[derive(Debug, thiserror::Error)]
pub enum BuilderError {
    /// `build` called before any materials were added.
    #[error("P-graph is empty: no materials declared")]
    NoMaterials,
    /// `build` called before any units were added. ABB needs at least
    /// one unit to be meaningful.
    #[error("P-graph has no operating units")]
    NoUnits,
    /// A duplicate material or unit name was added.
    #[error("duplicate name: {0:?}")]
    DuplicateName(String),
    /// A unit referenced an unknown material name.
    #[error("unit {unit:?} references unknown material {target:?}")]
    UnknownMaterial {
        /// The unit's name (already added to the builder).
        unit: String,
        /// The unknown material name referenced by the unit.
        target: String,
    },
    /// Constructing the bipartite schema failed (overlap between
    /// material and unit roles).
    #[error("schema construction failed: {0}")]
    Schema(String),
}

/// Fluent builder for a [`LoweredPGraph`] from code.
///
/// Add materials with [`Self::add_material`], add units with
/// [`Self::add_unit`] or [`Self::add_unit_multi_cost`], then call
/// [`Self::build`] to produce the lowered IR. The output is
/// indistinguishable from one produced by `parse_description` +
/// `lower` on equivalent `.hymeko` source.
#[derive(Debug, Default, Clone)]
pub struct PgraphBuilder {
    materials: BTreeMap<String, MaterialKind>,
    units: BTreeMap<String, UnitDef>,
}

impl PgraphBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a material with the given role.
    pub fn add_material(
        &mut self,
        name: &str,
        kind: MaterialKind,
    ) -> Result<&mut Self, BuilderError> {
        if self.materials.contains_key(name) || self.units.contains_key(name) {
            return Err(BuilderError::DuplicateName(name.to_string()));
        }
        self.materials.insert(name.to_string(), kind);
        Ok(self)
    }

    /// Declare an operating unit with a scalar cost (Friedler 1992 form).
    pub fn add_unit(
        &mut self,
        name: &str,
        inputs: &[&str],
        outputs: &[&str],
        cost: f64,
    ) -> Result<&mut Self, BuilderError> {
        self.add_unit_multi_cost(name, inputs, outputs, cost, BTreeMap::new())
    }

    /// Declare an operating unit with a scalar cost AND a multi-cost
    /// vector for Stage P-mo weighted-sum ABB. Cost-dimension names
    /// will be alphabetised at [`Self::build`] time.
    pub fn add_unit_multi_cost(
        &mut self,
        name: &str,
        inputs: &[&str],
        outputs: &[&str],
        cost: f64,
        multi_cost: BTreeMap<String, f64>,
    ) -> Result<&mut Self, BuilderError> {
        if self.materials.contains_key(name) || self.units.contains_key(name) {
            return Err(BuilderError::DuplicateName(name.to_string()));
        }
        self.units.insert(
            name.to_string(),
            UnitDef {
                inputs: inputs.iter().map(|s| s.to_string()).collect(),
                outputs: outputs.iter().map(|s| s.to_string()).collect(),
                cost,
                multi_cost,
            },
        );
        Ok(self)
    }

    /// Convert the builder into a [`LoweredPGraph`].
    pub fn build(self) -> Result<LoweredPGraph, BuilderError> {
        if self.materials.is_empty() {
            return Err(BuilderError::NoMaterials);
        }
        if self.units.is_empty() {
            return Err(BuilderError::NoUnits);
        }

        // ─── 1. Assign DeclIds to materials and units. ─────────────
        let mut name_to_decl: BTreeMap<String, DeclId> = BTreeMap::new();
        let mut decl_to_name: BTreeMap<DeclId, String> = BTreeMap::new();
        let mut kinds: BTreeMap<DeclId, PNodeKind> = BTreeMap::new();
        let mut raws: BTreeSet<DeclId> = BTreeSet::new();
        let mut products: BTreeSet<DeclId> = BTreeSet::new();
        let mut materials_set: BTreeSet<DeclId> = BTreeSet::new();
        let mut units_set: BTreeSet<DeclId> = BTreeSet::new();
        let mut next_decl: usize = 0;

        for (name, kind) in &self.materials {
            let d = DeclId::new(next_decl);
            next_decl += 1;
            name_to_decl.insert(name.clone(), d);
            decl_to_name.insert(d, name.clone());
            kinds.insert(d, PNodeKind::Material);
            materials_set.insert(d);
            match kind {
                MaterialKind::Raw => {
                    raws.insert(d);
                }
                MaterialKind::Product => {
                    products.insert(d);
                }
                MaterialKind::Intermediate => {}
            }
        }
        for name in self.units.keys() {
            let d = DeclId::new(next_decl);
            next_decl += 1;
            name_to_decl.insert(name.clone(), d);
            decl_to_name.insert(d, name.clone());
            kinds.insert(d, PNodeKind::OperatingUnit);
            units_set.insert(d);
        }

        // ─── 2. Build the directed edge set (signed incidence). ──────
        //       Per-unit input/output sets are derived from these edges
        //       by the schema; they are not stored on the graph.
        let mut edges: BTreeMap<EdgeId, (DeclId, DeclId)> = BTreeMap::new();
        let mut costs: BTreeMap<DeclId, f64> = BTreeMap::new();
        let mut per_unit_dim: BTreeMap<DeclId, BTreeMap<String, f64>> = BTreeMap::new();
        let mut dim_set: BTreeSet<String> = BTreeSet::new();
        let mut next_edge: usize = 0;

        for (uname, def) in &self.units {
            let u_decl = name_to_decl[uname];
            costs.insert(u_decl, def.cost);
            for inp in &def.inputs {
                let m = *self
                    .materials
                    .get(inp)
                    .map(|_| name_to_decl.get(inp).unwrap())
                    .ok_or_else(|| BuilderError::UnknownMaterial {
                        unit: uname.clone(),
                        target: inp.clone(),
                    })?;
                edges.insert(EdgeId::new(next_edge), (m, u_decl));
                next_edge += 1;
            }
            for out in &def.outputs {
                let m = *self
                    .materials
                    .get(out)
                    .map(|_| name_to_decl.get(out).unwrap())
                    .ok_or_else(|| BuilderError::UnknownMaterial {
                        unit: uname.clone(),
                        target: out.clone(),
                    })?;
                edges.insert(EdgeId::new(next_edge), (u_decl, m));
                next_edge += 1;
            }
            if !def.multi_cost.is_empty() {
                let mut dims = BTreeMap::new();
                for (dim, v) in &def.multi_cost {
                    dims.insert(dim.clone(), *v);
                    dim_set.insert(dim.clone());
                }
                per_unit_dim.insert(u_decl, dims);
            } else {
                per_unit_dim.insert(u_decl, BTreeMap::new());
            }
        }

        // ─── 3. Construct schema (bipartite invariant). ────────────
        let schema = PGraphSchema::try_new(kinds, edges)
            .map_err(|e| BuilderError::Schema(format!("{e}")))?;

        // ─── 4. Alphabetise dim names + build cost_vectors. ────────
        let cost_dimensions: Vec<String> = dim_set.iter().cloned().collect();
        let cost_vectors: BTreeMap<DeclId, Vec<f64>> = if cost_dimensions.is_empty() {
            BTreeMap::new()
        } else {
            per_unit_dim
                .into_iter()
                .map(|(u, dims)| {
                    let v: Vec<f64> = cost_dimensions
                        .iter()
                        .map(|d| dims.get(d).copied().unwrap_or(0.0))
                        .collect();
                    (u, v)
                })
                .collect()
        };

        Ok(LoweredPGraph {
            schema,
            name_to_decl,
            decl_to_name,
            raws,
            products,
            materials: materials_set,
            units: units_set,
            costs,
            cost_dimensions,
            cost_vectors,
        })
    }
}
