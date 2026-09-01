//! Coverage-closing tests: the strict `TargetNotANode` path (needs a hand-built malformed `Ir`,
//! which well-formed source cannot produce), error `Display`, the trivial accessors, and the
//! Berge-enumeration depth / expansion budget bounds.

mod direct_ir_common;

use hymeko::common::ids::{DeclId, EdgeId, NodeId, SymId};
use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::{
    AnnoR, ArcRec, DeclKind, DeclNode, EdgeRec, Ir, NodeRec, RefAtomR, SignedRefR,
};

use hymeko_graph::direct_ir::{
    BudgetKind, DirectIrError, EnumBudget, HyperTraversal, TraversalWorkspace,
    connected_components, dual_reachability, enumerate_berge_paths, s_connected_components,
};

use direct_ir_common::compile;

fn decl(kind: DeclKind) -> DeclNode {
    DeclNode {
        kind,
        name: SymId::new(0),
        parent: DeclId::NONE,
        first_child: DeclId::NONE,
        last_child: DeclId::NONE,
        next_sibling: DeclId::NONE,
        anno: AnnoR::default(),
    }
}

fn plus(target: DeclId) -> SignedRefR {
    SignedRefR::Plus(RefAtomR {
        target,
        anno: AnnoR::default(),
        weights: None,
    })
}

/// A hand-built `Ir` with well-formed nodes A (decl 0) and B (decl 2), an edge E (decl 1), and an
/// arc whose first member references the EDGE decl -- a non-node target that well-formed source
/// cannot produce. A and B are also members, so both are incident to the arc.
fn ir_with_non_node_target() -> Ir {
    let mut ir = Ir::default();
    let a = ir.push_decl(decl(DeclKind::Node));
    ir.nodes.push(NodeRec::new(a, Vec::new()));
    ir.bind_node_decl(a, NodeId::new(0)).unwrap();

    let e = ir.push_decl(decl(DeclKind::Edge));
    ir.edges.push(EdgeRec::new(e, Vec::new()));
    ir.bind_edge_decl(e, EdgeId::new(0)).unwrap();

    let b = ir.push_decl(decl(DeclKind::Node));
    ir.nodes.push(NodeRec::new(b, Vec::new()));
    ir.bind_node_decl(b, NodeId::new(1)).unwrap();

    // arc: (+ E, + A, + B) -- E is an edge decl, so it is not a node.
    ir.arcs.push(ArcRec {
        anno: AnnoR::default(),
        in_edge: e,
        refs: vec![plus(e), plus(a), plus(b)],
    });
    ir
}

#[test]
fn non_node_target_is_a_typed_error_in_every_traversal() {
    let ir = ir_with_non_node_target();
    let index = build_incidence_index(&ir).expect("index still builds (target is in range)");
    let trav = HyperTraversal::new(&index);
    let mut ws = TraversalWorkspace::new();

    // reachable_nodes: expand(A) hits the non-node member E
    assert!(matches!(
        trav.reachable_nodes(&mut ws, NodeId::new(0)),
        Err(DirectIrError::TargetNotANode { .. })
    ));
    // berge_shortest_path A -> B: start != goal so expand runs and hits E
    assert!(matches!(
        trav.berge_shortest_path(&mut ws, NodeId::new(0), NodeId::new(1)),
        Err(DirectIrError::TargetNotANode { .. })
    ));
    // dual_reachability (arc 0 refs E first)
    assert!(matches!(
        dual_reachability(&index, &mut ws, hymeko::common::ids::HyperArcId::new(0)),
        Err(DirectIrError::TargetNotANode { .. })
    ));
    // bounded enumeration
    let budget = EnumBudget::new(4, 10, 1000);
    assert!(matches!(
        enumerate_berge_paths(&index, NodeId::new(0), None, budget),
        Err(DirectIrError::TargetNotANode { .. })
    ));
}

#[test]
fn error_display_covers_every_variant() {
    let arc = hymeko::common::ids::HyperArcId::new(3);
    let variants = [
        DirectIrError::InvalidStart {
            node: NodeId::new(7),
        },
        DirectIrError::InvalidStartArc { arc },
        DirectIrError::TargetNotANode {
            arc,
            slot: 1,
            target: DeclId::new(2),
        },
        DirectIrError::PredecessorInconsistency {
            node: DeclId::new(5),
        },
        DirectIrError::BudgetExceeded {
            which: BudgetKind::Depth,
        },
    ];
    for v in variants {
        let s = format!("{v}");
        assert!(!s.is_empty(), "Display must render {v:?}");
        // std::error::Error is implemented
        let _: &dyn std::error::Error = &v;
    }
    // BudgetKind Debug on each
    for k in [
        BudgetKind::Depth,
        BudgetKind::Results,
        BudgetKind::Expansions,
    ] {
        assert!(!format!("{k:?}").is_empty());
    }
}

#[test]
fn accessors_and_views() {
    let w = compile(
        r#"
vv {}
g { a {} b {} c {}
    @e1 { (+ a, + b); }
    @e2 { (+ b, + c); }
}
"#,
    );
    let ir = w.ir();
    let index = build_incidence_index(ir).expect("index");
    let trav = HyperTraversal::new(&index);
    // ir() / index() accessors
    assert!(std::ptr::eq(trav.ir(), ir));
    assert_eq!(trav.index().incidence_count(), index.incidence_count());

    let mut ws = TraversalWorkspace::new();
    let set = trav.reachable_nodes(&mut ws, w.node("a")).unwrap();
    assert!(!set.is_empty());
    assert_eq!(set.decls().len(), set.len());

    // BergePath::arcs()
    let path = trav
        .berge_shortest_path(&mut ws, w.node("a"), w.node("c"))
        .unwrap()
        .unwrap();
    assert_eq!(path.arcs().count(), path.hyperedge_len());

    // dual view is_empty
    let dr = dual_reachability(&index, &mut ws, w.edge_arc("e1")).unwrap();
    assert!(!dr.is_empty());

    // ComponentLabeling / SComponentLabeling out-of-range -> None; labels() length
    let cc = connected_components(&index, &mut ws).unwrap();
    assert_eq!(cc.component_of(NodeId::new(9999)), None);
    assert_eq!(cc.labels().len(), ir.nodes.len());
    let sc = s_connected_components(&index, 1);
    assert_eq!(
        sc.component_of(hymeko::common::ids::HyperArcId::new(9999)),
        None
    );
    assert_eq!(sc.labels().len(), ir.arcs.len());
}

#[test]
fn berge_enum_budget_depth_and_expansion_bounds() {
    // a long chain so max_depth and max_expansions actually bind
    let mut s = String::from("vv {}\ng {\n");
    use std::fmt::Write as _;
    let n = 12;
    for i in 0..n {
        let _ = writeln!(s, "    x{i} {{}}");
    }
    for i in 0..n - 1 {
        let _ = writeln!(s, "    @e{i} {{ (+ x{i}, + x{}); }}", i + 1);
    }
    s.push('}');
    let w = compile(&s);
    let index = build_incidence_index(w.ir()).unwrap();

    // depth bound: cap at 2 hyperedges -> cannot reach x11 from x0
    let depth_budget = EnumBudget::new(2, 1000, 1_000_000);
    let out =
        enumerate_berge_paths(&index, w.node("x0"), Some(w.node("x11")), depth_budget).unwrap();
    assert!(out.items.is_empty(), "depth 2 cannot reach the far end");

    // expansion bound: a tiny expansion budget stops early and reports it
    let exp_budget = EnumBudget::new(100, 1000, 1);
    let out2 = enumerate_berge_paths(&index, w.node("x0"), None, exp_budget).unwrap();
    assert_eq!(out2.bound, Some(BudgetKind::Expansions));
}
