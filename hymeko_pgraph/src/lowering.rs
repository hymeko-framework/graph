//! Lower a parsed HyMeKo [`Description`] into a [`PGraphSchema`] +
//! P-graph side-tables (raws, products, costs).
//!
//! ## Encoding contract — idiomatic HyMeKo
//!
//! P-graphs map onto the canonical signed-incidence hypergraph IR
//! by reading:
//!
//! - **HyMeKo nodes** (no `@` prefix) tagged `<material>`  →  M-set.
//! - **HyMeKo hyperedges** (`@`-prefix) tagged `<unit>`    →  O-set.
//!
//! This is the natural mapping: a P-graph operating unit *is* a
//! hyperedge connecting input materials to output materials, so it
//! lives on the edge side of the IR. A material is a passive
//! resource type, so it lives on the node side.
//!
//! ### Sub-tags on materials
//!
//! - `<raw>`     — material is in the raw-material set $R$.
//! - `<product>` — material is in the required-product set $P$.
//!
//! ### Unit body
//!
//! An edge body holds **one** signed [`HyperArc`] whose signs encode
//! the unit's I/O signature:
//!
//! - `-Material` ⇒ Material is consumed (directed schema edge
//!   `Material → unit`).
//! - `+Material` ⇒ Material is produced (directed schema edge
//!   `unit → Material`).
//!
//! ### Cost
//!
//! The unit's scalar cost is the edge's numeric value, e.g.
//! `@Mixer <unit> 100 { (-A, -B, +C); }`. Default is $1.0$.
//!
//! For backwards compatibility a `cost N;` child node is also
//! recognised inside an edge body.

use std::collections::{BTreeMap, BTreeSet};

use hymeko::common::ids::{DeclId, EdgeId};
use parser::ast::{Anno, Description, EdgeDecl, HyperItem, NodeDecl, SignedRef, Value};
use thiserror::Error;

use crate::schema::{PGraphError, PGraphSchema, PNodeKind};

/// Errors raised while lowering a parsed [`Description`] into a P-graph.
#[derive(Debug, Error)]
pub enum LowerError {
    /// A declaration carried a tag set that doesn't classify it as
    /// either a Material or an Operating Unit.
    #[error("decl `{name}` has ambiguous P-graph role: {detail}")]
    AmbiguousRole {
        /// Offending name.
        name: String,
        /// Free-form detail.
        detail: String,
    },
    /// A unit body referenced a material name that is not declared.
    #[error("unit `{unit}` references unknown material `{target}`")]
    UnknownMaterial {
        /// Unit that contained the bad reference.
        unit: String,
        /// Referenced name.
        target: String,
    },
    /// A unit's hyperarc used a `~` (neutral) sign — P-graph edges must
    /// be signed (consume `-` or produce `+`).
    #[error("unit `{unit}` has a neutral (~) reference; P-graph edges must be + or -")]
    NeutralRef {
        /// Offending unit name.
        unit: String,
    },
    /// The lowered schema failed the bipartite invariant.
    #[error(transparent)]
    SchemaError(#[from] PGraphError),
}

/// Lowered P-graph: schema plus the side-tables MSG/SSG/ABB need.
#[derive(Debug, Clone)]
pub struct LoweredPGraph {
    /// Bipartite Material/Operating-Unit overlay.
    pub schema: PGraphSchema,
    /// Mapping from source name to assigned [`DeclId`].
    pub name_to_decl: BTreeMap<String, DeclId>,
    /// Reverse mapping for diagnostics.
    pub decl_to_name: BTreeMap<DeclId, String>,
    /// Materials in the raw-material set $R$.
    pub raws: BTreeSet<DeclId>,
    /// Materials in the required-product set $P$.
    pub products: BTreeSet<DeclId>,
    /// All M-nodes (mirrors `schema.m_nodes()` for convenience).
    pub materials: BTreeSet<DeclId>,
    /// All O-nodes (mirrors `schema.o_nodes()`).
    pub units: BTreeSet<DeclId>,
    /// Per-unit cost (default 1.0). This is the *scalar* cost used
    /// by the single-criterion ABB path; it is the canonical
    /// fallback when [`Self::cost_vectors`] is empty or when
    /// [`crate::abb::AbbOptions::cost_weights`] is `None`.
    pub costs: BTreeMap<DeclId, f64>,
    /// Stage P-mo (2026-05-19): names of the multi-criterion cost
    /// dimensions in canonical (alphabetised) order. Empty when the
    /// source `.hymeko` carries no tagged `cost <name> N;` children.
    pub cost_dimensions: Vec<String>,
    /// Per-unit cost vector aligned with [`Self::cost_dimensions`].
    /// A unit missing a dimension defaults to `0.0` for that
    /// dimension. Empty when no tagged cost children are present
    /// (scalar-only path).
    pub cost_vectors: BTreeMap<DeclId, Vec<f64>>,
}

impl LoweredPGraph {
    /// Materials *consumed* by unit `u` — the `-` refs of its hyperarc,
    /// i.e. the predecessors of `u` in the signed-incidence schema
    /// (edges `m → u`).
    ///
    /// # Postconditions
    /// Equal to `schema.predecessors(u)`; a unit with no inputs yields
    /// the empty set. This is a *query* over the single source of truth
    /// (the directed edge set), not a stored side table.
    pub fn inputs(&self, u: DeclId) -> &BTreeSet<DeclId> {
        self.schema.predecessors(u)
    }

    /// Materials *produced* by unit `u` — the `+` refs of its hyperarc,
    /// i.e. the successors of `u` in the schema (edges `u → m`).
    ///
    /// # Postconditions
    /// Equal to `schema.successors(u)`; a disposal-sink unit (no
    /// outputs) yields the empty set.
    pub fn outputs(&self, u: DeclId) -> &BTreeSet<DeclId> {
        self.schema.successors(u)
    }

    /// Operating units that *produce* material `x` — the
    /// $\Delta(x)$ of Friedler's decision-mapping SSG (book Def. 5.1).
    ///
    /// A produced edge is `unit → material`, so the producers of a
    /// material are exactly its schema predecessors. This is the dual
    /// of [`Self::outputs`] and, like it, a *query* over the single
    /// directed edge set — not a stored side table.
    ///
    /// # Postconditions
    /// Equal to `schema.predecessors(x)`; a raw material (produced by
    /// no unit) yields the empty set.
    pub fn producers(&self, x: DeclId) -> &BTreeSet<DeclId> {
        self.schema.predecessors(x)
    }

    /// Operating units that *consume* material `x`. The dual of
    /// [`Self::inputs`]; a consumed edge is `material → unit`, so the
    /// consumers of a material are its schema successors.
    ///
    /// # Postconditions
    /// Equal to `schema.successors(x)`; a material consumed by no unit
    /// yields the empty set.
    pub fn consumers(&self, x: DeclId) -> &BTreeSet<DeclId> {
        self.schema.successors(x)
    }
}

/// Lower a parsed [`Description`] into a P-graph.
///
/// Materials are collected from `HyperItem::Node` decls tagged
/// `<material>`; units are collected from `HyperItem::Edge` decls
/// tagged `<unit>`. A wrapper such as `context { ... }` is followed
/// transparently — the lowering recurses one level into untagged
/// node bodies.
pub fn lower<'a>(d: &Description<'a, &'a str>) -> Result<LoweredPGraph, LowerError> {
    // ── 1. Walk top-level items, separating materials (nodes) and
    //       units (edges).
    let mut mats: Vec<&NodeDecl<'a, &'a str>> = Vec::new();
    let mut units: Vec<&EdgeDecl<'a, &'a str>> = Vec::new();
    for item in &d.items {
        collect(item, &mut mats, &mut units);
    }

    // ── 2. Assign DeclIds, classify, build the kind map and side-sets.
    let mut name_to_decl: BTreeMap<String, DeclId> = BTreeMap::new();
    let mut decl_to_name: BTreeMap<DeclId, String> = BTreeMap::new();
    let mut kinds: BTreeMap<DeclId, PNodeKind> = BTreeMap::new();
    let mut raws: BTreeSet<DeclId> = BTreeSet::new();
    let mut products: BTreeSet<DeclId> = BTreeSet::new();

    for n in &mats {
        if !is_material(&n.anno) {
            return Err(LowerError::AmbiguousRole {
                name: n.inner.name.to_string(),
                detail: "node lacks <material> tag".into(),
            });
        }
        let id = intern(&mut name_to_decl, &mut decl_to_name, n.inner.name);
        kinds.insert(id, PNodeKind::Material);
        if n.anno.tags.iter().any(|t| *t == "raw") {
            raws.insert(id);
        }
        if n.anno.tags.iter().any(|t| *t == "product") {
            products.insert(id);
        }
    }
    for e in &units {
        if !is_unit(&e.anno) {
            return Err(LowerError::AmbiguousRole {
                name: e.inner.name.to_string(),
                detail: "@-edge lacks <unit> tag".into(),
            });
        }
        let id = intern(&mut name_to_decl, &mut decl_to_name, e.inner.name);
        kinds.insert(id, PNodeKind::OperatingUnit);
    }

    // ── 3. Walk each unit's body for the I/O hyperarc + cost.
    //
    // Cost sources, in priority order:
    //   1. Edge value (idiomatic):  `@U <unit> 100 { ... }`
    //   2. Untagged `cost N;` child node (back-compat).
    //   3. Tagged   `cost <dim_name> N;` child node (Stage P-mo,
    //      multi-objective). Sub-tag identifies the dimension.
    //
    // Source 3 is additive: it populates `cost_vectors` but does
    // NOT modify `costs` (the scalar path stays byte-identical).
    let mut edges: BTreeMap<EdgeId, (DeclId, DeclId)> = BTreeMap::new();
    let mut costs: BTreeMap<DeclId, f64> = BTreeMap::new();
    let mut per_unit_dim_costs: BTreeMap<DeclId, BTreeMap<String, f64>> = BTreeMap::new();
    let mut all_dimensions: BTreeSet<String> = BTreeSet::new();
    let mut next_edge: usize = 0;

    for e in &units {
        let unit_id = name_to_decl[e.inner.name];
        let unit_name = e.inner.name.to_string();
        per_unit_dim_costs.entry(unit_id).or_default();

        // Cost source 1: the edge's value (idiomatic).
        let cost = match &e.anno.value {
            Some(Value::Num(v)) => *v,
            _ => 1.0,
        };
        costs.insert(unit_id, cost);

        for it in &e.inner.body {
            match it {
                HyperItem::Arc(a) => {
                    for sref in &a.inner.refs {
                        let (sign, target_name) = match sref {
                            SignedRef::Plus(r) => (1, leaf(&r.target.path)),
                            SignedRef::Minus(r) => (-1, leaf(&r.target.path)),
                            SignedRef::Neutral(_) => {
                                return Err(LowerError::NeutralRef {
                                    unit: unit_name.clone(),
                                });
                            }
                        };
                        let mat_id = *name_to_decl.get(target_name).ok_or_else(|| {
                            LowerError::UnknownMaterial {
                                unit: unit_name.clone(),
                                target: target_name.to_string(),
                            }
                        })?;
                        if sign < 0 {
                            edges.insert(EdgeId::new(next_edge), (mat_id, unit_id));
                        } else {
                            edges.insert(EdgeId::new(next_edge), (unit_id, mat_id));
                        }
                        next_edge += 1;
                    }
                }
                HyperItem::Node(child) => {
                    if child.inner.name == "cost" {
                        if let Some(Value::Num(v)) = &child.anno.value {
                            // Source 2: untagged `cost N;` — back-compat
                            // updates the scalar cost.
                            // Source 3: tagged `cost <dim> N;` — populates
                            // the named dimension in cost_vectors but
                            // does NOT change the scalar.
                            let dim_tags: Vec<&str> =
                                child.anno.tags.iter().map(|t| t.as_ref()).collect();
                            if dim_tags.is_empty() {
                                costs.insert(unit_id, *v);
                            } else {
                                // Multi-objective tagged form. Use the
                                // first tag as the dimension name (a
                                // unit declaring `cost <capex, opex>`
                                // is unusual but accept the first tag).
                                let dim_name = dim_tags[0].to_string();
                                all_dimensions.insert(dim_name.clone());
                                per_unit_dim_costs
                                    .get_mut(&unit_id)
                                    .unwrap()
                                    .insert(dim_name, *v);
                            }
                        }
                    }
                }
                HyperItem::Edge(_) => { /* nested edges ignored */ }
            }
        }
    }

    // ── 3b. Build the canonical cost_dimensions Vec and cost_vectors.
    //        Alphabetical order for deterministic output across runs
    //        and across source-file edits. Units missing a dimension
    //        default to 0.0 for that dimension (sensible default:
    //        "this unit does not emit CO2" rather than "this unit's
    //        CO2 cost is unspecified").
    let cost_dimensions: Vec<String> = all_dimensions.iter().cloned().collect();
    let cost_vectors: BTreeMap<DeclId, Vec<f64>> = if cost_dimensions.is_empty() {
        BTreeMap::new()
    } else {
        per_unit_dim_costs
            .into_iter()
            .map(|(u, dims)| {
                let v: Vec<f64> = cost_dimensions
                    .iter()
                    .map(|name| dims.get(name).copied().unwrap_or(0.0))
                    .collect();
                (u, v)
            })
            .collect()
    };

    // ── 4. Construct the schema (bipartite invariant check).
    let schema = PGraphSchema::try_new(kinds.clone(), edges)?;

    let materials: BTreeSet<DeclId> = kinds
        .iter()
        .filter_map(|(d, k)| (*k == PNodeKind::Material).then_some(*d))
        .collect();
    let units_set: BTreeSet<DeclId> = kinds
        .iter()
        .filter_map(|(d, k)| (*k == PNodeKind::OperatingUnit).then_some(*d))
        .collect();

    Ok(LoweredPGraph {
        schema,
        name_to_decl,
        decl_to_name,
        raws,
        products,
        materials,
        units: units_set,
        costs,
        cost_dimensions,
        cost_vectors,
    })
}

// ─── helpers ────────────────────────────────────────────────────────

fn intern(
    name_to_decl: &mut BTreeMap<String, DeclId>,
    decl_to_name: &mut BTreeMap<DeclId, String>,
    name: &str,
) -> DeclId {
    if let Some(id) = name_to_decl.get(name) {
        return *id;
    }
    let id = DeclId::new(name_to_decl.len());
    name_to_decl.insert(name.to_string(), id);
    decl_to_name.insert(id, name.to_string());
    id
}

fn is_material(a: &Anno<'_, &str>) -> bool {
    a.tags.iter().any(|t| *t == "material")
}

fn is_unit(a: &Anno<'_, &str>) -> bool {
    a.tags.iter().any(|t| *t == "unit")
}

/// Walk an item tree, collecting tagged materials and tagged units.
/// Untagged nodes with bodies are descended into (this lets a
/// `context { ... }` wrapper hold the actual P-graph items without
/// itself being part of the P-graph).
fn collect<'a, 'b>(
    item: &'b HyperItem<'a, &'a str>,
    mats: &mut Vec<&'b NodeDecl<'a, &'a str>>,
    units: &mut Vec<&'b EdgeDecl<'a, &'a str>>,
) {
    match item {
        HyperItem::Node(n) => {
            if is_material(&n.anno) {
                mats.push(n);
            } else if let Some(body) = &n.inner.body {
                // Untagged node — likely a `context { ... }` wrapper.
                for child in body {
                    collect(child, mats, units);
                }
            }
        }
        HyperItem::Edge(e) => {
            if is_unit(&e.anno) {
                units.push(e);
            }
            // Untagged edges are ignored (they belong to other passes).
        }
        HyperItem::Arc(_) => { /* top-level arcs are ignored */ }
    }
}

fn leaf<'a>(path: &'a [&'a str]) -> &'a str {
    path.last().copied().unwrap_or("")
}
