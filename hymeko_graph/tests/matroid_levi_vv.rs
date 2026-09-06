//! P3.4 §3 — verification & validation for the **implicit incidence/Levi graphic matroid**. Independent
//! oracles only: a from-scratch union-find over the combined `N + A` coordinate space, exhaustive subset
//! enumeration, and the engine's own `connected_components` as the §5.3 differential oracle. The Levi and
//! node-component counts are reconciled by the identity `C_levi = C_node + Z`, where `Z` is the number of
//! zero-node-incidence hyperarcs (each an extra Levi singleton). Ground elements are exact incidence
//! coordinates `(HyperArcId, slot)`; a non-node target is a typed error, never a silent skip.

use std::collections::BTreeSet;

use hymeko::common::ids::HyperArcId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::{
    GraphicMatroidWorkspace, IncrementalLevi, LeviCoord, MatroidError, MatroidRef,
    TraversalWorkspace, connected_components, levi_components, levi_independent, levi_rank,
    levi_spanning_basis,
};

#[path = "informed_common/mod.rs"]
mod ic;
use ic::SignMode;

// ---------------------------------------------------------------------------------------------------
// Independent oracles (from-scratch; never call into the matroid module).
// ---------------------------------------------------------------------------------------------------

/// The node index a reference targets (via the public `Ir::as_node`), or `None` if it is not a node.
fn target_node(ir: &Ir, arc: usize, slot: usize) -> Option<usize> {
    ir.as_node(ir.arcs[arc].refs[slot].target())
        .map(|n| n.raw())
}

/// Every incidence coordinate whose target is a node (the Levi ground set), in canonical order.
fn all_node_coords(ir: &Ir) -> Vec<LeviCoord> {
    let mut v = Vec::new();
    for (ai, arc) in ir.arcs.iter().enumerate() {
        for slot in 0..arc.refs.len() {
            if target_node(ir, ai, slot).is_some() {
                v.push(LeviCoord {
                    arc: HyperArcId::new(ai),
                    slot: slot as u32,
                });
            }
        }
    }
    v
}

struct OracleDsu {
    parent: Vec<usize>,
}
impl OracleDsu {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] == x {
            x
        } else {
            let r = self.find(self.parent[x]);
            self.parent[x] = r;
            r
        }
    }
    fn union(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            false
        } else {
            self.parent[ra] = rb;
            true
        }
    }
}

/// `(vertex_count, rank, components)` over the implicit Levi graph via an independent DSU on `N + A`
/// coordinates (node `n` → `n`, hyperarc `a` → `N + a`).
fn oracle_levi(ir: &Ir) -> (usize, usize, usize) {
    let n = ir.nodes.len();
    let a = ir.arcs.len();
    let vtx = n + a;
    let mut dsu = OracleDsu::new(vtx);
    let mut rank = 0usize;
    for ai in 0..a {
        for slot in 0..ir.arcs[ai].refs.len() {
            if let Some(node) = target_node(ir, ai, slot)
                && dsu.union(node, n + ai)
            {
                rank += 1;
            }
        }
    }
    let roots: BTreeSet<usize> = (0..vtx).map(|x| dsu.find(x)).collect();
    (vtx, rank, roots.len())
}

/// Independence of a coordinate subset in the Levi matroid (acyclic in the implicit bipartite graph).
fn oracle_subset_independent(ir: &Ir, subset: &[LeviCoord]) -> bool {
    let n = ir.nodes.len();
    let mut dsu = OracleDsu::new(n + ir.arcs.len());
    for c in subset {
        match target_node(ir, c.arc.raw(), c.slot as usize) {
            Some(node) => {
                if !dsu.union(node, n + c.arc.raw()) {
                    return false;
                }
            }
            None => return false,
        }
    }
    true
}

/// Zero-node-incidence hyperarcs (each an extra Levi singleton component).
fn zero_incidence_arcs(ir: &Ir) -> usize {
    ir.arcs
        .iter()
        .enumerate()
        .filter(|(ai, arc)| (0..arc.refs.len()).all(|s| target_node(ir, *ai, s).is_none()))
        .count()
}

fn node_component_count(ir: &Ir) -> u32 {
    let idx = build_incidence_index(ir).unwrap();
    let mut tw = TraversalWorkspace::new();
    connected_components(&idx, &mut tw).unwrap().count()
}

fn mref<'a, 'ir>(idx: &'a hymeko::ir::incidence::IncidenceIndex<'ir>) -> MatroidRef<'a, 'ir> {
    MatroidRef::new(idx)
}

/// A cheap structural fingerprint of the Ir (to prove no matroid op mutates it).
fn fingerprint(ir: &Ir) -> (usize, usize, usize, u64) {
    let inc: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
    let sum: u64 = ir
        .arcs
        .iter()
        .flat_map(|a| a.refs.iter())
        .map(|r| r.target().raw() as u64)
        .sum();
    (ir.nodes.len(), ir.arcs.len(), inc, sum)
}

// ---------------------------------------------------------------------------------------------------
// §3.1 — the rank formula rank = (N+A) − C, triangulated; direct-Ir connectivity parity.
// ---------------------------------------------------------------------------------------------------

#[test]
fn levi_rank_formula_and_connectivity_parity() {
    let (two_sts, _) = ic::two_disjoint_sts_ir(3);
    let cases: Vec<(&str, Ir)> = vec![
        ("path8", ic::window_ir(8, 2, false, SignMode::AllPlus)),
        ("grid4x4", ic::grid_ir(4, 4)),
        ("three_comps", ic::multi_component_ir(3, 5, 2)),
        ("multiway3", ic::window_ir(8, 3, false, SignMode::AllPlus)),
        ("dup_path5", ic::dup_heavy_ir(5, 2)),
        ("sts31", ic::steiner_sts_ir(5)),
        ("two_sts7", two_sts),
    ];
    let mut ws = GraphicMatroidWorkspace::new();
    for (name, ir) in &cases {
        let (vtx, orank, oc) = oracle_levi(ir);
        let idx = build_incidence_index(ir).unwrap();
        let m = mref(&idx);

        let rank = levi_rank(m, &mut ws).unwrap();
        let comps = levi_components(m, &mut ws).unwrap();
        let mut basis = Vec::new();
        let bsize = levi_spanning_basis(m, &mut ws, &mut basis).unwrap();

        assert_eq!(rank, orank, "[{name}] levi_rank == oracle rank");
        assert_eq!(rank, vtx - oc, "[{name}] rank == (N+A) - C");
        assert_eq!(comps, oc, "[{name}] levi_components == oracle C");
        assert_eq!(bsize, rank, "[{name}] basis size == rank");
        assert_eq!(basis.len(), rank);
        assert!(
            oracle_subset_independent(ir, &basis),
            "[{name}] returned basis is acyclic"
        );

        // direct-Ir connectivity parity: C_levi = C_node + Z.
        let z = zero_incidence_arcs(ir);
        assert_eq!(
            comps,
            node_component_count(ir) as usize + z,
            "[{name}] C_levi == direct-Ir node components + zero-incidence arcs"
        );
    }
}

// ---------------------------------------------------------------------------------------------------
// §3.2 — incremental == full rebuild; deterministic output; workspace-reuse parity.
// ---------------------------------------------------------------------------------------------------

#[test]
fn levi_incremental_equals_full_rebuild() {
    let ir = ic::grid_ir(5, 4);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();

    let mut batch = Vec::new();
    levi_spanning_basis(mref(&idx), &mut ws, &mut batch).unwrap();

    let mut streamed = Vec::new();
    {
        let mut inc = IncrementalLevi::begin(&mut ws, &ir).unwrap();
        for c in all_node_coords(&ir) {
            if inc.offer(&ir, c.arc, c.slot).unwrap() {
                streamed.push(c);
            }
        }
        assert_eq!(
            inc.accepted(),
            batch.len(),
            "incremental accept count == basis size"
        );
    }
    assert_eq!(
        streamed, batch,
        "streamed basis identical to full rebuild (canonical order)"
    );
}

#[test]
fn levi_output_deterministic_and_workspace_reuse() {
    let a = ic::grid_ir(4, 4);
    let b = ic::window_ir(20, 2, false, SignMode::AllPlus);
    let (ia, ib) = (
        build_incidence_index(&a).unwrap(),
        build_incidence_index(&b).unwrap(),
    );
    let mut ws = GraphicMatroidWorkspace::new();

    let mut first = Vec::new();
    levi_spanning_basis(mref(&ia), &mut ws, &mut first).unwrap();
    let mut other = Vec::new();
    levi_spanning_basis(mref(&ib), &mut ws, &mut other).unwrap();
    let mut again = Vec::new();
    levi_spanning_basis(mref(&ia), &mut ws, &mut again).unwrap();

    let mut fresh_ws = GraphicMatroidWorkspace::new();
    let mut fresh = Vec::new();
    levi_spanning_basis(mref(&ia), &mut fresh_ws, &mut fresh).unwrap();

    assert_eq!(
        first, again,
        "reused workspace yields the identical Levi basis"
    );
    assert_eq!(
        first, fresh,
        "reused == fresh (no stale union state leaked)"
    );
}

#[test]
fn levi_incremental_rejects_a_foreign_snapshot() {
    let a = ic::grid_ir(3, 2);
    let foreign = ic::window_ir(6, 3, false, ic::SignMode::AllPlus);
    let mut ws = GraphicMatroidWorkspace::new();
    let mut stream = IncrementalLevi::begin(&mut ws, &a).unwrap();
    assert!(
        stream.offer(&foreign, HyperArcId::new(0), 0).is_err(),
        "an incremental Levi session must not accept coordinates from a foreign canonical snapshot"
    );
}

// ---------------------------------------------------------------------------------------------------
// §3.3 — independence axioms exhaustive on a small cyclic Levi instance.
// ---------------------------------------------------------------------------------------------------

#[test]
fn levi_independence_axioms_exhaustive_grid2x2() {
    // grid2x2: 4 nodes + 4 arcs (2-member each) → 8 Levi vertices, 8 incidence coordinates, one cycle.
    let ir = ic::grid_ir(2, 2);
    let coords = all_node_coords(&ir);
    assert!(
        coords.len() <= 20,
        "exhaustive enumeration must stay small (got {})",
        coords.len()
    );
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();

    let mut independent_sets: Vec<Vec<LeviCoord>> = Vec::new();
    for mask in 0u32..(1u32 << coords.len()) {
        let subset: Vec<LeviCoord> = (0..coords.len())
            .filter(|&b| (mask >> b) & 1 == 1)
            .map(|b| coords[b])
            .collect();
        let oracle = oracle_subset_independent(&ir, &subset);
        let matroid = levi_independent(mref(&idx), &subset, &mut ws).unwrap();
        assert_eq!(
            oracle, matroid,
            "matroid independence == acyclicity for {subset:?}"
        );
        if oracle {
            independent_sets.push(subset);
        }
    }

    assert!(
        oracle_subset_independent(&ir, &[]),
        "I1: empty set independent"
    );
    for s in &independent_sets {
        for drop in 0..s.len() {
            let sub: Vec<LeviCoord> = s
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != drop)
                .map(|(_, &c)| c)
                .collect();
            assert!(
                oracle_subset_independent(&ir, &sub),
                "I2: subset of independent is independent"
            );
        }
    }
    let max_rank = independent_sets.iter().map(|s| s.len()).max().unwrap();
    for a in &independent_sets {
        for b in &independent_sets {
            if a.len() >= b.len() {
                continue;
            }
            let aset: BTreeSet<(usize, u32)> = a.iter().map(|c| (c.arc.raw(), c.slot)).collect();
            let found = b
                .iter()
                .filter(|c| !aset.contains(&(c.arc.raw(), c.slot)))
                .any(|&c| {
                    let mut ext = a.clone();
                    ext.push(c);
                    oracle_subset_independent(&ir, &ext)
                });
            assert!(found, "I3: augmentation for A={a:?} B={b:?}");
        }
    }
    assert_eq!(
        levi_rank(mref(&idx), &mut ws).unwrap(),
        max_rank,
        "rank == max independent set"
    );
}

// ---------------------------------------------------------------------------------------------------
// §3.4 — inclusion policy: isolated node, zero-incidence hyperarc, duplicate incidence.
// ---------------------------------------------------------------------------------------------------

#[test]
fn levi_inclusion_policy_isolated_node_and_empty_arc() {
    // triangle on nodes {0,1,2} + one ISOLATED node 3 + one ZERO-INCIDENCE (empty-refs) hyperarc.
    let mut ir = Ir::default();
    let d: Vec<_> = (0..4).map(|_| ic::node_decl(&mut ir)).collect();
    ic::push_arc(
        &mut ir,
        vec![ic::plus_ref(d[0], None), ic::plus_ref(d[1], None)],
    );
    ic::push_arc(
        &mut ir,
        vec![ic::plus_ref(d[1], None), ic::plus_ref(d[2], None)],
    );
    ic::push_arc(
        &mut ir,
        vec![ic::plus_ref(d[2], None), ic::plus_ref(d[0], None)],
    );
    ic::push_arc(&mut ir, vec![]); // zero-incidence hyperarc (arity 0)

    // N=4, A=4, vtx=8. Triangle {0,1,2}+3 arcs = 1 component; node 3 isolated = +1; empty arc = +1.
    // Components: {0,1,2,arc0,arc1,arc2} , {node3} , {arc3} = 3. rank = 8 - 3 = 5.
    let (vtx, orank, oc) = oracle_levi(&ir);
    assert_eq!(
        (vtx, orank, oc),
        (8, 5, 3),
        "isolated node + empty arc each a singleton component"
    );
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    assert_eq!(levi_rank(mref(&idx), &mut ws).unwrap(), 5);
    assert_eq!(levi_components(mref(&idx), &mut ws).unwrap(), 3);
    // parity: C_node counts node 3 as a component; the empty arc is the one Z.
    assert_eq!(zero_incidence_arcs(&ir), 1);
    assert_eq!(
        3,
        node_component_count(&ir) as usize + zero_incidence_arcs(&ir)
    );
}

#[test]
fn levi_duplicate_incidence_preserved_but_second_is_dependent() {
    // one hyperarc with TWO references to the same node → two DISTINCT incidence coordinates that are
    // parallel edges in the Levi graph: the first joins, the second closes a cycle.
    let mut ir = Ir::default();
    let a = ic::node_decl(&mut ir);
    ic::push_arc(&mut ir, vec![ic::plus_ref(a, None), ic::plus_ref(a, None)]);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();

    // both coordinates exist in the ground set (arc 0, slots 0 and 1).
    assert_eq!(
        all_node_coords(&ir).len(),
        2,
        "both parallel incidence coordinates preserved"
    );
    // N=1, A=1, vtx=2; two parallel edges node0<->arc0 → rank 1, C=1.
    assert_eq!(
        levi_rank(mref(&idx), &mut ws).unwrap(),
        1,
        "second parallel incidence is dependent"
    );
    let mut inc = IncrementalLevi::begin(&mut ws, &ir).unwrap();
    assert!(
        inc.offer(&ir, HyperArcId::new(0), 0).unwrap(),
        "first incidence joins"
    );
    assert!(
        !inc.offer(&ir, HyperArcId::new(0), 1).unwrap(),
        "parallel incidence closes a cycle"
    );
}

// ---------------------------------------------------------------------------------------------------
// §3.5 — typed rejections: a non-node/dangling target is never silently skipped.
// ---------------------------------------------------------------------------------------------------

#[test]
fn levi_non_node_target_rejected() {
    let mut ir = Ir::default();
    let node = ic::node_decl(&mut ir);
    let edge = ic::edge_decl(&mut ir); // not a node
    ic::push_arc(
        &mut ir,
        vec![ic::plus_ref(node, None), ic::plus_ref(edge, None)],
    );
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    let err = levi_spanning_basis(mref(&idx), &mut ws, &mut out).unwrap_err();
    assert!(
        matches!(err, MatroidError::DanglingTarget { .. }),
        "non-node target rejected, got {err:?}"
    );
}

#[test]
fn levi_dangling_coordinate_rejected() {
    let ir = ic::grid_ir(3, 3);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    // a coordinate whose slot is out of range for its arc.
    let bad = [LeviCoord {
        arc: HyperArcId::new(0),
        slot: 99,
    }];
    let err = levi_independent(mref(&idx), &bad, &mut ws).unwrap_err();
    assert!(
        matches!(err, MatroidError::DanglingTarget { .. }),
        "out-of-range slot rejected, got {err:?}"
    );
}

// ---------------------------------------------------------------------------------------------------
// §3.6 — the Ir is not mutated; STS(1023) large-instance formula.
// ---------------------------------------------------------------------------------------------------

#[test]
fn levi_operations_do_not_mutate_the_ir() {
    let ir = ic::steiner_sts_ir(5); // STS(31)
    let before = fingerprint(&ir);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    let _ = levi_rank(mref(&idx), &mut ws).unwrap();
    let _ = levi_components(mref(&idx), &mut ws).unwrap();
    let _ = levi_spanning_basis(mref(&idx), &mut ws, &mut out).unwrap();
    assert_eq!(
        before,
        fingerprint(&ir),
        "Levi operations do not mutate the Ir"
    );
}

#[test]
fn levi_sts_1023_formula() {
    // STS(1023): v=1023 nodes, blocks=174251 hyperarcs, incidences=522753. Connected (a 2-design) → C=1,
    // so Levi rank = (N + A) - 1 = 1023 + 174251 - 1 = 175273.
    let (v, blocks, incidences, _deg) = ic::sts_expected_counts(10);
    assert_eq!((v, blocks, incidences), (1023, 174251, 522753));
    let ir = ic::steiner_sts_ir(10);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut basis = Vec::new();
    let rank = levi_spanning_basis(mref(&idx), &mut ws, &mut basis).unwrap();
    assert_eq!(
        levi_components(mref(&idx), &mut ws).unwrap(),
        1,
        "STS(1023) Levi graph connected"
    );
    assert_eq!(
        rank,
        v + blocks - 1,
        "Levi rank = (N+A) - 1 for a connected design"
    );
    assert_eq!(rank, 175273);
    assert_eq!(basis.len(), rank);
}
