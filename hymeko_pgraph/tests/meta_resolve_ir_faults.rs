//! Structural-fault behaviour of the meta-model → P-graph lowerer.
//!
//! APPROVED-GRAPH-EDIT: m2b-pgraph-meta-resolve-accessors (2026-08-30).
//!
//! `lower_resolved` is a strict consumer of a compiled `Ir`. These tests build
//! IRs that are deliberately malformed at one place each, call `lower_resolved`
//! directly (no filesystem, no parser), and assert the **exact**
//! `MetaResolveError::Ir(IrReadError::…)` variant. A well-formed baseline IR is
//! constructed the same way so the valid path is exercised through the identical
//! builder.
//!
//! The pre-migration behaviour these replace was: some faults flattened to an
//! empty result (a material silently dropped), others panicked on a direct
//! `Vec` index. Both are recorded in `reports/2026-08-30-graph-m2b-source.md`.
//!
//! Storage migration (core `70b4f36`): the three public `decl_to_{node,edge,arc}:
//! Vec<Option<Id>>` maps this fixture used to white-box were replaced by a sealed
//! compact `decl_to_record` slot. Construction now goes through the public API —
//! `push_decl` (append a declaration + a NONE dispatch slot) and the checked
//! `bind_{node,edge,arc}_decl` mutators — and reads through `as_{node,edge,arc}`.
//! Because `bind_*_decl` refuses re-binding a declaration to a *different* record
//! (a `debug_assert!` panic in test builds), a storage fault can no longer be a
//! post-hoc mutation of a valid IR: it must be baked in as the declaration's
//! first-and-only binding. Each such fault therefore builds a fresh IR via
//! [`Pg::build`] carrying exactly one [`Perturb`]; every other fault (a corrupt
//! *record*, not a corrupt *binding*) stays a mutation of the valid baseline.

use std::collections::BTreeMap;

use hymeko::common::ids::{DeclId, EdgeId, HyperArcId, NodeId, SymId};
use hymeko::ir::ir::{
    AnnoR, ArcRec, DeclKind, DeclNode, EdgeRec, Ir, NodeRec, RefAtomR, SignedRefR,
};
use hymeko::resolution::interner::Interner;
use hymeko_pgraph::{IrReadError, MetaResolveError, lower_resolved};

/// A bare `DeclNode` with the given name and kind: no parent, no children, no
/// annotation. The one construction seam every builder in this file shares, so
/// the seven-field literal is written once.
fn decl_node(name: SymId, kind: DeclKind) -> DeclNode {
    DeclNode {
        kind,
        name,
        parent: DeclId::NONE,
        first_child: DeclId::NONE,
        last_child: DeclId::NONE,
        next_sibling: DeclId::NONE,
        anno: AnnoR::default(),
    }
}

/// A single storage perturbation, applied to exactly one declaration *at build
/// time*.
///
/// The compact dispatch binder (`bind_*_decl`) refuses re-binding a declaration
/// to a different record, so a mapping fault cannot be a mutation of a valid IR —
/// it must be the declaration's first binding. Each mapping-fault test builds a
/// fresh baseline through [`Pg::build`] with one of these; [`Perturb::None`]
/// reproduces the well-formed baseline through the identical builder.
#[derive(Clone, Copy)]
enum Perturb {
    /// Well-formed: bind every declaration to its real record.
    None,
    /// Leave `decl`'s node-dispatch slot NONE — the old `decl_to_node[decl] = None`.
    UnmappedNode(DeclId),
    /// Bind `decl` to a node id with no record — the old `decl_to_node[decl] = Some(<absent>)`.
    NodeRecord(DeclId, NodeId),
    /// Leave `decl`'s edge-dispatch slot NONE — the old `decl_to_edge[decl] = None`.
    UnmappedEdge(DeclId),
    /// Bind `decl` to an edge id with no record — the old `decl_to_edge[decl] = Some(<absent>)`.
    EdgeRecord(DeclId, EdgeId),
}

/// Bind a Node declaration's dispatch slot, honoring a perturbation aimed at it.
fn bind_node(ir: &mut Ir, d: DeclId, nid: NodeId, perturb: Perturb) {
    match perturb {
        Perturb::UnmappedNode(x) if x == d => {} // leave the slot NONE
        Perturb::NodeRecord(x, forced) if x == d => {
            ir.bind_node_decl(d, forced)
                .expect("bind perturbed node slot");
        }
        _ => ir.bind_node_decl(d, nid).expect("bind node slot"),
    }
}

/// Bind an Edge declaration's dispatch slot, honoring a perturbation aimed at it.
fn bind_edge(ir: &mut Ir, d: DeclId, eid: EdgeId, perturb: Perturb) {
    match perturb {
        Perturb::UnmappedEdge(x) if x == d => {} // leave the slot NONE
        Perturb::EdgeRecord(x, forced) if x == d => {
            ir.bind_edge_decl(d, forced)
                .expect("bind perturbed edge slot");
        }
        _ => ir.bind_edge_decl(d, eid).expect("bind edge slot"),
    }
}

/// Minimal well-formed P-graph IR builder.
///
/// Layout is fixed and documented so each test can perturb exactly one thing:
///
/// | DeclId | name          | kind | role                         |
/// |--------|---------------|------|------------------------------|
/// | 0      | raw           | Node | archetype                    |
/// | 1      | product       | Node | archetype                    |
/// | 2      | intermediate  | Node | archetype                    |
/// | 3      | process       | Edge | archetype                    |
/// | 4      | A             | Node | material, `<isa> raw`        |
/// | 5      | P             | Node | material, `<isa> product`    |
/// | 6      | u             | Edge | unit: arc `-A +P`            |
///
/// The record store (nodes/edges/arcs) is always built in full, and the id each
/// declaration's record got is remembered in `node_of` / `edge_of`. That lets the
/// builder wire `<isa>` bases and arcs by record id **independently of the
/// dispatch bindings**, so a perturbed (unmapped or dangling) binding can coexist
/// with an otherwise well-formed structure.
struct Pg {
    ir: Ir,
    it: Interner,
    isa: SymId,
    node_of: BTreeMap<DeclId, NodeId>,
    edge_of: BTreeMap<DeclId, EdgeId>,
}

const RAW: DeclId = DeclId::new(0);
const PRODUCT: DeclId = DeclId::new(1);
#[allow(dead_code)]
const INTER: DeclId = DeclId::new(2);
#[allow(dead_code)]
const PROCESS: DeclId = DeclId::new(3);
const MAT_A: DeclId = DeclId::new(4);
const MAT_P: DeclId = DeclId::new(5);
const UNIT_U: DeclId = DeclId::new(6);

impl Pg {
    /// A structurally valid P-graph IR: two materials and one unit.
    fn valid() -> Self {
        Self::build(Perturb::None)
    }

    /// Build the fixed 7-declaration baseline, applying `perturb` to one
    /// declaration's record binding (see [`Perturb`]). `Perturb::None` is the
    /// well-formed baseline.
    fn build(perturb: Perturb) -> Self {
        let mut it = Interner::new();
        let isa = it.intern("isa");
        let mut ir = Ir::default();

        for (name, kind) in [
            ("raw", DeclKind::Node),
            ("product", DeclKind::Node),
            ("intermediate", DeclKind::Node),
            ("process", DeclKind::Edge),
            ("A", DeclKind::Node),
            ("P", DeclKind::Node),
            ("u", DeclKind::Edge),
        ] {
            let sym = it.intern(name);
            ir.push_decl(decl_node(sym, kind));
        }

        // Node records for every Node decl, back-linked; bound per `perturb`.
        let mut node_of = BTreeMap::new();
        for d in [RAW, PRODUCT, INTER, MAT_A, MAT_P] {
            let nid = NodeId::new(ir.nodes.len());
            ir.nodes.push(NodeRec::new(d, Vec::new()));
            node_of.insert(d, nid);
            bind_node(&mut ir, d, nid, perturb);
        }
        // Edge records for every Edge decl; bound per `perturb`.
        let mut edge_of = BTreeMap::new();
        for d in [PROCESS, UNIT_U] {
            let eid = EdgeId::new(ir.edges.len());
            ir.edges.push(EdgeRec::new(d, Vec::new()));
            edge_of.insert(d, eid);
            bind_edge(&mut ir, d, eid, perturb);
        }

        let mut pg = Self {
            ir,
            it,
            isa,
            node_of,
            edge_of,
        };
        // A <isa> raw ; P <isa> product
        pg.set_isa_base(MAT_A, RAW);
        pg.set_isa_base(MAT_P, PRODUCT);
        // unit u: arc  -A +P
        pg.add_unit_arc(UNIT_U, &[(MAT_A, -1), (MAT_P, 1)]);
        pg
    }

    /// The record id a Node declaration got in this fixture. Reads the builder's
    /// own record map, not the dispatch binding, so it is valid even for a decl
    /// whose binding was perturbed away.
    fn nid_of(&self, d: DeclId) -> NodeId {
        *self
            .node_of
            .get(&d)
            .expect("node decl built in this fixture")
    }

    /// Append a Node declaration with a fresh, bound node record and an `<isa>`
    /// base to `isa_target`. Returns the new `DeclId`. Extends the baseline with
    /// inheritance chains (see `deep_chain`).
    fn add_node_decl(&mut self, name: &str, isa_target: DeclId) -> DeclId {
        let sym = self.it.intern(name);
        let d = self.ir.push_decl(decl_node(sym, DeclKind::Node));
        let nid = NodeId::new(self.ir.nodes.len());
        self.ir.nodes.push(NodeRec::new(d, Vec::new()));
        self.ir.bind_node_decl(d, nid).expect("bind node slot");
        self.node_of.insert(d, nid);
        self.set_isa_base(d, isa_target);
        d
    }

    fn isa_base(&self, target: DeclId) -> SignedRefR {
        SignedRefR::Neutral(RefAtomR {
            target,
            anno: AnnoR {
                tags: vec![self.isa],
                value: None,
            },
            weights: None,
        })
    }

    fn set_isa_base(&mut self, d: DeclId, target: DeclId) {
        let base = self.isa_base(target);
        let nid = self.nid_of(d);
        self.ir.nodes[nid.raw()].bases.push(base);
    }

    /// Give an *edge* declaration an `<isa>` base (e.g. `u <isa> process`) so it
    /// is classified as a unit by the explicit rule even when its incidence is
    /// not all-material.
    fn set_isa_base_edge(&mut self, d: DeclId, target: DeclId) {
        let base = self.isa_base(target);
        let eid = *self
            .edge_of
            .get(&d)
            .expect("edge decl built in this fixture");
        self.ir.edges[eid.raw()].bases.push(base);
    }

    /// Append an arc to edge `owner` with the given signed material refs.
    fn add_unit_arc(&mut self, owner: DeclId, refs: &[(DeclId, i8)]) {
        let built: Vec<SignedRefR> = refs
            .iter()
            .map(|&(target, sign)| {
                let atom = RefAtomR {
                    target,
                    anno: AnnoR::default(),
                    weights: None,
                };
                match sign {
                    s if s < 0 => SignedRefR::Minus(atom),
                    s if s > 0 => SignedRefR::Plus(atom),
                    _ => SignedRefR::Neutral(atom),
                }
            })
            .collect();
        let aid = HyperArcId::new(self.ir.arcs.len());
        self.ir.arcs.push(ArcRec {
            anno: AnnoR::default(),
            in_edge: owner,
            refs: built,
        });
        let eid = *self
            .edge_of
            .get(&owner)
            .expect("edge decl built in this fixture");
        self.ir.edges[eid.raw()].arcs.push(aid);
    }

    fn lower(&self) -> Result<hymeko_pgraph::LoweredPGraph, MetaResolveError> {
        lower_resolved(&self.ir, &self.it)
    }

    fn ir_err(&self) -> IrReadError {
        match self.lower() {
            Err(MetaResolveError::Ir(e)) => e,
            other => panic!("expected MetaResolveError::Ir, got {other:?}"),
        }
    }
}

// ── valid baseline ─────────────────────────────────────────────────────────

#[test]
fn valid_pgraph_lowers_to_two_materials_and_one_unit() {
    let g = Pg::valid().lower().expect("valid IR lowers");
    assert_eq!(g.materials.len(), 2, "A and P");
    assert_eq!(g.raws.len(), 1);
    assert_eq!(g.products.len(), 1);
    assert_eq!(g.units.len(), 1, "u");
    assert_eq!(g.decl_to_name[&MAT_A], "A");
    assert_eq!(g.decl_to_name[&UNIT_U], "u");
}

// ── malformed: declaration layer ───────────────────────────────────────────

#[test]
fn missing_node_mapping_is_typed() {
    // A: Node decl whose node record is never bound (dispatch slot stays NONE).
    let pg = Pg::build(Perturb::UnmappedNode(MAT_A));
    assert_eq!(pg.ir_err(), IrReadError::MissingNodeMapping { decl: MAT_A });
}

#[test]
fn mapped_but_missing_node_record_is_typed() {
    // A maps to a node id for which no record exists.
    let pg = Pg::build(Perturb::NodeRecord(MAT_A, NodeId::new(999)));
    assert_eq!(
        pg.ir_err(),
        IrReadError::MissingNodeRecord {
            decl: MAT_A,
            node: NodeId::new(999)
        }
    );
}

#[test]
fn node_backlink_mismatch_is_typed() {
    let mut pg = Pg::valid();
    let nid = pg.nid_of(MAT_A);
    pg.ir.nodes[nid.raw()].decl = DeclId::new(42); // record back-links elsewhere
    assert_eq!(
        pg.ir_err(),
        IrReadError::NodeBacklinkMismatch {
            decl: MAT_A,
            record_decl: DeclId::new(42)
        }
    );
}

#[test]
fn missing_edge_mapping_is_typed() {
    // u: Edge decl whose edge record is never bound.
    let pg = Pg::build(Perturb::UnmappedEdge(UNIT_U));
    assert_eq!(
        pg.ir_err(),
        IrReadError::MissingEdgeMapping { decl: UNIT_U }
    );
}

#[test]
fn mapped_but_missing_edge_record_is_typed() {
    // u maps to an edge id for which no record exists.
    let pg = Pg::build(Perturb::EdgeRecord(UNIT_U, EdgeId::new(777)));
    assert_eq!(
        pg.ir_err(),
        IrReadError::MissingEdgeRecord {
            decl: UNIT_U,
            edge: EdgeId::new(777)
        }
    );
}

#[test]
fn edge_backlink_mismatch_is_typed() {
    let mut pg = Pg::valid();
    let eid = pg.ir.as_edge(UNIT_U).unwrap();
    pg.ir.edges[eid.raw()].decl = DeclId::new(9);
    assert_eq!(
        pg.ir_err(),
        IrReadError::EdgeBacklinkMismatch {
            decl: UNIT_U,
            record_decl: DeclId::new(9)
        }
    );
}

// ── malformed: reference / arc layer ───────────────────────────────────────

#[test]
fn dangling_base_target_is_typed() {
    let mut pg = Pg::valid();
    // A's <isa> base points at a declaration that does not exist.
    let nid = pg.nid_of(MAT_A);
    pg.ir.nodes[nid.raw()].bases.clear();
    pg.set_isa_base(MAT_A, DeclId::new(500));
    assert_eq!(
        pg.ir_err(),
        IrReadError::DanglingBaseTarget {
            base: DeclId::new(500)
        }
    );
}

#[test]
fn dangling_arc_record_is_typed() {
    let mut pg = Pg::valid();
    let eid = pg.ir.as_edge(UNIT_U).unwrap();
    pg.ir.edges[eid.raw()].arcs = vec![HyperArcId::new(999)]; // no such arc
    assert_eq!(
        pg.ir_err(),
        IrReadError::DanglingArcRecord {
            edge: UNIT_U,
            arc: HyperArcId::new(999)
        }
    );
}

#[test]
fn arc_ownership_mismatch_is_typed() {
    let mut pg = Pg::valid();
    // The arc under u claims to be owned by a different edge (process).
    // aid 0 is u's only arc.
    pg.ir.arcs[0].in_edge = PROCESS;
    assert_eq!(
        pg.ir_err(),
        IrReadError::ArcOwnershipMismatch {
            edge: UNIT_U,
            arc: HyperArcId::new(0),
            owner: PROCESS
        }
    );
}

#[test]
fn dangling_incidence_target_is_typed() {
    let mut pg = Pg::valid();
    // Make u an *explicit* unit (u <isa> process) so it is classified even though
    // its incidence is not all-material — otherwise the structural rule would
    // silently drop it before build_incidence ever reads the dangling target.
    pg.set_isa_base_edge(UNIT_U, PROCESS);
    // u's arc references a material id that does not exist.
    pg.ir.arcs[0].refs = vec![SignedRefR::Minus(RefAtomR {
        target: DeclId::new(600),
        anno: AnnoR::default(),
        weights: None,
    })];
    assert_eq!(
        pg.ir_err(),
        IrReadError::DanglingIncidenceTarget {
            edge: UNIT_U,
            target: DeclId::new(600)
        }
    );
}

// ── inheritance cycle: decision — cycle-safe, deterministic, preserved ──────

#[test]
fn inheritance_cycle_terminates_and_is_not_an_error() {
    // A <isa> P and P <isa> A. Neither reaches raw/product through the cycle,
    // so both are dropped from the P-graph — the same deterministic result the
    // visited-set walk produced before the migration. The point of this test is
    // that lowering TERMINATES and does not error on a cyclic <isa>.
    let mut pg = Pg::valid();
    let na = pg.nid_of(MAT_A);
    let np = pg.nid_of(MAT_P);
    pg.ir.nodes[na.raw()].bases.clear();
    pg.ir.nodes[np.raw()].bases.clear();
    pg.set_isa_base(MAT_A, MAT_P);
    pg.set_isa_base(MAT_P, MAT_A);
    let g = pg.lower().expect("a cyclic <isa> lowers without error");
    // Neither reaches a material archetype, so neither is a material; the unit's
    // structural rule then also fails (its targets are not materials), so the
    // unit is dropped too. Empty P-graph, deterministically.
    assert!(g.materials.is_empty(), "cyclic isa reaches no archetype");
}

// ── inheritance depth ladder (no recursion stack limit) ────────────────────

fn deep_chain(depth: usize) -> Pg {
    // A0 <isa> raw ; A1 <isa> A0 ; … ; A{depth-1} <isa> A{depth-2}
    // Every A{i} therefore reaches `raw` and is a raw material.
    let mut pg = Pg::valid();
    let mut prev = RAW;
    for i in 0..depth {
        prev = pg.add_node_decl(&format!("D{i}"), prev);
    }
    pg
}

#[test]
fn inheritance_depth_ladder_resolves_without_stack_overflow() {
    for depth in [64usize, 65, 128, 1024] {
        let pg = deep_chain(depth);
        let g = pg
            .lower()
            .unwrap_or_else(|e| panic!("depth {depth} lowers: {e:?}"));
        // The original A, P plus `depth` new raw materials D0..D{depth-1}.
        assert!(
            g.materials.len() >= depth,
            "depth {depth}: {} materials",
            g.materials.len()
        );
    }
}

// ── ordering & sign are semantic (non-symmetric fixtures) ──────────────────

/// A unit whose incidence is `-A +P` puts A on the input side and P on the
/// output side. This fixture is deliberately **non-symmetric**: swapping the two
/// signs (below) must change the result, so a test that passed on an all-positive
/// or commutative fixture would prove nothing.
#[test]
fn sign_direction_is_preserved_minus_input_plus_output() {
    let g = Pg::valid().lower().expect("valid");
    let u = g.name_to_decl["u"];
    let a = g.name_to_decl["A"];
    let p = g.name_to_decl["P"];
    assert_eq!(
        g.inputs(u),
        &std::collections::BTreeSet::from([a]),
        "-A is input"
    );
    assert_eq!(
        g.outputs(u),
        &std::collections::BTreeSet::from([p]),
        "+P is output"
    );
}

#[test]
fn swapping_signs_swaps_input_and_output() {
    let mut pg = Pg::valid();
    // Rebuild u's arc with the signs flipped: +A -P.
    pg.ir.arcs[0].refs = vec![
        SignedRefR::Plus(RefAtomR {
            target: MAT_A,
            anno: AnnoR::default(),
            weights: None,
        }),
        SignedRefR::Minus(RefAtomR {
            target: MAT_P,
            anno: AnnoR::default(),
            weights: None,
        }),
    ];
    let g = pg.lower().expect("valid");
    let u = g.name_to_decl["u"];
    let a = g.name_to_decl["A"];
    let p = g.name_to_decl["P"];
    // Flipped from the baseline: now A is output, P is input.
    assert_eq!(
        g.inputs(u),
        &std::collections::BTreeSet::from([p]),
        "-P is now input"
    );
    assert_eq!(
        g.outputs(u),
        &std::collections::BTreeSet::from([a]),
        "+A is now output"
    );
}

/// Two arcs under one edge, read in declaration order, then their refs in slot
/// order. A material appearing across both arcs is still collected once per ref;
/// the walk must not sort, dedup or reorder. Here u2 consumes A via arc0 and
/// produces P via arc1 — split across two arcs rather than one — and the result
/// must match the single-arc baseline direction.
#[test]
fn multi_arc_multi_ref_traversal_preserves_order_and_direction() {
    let mut pg = Pg::valid();
    // Replace u's single arc with two arcs: [-A] then [+P].
    let eid = pg.ir.as_edge(UNIT_U).unwrap();
    pg.ir.edges[eid.raw()].arcs.clear();
    pg.ir.arcs.clear();
    pg.add_unit_arc(UNIT_U, &[(MAT_A, -1)]);
    pg.add_unit_arc(UNIT_U, &[(MAT_P, 1)]);
    let g = pg.lower().expect("valid");
    let u = g.name_to_decl["u"];
    let a = g.name_to_decl["A"];
    let p = g.name_to_decl["P"];
    assert_eq!(g.inputs(u), &std::collections::BTreeSet::from([a]));
    assert_eq!(g.outputs(u), &std::collections::BTreeSet::from([p]));
}

// ── IrReadError Display: every variant renders ids only ────────────────────

#[test]
fn ir_read_error_display_covers_every_variant() {
    use hymeko::common::ids::{EdgeId, HyperArcId, NodeId};
    let cases = [
        IrReadError::DanglingDeclaration {
            decl: DeclId::new(1),
        },
        IrReadError::MissingNodeMapping {
            decl: DeclId::new(2),
        },
        IrReadError::MissingEdgeMapping {
            decl: DeclId::new(3),
        },
        IrReadError::MissingNodeRecord {
            decl: DeclId::new(4),
            node: NodeId::new(5),
        },
        IrReadError::MissingEdgeRecord {
            decl: DeclId::new(6),
            edge: EdgeId::new(7),
        },
        IrReadError::NodeBacklinkMismatch {
            decl: DeclId::new(8),
            record_decl: DeclId::new(9),
        },
        IrReadError::EdgeBacklinkMismatch {
            decl: DeclId::new(10),
            record_decl: DeclId::new(11),
        },
        IrReadError::DanglingArcRecord {
            edge: DeclId::new(12),
            arc: HyperArcId::new(13),
        },
        IrReadError::ArcOwnershipMismatch {
            edge: DeclId::new(14),
            arc: HyperArcId::new(15),
            owner: DeclId::new(16),
        },
        IrReadError::DanglingBaseTarget {
            base: DeclId::new(17),
        },
        IrReadError::DanglingIncidenceTarget {
            edge: DeclId::new(18),
            target: DeclId::new(19),
        },
    ];
    for c in cases {
        let s = format!("{c}");
        assert!(!s.is_empty());
        // Wrapping through MetaResolveError keeps the typed source (Error::source).
        let wrapped: MetaResolveError = c.into();
        assert!(format!("{wrapped}").contains("malformed IR"));
        assert!(std::error::Error::source(&wrapped).is_some());
    }
}

// ── domain-error paths (existing semantics, now exercised) ─────────────────

#[test]
fn conflicting_role_when_a_material_is_both_raw_and_product() {
    let mut pg = Pg::valid();
    // A already <isa> raw; add <isa> product too.
    pg.set_isa_base(MAT_A, PRODUCT);
    match pg.lower() {
        Err(MetaResolveError::ConflictingRole { material }) => assert_eq!(material, "A"),
        other => panic!("expected ConflictingRole, got {other:?}"),
    }
}

#[test]
fn non_material_incidence_is_a_domain_error_not_a_storage_fault() {
    // u <isa> process (explicit unit) with an incidence on `process` — a real
    // declaration that is not a material.
    let mut pg = Pg::valid();
    pg.set_isa_base_edge(UNIT_U, PROCESS);
    pg.ir.arcs[0].refs = vec![SignedRefR::Minus(RefAtomR {
        target: PROCESS,
        anno: AnnoR::default(),
        weights: None,
    })];
    match pg.lower() {
        Err(MetaResolveError::NonMaterialIo { unit, target }) => {
            assert_eq!(unit, "u");
            assert_eq!(target, "process");
        }
        other => panic!("expected NonMaterialIo, got {other:?}"),
    }
}

#[test]
fn neutral_reference_is_rejected() {
    let mut pg = Pg::valid();
    pg.set_isa_base_edge(UNIT_U, PROCESS);
    pg.ir.arcs[0].refs = vec![SignedRefR::Neutral(RefAtomR {
        target: MAT_A,
        anno: AnnoR::default(),
        weights: None,
    })];
    match pg.lower() {
        Err(MetaResolveError::NeutralRef { unit }) => assert_eq!(unit, "u"),
        other => panic!("expected NeutralRef, got {other:?}"),
    }
}

#[test]
fn scalar_cost_is_read_from_the_unit_annotation() {
    use hymeko::ir::ir::ValueR;
    let mut pg = Pg::valid();
    // Annotate u's declaration with a numeric cost.
    pg.ir.decl_nodes[UNIT_U.raw()].anno.value = Some(ValueR::Num(7.5));
    let g = pg.lower().expect("valid");
    assert_eq!(g.costs[&UNIT_U], 7.5);
}

#[test]
fn ambiguous_archetype_when_two_declarations_share_the_name() {
    let mut pg = Pg::valid();
    // Add a second "raw" Node declaration → find_decl sees two.
    let sym = pg.it.intern("raw");
    let d = pg.ir.push_decl(decl_node(sym, DeclKind::Node));
    let nid = NodeId::new(pg.ir.nodes.len());
    pg.ir.nodes.push(NodeRec::new(d, Vec::new()));
    pg.ir.bind_node_decl(d, nid).expect("bind node slot");
    match pg.lower() {
        Err(MetaResolveError::AmbiguousArchetype("raw")) => {}
        other => panic!("expected AmbiguousArchetype(raw), got {other:?}"),
    }
}

#[test]
fn a_base_without_the_isa_tag_is_skipped() {
    // A material whose only base lacks the <isa> tag reaches no archetype and is
    // dropped — the `continue` in the ancestry walk, exercised.
    let mut pg = Pg::valid();
    let na = pg.nid_of(MAT_A);
    pg.ir.nodes[na.raw()].bases.clear();
    // A base to `raw` but WITHOUT the isa tag.
    pg.ir.nodes[na.raw()]
        .bases
        .push(SignedRefR::Neutral(RefAtomR {
            target: RAW,
            anno: AnnoR::default(),
            weights: None,
        }));
    let g = pg.lower().expect("valid");
    // A no longer classifies as a material (its base is not <isa>).
    assert!(!g.materials.contains(&MAT_A));
}

#[test]
fn a_program_without_an_isa_symbol_yields_no_materials() {
    // Build the archetypes but never intern "isa": the ancestry walk short-%
    // circuits (`isa.is_none()`), so nothing classifies.
    use hymeko::ir::ir::Ir;
    let mut it = Interner::new();
    let mut ir = Ir::default();
    for (name, kind) in [
        ("raw", DeclKind::Node),
        ("product", DeclKind::Node),
        ("intermediate", DeclKind::Node),
        ("process", DeclKind::Edge),
        ("A", DeclKind::Node),
    ] {
        let sym = it.intern(name);
        ir.push_decl(decl_node(sym, kind));
    }
    for d in [0usize, 1, 2, 4] {
        let nid = NodeId::new(ir.nodes.len());
        ir.nodes.push(NodeRec::new(DeclId::new(d), Vec::new()));
        ir.bind_node_decl(DeclId::new(d), nid)
            .expect("bind node slot");
    }
    let eid = EdgeId::new(ir.edges.len());
    ir.edges.push(EdgeRec::new(DeclId::new(3), Vec::new()));
    ir.bind_edge_decl(DeclId::new(3), eid)
        .expect("bind edge slot");
    // "isa" is never interned, so get_id("isa") is None inside meta_resolve.
    let g = lower_resolved(&ir, &it).expect("lowers with no isa");
    assert!(g.materials.is_empty());
    assert!(g.units.is_empty());
}

#[test]
fn an_isa_base_pointing_at_a_hyperarc_declaration_contributes_no_bases() {
    // A material whose <isa> base targets a HyperArc declaration: the ancestry
    // walk visits it, `bases_of` returns the empty slice for a hyper-arc, and the
    // walk terminates. A never reaches raw, so it is dropped — no error.
    let mut pg = Pg::valid();
    // Append a HyperArc declaration.
    let arc_sym = pg.it.intern("some_arc");
    let arc_decl = pg.ir.push_decl(decl_node(arc_sym, DeclKind::HyperArc));
    // A <isa> some_arc (instead of raw).
    let na = pg.nid_of(MAT_A);
    pg.ir.nodes[na.raw()].bases.clear();
    pg.set_isa_base(MAT_A, arc_decl);
    let g = pg
        .lower()
        .expect("a hyper-arc isa base lowers without error");
    assert!(
        !g.materials.contains(&MAT_A),
        "A reaches no material archetype"
    );
}

#[test]
fn unit_arc_refs_reports_an_edge_backlink_mismatch_without_isa() {
    // With no `isa` symbol the ancestry walk short-circuits before `bases_of`,
    // so `unit_arc_refs` is the first reader of the edge record and its own
    // back-link check fires. Build a minimal no-isa program with one corrupt
    // edge record.
    use hymeko::ir::ir::Ir;
    let mut it = Interner::new();
    let mut ir = Ir::default();
    for (name, kind) in [
        ("raw", DeclKind::Node),
        ("product", DeclKind::Node),
        ("intermediate", DeclKind::Node),
        ("process", DeclKind::Edge),
        ("u", DeclKind::Edge),
    ] {
        let sym = it.intern(name);
        ir.push_decl(decl_node(sym, kind));
    }
    for d in [0usize, 1, 2] {
        let nid = NodeId::new(ir.nodes.len());
        ir.nodes.push(NodeRec::new(DeclId::new(d), Vec::new()));
        ir.bind_node_decl(DeclId::new(d), nid)
            .expect("bind node slot");
    }
    // process (decl 3) and u (decl 4) as edge records; u's record back-links
    // to a different declaration.
    let e_proc = EdgeId::new(ir.edges.len());
    ir.edges.push(EdgeRec::new(DeclId::new(3), Vec::new()));
    ir.bind_edge_decl(DeclId::new(3), e_proc)
        .expect("bind edge slot");
    let e_u = EdgeId::new(ir.edges.len());
    ir.edges.push(EdgeRec::new(DeclId::new(99), Vec::new())); // wrong back-link
    ir.bind_edge_decl(DeclId::new(4), e_u)
        .expect("bind edge slot");

    match lower_resolved(&ir, &it) {
        Err(MetaResolveError::Ir(IrReadError::EdgeBacklinkMismatch { decl, record_decl })) => {
            assert_eq!(decl, DeclId::new(4));
            assert_eq!(record_decl, DeclId::new(99));
        }
        other => panic!("expected EdgeBacklinkMismatch, got {other:?}"),
    }
}
