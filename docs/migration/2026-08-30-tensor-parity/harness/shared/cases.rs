//! Shared parity cases: input construction and output formatting only.
//!
//! This module contains **no graph arithmetic**. It builds deterministic inputs, calls whichever
//! `hymeko_graph` the enclosing crate links as `hg`, and prints one line per observation. The two
//! sides emit an identical format; the comparison is a diff of their output, so a mismatch names
//! the case, the coordinate and the bit pattern rather than a summary.
//!
//! `to_bits()` throughout. A tolerance would hide exactly the reduction-order change that a
//! divergent second implementation is most likely to carry.

use hymeko_graph::spine::{
    clifford_fir_backward, clifford_fir_forward, fir_cycle_forward, fir_cycle_scatter_mean,
    CliffordFIR, SignedCycleFIR,
};
use hymeko_graph::topk_cycles::TopKCyclesBatch;

fn pseudo(len: usize, salt: u32) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let h = (i as u32)
                .wrapping_mul(2_654_435_761)
                .wrapping_add(salt.wrapping_mul(40_503));
            (h >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        })
        .collect()
}

fn batch(nv: usize, n: usize, k: usize, salt: u32) -> TopKCyclesBatch {
    let cycles: Vec<u32> = (0..n * k)
        .map(|i| ((i.wrapping_mul(7) + i / k + salt as usize) % nv) as u32)
        .collect();
    // Mixed signs, including runs of equal sign so both branches are exercised per cycle.
    let signs: Vec<i8> = (0..n * k)
        .map(|i| if (i + i / k + salt as usize) % 3 == 0 { -1 } else { 1 })
        .collect();
    TopKCyclesBatch {
        cycles,
        signs,
        scores: (0..n).map(|i| i as f64 * 0.25).collect(),
        k,
    }
}

fn emit(case: &str, name: &str, v: &[f32]) {
    // Every scalar, not a digest: a digest tells you THAT two runs differ, never where.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in v {
        for b in x.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    println!("{case}\t{name}\tlen={}\tdigest={h:016x}", v.len());
    for (i, x) in v.iter().enumerate() {
        println!("{case}\t{name}\t[{i}]\t{:08x}", x.to_bits());
    }
}

pub fn run<T>() {
    // (nv, n_cycles, k, d, salt)
    let shapes = [
        (6usize, 3usize, 3usize, 2usize, 1u32),
        (6, 3, 3, 4, 2),
        (12, 9, 3, 8, 3),
        (12, 9, 4, 8, 4),
        (20, 17, 5, 4, 5),
        (4, 2, 3, 1, 6),   // minimal
        (32, 41, 3, 16, 7),
    ];
    for (nv, n, k, d, salt) in shapes {
        let b = batch(nv, n, k, salt);
        let feats = pseudo(nv * d, salt ^ 0x11);
        let sfir = SignedCycleFIR {
            coef_pos: pseudo(k, salt ^ 0x21),
            coef_neg: pseudo(k, salt ^ 0x22),
        };
        let cfir = CliffordFIR {
            a: pseudo(k, salt ^ 0x31),
            b: pseudo(k, salt ^ 0x32),
        };
        let case = format!("nv{nv}_n{n}_k{k}_d{d}_s{salt}");

        emit(&case, "fir_cycle_forward", &fir_cycle_forward(&b, &feats, nv, d, &sfir));
        emit(&case, "fir_cycle_scatter_mean", &fir_cycle_scatter_mean(&b, &feats, nv, d, &sfir));
        let fwd = clifford_fir_forward(&b, &feats, nv, d, &cfir);
        emit(&case, "clifford_fir_forward", &fwd);

        let grad_out = pseudo(fwd.len(), salt ^ 0x41);
        let (gx, gfir) = clifford_fir_backward(&b, &feats, nv, d, &cfir, &grad_out);
        emit(&case, "clifford_fir_backward.grad_x", &gx);
        emit(&case, "clifford_fir_backward.a", &gfir.a);
        emit(&case, "clifford_fir_backward.b", &gfir.b);
    }
}

// ── the rest of the Nagare-reachable shared surface ───────────────────────────────────────────
//
// Chosen by census, not by guess: `signed_tuples` (8 references), `hyper_operator` (3), `spine`
// (2), `walks`, `unsigned`, `enumerate_simple_cycles_noprune`, `build_csr`. The first two are
// tensor-only and have no parity to check -- they are pure migration. The rest are shared and
// drifted, and `cycle_enum` is the one where GRAPH is newer, so switching gives Nagare a body it
// has never executed.

use hymeko_graph::signed_graph::{Sign, SignedGraph};
use hymeko_graph::unsigned_cycles::{bs_clear, bs_get, bs_set, bs_words, build_csr, has_edge, neighbours};
use hymeko_graph::enumerate_simple_cycles_noprune;
use hymeko_graph::walks_unsigned::enumerate_walks;

fn emit_u32(case: &str, name: &str, v: &[u32]) {
    println!("{case}\t{name}\tlen={}", v.len());
    for (i, x) in v.iter().enumerate() {
        println!("{case}\t{name}\t[{i}]\t{x}");
    }
}

fn signed_graph_fixture(nv: u32, salt: u32) -> SignedGraph {
    let mut u = Vec::new();
    let mut v = Vec::new();
    let mut s = Vec::new();
    for i in 0..(nv as usize * 3) {
        let a = ((i * 5 + salt as usize) % nv as usize) as u32;
        let b = ((i * 11 + 3 + salt as usize) % nv as usize) as u32;
        if a == b {
            continue;
        }
        u.push(a);
        v.push(b);
        s.push(if (i + salt as usize) % 4 == 0 { -1i8 } else { 1 });
    }
    SignedGraph::from_parts(nv, &u, &v, &s)
}

pub fn run_rest() {
    for (nv, salt) in [(8u32, 1u32), (12, 2), (20, 3), (32, 4)] {
        let case = format!("g_nv{nv}_s{salt}");
        let g = signed_graph_fixture(nv, salt);

        // signed_graph: shape and sign observables
        println!("{case}\tsigned_graph.n_edges\t{}", g.n_edges());

        // cycle_enum -- graph is NEWER here; this is the highest-risk shared path
        for k in [3usize, 4, 5] {
            let cyc = enumerate_simple_cycles_noprune(&g, k);
            println!("{case}\tcycles_k{k}.count\t{}", cyc.len());
            for (i, c) in cyc.iter().enumerate() {
                println!("{case}\tcycles_k{k}\t[{i}]\t{c:?}");
            }
        }

        // unsigned_cycles::csr -- construction, membership, boundaries
        let edges: Vec<(u32, u32)> = (0..nv as usize * 2)
            .map(|i| {
                (
                    ((i * 3 + salt as usize) % nv as usize) as u32,
                    ((i * 7 + 1 + salt as usize) % nv as usize) as u32,
                )
            })
            .collect();
        for directed in [false, true] {
            let (row_ptr, col_idx) = build_csr(&edges, nv as usize, directed);
            emit_u32(&case, &format!("csr_d{directed}.row_ptr"), &row_ptr);
            emit_u32(&case, &format!("csr_d{directed}.col_idx"), &col_idx);
            for a in 0..nv {
                let nb = neighbours(&row_ptr, &col_idx, a);
                println!("{case}\tcsr_d{directed}.nb[{a}]\t{nb:?}");
                for b in 0..nv {
                    println!(
                        "{case}\tcsr_d{directed}.has_edge[{a},{b}]\t{}",
                        has_edge(&row_ptr, &col_idx, a, b)
                    );
                }
            }
        }
    }
    // Sign round-trip: a tiny contract, cheap to check, and it would catch an enum reorder.
    for s in [-1i8, 1] {
        let sg = Sign::from_i8(s);
        println!("sign\tfrom_i8({s}).as_i8\t{}", sg.as_i8());
    }
    // walks_unsigned: real signature is (edges_u, edges_v, n_nodes, walk_len, max_walks, seed).
    let eu: Vec<u32> = (0..20u32).map(|i| i % 10).collect();
    let ev: Vec<u32> = (0..20u32).map(|i| (i * 3 + 1) % 10).collect();
    for wl in [2usize, 3] {
        for seed in [1u64, 2] {
            let w = enumerate_walks(&eu, &ev, 10, wl, Some(64), seed);
            println!("walks_l{wl}_s{seed}.count\t{}", w.len());
            for (i, x) in w.iter().enumerate() {
                println!("walks_l{wl}_s{seed}\t[{i}]\t{x:?}");
            }
        }
    }

    // CSR bitsets: construction, membership, and the word boundary at 63/64/65.
    for n in [1usize, 63, 64, 65, 129] {
        let words = bs_words(n);
        println!("bs\twords({n})\t{words}");
        let mut bits = vec![0u64; words];
        for v in [0u32, 1, 62, 63, 64, 100] {
            if (v as usize) < n {
                bs_set(&mut bits, v);
                println!("bs\tn{n}.set_get({v})\t{}", bs_get(&bits, v));
                bs_clear(&mut bits, v);
                println!("bs\tn{n}.after_clear({v})\t{}", bs_get(&bits, v));
            }
        }
        println!("bs\tn{n}.words_hash\t{:016x}", bits.iter().fold(0u64, |a, w| a ^ w.rotate_left(7)));
    }
}
