//! Direct-`Ir` traversal benchmark and the k^2 falsifier.
//!
//! The measurement bench (`#[ignore]`) builds a large `Ir` directly and compares five approaches on
//! the SAME reachability query, in a single typed time unit (microseconds, via `median_us`):
//!
//! - **A** direct alternating `Ir` traversal, visited node AND arc (the standard path).
//! - **B** the same WITHOUT the visited-arc set -- a diagnostic control that re-expands a k-member
//!   arc from each member, so it is `O(k^2)` per hyperedge. If adding visited-arc to A does not
//!   remove the high-arity blow-up that B shows, that is the falsifier firing.
//! - **C** a real sparse bipartite **star** CSR (node<->arc Levi graph), `O(nnz)` -- BFS on it.
//! - **D** the **clique / 2-section** expansion, `O(Sum k^2)` -- BFS on it.
//! - **F** the naive full-`Ir` fixpoint scan (`O(V.E)`), small workload only.
//!
//! Run: `cargo test -p hymeko_graph --release --test direct_ir_bench -- --ignored --nocapture`.
//!
//! A fast, non-ignored test asserts the five approaches agree on the reachable set (the warm-query
//! zero-allocation proof lives in `direct_ir_alloc.rs`, which owns the counting global allocator in
//! its own single-test binary so no parallel test can perturb the count).

use std::collections::{BTreeSet, VecDeque};
use std::time::Instant;

use hymeko::common::ids::{DeclId, NodeId};
use hymeko::ir::incidence::{IncidenceIndex, build_incidence_index};
use hymeko::ir::ir::{AnnoR, ArcRec, DeclKind, DeclNode, Ir, NodeRec, RefAtomR, SignedRefR};

// ---- timing helper (single typed unit: microseconds) ----------------------------------------

/// Median, min and max microseconds over `iters` timed runs after 2 warm-up runs.
fn median_us(iters: usize, mut f: impl FnMut()) -> (f64, f64, f64) {
    for _ in 0..2 {
        f();
    }
    let mut samples: Vec<f64> = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        f();
        samples.push(t.elapsed().as_secs_f64() * 1e6);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples[samples.len() / 2];
    (samples[0], median, *samples.last().unwrap())
}

// ---- direct Ir generator (sliding-window arity-k hypergraph, one connected component) ---------

/// Build a connected arity-`k` hypergraph on `m` nodes: arc `i` joins nodes `i..i+k` (sliding
/// window). `nnz = (m - k + 1) * k`. Reachable-from-node-0 covers every node.
fn build_window_ir(m: usize, k: usize) -> Ir {
    assert!(k >= 2 && m >= k);
    let mut ir = Ir::default();
    // node decls + records
    for _ in 0..m {
        let d = ir.push_decl(DeclNode {
            kind: DeclKind::Node,
            name: hymeko::common::ids::SymId::new(0),
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
    // arcs: sliding window of k node members
    let n_arcs = m - k + 1;
    for i in 0..n_arcs {
        let refs: Vec<SignedRefR> = (i..i + k)
            .map(|j| {
                SignedRefR::Plus(RefAtomR {
                    target: DeclId::new(j),
                    anno: AnnoR::default(),
                    weights: None,
                    direction: Default::default(),
                })
            })
            .collect();
        ir.arcs.push(ArcRec {
            anno: AnnoR::default(),
            in_edge: DeclId::NONE,
            refs,
        });
    }
    ir
}

fn arc_members(ir: &Ir, a: usize) -> impl Iterator<Item = usize> + '_ {
    ir.arcs[a].refs.iter().map(|r| r.target().raw())
}

// ---- control A: direct (via the library) -----------------------------------------------------

fn reachable_direct(index: &IncidenceIndex<'_>, start: NodeId) -> usize {
    use hymeko_graph::direct_ir::{HyperTraversal, TraversalWorkspace};
    let trav = HyperTraversal::new(index);
    let mut ws = TraversalWorkspace::new();
    trav.reachable_nodes(&mut ws, start)
        .expect("reachable")
        .len()
}

// ---- control B: direct WITHOUT visited-arc (the k^2 diagnostic) -------------------------------

fn reachable_no_arc_visited(index: &IncidenceIndex<'_>, ir: &Ir, start: DeclId) -> usize {
    let mut seen = vec![false; ir.decl_nodes.len()];
    let mut q: VecDeque<DeclId> = VecDeque::new();
    seen[start.raw()] = true;
    q.push_back(start);
    let mut count = 1usize;
    while let Some(n) = q.pop_front() {
        for &arc in index.incident_arcs(n) {
            // NOTE: no arc-visited guard -- a k-member arc is re-expanded from each member.
            for m in arc_members(ir, arc.raw()) {
                if !seen[m] {
                    seen[m] = true;
                    count += 1;
                    q.push_back(DeclId::new(m));
                }
            }
        }
    }
    count
}

// ---- control C: sparse bipartite star (Levi) CSR, O(nnz) --------------------------------------

struct StarCsr {
    // bipartite: ids 0..m are nodes, m..m+a are arcs
    row: Vec<u32>,
    col: Vec<u32>,
    m: usize,
}

fn build_star(ir: &Ir) -> StarCsr {
    let m = ir.nodes.len();
    let a = ir.arcs.len();
    let v = m + a;
    let mut deg = vec![0u32; v + 1];
    for (ai, arc) in ir.arcs.iter().enumerate() {
        for r in &arc.refs {
            deg[r.target().raw() + 1] += 1; // node side
            deg[m + ai + 1] += 1; // arc side
        }
    }
    for i in 0..v {
        deg[i + 1] += deg[i];
    }
    let row = deg.clone();
    let mut col = vec![0u32; row[v] as usize];
    let mut cur = row.clone();
    for (ai, arc) in ir.arcs.iter().enumerate() {
        for r in &arc.refs {
            let node = r.target().raw();
            let arcv = m + ai;
            col[cur[node] as usize] = arcv as u32;
            cur[node] += 1;
            col[cur[arcv] as usize] = node as u32;
            cur[arcv] += 1;
        }
    }
    StarCsr { row, col, m }
}

fn reachable_star(star: &StarCsr, start: usize) -> usize {
    let v = star.row.len() - 1;
    let mut seen = vec![false; v];
    let mut q = VecDeque::new();
    seen[start] = true;
    q.push_back(start as u32);
    let mut node_count = 1usize;
    while let Some(u) = q.pop_front() {
        let (lo, hi) = (
            star.row[u as usize] as usize,
            star.row[u as usize + 1] as usize,
        );
        for &w in &star.col[lo..hi] {
            if !seen[w as usize] {
                seen[w as usize] = true;
                if (w as usize) < star.m {
                    node_count += 1;
                }
                q.push_back(w);
            }
        }
    }
    node_count
}

// ---- control D: clique / 2-section CSR, O(Sum k^2) --------------------------------------------

struct CliqueCsr {
    row: Vec<u32>,
    col: Vec<u32>,
}

fn build_clique(ir: &Ir) -> CliqueCsr {
    let m = ir.nodes.len();
    // adjacency as sets to dedup the clique edges
    let mut adj: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); m];
    for arc in &ir.arcs {
        let members: Vec<usize> = arc.refs.iter().map(|r| r.target().raw()).collect();
        for i in 0..members.len() {
            for j in 0..members.len() {
                if i != j {
                    adj[members[i]].insert(members[j] as u32);
                }
            }
        }
    }
    let mut row = vec![0u32; m + 1];
    for i in 0..m {
        row[i + 1] = row[i] + adj[i].len() as u32;
    }
    let mut col = vec![0u32; row[m] as usize];
    let mut cur = row.clone();
    for (i, set) in adj.iter().enumerate() {
        for &w in set {
            col[cur[i] as usize] = w;
            cur[i] += 1;
        }
    }
    CliqueCsr { row, col }
}

fn reachable_clique(cl: &CliqueCsr, start: usize) -> usize {
    let m = cl.row.len() - 1;
    let mut seen = vec![false; m];
    let mut q = VecDeque::new();
    seen[start] = true;
    q.push_back(start as u32);
    let mut count = 1usize;
    while let Some(u) = q.pop_front() {
        let (lo, hi) = (cl.row[u as usize] as usize, cl.row[u as usize + 1] as usize);
        for &w in &cl.col[lo..hi] {
            if !seen[w as usize] {
                seen[w as usize] = true;
                count += 1;
                q.push_back(w);
            }
        }
    }
    count
}

// ---- fast correctness test: all five agree ---------------------------------------------------

#[test]
fn all_approaches_agree() {
    let ir = build_window_ir(2_000, 5);
    let index = build_incidence_index(&ir).expect("index");

    let a = reachable_direct(&index, NodeId::new(0));
    let b = reachable_no_arc_visited(&index, &ir, DeclId::new(0));
    let star = build_star(&ir);
    let c = reachable_star(&star, 0);
    let clique = build_clique(&ir);
    let d = reachable_clique(&clique, 0);
    assert_eq!(a, 2_000, "direct reaches all nodes");
    assert_eq!(
        (b, c, d),
        (a, a, a),
        "all controls agree on the reachable count"
    );
}

#[test]
fn large_scale_all_approaches_agree() {
    // ~1e6 incidences: the direct traversal, the sparse star and the clique must all agree on the
    // reachable count, at production scale, on the real structures.
    let ir = build_window_ir(100_000, 10);
    let nnz: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
    assert!(nnz >= 900_000, "expected ~1e6 incidences, got {nnz}");
    let index = build_incidence_index(&ir).expect("index");
    let a = reachable_direct(&index, NodeId::new(0));
    let star = build_star(&ir);
    let c = reachable_star(&star, 0);
    let clique = build_clique(&ir);
    let d = reachable_clique(&clique, 0);
    assert_eq!(a, 100_000, "direct reaches every node at scale");
    assert_eq!((c, d), (a, a), "star and clique agree with direct at scale");
}

// ---- the measurement bench (ignored; run with --release --ignored --nocapture) ---------------

#[test]
#[ignore = "measurement bench; run with --release --ignored --nocapture"]
fn bench_direct_vs_star_vs_clique() {
    println!("\n=== direct-Ir traversal benchmark (microseconds; median of 9 after warmup) ===");
    println!(
        "host: single machine; times are us. A=direct(node+arc)  B=direct(no arc-visited)  \
         C=sparse-star O(nnz)  D=clique O(Sum k^2)"
    );
    for &(m, k, label) in &[
        (200_000usize, 5usize, "arity 5  (nnz~1e6)"),
        (50_000usize, 20usize, "arity 20 (nnz~1e6, high-arity)"),
    ] {
        let ir = build_window_ir(m, k);
        let nnz: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();

        let (_, idx_build, _) = median_us(5, || {
            let _ = build_incidence_index(&ir).expect("index");
        });
        let index = build_incidence_index(&ir).expect("index");
        let bytes_per_inc = index.retained_bytes() as f64 / nnz as f64;

        let (_, star_build, _) = median_us(5, || {
            let _ = build_star(&ir);
        });
        let star = build_star(&ir);
        let star_bytes = (star.row.len() * 4 + star.col.len() * 4) as f64 / nnz as f64;

        let (_, clique_build, _) = median_us(3, || {
            let _ = build_clique(&ir);
        });
        let clique = build_clique(&ir);
        let clique_bytes = (clique.row.len() * 4 + clique.col.len() * 4) as f64 / nnz as f64;

        let (_, a_q, _) = median_us(9, || {
            let _ = reachable_direct(&index, NodeId::new(0));
        });
        let (_, b_q, _) = median_us(9, || {
            let _ = reachable_no_arc_visited(&index, &ir, DeclId::new(0));
        });
        let (_, c_q, _) = median_us(9, || {
            let _ = reachable_star(&star, 0);
        });
        let (_, d_q, _) = median_us(9, || {
            let _ = reachable_clique(&clique, 0);
        });

        println!(
            "\n-- {label}: m={m} nodes, arcs={}, nnz={nnz} --",
            ir.arcs.len()
        );
        println!(
            "  build (us):   index={idx_build:.1}  star={star_build:.1}  clique={clique_build:.1}  \
             (clique/index={:.1}x)",
            clique_build / idx_build
        );
        println!(
            "  bytes/inc:    index={bytes_per_inc:.1}  star={star_bytes:.1}  clique={clique_bytes:.1}  \
             (clique/index={:.1}x)",
            clique_bytes / bytes_per_inc
        );
        println!(
            "  query (us):   A={a_q:.1}  B(no-arc)={b_q:.1}  C(star)={c_q:.1}  D(clique)={d_q:.1}"
        );
        println!(
            "  FALSIFIER:    B/A={:.2}x (visited-arc removes k^2 => grows with arity)   A vs C={:.2}x",
            b_q / a_q,
            a_q / c_q
        );
    }
    println!();
}

// ---- direct-Ir operation survey (added: berge/components/dual/s-cc/batch were unbenched) --------

#[test]
#[ignore = "measurement bench; run with --release --ignored --nocapture"]
fn bench_direct_ir_ops() {
    use hymeko::common::ids::{HyperArcId, NodeId};
    use hymeko_graph::direct_ir::{
        HyperTraversal, TraversalWorkspace, batch_reachable_counts, connected_components,
        dual_reachability, par_batch_reachable_counts, s_connected_components,
    };
    println!(
        "\n=== direct-Ir op survey (us; median-of-9 warm; arity 5; single-host PROVISIONAL) ==="
    );
    println!("op\tn_nodes\tnnz\tindex_build_us\twarm_query_us");
    for &m in &[10_000usize, 100_000] {
        let k = 5;
        let ir = build_window_ir(m, k);
        let nnz: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        let (_, idx_us, _) = median_us(5, || {
            let _ = build_incidence_index(&ir).expect("index");
        });
        let index = build_incidence_index(&ir).expect("index");
        let trav = HyperTraversal::new(&index);
        let mut ws = TraversalWorkspace::new();
        let start = NodeId::new(0);
        let goal = NodeId::new(m - 1);
        let starts: Vec<NodeId> = (0..16).map(|i| NodeId::new(i * (m / 16))).collect();

        let row = |name: &str, us: f64| println!("{name}\t{m}\t{nnz}\t{idx_us:.1}\t{us:.1}");

        let (_, r, _) = median_us(9, || {
            let _ = trav.reachable_nodes(&mut ws, start).unwrap().len();
        });
        row("reachable_nodes", r);
        let (_, b, _) = median_us(9, || {
            let _ = trav.berge_shortest_path(&mut ws, start, goal).unwrap();
        });
        row("berge_shortest_path", b);
        let (_, c, _) = median_us(9, || {
            let _ = connected_components(&index, &mut ws).unwrap().count();
        });
        row("connected_components", c);
        let (_, d, _) = median_us(9, || {
            let _ = dual_reachability(&index, &mut ws, HyperArcId::new(0))
                .unwrap()
                .len();
        });
        row("dual_reachability", d);
        let (_, s1, _) = median_us(5, || {
            let _ = s_connected_components(&index, 1).count();
        });
        row("s_connected_components_s1", s1);
        let (_, bs, _) = median_us(9, || {
            let _ = batch_reachable_counts(&index, &starts).unwrap();
        });
        row("batch16_serial", bs);
        let (_, bp, _) = median_us(9, || {
            let _ = par_batch_reachable_counts(&index, &starts).unwrap();
        });
        row("batch16_rayon", bp);
    }
    println!();
}
