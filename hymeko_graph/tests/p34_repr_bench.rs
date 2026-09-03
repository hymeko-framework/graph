//! P3.4 §4/§5/§8 — Levi performance, the connectivity-certificate break-even, and the representation
//! break-even against benchmark-only controls. **No control here is production**: the sparse star CSR and
//! the clique/2-section expansion are built inside this test purely to be compared against the canonical
//! direct-Ir + `IncidenceIndex` path and the Levi spanning-basis certificate. Every representation
//! computes the SAME quantity — the node connected-component count `C_node` — so they cross-check each
//! other, and their per-query cost is compared under `total(Q) = prep + Q·query`.
//!
//! Run: cargo test -p hymeko_graph --release --test p34_repr_bench measure_ -- --ignored --nocapture

use hymeko::ir::incidence::build_incidence_index;
use hymeko::ir::ir::Ir;
use hymeko_graph::{
    GraphicMatroidWorkspace, LeviCoord, MatroidRef, TraversalWorkspace, connected_components,
    levi_components, levi_spanning_basis,
};

#[path = "informed_common/mod.rs"]
mod ic;

// ---- a from-scratch DSU (benchmark-only; over an arbitrary dense coordinate space) ----------------
struct Dsu {
    parent: Vec<u32>,
}
impl Dsu {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
        }
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let gp = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = gp;
            x = gp;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra as usize] = rb;
        }
    }
    fn components_over(&mut self, n: usize) -> usize {
        use std::collections::BTreeSet;
        (0..n as u32)
            .map(|x| self.find(x))
            .collect::<BTreeSet<_>>()
            .len()
    }
}

fn node_of(ir: &Ir, arc: usize, slot: usize) -> Option<usize> {
    ir.as_node(ir.arcs[arc].refs[slot].target())
        .map(|n| n.raw())
}

/// C_node via the no-index scan: DSU over nodes, unioning each arc's first node with its others.
fn c_node_no_index(ir: &Ir) -> usize {
    let n = ir.nodes.len();
    let mut dsu = Dsu::new(n);
    for arc in &ir.arcs {
        let mut first: Option<usize> = None;
        for slot in 0..arc.refs.len() {
            if let Some(nd) = ir.as_node(arc.refs[slot].target()).map(|x| x.raw()) {
                match first {
                    None => first = Some(nd),
                    Some(f) => dsu.union(f as u32, nd as u32),
                }
            }
        }
    }
    dsu.components_over(n)
}

/// A genuine sparse **star / bipartite** CSR (node⊕arc vertices, one entry per incidence), built once.
struct StarCsr {
    row: Vec<u32>,
    col: Vec<u32>,
    n_nodes: usize,
    vtx: usize,
}
impl StarCsr {
    fn build(ir: &Ir) -> Self {
        let n = ir.nodes.len();
        let a = ir.arcs.len();
        let vtx = n + a;
        let mut deg = vec![0u32; vtx + 1];
        for (ai, arc) in ir.arcs.iter().enumerate() {
            for slot in 0..arc.refs.len() {
                if let Some(nd) = node_of(ir, ai, slot) {
                    deg[nd] += 1;
                    deg[n + ai] += 1;
                }
            }
        }
        let mut row = vec![0u32; vtx + 1];
        for i in 0..vtx {
            row[i + 1] = row[i] + deg[i];
        }
        let mut cur = row.clone();
        let mut col = vec![0u32; row[vtx] as usize];
        for (ai, arc) in ir.arcs.iter().enumerate() {
            for slot in 0..arc.refs.len() {
                if let Some(nd) = node_of(ir, ai, slot) {
                    let (u, v) = (nd as u32, (n + ai) as u32);
                    col[cur[u as usize] as usize] = v;
                    cur[u as usize] += 1;
                    col[cur[v as usize] as usize] = u;
                    cur[v as usize] += 1;
                }
            }
        }
        Self {
            row,
            col,
            n_nodes: n,
            vtx,
        }
    }
    fn retained_bytes(&self) -> usize {
        (self.row.capacity() + self.col.capacity()) * 4
    }
    /// C_node = components of the bipartite graph restricted to the ones containing ≥1 node vertex.
    fn c_node(&self) -> usize {
        let mut dsu = Dsu::new(self.vtx);
        for u in 0..self.vtx as u32 {
            for k in self.row[u as usize]..self.row[u as usize + 1] {
                dsu.union(u, self.col[k as usize]);
            }
        }
        use std::collections::BTreeSet;
        (0..self.n_nodes as u32)
            .map(|x| dsu.find(x))
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// A clique / 2-section expansion: every pair of nodes co-incident on a hyperarc becomes a node-node
/// edge. Prep is Θ(Σ k²) — it blows up for large hyperarcs (a documented cost, not a bug).
struct Clique {
    edges: Vec<(u32, u32)>,
    n_nodes: usize,
}
impl Clique {
    fn build(ir: &Ir) -> Self {
        let n = ir.nodes.len();
        let mut edges = Vec::new();
        for (ai, arc) in ir.arcs.iter().enumerate() {
            let members: Vec<u32> = (0..arc.refs.len())
                .filter_map(|s| node_of(ir, ai, s).map(|x| x as u32))
                .collect();
            for i in 0..members.len() {
                for j in (i + 1)..members.len() {
                    edges.push((members[i], members[j]));
                }
            }
        }
        Self { edges, n_nodes: n }
    }
    fn retained_bytes(&self) -> usize {
        self.edges.capacity() * std::mem::size_of::<(u32, u32)>()
    }
    fn c_node(&self) -> usize {
        let mut dsu = Dsu::new(self.n_nodes);
        for &(u, v) in &self.edges {
            dsu.union(u, v);
        }
        dsu.components_over(self.n_nodes)
    }
}

fn med(iters: usize, f: impl FnMut()) -> f64 {
    let (_, m, _) = ic::median_us(iters, f);
    m
}

// ---------------------------------------------------------------------------------------------------
// §4 — Levi performance (facade flat across sizes; prep; basis; ns/incidence; retained; output bytes).
// ---------------------------------------------------------------------------------------------------

#[test]
#[ignore = "bench; --release --ignored --nocapture"]
fn measure_levi_performance() {
    let cases: Vec<(&str, Ir)> = vec![
        ("empty", Ir::default()),
        (
            "path1k",
            ic::window_ir(1024, 2, false, ic::SignMode::AllPlus),
        ),
        ("sts127", ic::steiner_sts_ir(7)),
        ("sts1023", ic::steiner_sts_ir(10)),
        (
            "random1e6",
            ic::uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3),
        ),
    ];
    let mut rows = vec![
        [
            "workload",
            "nodes",
            "arcs",
            "incidences",
            "levi_vtx",
            "components",
            "rank",
            "basis_ratio",
            "facade_ns",
            "basis_us",
            "ns_per_inc",
            "levi_ws_b",
            "out_b",
        ]
        .join("\t"),
    ];
    println!("\n# P3.4 §4 Levi performance");
    println!(
        "{:<10} {:>9} {:>10} {:>9} {:>10} {:>9} {:>10} {:>9}",
        "workload", "inc", "levi_vtx", "comps", "rank", "facade_ns", "basis_us", "ns/inc"
    );
    for (name, ir) in &cases {
        let idx = build_incidence_index(ir).unwrap();
        let inc: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        let vtx = ir.nodes.len() + ir.arcs.len();
        let mut ws = GraphicMatroidWorkspace::new();
        let mut out: Vec<LeviCoord> = Vec::new();

        // facade construction: nanoseconds, must be flat across sizes (O(1)).
        let facade_ns = med(200, || {
            let m = MatroidRef::new(&idx);
            std::hint::black_box(m.incidence());
        }) * 1000.0;

        let rank = levi_spanning_basis(MatroidRef::new(&idx), &mut ws, &mut out).unwrap();
        let comps = levi_components(MatroidRef::new(&idx), &mut ws).unwrap();
        let basis_us = med(if inc > 200_000 { 5 } else { 20 }, || {
            let _ = levi_spanning_basis(MatroidRef::new(&idx), &mut ws, &mut out).unwrap();
        });
        let ns_per_inc = if inc > 0 {
            basis_us * 1000.0 / inc as f64
        } else {
            0.0
        };
        let ratio = if inc > 0 {
            rank as f64 / inc as f64
        } else {
            0.0
        };
        let out_b = out.capacity() * std::mem::size_of::<LeviCoord>();

        println!(
            "{name:<10} {inc:>9} {vtx:>10} {comps:>9} {rank:>10} {facade_ns:>9.2} {basis_us:>10.2} {ns_per_inc:>9.2}"
        );
        rows.push(
            [
                name.to_string(),
                ir.nodes.len().to_string(),
                ir.arcs.len().to_string(),
                inc.to_string(),
                vtx.to_string(),
                comps.to_string(),
                rank.to_string(),
                format!("{ratio:.4}"),
                format!("{facade_ns:.2}"),
                format!("{basis_us:.3}"),
                format!("{ns_per_inc:.3}"),
                ws.retained_bytes().to_string(),
                out_b.to_string(),
            ]
            .join("\t"),
        );
    }
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../reports/data/2026-09-03-p34-levi-perf.tsv"
    );
    std::fs::write(path, rows.join("\n") + "\n").unwrap();
    println!("wrote {path}");
}

// ---------------------------------------------------------------------------------------------------
// §5 + §8 — representation break-even (prep + Q·query), all computing C_node; break-even vs direct.
// ---------------------------------------------------------------------------------------------------

#[test]
#[ignore = "bench; --release --ignored --nocapture"]
fn measure_representation_breakeven() {
    let workloads: Vec<(&str, Ir)> = vec![
        ("sts1023", ic::steiner_sts_ir(10)),
        (
            "map512",
            ic::map_grid_ir(512, 512, &ic::map_obstacles(512, 512)),
        ),
        (
            "random1e6",
            ic::uniform_rank_k_ir(0x51EED, 200_000, 333_333, 3),
        ),
    ];
    let mut rows = vec![
        [
            "workload",
            "representation",
            "prep_us",
            "warm_query_us",
            "retained_b",
            "total_q1_us",
            "total_q16_us",
            "total_q256_us",
            "breakeven_vs_direct",
            "c_node",
            "semantic_loss",
        ]
        .join("\t"),
    ];

    for (wname, ir) in &workloads {
        let idx = build_incidence_index(ir).unwrap();
        let mut tw = TraversalWorkspace::new();

        // ground truth C_node (must match every representation).
        let truth = connected_components(&idx, &mut tw).unwrap().count() as usize;

        // per representation: (prep_us, warm_query_us, retained_b, c_node, semantic_loss)
        let mut recs: Vec<(&str, f64, f64, usize, usize, &str)> = Vec::new();

        // (1) direct: no prep; each query recomputes connected_components over the canonical Ir+index.
        {
            let q = med(10, || {
                let _ = connected_components(&idx, &mut tw).unwrap().count();
            });
            recs.push((
                "direct",
                0.0,
                q,
                idx.retained_bytes(),
                truth,
                "none (canonical)",
            ));
        }
        // (2) no-index scan: no prep; each query = DSU over arcs directly.
        {
            let mut c = 0usize;
            let q = med(10, || c = c_node_no_index(ir));
            recs.push(("no_index_scan", 0.0, q, 0, c, "none; but no reusable index"));
        }
        // (3) sparse star CSR (built once).
        {
            let mut star = None;
            let prep = med(5, || star = Some(StarCsr::build(ir)));
            let star = star.unwrap();
            let mut c = 0usize;
            let q = med(10, || c = star.c_node());
            recs.push((
                "sparse_star_csr",
                prep,
                q,
                star.retained_bytes(),
                c,
                "signs, weights, hyperedge identity",
            ));
        }
        // (4) clique / 2-section (built once).
        {
            let mut cl = None;
            let prep = med(5, || cl = Some(Clique::build(ir)));
            let cl = cl.unwrap();
            let mut c = 0usize;
            let q = med(10, || c = cl.c_node());
            recs.push((
                "clique_2section",
                prep,
                q,
                cl.retained_bytes(),
                c,
                "hyperedge cardinality (Sigma k^2 blow-up)",
            ));
        }
        // (5) Levi spanning-basis certificate: prep = basis; query = O(1) (C_node = C_levi - Z=0 here).
        {
            let mut lws = GraphicMatroidWorkspace::new();
            let mut out: Vec<LeviCoord> = Vec::new();
            let prep = med(5, || {
                let _ = levi_spanning_basis(MatroidRef::new(&idx), &mut lws, &mut out).unwrap();
            });
            let c = levi_components(MatroidRef::new(&idx), &mut lws).unwrap(); // C_levi (== C_node when Z=0)
            let q = med(1000, || {
                std::hint::black_box(c);
            });
            recs.push((
                "levi_certificate",
                prep,
                q,
                lws.retained_bytes(),
                c,
                "everything but connectivity",
            ));
        }

        let direct_q = recs[0].2;
        println!("\n# §8 representation break-even — {wname} (truth C_node = {truth})");
        println!(
            "{:<18} {:>9} {:>11} {:>11} {:>9}",
            "representation", "prep_us", "warm_q_us", "retained_KiB", "breakeven"
        );
        for (rep, prep, q, ret, c, loss) in &recs {
            assert_eq!(*c, truth, "[{wname}/{rep}] C_node must match ground truth");
            let t = |qq: f64| prep + qq * q;
            assert!(
                (t(1.0) - (prep + q)).abs() < 1e-6,
                "break-even total(1) must include preparation (guards prep-excluded-from-break-even)"
            );
            // break-even vs direct: smallest Q>=1 with prep + Q*q < Q*direct_q.
            let be = if *q >= direct_q {
                if *prep <= 0.0 {
                    "tie/NEVER".to_string()
                } else {
                    "NEVER".to_string()
                }
            } else {
                let qstar = (prep / (direct_q - q)).ceil().max(1.0);
                format!("Q>={}", qstar as u64)
            };
            println!(
                "{rep:<18} {prep:>9.2} {q:>11.4} {:>11.1} {be:>9}",
                *ret as f64 / 1024.0
            );
            rows.push(
                [
                    wname.to_string(),
                    rep.to_string(),
                    format!("{prep:.3}"),
                    format!("{q:.4}"),
                    ret.to_string(),
                    format!("{:.3}", t(1.0)),
                    format!("{:.3}", t(16.0)),
                    format!("{:.3}", t(256.0)),
                    be,
                    c.to_string(),
                    loss.to_string(),
                ]
                .join("\t"),
            );
        }
    }
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../reports/data/2026-09-03-p34-representation.tsv"
    );
    std::fs::write(path, rows.join("\n") + "\n").unwrap();
    println!("\nwrote {path}");
}
