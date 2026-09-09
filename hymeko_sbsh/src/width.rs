//! The **SBSH width certificate** — the one cheap structural number that says whether a
//! bounded-width solve over the scene is affordable.
//!
//! The certificate is a *treewidth upper bound*: the maximum induced degree along a **min-degree
//! elimination order** (the elimination game). Small width means a direct, bounded-width solve is
//! cheap; large width routes an iterative one. It is the only number SBSH needs to make that
//! routing decision, and it costs a graph traversal rather than a solve.
//!
//! # Why min-degree specifically, and not something better-sounding
//!
//! On a chain the certificate must be exactly `1`. Nested dissection **over-separates** a
//! degenerate chain and reports `2`, which would route a trivially cheap problem to the expensive
//! path. That is an on-record guard carried over from the reference implementation's own test
//! suite, and `tests/width.rs` enforces it here: if someone swaps the ordering heuristic for one
//! that looks more sophisticated, the chain test fails.
//!
//! Exact oracles the implementation is held against: chain = 1, star = 1, triangle = 2,
//! complete `K_m` = `m-1`, empty = 0, and a `k x k` grid >= `k`.
//!
//! # Signs are carried, not consulted
//!
//! An SBSH edge is signed, but **width is a property of the unsigned adjacency**. No balance or
//! spectral computation enters here. Signs travel with the structure for fidelity and are used by
//! consumers; they do not change the elimination order or the certificate.

use std::collections::BTreeSet;

/// The cost of eliminating a graph in a given vertex order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElimCost {
    /// Fill edges introduced during elimination.
    pub fill: usize,
    /// Maximum induced degree — the treewidth upper bound, i.e. the certificate.
    pub width: usize,
    /// Approximate dense-factor flop count, `sum of d^2`.
    pub flops: f64,
}

/// Undirected adjacency; self-loops and duplicate edges are ignored.
///
/// # Preconditions
/// Every `(u, v)` satisfies `u < n` and `v < n`.
pub fn adjacency(n: usize, edges: &[(u32, u32)]) -> Vec<BTreeSet<u32>> {
    let mut adj = vec![BTreeSet::new(); n];
    for &(u, v) in edges {
        debug_assert!(
            (u as usize) < n && (v as usize) < n,
            "edge endpoint out of range"
        );
        if u != v {
            adj[u as usize].insert(v);
            adj[v as usize].insert(u);
        }
    }
    adj
}

/// A **min-degree elimination order**: repeatedly remove the still-present vertex of least
/// current degree, cliquing its remaining neighbours.
///
/// # Postconditions
/// The result is a permutation of `0..n` — each vertex exactly once.
pub fn order_min_degree(n: usize, adj: &[BTreeSet<u32>]) -> Vec<u32> {
    let mut a: Vec<BTreeSet<u32>> = adj.to_vec(); // a copy: the caller's adjacency is untouched
    let mut present: BTreeSet<u32> = (0..n as u32).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(&v) = present
        .iter()
        .min_by_key(|&&x| a[x as usize].intersection(&present).count())
    {
        let nb: Vec<u32> = a[v as usize].intersection(&present).copied().collect();
        for i in 0..nb.len() {
            for j in (i + 1)..nb.len() {
                a[nb[i] as usize].insert(nb[j]);
                a[nb[j] as usize].insert(nb[i]);
            }
        }
        present.remove(&v);
        order.push(v);
    }
    order
}

/// The **elimination game** cost of eliminating `adj` in `order`: fill edges, maximum induced
/// degree (the width), and dense-factor flops.
///
/// # Preconditions
/// `order` is a permutation of `0..adj.len()`.
pub fn elimination_cost(adj: &[BTreeSet<u32>], order: &[u32]) -> ElimCost {
    let n = adj.len();
    debug_assert_eq!(order.len(), n, "order must be a permutation of 0..n");
    let mut a: Vec<BTreeSet<u32>> = adj.to_vec();
    let mut present: Vec<bool> = vec![true; n];
    let (mut fill, mut width, mut flops) = (0usize, 0usize, 0.0f64);
    for &v in order {
        let nb: Vec<u32> = a[v as usize]
            .iter()
            .copied()
            .filter(|&u| present[u as usize])
            .collect();
        let d = nb.len();
        width = width.max(d);
        flops += (d * d) as f64;
        for i in 0..d {
            for j in (i + 1)..d {
                let (x, y) = (nb[i], nb[j]);
                if !a[x as usize].contains(&y) {
                    a[x as usize].insert(y);
                    a[y as usize].insert(x);
                    fill += 1;
                }
            }
        }
        present[v as usize] = false;
    }
    ElimCost { fill, width, flops }
}

/// The SBSH width certificate for a constraint graph on `n` vertices: a treewidth upper bound via
/// min-degree elimination. Signs, if the caller has them, are irrelevant here and may be dropped
/// before calling.
///
/// # Preconditions
/// Every edge endpoint is `< n`.
///
/// # Postconditions
/// `0` for an edgeless graph; `n - 1` for the complete graph; never more than `n - 1`.
pub fn certificate_width(n: usize, edges: &[(u32, u32)]) -> usize {
    if n == 0 {
        return 0;
    }
    let adj = adjacency(n, edges);
    let order = order_min_degree(n, &adj);
    elimination_cost(&adj, &order).width
}
