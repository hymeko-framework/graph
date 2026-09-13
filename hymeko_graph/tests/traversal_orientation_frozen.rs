//! **The frozen pre-migration corpus of the canonical Berge traversal.**
//!
//! Captured from the implementation **before** the step-orientation migration, so the migration has
//! an executable owner of the behaviour it must not change. Every value below was recorded from the
//! running code by `capture_the_corpus`, never hand-computed and never transcribed from a report.
//!
//! # What it freezes
//!
//! * the selected path and its tie-breaking, as the node and arc sequence;
//! * `total_cost` as raw bits;
//! * `WeightedBergePath::path_hash()` as an exact `u64`;
//! * the no-path and depth-bounded outcomes;
//! * all four producers: BFS shortest path, path DFS, cycle DFS, and Dijkstra.
//!
//! # What it is not
//!
//! It is **not** a second traversal implementation. It calls the production entry points and
//! records what they return. It is test-only and unreachable from production: this file is an
//! integration test target, not a library module.

#[path = "direct_ir_common/mod.rs"]
mod common;
use common::compile;

use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::direct_ir::{
    EnumBudget, HyperTraversal, InformedSearchWorkspace, TraversalWorkspace, Undirected, UnitCost,
    dijkstra_berge_path, enumerate_berge_cycles, enumerate_berge_paths,
};

/// The corpus fixture.
///
/// Deliberately shaped for the questions this migration raises rather than for coverage of the
/// parser: `@e3` holds **`c` twice**, which is the multi-slot incidence that makes a `refs` scan
/// ambiguous, and there are two distinct routes from `a` to `d` so tie-breaking is observable.
const FIXTURE: &str = r#"
vv {}
g { a {} b {} c {} d {} e {} f {}
    @e1 { (+ a, + b, - c); }
    @e2 { (+ b, + d); }
    @e3 { (+ c, - d, + c); }
    @e4 { (+ d, + e); }
    @e5 { (- e, + f); }
}
"#;

/// A world with an index, built once per call through the production path.
macro_rules! world {
    ($w:ident, $ix:ident) => {
        let $w = compile(FIXTURE);
        let $ix = build_incidence_index($w.ir()).expect("the fixture indexes");
    };
}

/// Node and arc sequence of a weighted path, as raw ids — the selected route and its tie-breaking.
fn route(p: &hymeko_graph::direct_ir::WeightedBergePath) -> (Vec<usize>, Vec<usize>) {
    (
        std::iter::once(p.start.raw())
            .chain(p.steps.iter().map(|s| s.to.node.raw()))
            .collect(),
        p.steps.iter().map(|s| s.arc.raw()).collect(),
    )
}

// =================================================================================================
// The recorded values. Every one produced by `capture_the_corpus`.
// =================================================================================================

/// Dijkstra `a -> f`, `UnitCost`, `Undirected`.
const DIJKSTRA_AF_NODES: &[usize] = &[1, 2, 4, 5, 6];
const DIJKSTRA_AF_ARCS: &[usize] = &[0, 1, 3, 4];
const DIJKSTRA_AF_COST_BITS: u64 = 0x4010_0000_0000_0000;
const DIJKSTRA_AF_PATH_HASH: u64 = 0x5b2e_c979_fd41_35b8;

/// BFS `a -> f`.
const BFS_AF_NODES: &[usize] = &[1, 2, 4, 5, 6];
const BFS_AF_ARCS: &[usize] = &[0, 1, 3, 4];

/// Path DFS from `a` to `f`, depth 6.
const DFS_AF_COUNT: usize = 3;
/// Cycle DFS through `b`, depth 6.
const CYCLES_B_COUNT: usize = 1;

#[test]
#[ignore = "generator: run only when the corpus is deliberately re-recorded"]
fn capture_the_corpus() {
    world!(w, ix);
    let trav = HyperTraversal::new(&ix);
    let (a, f, b) = (w.node("a"), w.node("f"), w.node("b"));

    let mut iws = InformedSearchWorkspace::default();
    let dj = dijkstra_berge_path(&trav, a, f, &UnitCost, &Undirected, &mut iws)
        .expect("dijkstra runs")
        .expect("a reaches f");
    let (n, ar) = route(&dj);
    println!("DIJKSTRA_AF_NODES = {n:?}");
    println!("DIJKSTRA_AF_ARCS = {ar:?}");
    println!("DIJKSTRA_AF_COST_BITS = 0x{:016x}", dj.total_cost.to_bits());
    println!("DIJKSTRA_AF_PATH_HASH = 0x{:016x}", dj.path_hash().raw());

    let mut tws = TraversalWorkspace::new();
    let bfs = trav
        .berge_shortest_path(&mut tws, a, f)
        .expect("bfs runs")
        .expect("a reaches f");
    println!(
        "BFS_AF_NODES = {:?}",
        std::iter::once(bfs.start.raw())
            .chain(bfs.steps.iter().map(|s| s.to.node.raw()))
            .collect::<Vec<_>>()
    );
    println!(
        "BFS_AF_ARCS = {:?}",
        bfs.steps.iter().map(|s| s.arc.raw()).collect::<Vec<_>>()
    );

    let budget = EnumBudget::new(6, 1000, 1_000_000);
    let paths = enumerate_berge_paths(&ix, a, Some(f), budget).expect("path dfs runs");
    println!("DFS_AF_COUNT = {}", paths.items.len());
    let cycles = enumerate_berge_cycles(&ix, b, budget).expect("cycle dfs runs");
    println!("CYCLES_B_COUNT = {}", cycles.items.len());

    // The no-path and bound-exceeded shapes, printed so a reader can see they were exercised.
    let mut iws2 = InformedSearchWorkspace::default();
    let none = dijkstra_berge_path(&trav, a, w.node("a"), &UnitCost, &Undirected, &mut iws2)
        .expect("self runs");
    println!(
        "SELF_IS_EMPTY_PATH = {}",
        none.is_some_and(|p| p.steps.is_empty())
    );
    let tight = EnumBudget::new(1, 1000, 1_000_000);
    let bounded = enumerate_berge_paths(&ix, a, Some(f), tight).expect("bounded runs");
    println!(
        "BOUNDED_ITEMS = {} BOUND = {:?}",
        bounded.items.len(),
        bounded.bound
    );
}

// =================================================================================================
// The gate
// =================================================================================================

#[test]
fn the_selected_route_cost_and_hash_are_unchanged() {
    world!(w, ix);
    let trav = HyperTraversal::new(&ix);
    let mut ws = InformedSearchWorkspace::default();
    let p = dijkstra_berge_path(
        &trav,
        w.node("a"),
        w.node("f"),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .expect("dijkstra runs")
    .expect("a reaches f");
    let (nodes, arcs) = route(&p);
    assert_eq!(nodes, DIJKSTRA_AF_NODES, "the selected route moved");
    assert_eq!(arcs, DIJKSTRA_AF_ARCS, "the selected arcs moved");
    assert_eq!(
        p.total_cost.to_bits(),
        DIJKSTRA_AF_COST_BITS,
        "total_cost moved at the bit level"
    );
    assert_eq!(
        p.path_hash().raw(),
        DIJKSTRA_AF_PATH_HASH,
        "path_hash v1 moved"
    );
}

#[test]
fn the_bfs_route_is_unchanged() {
    world!(w, ix);
    let trav = HyperTraversal::new(&ix);
    let mut ws = TraversalWorkspace::new();
    let p = trav
        .berge_shortest_path(&mut ws, w.node("a"), w.node("f"))
        .expect("bfs runs")
        .expect("a reaches f");
    let nodes: Vec<usize> = std::iter::once(p.start.raw())
        .chain(p.steps.iter().map(|s| s.to.node.raw()))
        .collect();
    let arcs: Vec<usize> = p.steps.iter().map(|s| s.arc.raw()).collect();
    assert_eq!(nodes, BFS_AF_NODES, "the BFS route moved");
    assert_eq!(arcs, BFS_AF_ARCS, "the BFS arcs moved");
}

#[test]
fn the_enumerated_counts_are_unchanged() {
    world!(w, ix);
    let budget = EnumBudget::new(6, 1000, 1_000_000);
    let paths =
        enumerate_berge_paths(&ix, w.node("a"), Some(w.node("f")), budget).expect("path dfs runs");
    assert_eq!(
        paths.items.len(),
        DFS_AF_COUNT,
        "the path enumeration moved"
    );
    let cycles = enumerate_berge_cycles(&ix, w.node("b"), budget).expect("cycle dfs runs");
    assert_eq!(
        cycles.items.len(),
        CYCLES_B_COUNT,
        "the cycle enumeration moved"
    );
}

#[test]
fn no_path_and_the_depth_bound_are_unchanged() {
    world!(w, ix);
    let trav = HyperTraversal::new(&ix);

    // A start that equals its goal is an empty path, not a refusal.
    let mut ws = InformedSearchWorkspace::default();
    let same = dijkstra_berge_path(
        &trav,
        w.node("a"),
        w.node("a"),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .expect("runs")
    .expect("a reaches itself");
    assert!(same.steps.is_empty(), "a self path must carry no steps");

    // The depth bound stops the enumeration and says so, rather than returning a short answer
    // indistinguishable from an exhausted one.
    let tight = EnumBudget::new(1, 1000, 1_000_000);
    let bounded =
        enumerate_berge_paths(&ix, w.node("a"), Some(w.node("f")), tight).expect("bounded runs");
    assert!(
        bounded.bound.is_some(),
        "the depth cap must be reported, not silently absorbed"
    );
}
