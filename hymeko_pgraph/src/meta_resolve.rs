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

use hymeko::common::ids::{DeclId, EdgeId, HyperArcId, NodeId, SymId};
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
    /// The resolved `Ir` was structurally malformed where the lowerer reads it.
    ///
    /// Distinct from the domain errors above (which describe a well-formed but
    /// *semantically* invalid P-graph): this arm is a storage-level fault, fully
    /// typed, carrying only ids. `#[from]` gives `?`-propagation from the helper
    /// layer and exposes the cause through [`std::error::Error::source`].
    #[error("malformed IR: {0}")]
    Ir(#[from] IrReadError),
}

/// A structural fault in the resolved `Ir`, found while lowering it to a P-graph.
///
/// Every variant means the `Ir` names something that is not there, or
/// contradicts itself, at a point `meta_resolve` must read. It carries **ids and
/// at most a `DeclKind` only** — never a declaration name or any other
/// program-supplied text — so a hostile description cannot make the error echo an
/// unbounded payload. Ids are small `Copy` integers, so the whole enum is `Copy`
/// and constructing one allocates nothing.
///
/// None of these is reachable from a well-formed `Ir`; a program that compiles
/// and whose coherence validator is clean never produces one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IrReadError {
    /// A declaration the lowerer read does not exist.
    #[error("declaration {} does not exist", .decl.raw())]
    DanglingDeclaration {
        /// The declaration id that was not found.
        decl: DeclId,
    },
    /// A `Node` declaration has no node-record mapping.
    #[error("node declaration {} has no record mapping", .decl.raw())]
    MissingNodeMapping {
        /// The node declaration whose mapping is absent.
        decl: DeclId,
    },
    /// An `Edge` declaration has no edge-record mapping.
    #[error("edge declaration {} has no record mapping", .decl.raw())]
    MissingEdgeMapping {
        /// The edge declaration whose mapping is absent.
        decl: DeclId,
    },
    /// A node mapping resolves to a record index that is not present.
    #[error("declaration {} maps to node record {}, which is absent", .decl.raw(), .node.raw())]
    MissingNodeRecord {
        /// The declaration whose mapping dangles.
        decl: DeclId,
        /// The node-record id it mapped to, for which no record exists.
        node: NodeId,
    },
    /// An edge mapping resolves to a record index that is not present.
    #[error("declaration {} maps to edge record {}, which is absent", .decl.raw(), .edge.raw())]
    MissingEdgeRecord {
        /// The declaration whose mapping dangles.
        decl: DeclId,
        /// The edge-record id it mapped to, for which no record exists.
        edge: EdgeId,
    },
    /// A node record's `decl` back-pointer names a different declaration.
    #[error("node record for declaration {} back-links to {} instead", .decl.raw(), .record_decl.raw())]
    NodeBacklinkMismatch {
        /// The declaration that was resolved.
        decl: DeclId,
        /// The declaration the record actually back-links to.
        record_decl: DeclId,
    },
    /// An edge record's `decl` back-pointer names a different declaration.
    #[error("edge record for declaration {} back-links to {} instead", .decl.raw(), .record_decl.raw())]
    EdgeBacklinkMismatch {
        /// The declaration that was resolved.
        decl: DeclId,
        /// The declaration the record actually back-links to.
        record_decl: DeclId,
    },
    /// An edge lists an arc id for which no arc record exists.
    #[error("edge {} lists arc {}, which does not exist", .edge.raw(), .arc.raw())]
    DanglingArcRecord {
        /// The edge that lists the arc.
        edge: DeclId,
        /// The arc id for which no record exists.
        arc: HyperArcId,
    },
    /// An arc an edge owns names a *different* edge as its owner
    /// ([`hymeko::ir::ir::ArcRec::in_edge`]).
    #[error("edge {} owns arc {}, but the arc names edge {} as its owner", .edge.raw(), .arc.raw(), .owner.raw())]
    ArcOwnershipMismatch {
        /// The edge whose arc list contains the arc.
        edge: DeclId,
        /// The arc with the mismatched owner.
        arc: HyperArcId,
        /// The edge the arc actually names as its owner.
        owner: DeclId,
    },
    /// A base names a declaration that does not exist. Distinct from a
    /// declaration that legitimately names *no* base, which is not an error.
    #[error("base names declaration {}, which does not exist", .base.raw())]
    DanglingBaseTarget {
        /// The non-existent declaration a base points at.
        base: DeclId,
    },
    /// A unit arc's incidence references a declaration that does not exist.
    /// Distinct from [`MetaResolveError::NonMaterialIo`], where the target
    /// exists but is not a material.
    #[error("unit {} has an incidence on declaration {}, which does not exist", .edge.raw(), .target.raw())]
    DanglingIncidenceTarget {
        /// The unit edge carrying the incidence.
        edge: DeclId,
        /// The non-existent declaration the incidence targets.
        target: DeclId,
    },
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
    classify_units(ir, it, &arch, &mut ctx)?;
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
    /// Record a classified declaration under its (already-resolved) name symbol.
    ///
    /// Takes the `SymId` rather than re-reading it, because the caller already
    /// holds the `&DeclNode` from `ir.decls()` and reading the name again would
    /// be a redundant lookup.
    fn register(&mut self, it: &Interner, d: DeclId, name: SymId, kind: PNodeKind) {
        let name = it.resolve(name).to_string();
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
    let mut walk = IsaWalk::default();
    for (d, node) in ir.decls() {
        if arch.is_archetype(d) || node.kind != DeclKind::Node {
            continue;
        }
        let is_raw = walk.reaches(ir, d, arch.raw, arch.isa)?;
        let is_prod = walk.reaches(ir, d, arch.product, arch.isa)?;
        let is_inter = walk.reaches(ir, d, arch.intermediate, arch.isa)?;
        if !(is_raw || is_prod || is_inter) {
            continue;
        }
        if is_raw && is_prod {
            return Err(MetaResolveError::ConflictingRole {
                material: it.resolve(node.name).to_string(),
            });
        }
        ctx.register(it, d, node.name, PNodeKind::Material);
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
fn classify_units(
    ir: &Ir,
    it: &Interner,
    arch: &Archetypes,
    ctx: &mut Lowered,
) -> Result<(), MetaResolveError> {
    let mut walk = IsaWalk::default();
    for (d, node) in ir.decls() {
        if arch.is_archetype(d) || node.kind != DeclKind::Edge {
            continue;
        }
        let explicit = walk.reaches(ir, d, arch.process, arch.isa)?;
        let structural = {
            let refs = unit_arc_targets(ir, d)?;
            !refs.is_empty() && refs.iter().all(|m| ctx.materials.contains(m))
        };
        if explicit || structural {
            ctx.register(it, d, node.name, PNodeKind::OperatingUnit);
            ctx.units.insert(d);
        }
    }
    Ok(())
}

/// Directed signed-incidence edge set plus per-unit scalar costs.
type Incidence = (BTreeMap<EdgeId, (DeclId, DeclId)>, BTreeMap<DeclId, f64>);

/// Build the directed signed incidence and per-unit scalar costs.
fn build_incidence(ir: &Ir, it: &Interner, ctx: &Lowered) -> Result<Incidence, MetaResolveError> {
    let mut edges = BTreeMap::new();
    let mut costs = BTreeMap::new();
    let mut next_edge = 0usize;
    for &u in &ctx.units {
        costs.insert(u, edge_cost(ir, u)?.unwrap_or(1.0));
        for sref in unit_arc_refs(ir, u)? {
            let m = sref.target();
            if !ctx.materials.contains(&m) {
                // A target that is not a material is either a genuine
                // non-material declaration (a domain error) or an id that names
                // nothing at all (a storage fault). Distinguish them: only an
                // existing declaration can be reported as `NonMaterialIo`.
                if ir.kind_of(m).is_none() {
                    return Err(IrReadError::DanglingIncidenceTarget { edge: u, target: m }.into());
                }
                return Err(MetaResolveError::NonMaterialIo {
                    unit: it.resolve(decl_name(ir, u)?).to_string(),
                    target: it.resolve(decl_name(ir, m)?).to_string(),
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
                        unit: it.resolve(decl_name(ir, u)?).to_string(),
                    });
                }
            }
            next_edge += 1;
        }
    }
    Ok((edges, costs))
}

// ─── IR helpers ──────────────────────────────────────────────────────────
//
// Every read here goes through the storage-independent accessor API of core
// `15223bc`; not one indexes `Ir::decl_nodes` / `nodes` / `edges` / `arcs`
// directly. A strict consumer must not flatten a structural fault to an empty
// result nor abort on a panicking index, so each helper that could hide one is
// fallible and returns a typed [`IrReadError`].

/// The interned name of a declaration, or a typed error if it does not exist.
fn decl_name(ir: &Ir, d: DeclId) -> Result<SymId, IrReadError> {
    ir.decl_node(d)
        .map(|n| n.name)
        .ok_or(IrReadError::DanglingDeclaration { decl: d })
}

/// The `<isa>` / inheritance bases of a declaration, with storage coherence
/// checked.
///
/// Mirrors `Ir::node_for_decl` / `edge_for_decl` — resolve the mapping, resolve
/// the record, require the record's `decl` to point back — but decomposes the
/// check through the map-only and total accessors so each inconsistency becomes
/// its own typed error, where the single `None` those methods return could not
/// say which fault fired.
///
/// # Postconditions
/// Returns the bases slice (empty for a hyper-arc declaration, a legitimate
/// absence); a typed [`IrReadError`] for a dangling declaration or incoherent
/// storage. Never panics, never allocates on the valid path.
fn bases_of(ir: &Ir, d: DeclId) -> Result<&[SignedRefR], IrReadError> {
    match ir.kind_of(d) {
        None => Err(IrReadError::DanglingDeclaration { decl: d }),
        Some(DeclKind::Node) => {
            let nid = ir
                .as_node(d)
                .ok_or(IrReadError::MissingNodeMapping { decl: d })?;
            let rec = ir
                .node(nid)
                .ok_or(IrReadError::MissingNodeRecord { decl: d, node: nid })?;
            if rec.decl != d {
                return Err(IrReadError::NodeBacklinkMismatch {
                    decl: d,
                    record_decl: rec.decl,
                });
            }
            Ok(rec.bases.as_slice())
        }
        Some(DeclKind::Edge) => {
            let eid = ir
                .as_edge(d)
                .ok_or(IrReadError::MissingEdgeMapping { decl: d })?;
            let rec = ir
                .edge(eid)
                .ok_or(IrReadError::MissingEdgeRecord { decl: d, edge: eid })?;
            if rec.decl != d {
                return Err(IrReadError::EdgeBacklinkMismatch {
                    decl: d,
                    record_decl: rec.decl,
                });
            }
            Ok(rec.bases.as_slice())
        }
        // A hyper-arc declaration carries no bases in this model; a legitimate
        // absence, not a fault.
        Some(DeclKind::HyperArc) => Ok(&[]),
    }
}

/// Reusable scratch for the `<isa>` ancestry walk.
///
/// Allocated **once** per classification pass and cleared per query, so the
/// per-`isa_reaches` valid path allocates nothing — the recursion this replaces
/// allocated a fresh visited set on every call, and a naive iterative rewrite
/// would add a per-call stack `Vec` on top. Both allocations are hoisted here.
#[derive(Default)]
struct IsaWalk {
    stack: Vec<DeclId>,
    seen: BTreeSet<DeclId>,
}

impl IsaWalk {
    /// Does `from`'s `<isa>` ancestry reach `target`? Cycle-safe via the visited
    /// set; does not count `from` itself, so an archetype never classifies
    /// itself.
    ///
    /// **Iterative** by design. The recursion it replaces was bounded only by
    /// the call stack, so a deep inheritance chain (tested to 1024) could
    /// overflow it. The traversal is the same depth-first reach over
    /// `<isa>`-tagged bases, the same visited-set cycle guard, and the same
    /// boolean result — an inheritance cycle still terminates and is still not an
    /// error (it is a property of the program's reference graph, not a storage
    /// malformation). What *is* new: a base naming a declaration that does not
    /// exist is now a typed [`IrReadError::DanglingBaseTarget`] rather than a
    /// panic.
    fn reaches(
        &mut self,
        ir: &Ir,
        from: DeclId,
        target: DeclId,
        isa: SymId,
    ) -> Result<bool, IrReadError> {
        if isa.is_none() {
            return Ok(false);
        }
        self.stack.clear();
        self.seen.clear();
        self.stack.push(from);
        while let Some(cur) = self.stack.pop() {
            for base in bases_of(ir, cur)? {
                if !base.atom().anno.tags.contains(&isa) {
                    continue;
                }
                let parent = base.target();
                if parent == target {
                    return Ok(true);
                }
                // The base names `parent`; in a strict consumer it must exist.
                if ir.kind_of(parent).is_none() {
                    return Err(IrReadError::DanglingBaseTarget { base: parent });
                }
                if self.seen.insert(parent) {
                    self.stack.push(parent);
                }
            }
        }
        Ok(false)
    }
}

/// All signed references across all arcs of an edge declaration, in
/// **arc-declaration order then ref-slot order** — both semantic for P-graph
/// incidence, so the walk preserves them exactly and inserts no `sort`/`set`.
///
/// Checked: the edge must resolve to a coherent edge record, every arc id it
/// lists must exist, and each arc must name this edge as its owner
/// ([`hymeko::ir::ir::ArcRec::in_edge`]).
fn unit_arc_refs(ir: &Ir, edge: DeclId) -> Result<Vec<&SignedRefR>, IrReadError> {
    let eid = ir
        .as_edge(edge)
        .ok_or(IrReadError::MissingEdgeMapping { decl: edge })?;
    let rec = ir.edge(eid).ok_or(IrReadError::MissingEdgeRecord {
        decl: edge,
        edge: eid,
    })?;
    if rec.decl != edge {
        return Err(IrReadError::EdgeBacklinkMismatch {
            decl: edge,
            record_decl: rec.decl,
        });
    }
    let mut out = Vec::new();
    for &aid in &rec.arcs {
        let arc = ir
            .arc(aid)
            .ok_or(IrReadError::DanglingArcRecord { edge, arc: aid })?;
        if arc.in_edge != edge {
            return Err(IrReadError::ArcOwnershipMismatch {
                edge,
                arc: aid,
                owner: arc.in_edge,
            });
        }
        out.extend(arc.refs.iter());
    }
    Ok(out)
}

/// The arc-target DeclIds of an edge (materials a unit is incident on), in the
/// same semantic order as [`unit_arc_refs`].
fn unit_arc_targets(ir: &Ir, edge: DeclId) -> Result<Vec<DeclId>, IrReadError> {
    Ok(unit_arc_refs(ir, edge)?
        .iter()
        .map(|r| r.target())
        .collect())
}

/// The numeric value annotation of a declaration (a unit's cost).
///
/// Distinguishes a **dangling declaration** (a typed error) from a declaration
/// that exists but carries **no** value (`Ok(None)` — a legitimate absence the
/// caller defaults). The pre-migration form flattened both into `None`.
fn edge_cost(ir: &Ir, edge: DeclId) -> Result<Option<f64>, IrReadError> {
    let node = ir
        .decl_node(edge)
        .ok_or(IrReadError::DanglingDeclaration { decl: edge })?;
    Ok(match node.anno.value {
        Some(ValueR::Num(v)) => Some(v),
        _ => None,
    })
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
    for (d, dn) in ir.decls() {
        if dn.name == sym && dn.kind == kind {
            if found.is_some() {
                return Err(MetaResolveError::AmbiguousArchetype(name));
            }
            found = Some(d);
        }
    }
    found.ok_or(MetaResolveError::MissingArchetype(name))
}
