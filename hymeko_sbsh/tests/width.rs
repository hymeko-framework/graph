//! The width certificate against EXACT treewidth oracles.
//!
//! These are not regression snapshots — each expected value is the true treewidth of the graph,
//! so a failure means the certificate is wrong, not merely different.

use hymeko_sbsh::{certificate_width, elimination_cost, width::adjacency, width::order_min_degree};

#[test]
fn a_chain_has_width_one_which_is_the_min_degree_guard() {
    // THE GUARD, carried over from the reference implementation's own suite: nested dissection
    // OVER-SEPARATES a degenerate chain and reports 2, which would route a trivially cheap
    // problem to the expensive solver. If someone swaps min-degree for a fancier ordering, this
    // is the test that catches it.
    let edges: Vec<(u32, u32)> = (0..9).map(|i| (i, i + 1)).collect();
    assert_eq!(
        certificate_width(10, &edges),
        1,
        "a chain must certify at width 1, not 2"
    );
}

#[test]
fn a_star_has_width_one() {
    let edges: Vec<(u32, u32)> = (1..8).map(|i| (0, i)).collect();
    assert_eq!(certificate_width(8, &edges), 1);
}

#[test]
fn a_triangle_has_width_two() {
    assert_eq!(certificate_width(3, &[(0, 1), (1, 2), (0, 2)]), 2);
}

#[test]
fn a_complete_graph_has_width_m_minus_one() {
    for m in 2u32..7 {
        let mut edges = Vec::new();
        for a in 0..m {
            for b in (a + 1)..m {
                edges.push((a, b));
            }
        }
        assert_eq!(
            certificate_width(m as usize, &edges),
            m as usize - 1,
            "K_{m} must certify at width {}",
            m - 1
        );
    }
}

#[test]
fn an_edgeless_graph_has_width_zero() {
    assert_eq!(certificate_width(6, &[]), 0);
    assert_eq!(certificate_width(0, &[]), 0);
}

#[test]
fn a_k_by_k_grid_is_at_least_k() {
    // The grid's treewidth is exactly k; an upper-bound certificate may not go BELOW it, which is
    // the direction that would be unsound.
    let k = 4u32;
    let idx = |r: u32, c: u32| r * k + c;
    let mut edges = Vec::new();
    for r in 0..k {
        for c in 0..k {
            if r + 1 < k {
                edges.push((idx(r, c), idx(r + 1, c)));
            }
            if c + 1 < k {
                edges.push((idx(r, c), idx(r, c + 1)));
            }
        }
    }
    assert!(certificate_width((k * k) as usize, &edges) >= k as usize);
}

#[test]
fn duplicate_edges_and_self_loops_do_not_change_the_certificate() {
    let clean = certificate_width(4, &[(0, 1), (1, 2), (2, 3)]);
    let dirty = certificate_width(4, &[(0, 1), (1, 0), (1, 2), (2, 2), (2, 3), (3, 2), (0, 0)]);
    assert_eq!(clean, dirty, "self-loops and duplicates must be ignored");
}

#[test]
fn the_elimination_order_is_a_permutation() {
    let edges: Vec<(u32, u32)> = (0..7).map(|i| (i, i + 1)).collect();
    let adj = adjacency(8, &edges);
    let mut order = order_min_degree(8, &adj);
    assert_eq!(order.len(), 8);
    order.sort_unstable();
    assert_eq!(
        order,
        (0..8).collect::<Vec<u32>>(),
        "every vertex exactly once"
    );
}

#[test]
fn a_tree_needs_no_fill() {
    let edges: Vec<(u32, u32)> = (0..9).map(|i| (i, i + 1)).collect();
    let adj = adjacency(10, &edges);
    let order = order_min_degree(10, &adj);
    assert_eq!(
        elimination_cost(&adj, &order).fill,
        0,
        "a chain is chordal; no fill is needed"
    );
}
