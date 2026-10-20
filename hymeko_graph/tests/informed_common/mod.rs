//! Shared, **engine-independent** oracle + workload harness for the informed-Berge-search tests.
//!
//! Nothing here references `hymeko_graph::direct_ir::informed` — these are the *reference* implementations
//! the engine is checked against (a shared bug cannot hide behind both sides of the comparison), plus the
//! deterministic workload generators. Consumed by `informed_oracles.rs` (freeze) and
//! `informed_search_vv.rs` / `informed_search_bench.rs` (engine comparison) via `#[path]`.
#![allow(dead_code)] // shared across test binaries; not every binary uses every helper.

pub mod search;

use hymeko::ir::ir::IrConstruction;
use hymeko::util::measurement::{MeasurementRunner, Runner};
use std::collections::BTreeMap;

use hymeko::common::ids::{DeclId, NodeId, SymId};
use hymeko::ir::incidence::{IncidenceIndex, IncidenceRef};
use hymeko::ir::ir::{
    AnnoR, ArcRec, DeclKind, DeclNode, IncidenceSign, Ir, RefAtomR, ReferenceDraft, SignedRefR,
    ValueR,
};

// ---- timing helper (single typed unit: microseconds) ----------------------------------------------

/// Median, min, max microseconds over `iters` timed runs after 2 warm-ups.
pub fn median_us(iters: usize, f: impl FnMut()) -> (f64, f64, f64) {
    let d = MeasurementRunner::new(2, iters).run(f);
    (d.min, d.median, d.worst)
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
    dfs(
        ir,
        index,
        start_d,
        goal_d,
        0.0,
        cost,
        &mut visited,
        &mut best,
        &mut steps,
        node_budget,
    );
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
            dfs(
                ir,
                index,
                v,
                goal,
                acc + cost.step(&to),
                cost,
                visited,
                best,
                steps,
                budget,
            );
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
        node_decl(ir);
    }
}

/// The named P3.4 measurement workloads, shared by every subprocess cell / bench so the definitions
/// never drift between harnesses (§6.1).
pub fn named_workload(name: &str) -> Ir {
    match name {
        "sts511" => steiner_sts_ir(9),
        "sts1023" => steiner_sts_ir(10),
        "mycielski" => {
            let (n, e) = base_c5();
            mycielski_ir(n, &e, 8)
        }
        "random1e6" => uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3),
        "map512" => map_grid_ir(512, 512, &map_obstacles(512, 512)),
        "transport" => map_grid_with_transport_ir(256, 256, &map_obstacles(256, 256), 8, 5.0),
        other => panic!("unknown workload {other}"),
    }
}

/// Push one node declaration + record + binding; return its `DeclId` (which, when nodes are pushed
/// first, equals its `NodeId`). Shared inline builder for hand-constructed fixtures.
pub fn node_decl(ir: &mut Ir) -> DeclId {
    IrConstruction::new(ir)
        .append_node(SymId::new(0), AnnoR::default(), Vec::new())
        .expect("bind node")
}

/// Push one **non-node** (edge-kind) declaration; return its `DeclId`. Used to build a target that is
/// deliberately not a node (so `decl_to_node` returns `None`).
pub fn edge_decl(ir: &mut Ir) -> DeclId {
    ir.push_decl(DeclNode::root(
        DeclKind::Edge,
        SymId::new(0),
        AnnoR::default(),
    ))
}

/// A `+` incidence reference to `target`, optionally weighted.
pub fn plus_ref(target: DeclId, weight: Option<f64>) -> ReferenceDraft {
    ReferenceDraft::new(
        IncidenceSign::Plus,
        RefAtomR::plain(target).with_weights(weight.map(|w| vec![ValueR::Num(w)])),
    )
}

/// Push one hyperarc with the given references.
pub fn push_arc(ir: &mut Ir, refs: Vec<ReferenceDraft>) {
    let refs = refs
        .into_iter()
        .map(|draft| ir.store_reference(draft))
        .collect();
    ir.arcs.push(ArcRec::unowned(refs));
}

/// A weighted pairwise graph on `n` nodes from an explicit `(u, v, w)` edge list.
pub fn weighted_graph_ir(n: usize, edges: &[(usize, usize, f64)]) -> Ir {
    let mut ir = Ir::default();
    let decls: Vec<DeclId> = (0..n).map(|_| node_decl(&mut ir)).collect();
    for &(u, v, w) in edges {
        push_arc(
            &mut ir,
            vec![plus_ref(decls[u], Some(w)), plus_ref(decls[v], Some(w))],
        );
    }
    ir
}

fn signed_ref(
    sign: SignMode,
    member_idx: usize,
    target: usize,
    weight: Option<f64>,
) -> ReferenceDraft {
    let atom =
        RefAtomR::plain(DeclId::new(target)).with_weights(weight.map(|w| vec![ValueR::Num(w)]));
    match sign {
        SignMode::AllPlus => ReferenceDraft::new(IncidenceSign::Plus, atom),
        SignMode::Alternating => match member_idx % 3 {
            0 => ReferenceDraft::new(IncidenceSign::Plus, atom),
            1 => ReferenceDraft::new(IncidenceSign::Minus, atom),
            _ => ReferenceDraft::new(IncidenceSign::Neutral, atom),
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
        let refs: Vec<ReferenceDraft> = (i..i + k)
            .enumerate()
            .map(|(mi, j)| {
                let w = if weighted {
                    Some(1.0 + (j % 5) as f64 * 0.5)
                } else {
                    None
                };
                signed_ref(signs, mi, j, w)
            })
            .collect();
        push_arc(&mut ir, refs);
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
            let refs: Vec<ReferenceDraft> = (i..i + k)
                .map(|j| signed_ref(SignMode::AllPlus, 0, base + j, None))
                .collect();
            push_arc(&mut ir, refs);
        }
    }
    ir
}

/// A window where every arc is duplicated (duplicate-incidence-heavy). Distances are unchanged; only the
/// resolved-incidence count grows.
pub fn dup_heavy_ir(m: usize, k: usize) -> Ir {
    let mut ir = window_ir(m, k, false, SignMode::AllPlus);
    let dup: Vec<ArcRec> = ir.arcs.to_vec();
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

/// A `w x h` 4-neighbour grid as a hypergraph: node `(r,c)` at index `r*w + c`; an arity-2 arc to the
/// right neighbour and one to the down neighbour. A **branching** workload where a geometric heuristic can
/// prune (unlike a line). Positions are the grid coordinates.
pub fn grid_ir(w: usize, h: usize) -> Ir {
    assert!(w >= 2 && h >= 2);
    let mut ir = Ir::default();
    push_nodes(&mut ir, w * h);
    let edge = |ir: &mut Ir, a: usize, b: usize| {
        push_arc(
            ir,
            vec![
                ReferenceDraft::plain(DeclId::new(a), hymeko::ir::ir::IncidenceSign::Plus),
                ReferenceDraft::plain(DeclId::new(b), hymeko::ir::ir::IncidenceSign::Plus),
            ],
        );
    };
    for r in 0..h {
        for c in 0..w {
            let id = r * w + c;
            if c + 1 < w {
                edge(&mut ir, id, id + 1);
            }
            if r + 1 < h {
                edge(&mut ir, id, id + w);
            }
        }
    }
    ir
}

/// Grid coordinates as positions: node `r*w + c -> [c, r]`.
pub fn grid_positions(w: usize, h: usize) -> Vec<[f64; 2]> {
    let mut v = Vec::with_capacity(w * h);
    for r in 0..h {
        for c in 0..w {
            v.push([c as f64, r as f64]);
        }
    }
    v
}

// ---- P3 workloads: Steiner triple systems, Mycielski, seeded random --------------------------------

/// Expected STS(v) counts for v = 2^m - 1: (vertices, blocks = v(v-1)/6, incidences = v(v-1)/2,
/// per-vertex degree = (v-1)/2). Formula-derived, independent of the emitted edge list.
pub fn sts_expected_counts(m: u32) -> (usize, usize, usize, usize) {
    let v = (1usize << m) - 1;
    (v, v * (v - 1) / 6, v * (v - 1) / 2, (v - 1) / 2)
}

/// STS(2^m - 1) as a rank-3 hypergraph on the canonical `Ir` (the projective geometry PG(m-1,2)):
/// points are the nonzero m-bit vectors `1..=v` (node index = value-1); a block is any triple
/// `{a,b,c}` with `a ^ b ^ c == 0`. Emitting only when `c = a^b > b` yields each block exactly once
/// (a<b<c). Each block is one arity-3 hyperarc. No pairwise projection.
pub fn steiner_sts_ir(m: u32) -> Ir {
    assert!((2..=12).contains(&m));
    let v = (1usize << m) - 1;
    let mut ir = Ir::default();
    push_nodes(&mut ir, v);
    for a in 1..=v {
        for b in (a + 1)..=v {
            let c = a ^ b;
            if c > b {
                let refs: Vec<ReferenceDraft> = [a, b, c]
                    .into_iter()
                    .map(|p| {
                        ReferenceDraft::plain(
                            DeclId::new(p - 1),
                            hymeko::ir::ir::IncidenceSign::Plus,
                        )
                    })
                    .collect();
                push_arc(&mut ir, refs);
            }
        }
    }
    ir
}

/// Mycielski recurrence from a base `(n, e)` after `iters` iterations: `n' = 2n+1`, `e' = 3e+n`.
pub fn mycielski_expected_counts(base_n: usize, base_e: usize, iters: u32) -> (usize, usize) {
    let (mut n, mut e) = (base_n, base_e);
    for _ in 0..iters {
        let (nn, ne) = (2 * n + 1, 3 * e + n);
        n = nn;
        e = ne;
    }
    (n, e)
}

/// The 5-cycle C5 (triangle-free, chromatic number 3), the classic Mycielski base — `μ(C5)` is the
/// Grötzsch graph. Returns `(n, edges)`.
pub fn base_c5() -> (usize, Vec<(usize, usize)>) {
    (5, vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)])
}

/// Iterated Mycielskian of a base graph as a rank-2 hypergraph on the canonical `Ir`. `μ(G)` on
/// `V={v_i} ∪ U={u_i} ∪ {w}`: original edges `{v_i,v_j}`; for each original edge the two shadow edges
/// `{u_i,v_j}` and `{v_i,u_j}`; apex edges `{w,u_i}` for all `i`. `u_i = n+i`, `w = 2n`.
pub fn mycielski_ir(base_n: usize, base_edges: &[(usize, usize)], iters: u32) -> Ir {
    use hymeko::generators::{HypergraphDesign, mycielskian};
    let mut design = HypergraphDesign {
        n_vertices: base_n,
        edges: base_edges.iter().map(|&(a, b)| vec![a, b]).collect(),
    };
    for _ in 0..iters {
        design = mycielskian(&design);
    }
    let mut ir = Ir::default();
    push_nodes(&mut ir, design.n_vertices);
    for edge in design.edges {
        push_arc(
            &mut ir,
            edge.into_iter()
                .map(|node| plus_ref(DeclId::new(node), None))
                .collect(),
        );
    }
    ir
}

/// A deterministic 64-bit LCG (Numerical Recipes constants). Test-support only — the production
/// `rand_lcg` in `src/` is not test-visible; this is a local, seeded, reproducible stream.
pub struct Lcg(u64);
impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    /// Uniform in `0..bound` (bound > 0).
    pub fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

/// Seeded uniform rank-`k` hypergraph: `m_edges` arcs, each `k` distinct nodes drawn uniformly from
/// `n`. Deterministic for a given `seed`. Incidences = `m_edges * k`.
pub fn uniform_rank_k_ir(seed: u64, n: usize, m_edges: usize, k: usize) -> Ir {
    assert!(k >= 2 && n >= k);
    let mut rng = Lcg::new(seed);
    let mut ir = Ir::default();
    push_nodes(&mut ir, n);
    for _ in 0..m_edges {
        let mut members: Vec<usize> = Vec::with_capacity(k);
        while members.len() < k {
            let x = rng.below(n);
            if !members.contains(&x) {
                members.push(x);
            }
        }
        let refs = members
            .into_iter()
            .map(|t| ReferenceDraft::plain(DeclId::new(t), hymeko::ir::ir::IncidenceSign::Plus))
            .collect();
        push_arc(&mut ir, refs);
    }
    ir
}

/// Two disjoint STS(2^m-1) components on one `Ir`; returns `(ir, offset)` where `offset` is the first
/// node id of the second component. Cross-component pairs are unreachable (a disconnected control).
pub fn two_disjoint_sts_ir(m: u32) -> (Ir, usize) {
    let single = steiner_sts_ir(m);
    let off = single.decl_nodes.len();
    let mut ir = steiner_sts_ir(m);
    push_nodes(&mut ir, off);
    for arc in &single.arcs {
        let refs = arc
            .refs
            .iter()
            .map(|r| {
                ReferenceDraft::plain(
                    DeclId::new(r.target().raw() + off),
                    hymeko::ir::ir::IncidenceSign::Plus,
                )
            })
            .collect();
        push_arc(&mut ir, refs);
    }
    (ir, off)
}

// ---- P3.1 map/routing workload (coords + weights in the canonical Ir; no parallel DTO) -------------

use std::collections::HashSet;

fn push_positioned_node(ir: &mut Ir, x: f64, y: f64) {
    IrConstruction::new(ir)
        .append_node(
            SymId::new(0),
            AnnoR {
                tags: Vec::new(),
                value: Some(ValueR::List(vec![ValueR::Num(x), ValueR::Num(y)])),
            },
            Vec::new(),
        )
        .expect("bind node");
}

/// Read a node's `[x, y]` coordinate back out of the canonical `Ir` (`DeclNode.anno.value`).
pub fn ir_position(ir: &Ir, decl: DeclId) -> Option<[f64; 2]> {
    match &ir.decl_annotation(decl).unwrap().value {
        Some(ValueR::List(v)) if v.len() == 2 => match (&v[0], &v[1]) {
            (ValueR::Num(x), ValueR::Num(y)) => Some([*x, *y]),
            _ => None,
        },
        _ => None,
    }
}

/// The global minimum incidence weight across the map (the discrete heuristic's per-hop lower bound).
pub fn map_min_edge_cost(ir: &Ir) -> f64 {
    let mut m = f64::INFINITY;
    for a in &ir.arcs {
        for r in &a.refs {
            let w = first_num_weight(ir.reference_atom(r).unwrap().weights);
            if w > 0.0 && w < m {
                m = w;
            }
        }
    }
    if m.is_finite() { m } else { 1.0 }
}

fn weighted_edge(ir: &mut Ir, a: usize, b: usize, w: f64) {
    let mk = |t: usize| {
        ReferenceDraft::new(
            IncidenceSign::Plus,
            RefAtomR::plain(DeclId::new(t)).with_weights(Some(vec![ValueR::Num(w)])),
        )
    };
    push_arc(ir, vec![mk(a), mk(b)]);
}

/// A weighted street grid on `w x h` intersections as canonical `Ir`. Node `(r,c) = r*w+c` carries its
/// coordinate in `DeclNode.anno.value = [c, r]`. Street segments are arity-2 arcs to the right and down
/// neighbour with weight `euclidean(=1) * class_factor`: **arterial** lines (`r%4==0 || c%4==0`) cost
/// `1.0`, **local** roads cost `1.5`. Every weight `>= 1.0 = euclidean` (so the metric heuristic is
/// admissible). `blocked` intersections get no incident edges (isolated -> unreachable regions/obstacles).
pub fn map_grid_ir(w: usize, h: usize, blocked: &HashSet<usize>) -> Ir {
    assert!(w >= 2 && h >= 2);
    let mut ir = Ir::default();
    for r in 0..h {
        for c in 0..w {
            push_positioned_node(&mut ir, c as f64, r as f64);
        }
    }
    let arterial = |r: usize, c: usize| r.is_multiple_of(4) || c.is_multiple_of(4);
    for r in 0..h {
        for c in 0..w {
            let id = r * w + c;
            if blocked.contains(&id) {
                continue;
            }
            if c + 1 < w && !blocked.contains(&(id + 1)) {
                let cost = if arterial(r, c) && arterial(r, c + 1) {
                    1.0
                } else {
                    1.5
                };
                weighted_edge(&mut ir, id, id + 1, cost);
            }
            if r + 1 < h && !blocked.contains(&(id + w)) {
                let cost = if arterial(r, c) && arterial(r + 1, c) {
                    1.0
                } else {
                    1.5
                };
                weighted_edge(&mut ir, id, id + w, cost);
            }
        }
    }
    ir
}

/// A deterministic diagonal obstacle set for a `w x h` grid (blocks a broken diagonal wall, leaving a
/// reachable detour). Seeded only by the dimensions.
pub fn map_obstacles(w: usize, h: usize) -> HashSet<usize> {
    let mut s = HashSet::new();
    let d = w.min(h);
    for i in 1..(d - 1) {
        if !i.is_multiple_of(5) {
            // leave a gap every 5th cell so the grid stays connected
            s.insert(i * w + i);
        }
    }
    s
}

/// `map_grid_ir` plus one genuinely multiway **transport** hyperedge (a metro line): an arity-`k` arc
/// connecting `k` evenly-spaced stations at a flat cheap `fare`. This edge is *not* metric-admissible
/// (its fare can be below the station-pair euclidean distance), which is exactly why the discrete
/// heuristic is needed there; returned separately so metric tests can use the pure grid.
pub fn map_grid_with_transport_ir(
    w: usize,
    h: usize,
    blocked: &HashSet<usize>,
    k: usize,
    fare: f64,
) -> Ir {
    let mut ir = map_grid_ir(w, h, blocked);
    let stations: Vec<usize> = (0..k).map(|i| (i * (w * h - 1)) / (k - 1)).collect();
    let refs = stations
        .into_iter()
        .filter(|s| !blocked.contains(s))
        .map(|s| {
            ReferenceDraft::new(
                IncidenceSign::Plus,
                RefAtomR::plain(DeclId::new(s)).with_weights(Some(vec![ValueR::Num(fare)])),
            )
        })
        .collect();
    push_arc(&mut ir, refs);
    ir
}

/// Unit-hop Berge distances from `goal` to every reachable node (a plain BFS over the incidence index),
/// used to prepare the discrete heuristic. `u32::MAX` for unreachable. Engine-independent.
pub fn hop_distances_from(ir: &Ir, index: &IncidenceIndex<'_>, goal: NodeId) -> Vec<u32> {
    let n = ir.decl_nodes.len();
    let mut dist = vec![u32::MAX; n];
    let goal_d = ir.nodes[goal.raw()].decl.raw();
    dist[goal_d] = 0;
    let mut frontier = vec![DeclId::new(goal_d)];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for u in frontier {
            let du = dist[u.raw()];
            for &arc in index.incident_arcs(u) {
                for (slot, _) in ir.arcs[arc.raw()].refs.iter().enumerate() {
                    let v = index.resolve(arc, slot as u32).target;
                    if dist[v.raw()] == u32::MAX {
                        dist[v.raw()] = du + 1;
                        next.push(v);
                    }
                }
            }
        }
        frontier = next;
    }
    dist
}

/// The first numeric weight of a signed ref (0.0 if none) — public accessor for consistency checks.
pub fn first_num_weight_pub(ir: &Ir, sref: &SignedRefR) -> f64 {
    let w = ir.reference_atom(sref).unwrap().weights;
    first_num_weight(w)
}

/// A deterministic serpentine **corridor** map on a `w x h` grid: horizontal runs on every row, vertical
/// links only at alternating turn columns, so the goal sits behind a long forced path (the discriminating
/// workload where a tight heuristic can prune). Coordinates + unit weights live in the Ir.
pub fn corridor_map_ir(w: usize, h: usize) -> Ir {
    assert!(w >= 3 && h >= 3);
    let mut ir = Ir::default();
    for r in 0..h {
        for c in 0..w {
            push_positioned_node(&mut ir, c as f64, r as f64);
        }
    }
    for r in 0..h {
        for c in 0..(w - 1) {
            weighted_edge(&mut ir, r * w + c, r * w + c + 1, 1.0);
        }
        if r + 1 < h {
            let turn = if r.is_multiple_of(2) { w - 1 } else { 0 };
            weighted_edge(&mut ir, r * w + turn, (r + 1) * w + turn, 1.0);
        }
    }
    ir
}

/// Read-only 2-D positions for this test workload vocabulary, keyed by DeclId.
pub struct MapPositions(pub Vec<Option<[f64; 2]>>);
impl MapPositions {
    pub fn from_ir(ir: &Ir) -> Self {
        Self(
            (0..ir.decl_nodes.len())
                .map(|raw| ir_position(ir, DeclId::new(raw)))
                .collect(),
        )
    }

    pub fn from_grid(width: usize, height: usize) -> Self {
        let positions = grid_positions(width, height)
            .into_iter()
            .map(Some)
            .collect();
        Self(positions)
    }
}
impl hymeko_graph::direct_ir::PositionProvider for MapPositions {
    fn position(&self, node: DeclId) -> Option<&[f64]> {
        self.0
            .get(node.raw())
            .and_then(Option::as_ref)
            .map(|position| position.as_slice())
    }
}
