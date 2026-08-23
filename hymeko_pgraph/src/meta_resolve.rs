//! Meta-model → P-graph adapter.
//!
//! Lets a P-graph be authored in the general HyMeKo meta-model style — an
//! `@"meta_pgraph.hymeko"` include, `using pgraph.raw as raw` aliases, and
//! instances typed by `<isa>` (e.g. `A: + <isa> raw {}`) — and lowers it to a
//! [`LoweredPGraph`].
//!
//! The include + alias resolution is done by [`hymeko_core`]'s public
//! [`ModuleStore::compile`] pipeline (used read-only — no core edit, no
//! reimplementation), which yields a fully-resolved [`Ir`]. This module then
//! walks the IR's `<isa>` ancestry to classify declarations into the Friedler
//! sets `M` (materials, with raw/product roles `R`/`P`) and `O` (operating
//! units), and builds the engine-ready [`LoweredPGraph`]. HyMeKo itself gains
//! no P-graph keywords; only the P-graph *archetype contract* (the names
//! `raw` / `product` / `intermediate` / `process`) lives here.
//!
//! ```text
//!   *.hymeko ──ModuleStore::compile──▶ resolved Ir ──this module──▶ LoweredPGraph
//!   (core, read-only)                                (non-core adapter)
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hymeko::common::ids::{DeclId, EdgeId, SymId};
use hymeko::ir::ir::{DeclKind, Ir, SignedRefR, ValueR};
use hymeko::module_store::module_store::{HymekoParser, ModuleStore};
use hymeko::module_store::source_provider::{MemProvider, SourceProvider, StdFsProvider};
use hymeko::resolution::interner::Interner;
use parser::ast::AstStr;
use thiserror::Error;

use crate::lowering::LoweredPGraph;
use crate::schema::{PGraphSchema, PNodeKind};

/// The four archetype names the pgraph meta-model is contracted to declare.
const A_RAW: &str = "raw";
const A_PRODUCT: &str = "product";
const A_INTERMEDIATE: &str = "intermediate";
const A_PROCESS: &str = "process";

/// Errors raised while compiling / lowering a meta-model P-graph file.
#[derive(Debug, Error)]
pub enum MetaResolveError {
    /// `hymeko_core` failed to load / resolve / lower the source.
    #[error("compile: {0}")]
    Compile(String),
    /// An archetype name (e.g. `raw`) is not declared in the resolved program
    /// — the file likely does not include the pgraph meta-model.
    #[error("missing pgraph archetype `{0}` (did you include meta_pgraph.hymeko?)")]
    MissingArchetype(&'static str),
    /// An archetype name resolves to more than one declaration of the expected
    /// kind, so classification would be ambiguous.
    #[error("ambiguous pgraph archetype `{0}`: multiple declarations match")]
    AmbiguousArchetype(&'static str),
    /// A material's `<isa>` ancestry reaches both `raw` and `product`.
    #[error("material {material:?} is typed as both raw and product")]
    ConflictingRole {
        /// Offending material name.
        material: String,
    },
    /// A unit edge references a non-material vertex in its incidence.
    #[error("unit {unit:?} has incidence on non-material {target:?}")]
    NonMaterialIo {
        /// Operating-unit name.
        unit: String,
        /// The non-material target name.
        target: String,
    },
    /// A unit arc used a neutral (`~`) sign; P-graph incidence must be `+`/`-`.
    #[error("unit {unit:?} has a neutral (~) reference; P-graph edges must be + or -")]
    NeutralRef {
        /// Offending unit name.
        unit: String,
    },
    /// The bipartite schema construction failed.
    #[error("schema: {0}")]
    Schema(String),
}

/// Thin [`HymekoParser`] over the LALR(1) [`parser::parse_description`].
struct PgraphParser;

impl HymekoParser for PgraphParser {
    fn parse<'a>(&self, src: &'a str) -> Result<AstStr<'a>, String> {
        parser::parse_description(src).map_err(|e| format!("{e:?}"))
    }
}

/// Compile a `.hymeko` file — resolving `@"includes"` and `using ... as`
/// aliases via `hymeko_core` — and lower it to a [`LoweredPGraph`].
///
/// # Errors
/// [`MetaResolveError::Compile`] on parse/resolve failure; the classification
/// errors below on a malformed P-graph.
pub fn compile_to_lowered(path: &Path) -> Result<LoweredPGraph, MetaResolveError> {
    let mut ms = ModuleStore::new(StdFsProvider::new(), PgraphParser);
    lower_compiled(&mut ms, path)
}

/// Compile from in-memory source strings (no filesystem) and lower to a
/// [`LoweredPGraph`]. The browser / WASM entry point, and a filesystem-free
/// path for tests.
///
/// `files` are `(name, content)` pairs; `root` names the entry file. Includes
/// resolve by name against `files` — e.g. an instance whose body contains
/// `@"meta_pgraph.hymeko"` needs a `("meta_pgraph.hymeko", <meta source>)`
/// entry.
///
/// # Preconditions
/// `root` matches one of the `files` names; every `@"..."` include in the
/// reachable set has a corresponding `files` entry.
pub fn compile_sources(
    root: &str,
    files: &[(&str, &str)],
) -> Result<LoweredPGraph, MetaResolveError> {
    let mut provider = MemProvider::default();
    for (name, content) in files {
        provider.insert_file(PathBuf::from(*name), *content);
    }
    let mut ms = ModuleStore::new(provider, PgraphParser);
    lower_compiled(&mut ms, Path::new(root))
}

/// Shared back end for [`compile_to_lowered`] / [`compile_sources`]: compile via
/// any [`SourceProvider`], then lower the resolved IR. No duplication across the
/// filesystem and in-memory paths.
fn lower_compiled<P, R>(
    ms: &mut ModuleStore<P, R>,
    root: &Path,
) -> Result<LoweredPGraph, MetaResolveError>
where
    P: SourceProvider,
    R: HymekoParser,
{
    let prog = ms
        .compile(root)
        .map_err(|e| MetaResolveError::Compile(format!("{e:?}")))?;
    lower_resolved(&prog.ir, &ms.it)
}

/// Lower an already-resolved [`Ir`] into a [`LoweredPGraph`] by `<isa>`
/// classification. Split out from [`compile_to_lowered`] so the classifier is
/// testable without touching the filesystem.
///
/// # Postconditions
/// Every schema edge connects a material to a unit; `raws`/`products` ⊆
/// materials. Declarations whose `<isa>` ancestry reaches no archetype are
/// ignored (they are not part of the P-graph).
pub fn lower_resolved(ir: &Ir, it: &Interner) -> Result<LoweredPGraph, MetaResolveError> {
    let arch = Archetypes::find(ir, it)?;

    let mut ctx = Lowered::default();
    classify_materials(ir, it, &arch, &mut ctx)?;
    classify_units(ir, it, &arch, &mut ctx);
    let (edges, costs) = build_incidence(ir, it, &ctx)?;

    let schema = PGraphSchema::try_new(ctx.kinds, edges)
        .map_err(|e| MetaResolveError::Schema(format!("{e}")))?;
    Ok(LoweredPGraph {
        schema,
        name_to_decl: ctx.name_to_decl,
        decl_to_name: ctx.decl_to_name,
        raws: ctx.raws,
        products: ctx.products,
        materials: ctx.materials,
        units: ctx.units,
        costs,
        cost_dimensions: Vec::new(),
        cost_vectors: BTreeMap::new(),
    })
}

/// Resolved archetype DeclIds plus the interned `"isa"` tag symbol.
struct Archetypes {
    raw: DeclId,
    product: DeclId,
    intermediate: DeclId,
    process: DeclId,
    isa: SymId,
}

impl Archetypes {
    fn find(ir: &Ir, it: &Interner) -> Result<Self, MetaResolveError> {
        Ok(Self {
            raw: find_decl(ir, it, A_RAW, DeclKind::Node)?,
            product: find_decl(ir, it, A_PRODUCT, DeclKind::Node)?,
            intermediate: find_decl(ir, it, A_INTERMEDIATE, DeclKind::Node)?,
            process: find_decl(ir, it, A_PROCESS, DeclKind::Edge)?,
            // No `<isa>` in the program ⇒ NONE, which no tag list contains, so
            // every `isa_reaches` is vacuously false (empty P-graph).
            isa: it.get_id("isa").unwrap_or(SymId::NONE),
        })
    }

    fn is_archetype(&self, d: DeclId) -> bool {
        d == self.raw || d == self.product || d == self.intermediate || d == self.process
    }
}

/// Mutable accumulator threaded through the classification passes.
#[derive(Default)]
struct Lowered {
    kinds: BTreeMap<DeclId, PNodeKind>,
    name_to_decl: BTreeMap<String, DeclId>,
    decl_to_name: BTreeMap<DeclId, String>,
    materials: BTreeSet<DeclId>,
    units: BTreeSet<DeclId>,
    raws: BTreeSet<DeclId>,
    products: BTreeSet<DeclId>,
}

impl Lowered {
    fn register(&mut self, ir: &Ir, it: &Interner, d: DeclId, kind: PNodeKind) {
        let name = it.resolve(decl_name(ir, d)).to_string();
        self.name_to_decl.insert(name.clone(), d);
        self.decl_to_name.insert(d, name);
        self.kinds.insert(d, kind);
    }
}

/// Pass 1: every `Node` decl whose `<isa>` ancestry reaches a material
/// archetype becomes a material, with its raw/product role recorded.
fn classify_materials(
    ir: &Ir,
    it: &Interner,
    arch: &Archetypes,
    ctx: &mut Lowered,
) -> Result<(), MetaResolveError> {
    for d in decl_ids(ir) {
        if arch.is_archetype(d) || decl_kind(ir, d) != DeclKind::Node {
            continue;
        }
        let is_raw = isa_reaches(ir, d, arch.raw, arch.isa);
        let is_prod = isa_reaches(ir, d, arch.product, arch.isa);
        let is_inter = isa_reaches(ir, d, arch.intermediate, arch.isa);
        if !(is_raw || is_prod || is_inter) {
            continue;
        }
        if is_raw && is_prod {
            return Err(MetaResolveError::ConflictingRole {
                material: it.resolve(decl_name(ir, d)).to_string(),
            });
        }
        ctx.register(ir, it, d, PNodeKind::Material);
        ctx.materials.insert(d);
        if is_raw {
            ctx.raws.insert(d);
        }
        if is_prod {
            ctx.products.insert(d);
        }
    }
    Ok(())
}

/// Pass 2 (hybrid rule): an `Edge` decl is an operating unit iff its `<isa>`
/// ancestry reaches `process`, or its arcs are non-empty and every arc target
/// is a material (so e.g. a `@dataflow` edge over non-materials is skipped).
fn classify_units(ir: &Ir, it: &Interner, arch: &Archetypes, ctx: &mut Lowered) {
    for d in decl_ids(ir) {
        if arch.is_archetype(d) || decl_kind(ir, d) != DeclKind::Edge {
            continue;
        }
        let explicit = isa_reaches(ir, d, arch.process, arch.isa);
        let structural = {
            let refs = unit_arc_targets(ir, d);
            !refs.is_empty() && refs.iter().all(|m| ctx.materials.contains(m))
        };
        if explicit || structural {
            ctx.register(ir, it, d, PNodeKind::OperatingUnit);
            ctx.units.insert(d);
        }
    }
}

/// Directed signed-incidence edge set plus per-unit scalar costs.
type Incidence = (BTreeMap<EdgeId, (DeclId, DeclId)>, BTreeMap<DeclId, f64>);

/// Build the directed signed incidence and per-unit scalar costs.
fn build_incidence(ir: &Ir, it: &Interner, ctx: &Lowered) -> Result<Incidence, MetaResolveError> {
    let mut edges = BTreeMap::new();
    let mut costs = BTreeMap::new();
    let mut next_edge = 0usize;
    for &u in &ctx.units {
        costs.insert(u, edge_cost(ir, u).unwrap_or(1.0));
        for sref in unit_arc_refs(ir, u) {
            let m = sref.target();
            if !ctx.materials.contains(&m) {
                return Err(MetaResolveError::NonMaterialIo {
                    unit: it.resolve(decl_name(ir, u)).to_string(),
                    target: it.resolve(decl_name(ir, m)).to_string(),
                });
            }
            match sref.sign() {
                s if s < 0 => {
                    edges.insert(EdgeId::new(next_edge), (m, u));
                }
                s if s > 0 => {
                    edges.insert(EdgeId::new(next_edge), (u, m));
                }
                _ => {
                    return Err(MetaResolveError::NeutralRef {
                        unit: it.resolve(decl_name(ir, u)).to_string(),
                    });
                }
            }
            next_edge += 1;
        }
    }
    Ok((edges, costs))
}

// ─── IR helpers ──────────────────────────────────────────────────────────

fn decl_ids(ir: &Ir) -> impl Iterator<Item = DeclId> {
    (0..ir.decl_nodes.len()).map(DeclId::new)
}

fn decl_kind(ir: &Ir, d: DeclId) -> DeclKind {
    ir.decl_nodes[d.0].kind
}

fn decl_name(ir: &Ir, d: DeclId) -> SymId {
    ir.decl_nodes[d.0].name
}

/// The `<isa>` / inheritance bases of a declaration (node or edge).
fn bases_of(ir: &Ir, d: DeclId) -> &[SignedRefR] {
    match decl_kind(ir, d) {
        DeclKind::Node => ir.as_node(d).map(|n| ir.nodes[n.0].bases.as_slice()),
        DeclKind::Edge => ir.as_edge(d).map(|e| ir.edges[e.0].bases.as_slice()),
        DeclKind::HyperArc => None,
    }
    .unwrap_or(&[])
}

/// Does `from`'s `<isa>` ancestry reach `target`? Cycle-safe (visited set);
/// does not count `from` itself, so an archetype never classifies itself.
fn isa_reaches(ir: &Ir, from: DeclId, target: DeclId, isa: SymId) -> bool {
    fn walk(
        ir: &Ir,
        from: DeclId,
        target: DeclId,
        isa: SymId,
        seen: &mut BTreeSet<DeclId>,
    ) -> bool {
        for base in bases_of(ir, from) {
            if !base.atom().anno.tags.contains(&isa) {
                continue;
            }
            let parent = base.target();
            if parent == target {
                return true;
            }
            if seen.insert(parent) && walk(ir, parent, target, isa, seen) {
                return true;
            }
        }
        false
    }
    if isa.is_none() {
        return false;
    }
    walk(ir, from, target, isa, &mut BTreeSet::new())
}

/// All signed references across all arcs of an edge declaration.
fn unit_arc_refs(ir: &Ir, edge: DeclId) -> Vec<&SignedRefR> {
    let Some(eid) = ir.as_edge(edge) else {
        return Vec::new();
    };
    ir.edges[eid.0]
        .arcs
        .iter()
        .flat_map(|aid| ir.arcs[aid.0].refs.iter())
        .collect()
}

/// The arc-target DeclIds of an edge (materials a unit is incident on).
fn unit_arc_targets(ir: &Ir, edge: DeclId) -> Vec<DeclId> {
    unit_arc_refs(ir, edge).iter().map(|r| r.target()).collect()
}

/// The numeric value annotation of a declaration, if any (a unit's cost).
fn edge_cost(ir: &Ir, edge: DeclId) -> Option<f64> {
    match ir.decl_node(edge)?.anno.value {
        Some(ValueR::Num(v)) => Some(v),
        _ => None,
    }
}

/// Find the single declaration named `name` of the given `kind`.
fn find_decl(
    ir: &Ir,
    it: &Interner,
    name: &'static str,
    kind: DeclKind,
) -> Result<DeclId, MetaResolveError> {
    let sym = it
        .get_id(name)
        .ok_or(MetaResolveError::MissingArchetype(name))?;
    let mut found = None;
    for d in decl_ids(ir) {
        let dn = &ir.decl_nodes[d.0];
        if dn.name == sym && dn.kind == kind {
            if found.is_some() {
                return Err(MetaResolveError::AmbiguousArchetype(name));
            }
            found = Some(d);
        }
    }
    found.ok_or(MetaResolveError::MissingArchetype(name))
}
