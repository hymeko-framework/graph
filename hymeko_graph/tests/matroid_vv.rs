//! P3.3 §5 — verification & validation for the zero-copy **rank-2 graphic matroid** over the canonical
//! `Ir` + `IncidenceIndex`. Every claim is checked against an **independent** oracle written in this file
//! (a from-scratch union-find, exhaustive subset enumeration, an independent Kruskal) and, for
//! connectivity, against the engine's own `connected_components` (the §5.3 differential oracle). Nothing
//! here trusts the matroid's own DSU to certify the matroid's own DSU.
//!
//! Matroid facts verified: the rank formula `rank = N − C`; the independence axioms (I1 non-empty/empty
//! independent, I2 hereditary, I3 augmentation/exchange) exhaustively on a small instance; greedy == the
//! weighted optimum (Kruskal); incremental == full rebuild; workspace-reuse determinism; arc-permutation
//! invariance of rank and optimum weight; and the typed rejections (multiway, self-loop, dangling target,
//! non-finite weight) that keep a hyperarc from being silently misread as a pairwise edge.

use std::collections::BTreeSet;

use hymeko::common::ids::{HyperArcId, SymId};
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::{AnnoR, DeclKind, DeclNode, Ir};
use hymeko_graph::{
    GraphicMatroidWorkspace, IncrementalRank2, MatroidError, MatroidRef, TraversalWorkspace,
    connected_components, rank2_rank, rank2_spanning_forest, rank2_weighted_basis,
};

#[path = "informed_common/mod.rs"]
mod ic;
use ic::SignMode;
use ic::{node_decl as one_node_decl, plus_ref, push_arc, weighted_graph_ir};

// ---------------------------------------------------------------------------------------------------
// Independent oracles (from-scratch; never call into the matroid module).
// ---------------------------------------------------------------------------------------------------

/// The two node-decl indices of a 2-member arc (the test graphs are all rank-2).
fn oracle_endpoints(ir: &Ir, arc: usize) -> (usize, usize) {
    let a = &ir.arcs[arc];
    let ns: Vec<usize> = a.refs.iter().map(|r| r.target().raw()).collect();
    assert_eq!(ns.len(), 2, "oracle only applies to 2-member arcs");
    (ns[0], ns[1])
}

/// The first numeric weight of an arc's first reference (mirrors the module's `arc_weight` on the
/// weighted test graphs, where every reference carries a weight), defaulting to 1.0.
fn oracle_weight(ir: &Ir, arc: usize) -> f64 {
    ic::first_num_weight_pub(ir, &ir.arcs[arc].refs[0])
}

/// A from-scratch union-find, textbook (recursive find + path compression), sharing no code with the
/// matroid's generation-stamped DSU.
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

/// (forest size, component count) over **all** 2-member arcs, via the independent DSU.
fn oracle_forest_and_components(ir: &Ir) -> (usize, usize) {
    let n = ir.nodes.len();
    let mut dsu = OracleDsu::new(n);
    let mut forest = 0usize;
    for arc in 0..ir.arcs.len() {
        let (u, v) = oracle_endpoints(ir, arc);
        if dsu.union(u, v) {
            forest += 1;
        }
    }
    let roots: BTreeSet<usize> = (0..n).map(|x| dsu.find(x)).collect();
    (forest, roots.len())
}

/// Is this subset of arc indices independent in the graphic matroid (i.e. acyclic)? Independent oracle.
fn oracle_subset_independent(ir: &Ir, subset: &[usize]) -> bool {
    let mut dsu = OracleDsu::new(ir.nodes.len());
    for &arc in subset {
        let (u, v) = oracle_endpoints(ir, arc);
        if !dsu.union(u, v) {
            return false;
        }
    }
    true
}

/// Weighted minimum spanning forest total via an independent Kruskal (min if `minimize`, else max).
fn oracle_kruskal_total(ir: &Ir, minimize: bool) -> f64 {
    let mut edges: Vec<(f64, usize, usize)> = (0..ir.arcs.len())
        .map(|a| {
            let (u, v) = oracle_endpoints(ir, a);
            (oracle_weight(ir, a), u, v)
        })
        .collect();
    edges.sort_by(|x, y| {
        if minimize {
            x.0.total_cmp(&y.0)
        } else {
            y.0.total_cmp(&x.0)
        }
    });
    let mut dsu = OracleDsu::new(ir.nodes.len());
    let mut total = 0.0;
    for (w, u, v) in edges {
        if dsu.union(u, v) {
            total += w;
        }
    }
    total
}

/// The engine's own component count (the §5.3 differential oracle) over the incidence index.
fn engine_component_count(ir: &Ir) -> u32 {
    let idx = build_incidence_index(ir).unwrap();
    let mut tw = TraversalWorkspace::new();
    connected_components(&idx, &mut tw).unwrap().count()
}

// ---- small inline builders for the fault cases ----------------------------------------------------

fn mref<'a, 'ir>(idx: &'a hymeko::ir::incidence::IncidenceIndex<'ir>) -> MatroidRef<'a, 'ir> {
    MatroidRef::new(idx)
}

// ---------------------------------------------------------------------------------------------------
// §5.2 — the rank formula rank = N − C, triangulated three ways.
// ---------------------------------------------------------------------------------------------------

#[test]
fn rank_equals_n_minus_components_across_families() {
    let cases: Vec<(&str, Ir)> = vec![
        ("path8", ic::window_ir(8, 2, false, SignMode::AllPlus)),
        ("grid4x4", ic::grid_ir(4, 4)),
        ("grid6x3", ic::grid_ir(6, 3)),
        ("three_comps", ic::multi_component_ir(3, 5, 2)),
        ("dup_path5", ic::dup_heavy_ir(5, 2)),
    ];
    let mut ws = GraphicMatroidWorkspace::new();
    for (name, ir) in &cases {
        let n = ir.nodes.len();
        let idx = build_incidence_index(ir).unwrap();
        let m = mref(&idx);

        let matroid_rank = rank2_rank(m, &mut ws).unwrap();
        let (oracle_forest, oracle_c) = oracle_forest_and_components(ir);
        let engine_c = engine_component_count(ir) as usize;

        assert_eq!(oracle_c, engine_c, "[{name}] independent DSU C == engine C");
        assert_eq!(matroid_rank, n - oracle_c, "[{name}] rank == N - C");
        assert_eq!(
            matroid_rank, oracle_forest,
            "[{name}] rank == independent forest size"
        );

        let mut basis = Vec::new();
        let basis_size = rank2_spanning_forest(m, &mut ws, &mut basis).unwrap();
        assert_eq!(basis_size, matroid_rank, "[{name}] |basis| == rank");
        assert_eq!(
            basis.len(),
            matroid_rank,
            "[{name}] basis vec length == rank"
        );
        assert!(
            oracle_subset_independent(ir, &basis.iter().map(|a| a.raw()).collect::<Vec<_>>()),
            "[{name}] the returned basis is actually acyclic"
        );
    }
}

#[test]
fn rank_correct_with_offset_node_decl_ids() {
    // A graph whose node DeclIds are NOT equal to their NodeIds: a non-node (Edge) decl is pushed FIRST,
    // so the three node decls get DeclIds 1,2,3 while their NodeIds are 0,1,2. This discriminates a
    // correct NodeId-keyed DSU from a bug that used the raw DeclId as the dense DSU coordinate (which
    // would index out of the N-sized DSU). A triangle → rank 2.
    let mut ir = Ir::default();
    let _edge = ir.push_decl(DeclNode::root(
        DeclKind::Edge,
        SymId::new(0),
        AnnoR::default(),
    ));
    let a = one_node_decl(&mut ir);
    let b = one_node_decl(&mut ir);
    let c = one_node_decl(&mut ir);
    assert_ne!(
        a.raw(),
        0,
        "node DeclId is offset from its NodeId by the leading edge decl"
    );
    push_arc(&mut ir, vec![plus_ref(a, None), plus_ref(b, None)]);
    push_arc(&mut ir, vec![plus_ref(b, None), plus_ref(c, None)]);
    push_arc(&mut ir, vec![plus_ref(c, None), plus_ref(a, None)]);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    assert_eq!(
        rank2_rank(mref(&idx), &mut ws).unwrap(),
        2,
        "triangle rank = 3 nodes - 1 comp"
    );
    assert_eq!(engine_component_count(&ir), 1);
}

#[test]
fn empty_arc_set_has_rank_zero() {
    let mut ir = Ir::default();
    for _ in 0..6 {
        one_node_decl(&mut ir);
    }
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut basis = Vec::new();
    assert_eq!(rank2_rank(mref(&idx), &mut ws).unwrap(), 0);
    assert_eq!(
        rank2_spanning_forest(mref(&idx), &mut ws, &mut basis).unwrap(),
        0
    );
    assert!(basis.is_empty());
    assert_eq!(
        engine_component_count(&ir),
        6,
        "6 isolated nodes = 6 components"
    );
}

// ---------------------------------------------------------------------------------------------------
// §5.1 — the independence axioms, exhaustive on a small cyclic instance.
// ---------------------------------------------------------------------------------------------------

/// Does the matroid consider this arc subset independent? (feed it incrementally; independent iff no
/// offered arc closes a cycle). Uses only the public matroid API.
fn matroid_subset_independent(ir: &Ir, ws: &mut GraphicMatroidWorkspace, subset: &[usize]) -> bool {
    let mut inc = IncrementalRank2::begin(ws, ir.nodes.len()).unwrap();
    for &arc in subset {
        if !inc.offer(ir, HyperArcId::new(arc)).unwrap() {
            return false;
        }
    }
    true
}

#[test]
fn independence_axioms_exhaustive_grid2x3() {
    // grid_ir(2,3): 6 nodes, 7 edges (3 right + 4 down), two independent cycles → a genuine matroid.
    let ir = ic::grid_ir(2, 3);
    let n_arcs = ir.arcs.len();
    assert!(
        n_arcs <= 20,
        "exhaustive enumeration must stay small (got {n_arcs})"
    );
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();

    // enumerate every subset once; record the independent ones and the matroid/oracle agreement.
    let mut independent_sets: Vec<Vec<usize>> = Vec::new();
    for mask in 0u32..(1u32 << n_arcs) {
        let subset: Vec<usize> = (0..n_arcs).filter(|&b| (mask >> b) & 1 == 1).collect();
        let oracle = oracle_subset_independent(&ir, &subset);
        let matroid = matroid_subset_independent(&ir, &mut ws, &subset);
        assert_eq!(
            oracle, matroid,
            "matroid independence == acyclicity for subset {subset:?}"
        );
        if oracle {
            independent_sets.push(subset);
        }
    }

    // (I1) the empty set is independent.
    assert!(oracle_subset_independent(&ir, &[]), "empty set independent");

    // (I2) hereditary: every subset of an independent set is independent.
    for s in &independent_sets {
        for drop in 0..s.len() {
            let sub: Vec<usize> = s
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != drop)
                .map(|(_, &e)| e)
                .collect();
            assert!(
                oracle_subset_independent(&ir, &sub),
                "hereditary: subset of {s:?} independent"
            );
        }
    }

    // (I3) exchange: |A| < |B| independent ⇒ ∃ e ∈ B∖A with A∪{e} independent.
    let max_rank = independent_sets.iter().map(|s| s.len()).max().unwrap();
    for a in &independent_sets {
        for b in &independent_sets {
            if a.len() >= b.len() {
                continue;
            }
            let aset: BTreeSet<usize> = a.iter().copied().collect();
            let found = b.iter().filter(|e| !aset.contains(e)).any(|&e| {
                let mut ext = a.clone();
                ext.push(e);
                oracle_subset_independent(&ir, &ext)
            });
            assert!(found, "exchange axiom for A={a:?} B={b:?}");
        }
    }

    // the rank (max independent set size) matches rank2_rank.
    assert_eq!(
        rank2_rank(mref(&idx), &mut ws).unwrap(),
        max_rank,
        "rank == max independent set"
    );
}

// ---------------------------------------------------------------------------------------------------
// §5.4 — greedy == weighted optimum (Kruskal).
// ---------------------------------------------------------------------------------------------------

#[test]
fn weighted_basis_matches_independent_kruskal() {
    // a graph with cycles and distinct weights, so the MST must drop specific heavy edges.
    let cases: Vec<(&str, Ir)> = vec![
        (
            "diamond",
            weighted_graph_ir(
                4,
                &[
                    (0, 1, 1.0),
                    (1, 2, 2.0),
                    (2, 0, 3.0),
                    (2, 3, 4.0),
                    (0, 3, 5.0),
                ],
            ),
        ),
        (
            "k4",
            weighted_graph_ir(
                4,
                &[
                    (0, 1, 4.0),
                    (0, 2, 1.0),
                    (0, 3, 3.0),
                    (1, 2, 2.0),
                    (1, 3, 5.0),
                    (2, 3, 6.0),
                ],
            ),
        ),
        (
            "weighted_path",
            ic::window_ir(8, 2, true, SignMode::AllPlus),
        ),
    ];
    let mut ws = GraphicMatroidWorkspace::new();
    for (name, ir) in &cases {
        let idx = build_incidence_index(ir).unwrap();
        let m = mref(&idx);
        let mut out = Vec::new();

        let min_total = rank2_weighted_basis(m, true, &mut ws, &mut out).unwrap();
        let max_total = rank2_weighted_basis(m, false, &mut ws, &mut out).unwrap();
        assert!(
            (min_total - oracle_kruskal_total(ir, true)).abs() < 1e-9,
            "[{name}] min == Kruskal-min"
        );
        assert!(
            (max_total - oracle_kruskal_total(ir, false)).abs() < 1e-9,
            "[{name}] max == Kruskal-max"
        );
        assert!(min_total <= max_total + 1e-9, "[{name}] min <= max");

        // the min-weight basis is a spanning forest of the correct size.
        assert_eq!(
            out.len(),
            rank2_rank(m, &mut ws).unwrap(),
            "[{name}] weighted basis size == rank"
        );
    }
}

// ---------------------------------------------------------------------------------------------------
// §5.5 — incremental == full rebuild; workspace-reuse determinism; permutation invariance.
// ---------------------------------------------------------------------------------------------------

#[test]
fn incremental_stream_equals_full_rebuild() {
    let ir = ic::grid_ir(5, 4);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();

    let mut batch = Vec::new();
    rank2_spanning_forest(mref(&idx), &mut ws, &mut batch).unwrap();

    // stream the arcs in the same (canonical) order and collect the accepted ones.
    let mut streamed = Vec::new();
    {
        let mut inc = IncrementalRank2::begin(&mut ws, ir.nodes.len()).unwrap();
        for a in 0..ir.arcs.len() {
            let arc = HyperArcId::new(a);
            if inc.offer(&ir, arc).unwrap() {
                streamed.push(arc);
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
        "streamed basis identical to full-rebuild basis (order included)"
    );
}

#[test]
fn incremental_stream_rejects_a_foreign_snapshot_after_binding() {
    let a = ic::weighted_graph_ir(3, &[(0, 1, 1.0), (1, 2, 1.0)]);
    let foreign = ic::weighted_graph_ir(3, &[(0, 2, 1.0)]);
    let mut ws = GraphicMatroidWorkspace::new();
    let mut stream = IncrementalRank2::begin(&mut ws, a.nodes.len()).unwrap();
    assert!(stream.offer(&a, HyperArcId::new(0)).unwrap());
    assert!(
        stream.offer(&foreign, HyperArcId::new(0)).is_err(),
        "an incremental session must not mix canonical snapshots"
    );
}

#[test]
fn incremental_stream_rejects_an_out_of_range_arc_without_panicking() {
    let ir = ic::weighted_graph_ir(2, &[(0, 1, 1.0)]);
    let mut ws = GraphicMatroidWorkspace::new();
    let mut stream = IncrementalRank2::begin(&mut ws, ir.nodes.len()).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stream.offer(&ir, HyperArcId::new(ir.arcs.len()))
    }));
    assert!(
        result.is_ok(),
        "a public typed API must not panic on an unknown arc ID"
    );
    assert!(
        result.unwrap().is_err(),
        "an unknown arc ID must be a typed error"
    );
}

#[test]
fn incremental_stream_rejects_a_mismatched_node_universe_without_panicking() {
    let ir = ic::weighted_graph_ir(2, &[(0, 1, 1.0)]);
    let mut ws = GraphicMatroidWorkspace::new();
    let mut stream = IncrementalRank2::begin(&mut ws, 1).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stream.offer(&ir, HyperArcId::new(0))
    }));
    assert!(
        result.is_ok(),
        "a public typed API must not panic on a mismatched universe"
    );
    assert!(
        result.unwrap().is_err(),
        "an arc outside the declared node universe must be a typed error"
    );
}

#[test]
fn workspace_reuse_is_deterministic_across_interleaved_graphs() {
    let a = ic::grid_ir(4, 4);
    let b = ic::window_ir(20, 2, false, SignMode::AllPlus);
    let (ia, ib) = (
        build_incidence_index(&a).unwrap(),
        build_incidence_index(&b).unwrap(),
    );
    let mut ws = GraphicMatroidWorkspace::new();

    let mut first = Vec::new();
    rank2_spanning_forest(mref(&ia), &mut ws, &mut first).unwrap();

    // run a different-sized graph on the same workspace, then re-run the first.
    let mut other = Vec::new();
    rank2_spanning_forest(mref(&ib), &mut ws, &mut other).unwrap();
    let mut again = Vec::new();
    rank2_spanning_forest(mref(&ia), &mut ws, &mut again).unwrap();

    // and a truly fresh workspace, for the ground truth.
    let mut fresh_ws = GraphicMatroidWorkspace::new();
    let mut fresh = Vec::new();
    rank2_spanning_forest(mref(&ia), &mut fresh_ws, &mut fresh).unwrap();

    assert_eq!(
        first, again,
        "reused workspace yields the identical basis (generation reset works)"
    );
    assert_eq!(
        first, fresh,
        "reused == fresh (no stale union state leaked between graphs)"
    );
}

#[test]
fn rank_and_optimum_are_arc_permutation_invariant() {
    // a weighted graph with a unique MST weight; permuting arc order must not change rank or optimum.
    let base = weighted_graph_ir(
        6,
        &[
            (0, 1, 1.0),
            (1, 2, 2.0),
            (2, 3, 3.0),
            (3, 4, 4.0),
            (4, 5, 5.0),
            (0, 2, 6.0),
            (1, 3, 7.0),
            (2, 4, 8.0),
            (3, 5, 9.0),
            (0, 5, 10.0),
        ],
    );
    let idx = build_incidence_index(&base).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    let base_rank = rank2_rank(mref(&idx), &mut ws).unwrap();
    let base_min = rank2_weighted_basis(mref(&idx), true, &mut ws, &mut out).unwrap();

    // a deterministic reversal + rotation permutation of the arcs.
    let mut perm = base.clone();
    perm.arcs.reverse();
    perm.arcs.rotate_left(3);
    let pidx = build_incidence_index(&perm).unwrap();
    let perm_rank = rank2_rank(mref(&pidx), &mut ws).unwrap();
    let perm_min = rank2_weighted_basis(mref(&pidx), true, &mut ws, &mut out).unwrap();

    assert_eq!(base_rank, perm_rank, "rank is permutation invariant");
    assert!(
        (base_min - perm_min).abs() < 1e-9,
        "optimum weight is permutation invariant"
    );
}

// ---------------------------------------------------------------------------------------------------
// §5.6 — duplicate incidences counted once.
// ---------------------------------------------------------------------------------------------------

#[test]
fn parallel_edges_counted_once() {
    // dup_heavy_ir(5,2): a path 0-1-2-3-4 with every edge duplicated → 8 arcs, rank still 4.
    let ir = ic::dup_heavy_ir(5, 2);
    assert_eq!(ir.arcs.len(), 8, "4 path edges duplicated");
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut basis = Vec::new();
    let rank = rank2_spanning_forest(mref(&idx), &mut ws, &mut basis).unwrap();
    assert_eq!(rank, 4, "the duplicate of each edge closes a cycle");
    assert_eq!(basis.len(), 4);
    // offering the parallel copy after the original must close a cycle.
    {
        let mut inc = IncrementalRank2::begin(&mut ws, ir.nodes.len()).unwrap();
        assert!(
            inc.offer(&ir, HyperArcId::new(0)).unwrap(),
            "first 0-1 edge accepted"
        );
        // arc index 4 is the duplicate of arc 0 (dup_heavy appends a clone of the arc list).
        assert!(
            !inc.offer(&ir, HyperArcId::new(4)).unwrap(),
            "the parallel 0-1 edge closes a cycle"
        );
    }
}

// ---------------------------------------------------------------------------------------------------
// §5.7 — typed rejections: a hyperarc is never silently misread as a pairwise edge.
// ---------------------------------------------------------------------------------------------------

#[test]
fn multiway_hyperarc_is_rejected() {
    let ir = ic::window_ir(6, 3, false, SignMode::AllPlus); // arity-3 arcs
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    let err = rank2_spanning_forest(mref(&idx), &mut ws, &mut out).unwrap_err();
    assert!(
        matches!(
            err,
            MatroidError::WrongCardinality {
                node_incidences: 3,
                ..
            }
        ),
        "arity-3 hyperarc must be a typed WrongCardinality, got {err:?}"
    );
}

#[test]
fn self_loop_is_rejected() {
    let mut ir = Ir::default();
    let d = one_node_decl(&mut ir);
    let _ = one_node_decl(&mut ir);
    push_arc(&mut ir, vec![plus_ref(d, None), plus_ref(d, None)]); // both ends the same node
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    let err = rank2_spanning_forest(mref(&idx), &mut ws, &mut out).unwrap_err();
    assert!(
        matches!(err, MatroidError::SelfLoop { .. }),
        "self-loop rejected, got {err:?}"
    );
}

#[test]
fn dangling_non_node_target_is_rejected() {
    let mut ir = Ir::default();
    let node = one_node_decl(&mut ir);
    // an edge-kind decl: not a node, so decl_to_node returns None for it.
    let edge = ir.push_decl(DeclNode::root(
        DeclKind::Edge,
        SymId::new(0),
        AnnoR::default(),
    ));
    push_arc(&mut ir, vec![plus_ref(node, None), plus_ref(edge, None)]);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    let err = rank2_spanning_forest(mref(&idx), &mut ws, &mut out).unwrap_err();
    assert!(
        matches!(err, MatroidError::DanglingTarget { .. }),
        "non-node target rejected, got {err:?}"
    );
}

#[test]
fn non_finite_weight_is_rejected() {
    let ir = weighted_graph_ir(3, &[(0, 1, 1.0), (1, 2, f64::NAN)]);
    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    let err = rank2_weighted_basis(mref(&idx), true, &mut ws, &mut out).unwrap_err();
    assert!(
        matches!(err, MatroidError::NonFiniteWeight { .. }),
        "NaN weight rejected, got {err:?}"
    );
}

// ---------------------------------------------------------------------------------------------------
// §5.8 — the canonical Ir is not mutated by any matroid operation (belt & suspenders on the &-borrow).
// ---------------------------------------------------------------------------------------------------

#[test]
fn matroid_operations_do_not_mutate_the_ir() {
    let ir = weighted_graph_ir(
        5,
        &[
            (0, 1, 1.0),
            (1, 2, 2.0),
            (2, 0, 3.0),
            (2, 3, 4.0),
            (3, 4, 5.0),
        ],
    );
    let before = (ir.nodes.len(), ir.arcs.len(), engine_component_count(&ir));
    let weight_sum_before: f64 = (0..ir.arcs.len()).map(|a| oracle_weight(&ir, a)).sum();

    let idx = build_incidence_index(&ir).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut out = Vec::new();
    let _ = rank2_rank(mref(&idx), &mut ws).unwrap();
    let _ = rank2_spanning_forest(mref(&idx), &mut ws, &mut out).unwrap();
    let _ = rank2_weighted_basis(mref(&idx), true, &mut ws, &mut out).unwrap();

    let after = (ir.nodes.len(), ir.arcs.len(), engine_component_count(&ir));
    let weight_sum_after: f64 = (0..ir.arcs.len()).map(|a| oracle_weight(&ir, a)).sum();
    assert_eq!(before, after, "node/arc/component counts unchanged");
    assert!(
        (weight_sum_before - weight_sum_after).abs() < 1e-12,
        "weights unchanged"
    );
}
