//! **The oriented Berge step: what it records, and what each digest means.**
//!
//! Every control proves its mutation was **applied** before asking whether the result moved.
//!
//! Downstream client only: these tests call the production entry points and read what they return.
//! No traversal, no cost model and no digest is reimplemented here.

#[path = "direct_ir_common/mod.rs"]
mod common;
use common::compile;

use hymeko::ir::incidence::build_incidence_index;
use hymeko_graph::direct_ir::{
    BergeIncidence, BergeStep, EnumBudget, HyperTraversal, InformedSearchWorkspace,
    TraversalWorkspace, Undirected, UnitCost, WeightedBergePath, dijkstra_berge_path,
    enumerate_berge_paths,
};

/// A two-node hyperedge, and an arc holding one node **twice** so a `refs` scan is ambiguous.
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

/// Two nodes only: the simplest step there is.
const PAIR: &str = r#"
vv {}
g { a {} b {}
    @e1 { (+ a, - b); }
}
"#;

macro_rules! world {
    ($src:expr, $w:ident, $ix:ident) => {
        let $w = compile($src);
        let $ix = build_incidence_index($w.ir()).expect("the fixture indexes");
    };
}

fn dijkstra(
    w: &common::World,
    ix: &hymeko::ir::incidence::IncidenceIndex<'_>,
    from: &str,
    to: &str,
) -> WeightedBergePath {
    let trav = HyperTraversal::new(ix);
    let mut ws = InformedSearchWorkspace::default();
    dijkstra_berge_path(
        &trav,
        w.node(from),
        w.node(to),
        &UnitCost,
        &Undirected,
        &mut ws,
    )
    .expect("dijkstra runs")
    .expect("the goal is reachable")
}

#[test]
fn frozen_path_digest_frames() {
    world!(FIXTURE, w, ix);
    let values: Vec<_> = [("a", "f"), ("f", "a"), ("c", "d"), ("a", "a")]
        .into_iter()
        .map(|(from, to)| {
            let mut path = dijkstra(&w, &ix, from, to);
            let normal = [
                path.path_hash().raw(),
                path.oriented_materialisation_digest().raw(),
            ];
            path.total_cost = -0.0;
            [
                normal,
                [
                    path.path_hash().raw(),
                    path.oriented_materialisation_digest().raw(),
                ],
            ]
        })
        .collect();
    assert_eq!(
        values,
        [
            [
                [6570410432157267384, 3045687703550679801],
                [6554894124062981352, 3030312132944804777]
            ],
            [
                [149081082730883393, 14029118310993919998],
                [133986987101830385, 14044634619088206030]
            ],
            [
                [865311728025539767, 14279995148346507503],
                [666455154498285318, 14326074581164119502]
            ],
            [
                [6428609956713447147, 13806016252684476177],
                [6428469219225036139, 13806156990172887185]
            ]
        ]
    );
}

// =================================================================================================
// 1. Exact from/to on a two-node step
// =================================================================================================

#[test]
fn a_two_node_step_records_both_incidences_exactly() {
    world!(PAIR, w, ix);
    let p = dijkstra(&w, &ix, "a", "b");
    assert_eq!(p.steps.len(), 1, "a two-node arc is one step");
    let s = p.steps[0];
    assert_eq!(s.from.node, w.node("a"), "from is the start");
    assert_eq!(s.to.node, w.node("b"), "to is the goal");
    assert_eq!(s.from.slot, 0, "a is the arc's first reference");
    assert_eq!(s.to.slot, 1, "b is the second");
    assert_eq!(s.from.sign, 1, "a's incidence is positive");
    assert_eq!(s.to.sign, -1, "b's incidence is negative");
    assert_ne!(
        s.from.slot, s.to.slot,
        "the two ends are distinct coordinates"
    );

    // Both ends resolve back through the index at their own coordinate.
    for end in [s.from, s.to] {
        let iref = ix.resolve(s.arc, end.slot);
        assert_eq!(iref.sign, end.sign, "the recorded sign is the index's");
        assert_eq!(
            iref.target,
            w.ir().nodes[end.node.raw()].decl,
            "the recorded node is the reference's target"
        );
    }
}

// =================================================================================================
// 2-3. Distinguishable entry slots; a repeated node resolves by coordinate, never by first match
// =================================================================================================

/// `@e3` holds `c` at slot 0 **and** slot 2. Two steps into `d` through `@e3` that entered at
/// different `c` slots are different steps, and the type says so.
#[test]
fn two_entry_slots_into_one_arc_are_distinguishable() {
    world!(FIXTURE, w, ix);
    let (c, d) = (w.node("c"), w.node("d"));
    let e3 = w.edge_arc("e3");

    // The fixture really does hold `c` twice in `@e3` -- the premise of this control.
    let slots: Vec<u32> = (0..w.ir().arcs[e3.raw()].refs.len() as u32)
        .filter(|s| ix.resolve(e3, *s).target == w.ir().nodes[c.raw()].decl)
        .collect();
    assert_eq!(
        slots.len(),
        2,
        "@e3 must hold c twice for this control to mean anything"
    );
    assert_eq!(slots, vec![0, 2]);

    let d_slot = (0..w.ir().arcs[e3.raw()].refs.len() as u32)
        .find(|s| ix.resolve(e3, *s).target == w.ir().nodes[d.raw()].decl)
        .expect("d is in @e3");

    let step_at = |entry: u32| BergeStep {
        arc: e3,
        from: BergeIncidence::at(w.ir(), e3, entry).expect("a live reference"),
        to: BergeIncidence::at(w.ir(), e3, d_slot).expect("a live reference"),
    };
    let first = step_at(slots[0]);
    let second = step_at(slots[1]);

    // The mutation was applied: the two steps differ only in the entry coordinate.
    assert_eq!(first.arc, second.arc);
    assert_eq!(first.to, second.to);
    assert_eq!(
        first.from.node, second.from.node,
        "the same node at both entries"
    );
    assert_ne!(
        first.from.slot, second.from.slot,
        "the entry slots must differ"
    );
    assert_ne!(
        first, second,
        "two entry slots must give two distinguishable steps"
    );

    // And a first-match rule would have collapsed them: both entries carry the same node, so
    // "find the predecessor in refs" cannot tell which one the traversal used.
    assert_eq!(
        first.from.node, second.from.node,
        "a refs-scan keyed on the node alone has no unique answer here"
    );
}

// =================================================================================================
// 4. Path reversal exchanges from and to
// =================================================================================================

#[test]
fn reversing_a_path_exchanges_the_two_incidences_on_every_step() {
    world!(FIXTURE, w, ix);
    let fwd = dijkstra(&w, &ix, "a", "f");
    let back = dijkstra(&w, &ix, "f", "a");
    assert_eq!(
        fwd.steps.len(),
        back.steps.len(),
        "the two routes differ in length"
    );

    // The mutation was applied: the routes are traversed in opposite directions.
    assert_ne!(fwd.start, back.start, "the endpoints were not swapped");

    for (i, f) in fwd.steps.iter().enumerate() {
        let b = back.steps[fwd.steps.len() - 1 - i];
        assert_eq!(f.arc, b.arc, "step {i}: the same arcs in the reverse order");
        assert_eq!(
            f.from, b.to,
            "step {i}: the forward entry is the reverse exit"
        );
        assert_eq!(
            f.to, b.from,
            "step {i}: the forward exit is the reverse entry"
        );
    }
}

// =================================================================================================
// 5. Sign and direction are independent
// =================================================================================================

#[test]
fn a_sign_mutation_does_not_change_the_direction() {
    world!(PAIR, w, ix);
    let p = dijkstra(&w, &ix, "a", "b");
    let s = p.steps[0];
    let flipped = BergeStep {
        arc: s.arc,
        from: BergeIncidence {
            sign: -s.from.sign,
            ..s.from
        },
        to: BergeIncidence {
            sign: -s.to.sign,
            ..s.to
        },
    };
    // The mutation was applied.
    assert_ne!(flipped.from.sign, s.from.sign);
    assert_ne!(flipped.to.sign, s.to.sign);
    // The direction is untouched.
    assert_eq!(flipped.from.node, s.from.node, "the entry node moved");
    assert_eq!(flipped.to.node, s.to.node, "the exit node moved");
    assert_eq!(flipped.from.slot, s.from.slot);
    assert_eq!(flipped.to.slot, s.to.slot);
}

#[test]
fn a_direction_mutation_does_not_change_the_signs() {
    world!(PAIR, w, ix);
    let p = dijkstra(&w, &ix, "a", "b");
    let s = p.steps[0];
    let reversed = BergeStep {
        arc: s.arc,
        from: s.to,
        to: s.from,
    };
    // The mutation was applied.
    assert_ne!(
        reversed.from.node, s.from.node,
        "the direction did not change"
    );
    // Every sign travels with its own incidence and none of them changed value.
    assert_eq!(
        reversed.from.sign, s.to.sign,
        "a sign was rewritten by the reversal"
    );
    assert_eq!(
        reversed.to.sign, s.from.sign,
        "a sign was rewritten by the reversal"
    );
}

// =================================================================================================
// 6. start / from / to chain continuity
// =================================================================================================

#[test]
fn the_start_from_to_chain_is_continuous() {
    world!(FIXTURE, w, ix);
    for (goal, _) in [("f", 0), ("e", 0), ("d", 0)] {
        let p = dijkstra(&w, &ix, "a", goal);
        assert_eq!(
            p.steps[0].from.node, p.start,
            "the first step must enter at the path's start"
        );
        for pair in p.steps.windows(2) {
            assert_eq!(
                pair[0].to.node, pair[1].from.node,
                "a step must enter where the previous one left"
            );
        }
    }

    // The BFS producer keeps the same chain.
    let trav = HyperTraversal::new(&ix);
    let mut ws = TraversalWorkspace::new();
    let bfs = trav
        .berge_shortest_path(&mut ws, w.node("a"), w.node("f"))
        .expect("bfs runs")
        .expect("a reaches f");
    assert_eq!(bfs.steps[0].from.node, bfs.start);
    for pair in bfs.steps.windows(2) {
        assert_eq!(pair[0].to.node, pair[1].from.node);
    }

    // And so does the DFS enumerator.
    let out = enumerate_berge_paths(
        &ix,
        w.node("a"),
        Some(w.node("f")),
        EnumBudget::new(6, 50, 1 << 20),
    )
    .expect("dfs runs");
    assert!(!out.items.is_empty(), "the enumeration must produce a path");
    for path in &out.items {
        assert_eq!(path.steps[0].from.node, path.start);
        for pair in path.steps.windows(2) {
            assert_eq!(pair[0].to.node, pair[1].from.node);
        }
    }
}

// =================================================================================================
// 7. The two digests
// =================================================================================================

/// **Measurement B.** Two paths agreeing in every quantity v1 hashes, differing only in one
/// `from.slot`: v1 must agree, the oriented digest must not.
#[test]
fn the_oriented_digest_separates_what_the_topological_one_cannot() {
    world!(FIXTURE, w, ix);
    let base = dijkstra(&w, &ix, "a", "f");
    let e3 = w.edge_arc("e3");

    // Build the discriminating pair by hand from live coordinates: same arc, same `to`, two
    // different entry slots of the same node.
    let c_slots: Vec<u32> = (0..w.ir().arcs[e3.raw()].refs.len() as u32)
        .filter(|s| ix.resolve(e3, *s).target == w.ir().nodes[w.node("c").raw()].decl)
        .collect();
    assert_eq!(c_slots.len(), 2, "the fixture must hold c twice in @e3");
    let d_slot = (0..w.ir().arcs[e3.raw()].refs.len() as u32)
        .find(|s| ix.resolve(e3, *s).target == w.ir().nodes[w.node("d").raw()].decl)
        .expect("d is in @e3");

    let with_entry = |entry: u32| WeightedBergePath {
        start: w.node("c"),
        steps: vec![BergeStep {
            arc: e3,
            from: BergeIncidence::at(w.ir(), e3, entry).expect("live"),
            to: BergeIncidence::at(w.ir(), e3, d_slot).expect("live"),
        }],
        total_cost: base.total_cost,
    };
    let first = with_entry(c_slots[0]);
    let second = with_entry(c_slots[1]);

    // The mutation was applied.
    assert_ne!(
        first.steps[0].from.slot, second.steps[0].from.slot,
        "the entry slots must differ"
    );
    assert_eq!(
        first.steps[0].to, second.steps[0].to,
        "the exits must agree"
    );

    assert_eq!(
        first.path_hash(),
        second.path_hash(),
        "v1 hashes arrival only, so these two must be indistinguishable to it"
    );
    assert_ne!(
        first.oriented_materialisation_digest(),
        second.oriented_materialisation_digest(),
        "the oriented digest must separate two different entry slots"
    );
}

/// Every field of an oriented step moves the oriented digest.
#[test]
fn every_field_moves_the_oriented_digest() {
    world!(FIXTURE, w, ix);
    let base = dijkstra(&w, &ix, "a", "f");
    let d0 = base.oriented_materialisation_digest();

    let mutate = |f: &dyn Fn(&mut WeightedBergePath)| -> WeightedBergePath {
        let mut p = base.clone();
        f(&mut p);
        p
    };
    /// One named mutation of a path: what it is called, and what it changes.
    type Case<'a> = (&'a str, &'a dyn Fn(&mut WeightedBergePath));
    let cases: [Case<'_>; 8] = [
        ("start", &|p| p.start = p.steps[0].to.node),
        ("arc", &|p| p.steps[0].arc = p.steps[1].arc),
        ("from.node", &|p| p.steps[0].from.node = p.steps[0].to.node),
        ("from.slot", &|p| {
            p.steps[0].from.slot = p.steps[0].from.slot.wrapping_add(1)
        }),
        ("from.sign", &|p| {
            p.steps[0].from.sign = -p.steps[0].from.sign
        }),
        ("to.node", &|p| p.steps[0].to.node = p.steps[0].from.node),
        ("to.slot", &|p| {
            p.steps[0].to.slot = p.steps[0].to.slot.wrapping_add(1)
        }),
        ("to.sign", &|p| p.steps[0].to.sign = -p.steps[0].to.sign),
    ];
    for (name, f) in cases {
        let m = mutate(f);
        assert_ne!(m, base, "{name}: the mutation was not applied");
        assert_ne!(
            m.oriented_materialisation_digest(),
            d0,
            "{name}: the oriented digest did not move"
        );
    }
    // The cost bits too.
    let mut c = base.clone();
    c.total_cost += 1.0;
    assert_ne!(c.total_cost, base.total_cost);
    assert_ne!(
        c.oriented_materialisation_digest(),
        d0,
        "cost: the digest did not move"
    );
}

/// The two digests answer different questions and are not interchangeable by raw value.
#[test]
fn the_two_digests_are_distinct_types_with_distinct_domains() {
    world!(FIXTURE, w, ix);
    let p = dijkstra(&w, &ix, "a", "f");
    // Different domain tags mean the same path digests differently under the two contracts.
    assert_ne!(
        p.path_hash().raw(),
        p.oriented_materialisation_digest().raw(),
        "the two domains produced the same value; the separation is not doing its work"
    );
    // Type-level: this file cannot compile a comparison between them, which is the point. The raw
    // accessors exist for records and are deliberately the only bridge.
    let _: u64 = p.path_hash().raw();
    let _: u64 = p.oriented_materialisation_digest().raw();
}
