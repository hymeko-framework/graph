//! Shared, **engine-independent** oracle + workload harness for the informed-Berge-search tests.
//!
//! Nothing here references `hymeko_graph::direct_ir::informed` — these are the *reference* implementations
//! the engine is checked against (a shared bug cannot hide behind both sides of the comparison), plus the
//! deterministic workload generators. Consumed by `informed_oracles.rs` (freeze) and
//! `informed_search_vv.rs` / `informed_search_bench.rs` (engine comparison) via `#[path]`.
#![allow(dead_code)] // shared across test binaries; not every binary uses every helper.

use std::collections::BTreeMap;
use std::time::Instant;

use hymeko::common::ids::{DeclId, NodeId, SymId};
use hymeko::ir::incidence::{IncidenceIndex, IncidenceRef};
use hymeko::ir::ir::{AnnoR, ArcRec, DeclKind, DeclNode, Ir, NodeRec, RefAtomR, SignedRefR, ValueR};

// ---- timing helper (single typed unit: microseconds) ----------------------------------------------

/// Median, min, max microseconds over `iters` timed runs after 2 warm-ups.
pub fn median_us(iters: usize, mut f: impl FnMut()) -> (f64, f64, f64) {
    for _ in 0..2 {
        f();
    }
    let mut s: Vec<f64> = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        f();
        s.push(t.elapsed().as_secs_f64() * 1e6);
    }
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (s[0], s[s.len() / 2], *s.last().unwrap())
}

// ---- cost spec (shared spec; the SEARCH below is independent of the engine) ------------------------

/// The cost model the oracle scores, matching the engine's contracts by *specification* (not code).
#[derive(Clone, Copy)]
pub enum OracleCost {
    /// Every hop costs 1.
    Unit,
    /// `base + |first Num weight of the target incidence|`.
    Weight { base: f64 },
}

impl OracleCost {
    fn step(&self, to: &IncidenceRef<'_>) -> f64 {
        match *self {
            Self::Unit => 1.0,
            Self::Weight { base } => base + first_num_weight(to.weights).abs(),
        }
    }
}

fn first_num_weight(w: Option<&[ValueR]>) -> f64 {
    w.and_then(|ws| {
        ws.iter().find_map(|v| match v {
            ValueR::Num(n) => Some(*n),
            _ => None,
        })
    })
    .unwrap_or(0.0)
}

// ---- oracle 1: an independent O(V^2) Dijkstra over the implicit incidence state space ---------------

/// Reference optimal cost `start -> goal` on the Berge incidence space, by a **linear-scan** Dijkstra
/// (no binary heap, no shared code with the engine). `None` if unreachable. Undirected policy.
pub fn oracle_dijkstra_cost(
    ir: &Ir,
    index: &IncidenceIndex<'_>,
    start: NodeId,
    goal: NodeId,
    cost: OracleCost,
) -> Option<f64> {
    let n = ir.decl_nodes.len();
    let start_d = ir.nodes[start.raw()].decl.raw();
    let goal_d = ir.nodes[goal.raw()].decl.raw();
    let mut dist = vec![f64::INFINITY; n];
    let mut done = vec![false; n];
    dist[start_d] = 0.0;
    loop {
        // pick the unsettled node of least dist (linear scan; deterministic by index on ties).
        let mut u = usize::MAX;
        let mut best = f64::INFINITY;
        for i in 0..n {
            if !done[i] && dist[i].total_cmp(&best) == std::cmp::Ordering::Less {
                best = dist[i];
                u = i;
            }
        }
        if u == usize::MAX || !best.is_finite() {
            break;
        }
        if u == goal_d {
            return Some(dist[goal_d]);
        }
        done[u] = true;
        let ud = DeclId::new(u);
        for &arc in index.incident_arcs(ud) {
            for (slot, sref) in ir.arcs[arc.raw()].refs.iter().enumerate() {
                let v = sref.target();
                if v.raw() == u {
                    continue;
                }
                let to = index.resolve(arc, slot as u32);
                let c = cost.step(&to);
                let cand = dist[u] + c;
                if cand.total_cmp(&dist[v.raw()]) == std::cmp::Ordering::Less {
                    dist[v.raw()] = cand;
                }
            }
        }
    }
    if dist[goal_d].is_finite() {
        Some(dist[goal_d])
    } else {
        None
    }
}

// ---- oracle 2: exhaustive simple-Berge-path min cost (tiny graphs only) -----------------------------

/// Brute-force minimum-cost simple Berge path `start -> goal` by DFS over all simple node paths. Intended
/// for tiny graphs (guards on `node_budget`); `None` if unreachable or the budget trips first.
pub fn exhaustive_min_cost(
    ir: &Ir,
    index: &IncidenceIndex<'_>,
    start: NodeId,
    goal: NodeId,
    cost: OracleCost,
    node_budget: usize,
) -> Option<f64> {
    let start_d = ir.nodes[start.raw()].decl;
    let goal_d = ir.nodes[goal.raw()].decl;
    let mut visited: BTreeMap<usize, bool> = BTreeMap::new();
    let mut best = f64::INFINITY;
    let mut steps = 0usize;
    dfs(ir, index, start_d, goal_d, 0.0, cost, &mut visited, &mut best, &mut steps, node_budget);
    if best.is_finite() { Some(best) } else { None }
}

#[allow(clippy::too_many_arguments)]
fn dfs(
    ir: &Ir,
    index: &IncidenceIndex<'_>,
    cur: DeclId,
    goal: DeclId,
    acc: f64,
    cost: OracleCost,
    visited: &mut BTreeMap<usize, bool>,
    best: &mut f64,
    steps: &mut usize,
    budget: usize,
) {
    *steps += 1;
    if *steps > budget {
        return;
    }
    if cur == goal {
        if acc.total_cmp(best) == std::cmp::Ordering::Less {
            *best = acc;
        }
        return;
    }
    visited.insert(cur.raw(), true);
    for &arc in index.incident_arcs(cur) {
        for (slot, sref) in ir.arcs[arc.raw()].refs.iter().enumerate() {
            let v = sref.target();
            if v.raw() == cur.raw() || *visited.get(&v.raw()).unwrap_or(&false) {
                continue;
            }
            let to = index.resolve(arc, slot as u32);
            dfs(ir, index, v, goal, acc + cost.step(&to), cost, visited, best, steps, budget);
        }
    }
    visited.insert(cur.raw(), false);
}

// ---- deterministic workload generators --------------------------------------------------------------

/// Signs to attach to a window arc's members.
#[derive(Clone, Copy)]
pub enum SignMode {
    /// All `+`.
    AllPlus,
    /// Alternate `+ / - / ~` by member index.
    Alternating,
}

fn push_nodes(ir: &mut Ir, m: usize) {
    for _ in 0..m {
        let d = ir.push_decl(DeclNode {
            kind: DeclKind::Node,
            name: SymId::new(0),
            parent: DeclId::NONE,
            first_child: DeclId::NONE,
            last_child: DeclId::NONE,
            next_sibling: DeclId::NONE,
            anno: AnnoR::default(),
        });
        let nid = NodeId::new(ir.nodes.len());
        ir.nodes.push(NodeRec::new(d, Vec::new()));
        ir.bind_node_decl(d, nid).expect("bind node");
    }
}

fn signed_ref(sign: SignMode, member_idx: usize, target: usize, weight: Option<f64>) -> SignedRefR {
    let atom = RefAtomR {
        target: DeclId::new(target),
        anno: AnnoR::default(),
        weights: weight.map(|w| vec![ValueR::Num(w)]),
    };
    match sign {
        SignMode::AllPlus => SignedRefR::Plus(atom),
        SignMode::Alternating => match member_idx % 3 {
            0 => SignedRefR::Plus(atom),
            1 => SignedRefR::Minus(atom),
            _ => SignedRefR::Neutral(atom),
        },
    }
}

/// Arity-`k` sliding-window hypergraph on `m` nodes: arc `i` = nodes `i..i+k`. `weighted` attaches a
/// deterministic per-member weight (`1 + (target % 5) * 0.5`); `signs` picks the incidence signs.
pub fn window_ir(m: usize, k: usize, weighted: bool, signs: SignMode) -> Ir {
    assert!(k >= 2 && m >= k);
    let mut ir = Ir::default();
    push_nodes(&mut ir, m);
    for i in 0..(m - k + 1) {
        let refs: Vec<SignedRefR> = (i..i + k)
            .enumerate()
            .map(|(mi, j)| {
                let w = if weighted { Some(1.0 + (j % 5) as f64 * 0.5) } else { None };
                signed_ref(signs, mi, j, w)
            })
            .collect();
        ir.arcs.push(ArcRec { anno: AnnoR::default(), in_edge: DeclId::NONE, refs });
    }
    ir
}

/// `comps` disjoint arity-`k` window components of `m` nodes each (a disconnected workload).
pub fn multi_component_ir(comps: usize, m: usize, k: usize) -> Ir {
    assert!(k >= 2 && m >= k);
    let mut ir = Ir::default();
    push_nodes(&mut ir, comps * m);
    for c in 0..comps {
        let base = c * m;
        for i in 0..(m - k + 1) {
            let refs: Vec<SignedRefR> = (i..i + k)
                .map(|j| signed_ref(SignMode::AllPlus, 0, base + j, None))
                .collect();
            ir.arcs.push(ArcRec { anno: AnnoR::default(), in_edge: DeclId::NONE, refs });
        }
    }
    ir
}

/// A window where every arc is duplicated (duplicate-incidence-heavy). Distances are unchanged; only the
/// resolved-incidence count grows.
pub fn dup_heavy_ir(m: usize, k: usize) -> Ir {
    let mut ir = window_ir(m, k, false, SignMode::AllPlus);
    let dup: Vec<ArcRec> = ir.arcs.clone();
    ir.arcs.extend(dup);
    ir
}

/// 1-D positions along a line: `pos(node i) = [i as f64]`. A `PositionProvider` wrapper lives in the test
/// file (it implements the engine trait, which this engine-independent module must not reference).
pub fn line_positions(m: usize) -> Vec<[f64; 1]> {
    (0..m).map(|i| [i as f64]).collect()
}

/// Positions permuted to mislead a geometric heuristic (reverse the coordinate).
pub fn misleading_positions(m: usize) -> Vec<[f64; 1]> {
    (0..m).map(|i| [(m - 1 - i) as f64]).collect()
}
