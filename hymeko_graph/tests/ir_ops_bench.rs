//! P3.4 §12A — the unified canonical-Ir construction / manipulation operation matrix. Every operation
//! uses only **public, sanctioned** APIs (`Ir::default`, `preallocate_from_index`, `push_decl`,
//! `bind_node_decl`, `push_anonymous_arc`, the public `nodes`/`arcs` fields the whole suite builds
//! through, `Clone`, `canonical_program_hash`). A thread-local counting allocator gives allocs/op and
//! bytes/op; timing gives latency/op and throughput; scaling is read across sizes. The Ir is **append-
//! only** on this revision — canonical deletion and in-place transactional update have no public API and
//! are reported NOT IMPLEMENTED rather than faked.
//!
//! Run: cargo test -p hymeko_graph --release --test ir_ops_bench measure_ir_ops -- --ignored --nocapture

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Instant;

use hymeko::common::ids::{DeclId, NodeId, SymId};
use hymeko::ir::canonical_hash::{CanonHashCfg, canonical_program_hash};
use hymeko::ir::ir::{AnnoR, ArcRec, DeclKind, DeclNode, Ir, NodeRec, RefAtomR, SignedRefR};
use hymeko::resolution::interner::Interner;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
}
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        let _ = BYTES.try_with(|c| c.set(c.get() + l.size() as u64));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static GA: Counting = Counting;
fn snap() -> (u64, u64) {
    (ALLOCS.with(|c| c.get()), BYTES.with(|c| c.get()))
}

fn node_decl(ir: &mut Ir) -> DeclId {
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
    ir.bind_node_decl(d, nid).expect("bind");
    d
}

/// Build a connected rank-2 path Ir of `n` nodes via the public construction API.
fn build_path(n: usize) -> Ir {
    let mut ir = Ir::default();
    let mut decls = Vec::with_capacity(n);
    for _ in 0..n {
        decls.push(node_decl(&mut ir));
    }
    for i in 1..n {
        ir.arcs.push(ArcRec {
            anno: AnnoR::default(),
            in_edge: DeclId::NONE,
            refs: vec![
                SignedRefR::Plus(RefAtomR {
                    target: decls[i - 1],
                    anno: AnnoR::default(),
                    weights: None,
                }),
                SignedRefR::Plus(RefAtomR {
                    target: decls[i],
                    anno: AnnoR::default(),
                    weights: None,
                }),
            ],
        });
    }
    ir
}

/// median (min actually — best-of) op-wall in nanoseconds over `reps`.
fn best_ns(reps: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..2 {
        f();
    }
    let mut best = f64::INFINITY;
    for _ in 0..reps {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_secs_f64() * 1e9);
    }
    best
}

#[test]
#[ignore = "12A Ir-ops matrix; --release --ignored --nocapture"]
fn measure_ir_ops() {
    let sizes = [1usize, 2, 63, 64, 65, 1024, 16384, 131072];
    let mut rows = vec![
        [
            "op",
            "size",
            "lat_per_op_ns",
            "throughput_ops_s",
            "allocs_per_op",
            "bytes_per_op",
            "total_bytes",
            "notes",
        ]
        .join("\t"),
    ];

    // scalar, size-independent: create empty Ir + preallocate.
    {
        let ns = best_ns(1000, || {
            std::hint::black_box(Ir::default());
        });
        rows.push(
            [
                "create_empty",
                "-",
                &format!("{ns:.1}"),
                &format!("{:.0}", 1e9 / ns),
                "~0",
                "~0",
                "0",
                "Ir::default()",
            ]
            .join("\t"),
        );
        let ns2 = best_ns(500, || {
            let mut ir = Ir::default();
            ir.preallocate_from_index(1024);
            std::hint::black_box(&ir);
        });
        rows.push(
            [
                "create_preallocate1k",
                "-",
                &format!("{ns2:.1}"),
                &format!("{:.0}", 1e9 / ns2),
                "-",
                "-",
                "0",
                "default + preallocate_from_index(1024)",
            ]
            .join("\t"),
        );
    }

    println!("\n# P3.4 §12A canonical-Ir construction/manipulation matrix (path topology)");
    println!(
        "{:<20} {:>8} {:>14} {:>12} {:>12}",
        "op", "size", "lat/op_ns", "allocs/op", "bytes/op"
    );

    for &n in &sizes {
        // add_node (push_decl + nodes.push + bind), per-op over n nodes.
        {
            let b = snap();
            let t = Instant::now();
            let mut ir = Ir::default();
            for _ in 0..n {
                node_decl(&mut ir);
            }
            let wall = t.elapsed().as_secs_f64();
            let d = (snap().0 - b.0, snap().1 - b.1);
            emit(
                &mut rows,
                "add_node",
                n,
                wall / n as f64,
                d.0 as f64 / n as f64,
                d.1 as f64 / n as f64,
                d.1,
                "push_decl+nodes.push+bind",
            );
        }
        // add_hyperedge (rank-2 arc with 2 refs), per-op over n edges on a 2n-node graph.
        {
            let mut ir = Ir::default();
            let decls: Vec<DeclId> = (0..2 * n).map(|_| node_decl(&mut ir)).collect();
            let b = snap();
            let t = Instant::now();
            for i in 0..n {
                ir.arcs.push(ArcRec {
                    anno: AnnoR::default(),
                    in_edge: DeclId::NONE,
                    refs: vec![
                        SignedRefR::Plus(RefAtomR {
                            target: decls[2 * i],
                            anno: AnnoR::default(),
                            weights: None,
                        }),
                        SignedRefR::Plus(RefAtomR {
                            target: decls[2 * i + 1],
                            anno: AnnoR::default(),
                            weights: None,
                        }),
                    ],
                });
            }
            let wall = t.elapsed().as_secs_f64();
            let d = (snap().0 - b.0, snap().1 - b.1);
            emit(
                &mut rows,
                "add_hyperedge",
                n,
                wall / n as f64,
                d.0 as f64 / n as f64,
                d.1 as f64 / n as f64,
                d.1,
                "arcs.push(ArcRec, 2 refs)",
            );
        }
        // clone the built path Ir.
        {
            let ir = build_path(n);
            let b = snap();
            let ns = best_ns(if n > 20000 { 5 } else { 50 }, || {
                std::hint::black_box(ir.clone());
            });
            let d = (snap().0 - b.0, snap().1 - b.1);
            let per = (d.0 as f64 / (if n > 20000 { 7.0 } else { 52.0 })).max(0.0);
            emit(
                &mut rows,
                "clone_ir",
                n,
                ns / 1e9,
                per,
                0.0,
                d.1,
                "Ir::clone",
            );
        }
        // canonical hash (structural; Interner-free on placeholder names).
        {
            let ir = build_path(n);
            let mut it = Interner::new();
            it.intern("n"); // SymId(0) — every generator decl is named with this placeholder
            let cfg = CanonHashCfg::default();
            let ns = best_ns(if n > 20000 { 5 } else { 30 }, || {
                std::hint::black_box(canonical_program_hash(cfg, &ir, &it));
            });
            emit(
                &mut rows,
                "canonical_hash",
                n,
                ns / 1e9,
                0.0,
                0.0,
                0,
                "canonical_program_hash",
            );
        }
    }

    // honest NOT-IMPLEMENTED / boundary rows.
    for (op, note) in [
        (
            "canonical_delete",
            "NOT IMPLEMENTED — Ir is append-only; no remove/delete/clear/replace public API on this revision (70b4f364)",
        ),
        (
            "transactional_update",
            "NOT IMPLEMENTED — no in-place transactional/mutation API; construction is append-only",
        ),
        (
            "serialize_deserialize",
            "boundary = ir_stream section protocol / IrWriter HyMeKo text — requires a real Interner + named decls; not exercised on placeholder-named generator Ir this session",
        ),
        (
            "validate_coherence",
            "build_path_index / validate_hierarchy_index_coherence require distinctly-named decls + Index; not exercised on placeholder-named generator Ir this session",
        ),
    ] {
        rows.push([op, "-", "-", "-", "-", "-", "-", note].join("\t"));
        println!("{op:<20} {:>8}  {note}", "-");
    }

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../reports/data/2026-09-03-p34-ir-ops.tsv"
    );
    std::fs::write(path, rows.join("\n") + "\n").unwrap();
    println!("\nwrote {path}");
}

#[allow(clippy::too_many_arguments)]
fn emit(
    rows: &mut Vec<String>,
    op: &str,
    n: usize,
    per_op_s: f64,
    allocs_per_op: f64,
    bytes_per_op: f64,
    total_bytes: u64,
    notes: &str,
) {
    let ns = per_op_s * 1e9;
    println!("{op:<20} {n:>8} {ns:>14.2} {allocs_per_op:>12.3} {bytes_per_op:>12.1}");
    rows.push(
        [
            op.to_string(),
            n.to_string(),
            format!("{ns:.2}"),
            format!("{:.0}", if per_op_s > 0.0 { 1.0 / per_op_s } else { 0.0 }),
            format!("{allocs_per_op:.3}"),
            format!("{bytes_per_op:.1}"),
            total_bytes.to_string(),
            notes.to_string(),
        ]
        .join("\t"),
    );
}
