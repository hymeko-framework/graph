//! End-to-end P-graph pipeline benches: parse → lower → MSG → ABB
//! and standalone MSG / SSG / ABB on synthetic chain instances.
//!
//! Run:  `cargo bench -p hymeko_pgraph`

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use hymeko_pgraph::abb::solve_with_options;
use hymeko_pgraph::msg::{MaximalStructureOptions, maximal_structure_with_options};
use hymeko_pgraph::{
    AbbOptions, abb_solve, lower, maximal_structure, ssg_dm_enumerate, ssg_enumerate,
};
use parser::parse_description;

const HDA_SRC: &str = include_str!("../../data/pgraph/hda.hymeko");
// Real book instances (Friedler/Orosz/Pimentel, P-graphs for PSE):
//   Example 3.3 — 35 units, 3465 solution-structures (decision-mapping SSG).
//   Example 14.1 — 12 units, ABB optimum 16.
const EX3_3_SRC: &str = include_str!("../../data/pgraph/Chapter4/example4_3.hymeko");
const EX14_1_SRC: &str = include_str!("../../data/pgraph/book/example14_1.hymeko");

/// Generate a synthetic linear-chain P-graph with `n_units` units and
/// one product. Each unit consumes one intermediate and produces the
/// next — pure feed-through with cost = i.
fn gen_chain_pgraph(n_units: usize) -> String {
    let mut s = String::new();
    s.push_str("Chain{}\n");
    s.push_str("context {\n");
    s.push_str("    M0 <material, raw>;\n");
    for i in 1..=n_units {
        let tag = if i == n_units {
            "<material, product>"
        } else {
            "<material>"
        };
        s.push_str(&format!("    M{i} {tag};\n"));
    }
    for i in 0..n_units {
        s.push_str(&format!(
            "    @U{i} <unit> {} {{ (-M{i}, +M{j}); }}\n",
            (i as f64 + 1.0),
            j = i + 1,
        ));
    }
    s.push_str("}\n");
    s
}

/// Branching P-graph: each level doubles the number of intermediates;
/// at depth $d$ there are $2^d$ materials.  Stresses MSG/ABB on a
/// graph that grows quickly.
fn gen_tree_pgraph(depth: u32) -> String {
    let mut s = String::new();
    s.push_str("Tree{}\n");
    s.push_str("context {\n");
    s.push_str("    M0_0 <material, raw>;\n");
    for d in 1..=depth {
        let nodes = 1u32 << d;
        for i in 0..nodes {
            let tag = if d == depth {
                "<material, product>"
            } else {
                "<material>"
            };
            s.push_str(&format!("    M{d}_{i} {tag};\n"));
        }
    }
    let mut uid = 0u32;
    for d in 1..=depth {
        let parents = 1u32 << (d - 1);
        for p in 0..parents {
            for c in 0..2u32 {
                let child = 2 * p + c;
                s.push_str(&format!(
                    "    @U{uid} <unit> 1 {{ (-M{}_{}, +M{}_{}); }}\n",
                    d - 1,
                    p,
                    d,
                    child,
                ));
                uid += 1;
            }
        }
    }
    s.push_str("}\n");
    s
}

fn bench_parse_lower(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_lower");
    for &n in &[8usize, 32, 128, 512] {
        let src = gen_chain_pgraph(n);
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_with_input(BenchmarkId::new("chain", n), &n, |b, _| {
            b.iter(|| {
                let d = parse_description(&src).unwrap();
                let p = lower(&d).unwrap();
                black_box(p.units.len())
            })
        });
    }
    // HDA — the hand-written reference.
    group.bench_function(BenchmarkId::new("hda_reference", "hda"), |b| {
        b.iter(|| {
            let d = parse_description(HDA_SRC).unwrap();
            let p = lower(&d).unwrap();
            black_box(p.units.len())
        })
    });
    group.finish();
}

fn bench_msg(c: &mut Criterion) {
    let mut group = c.benchmark_group("msg");
    for &n in &[8usize, 32, 128, 512, 2048] {
        let src = gen_chain_pgraph(n);
        let d = parse_description(&src).unwrap();
        let p = lower(&d).unwrap();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("chain", n), &n, |b, _| {
            b.iter(|| {
                let m = maximal_structure(&p);
                black_box(m.units.len())
            })
        });
    }
    group.finish();
}

fn bench_ssg(c: &mut Criterion) {
    let mut group = c.benchmark_group("ssg");
    // SSG is 2^|O_max| — keep the chain small.
    for &n in &[4usize, 8, 16, 24] {
        let src = gen_chain_pgraph(n);
        let d = parse_description(&src).unwrap();
        let p = lower(&d).unwrap();
        let m = maximal_structure(&p);
        group.throughput(Throughput::Elements((1u64) << n));
        group.bench_with_input(BenchmarkId::new("chain", n), &n, |b, _| {
            b.iter(|| {
                let s = ssg_enumerate(&p, &m);
                black_box(s.len())
            })
        });
    }
    group.finish();
}

fn bench_abb(c: &mut Criterion) {
    let mut group = c.benchmark_group("abb");
    // ABB scales much further than SSG thanks to BnB.
    for &n in &[4usize, 8, 16, 32, 64, 128] {
        let src = gen_chain_pgraph(n);
        let d = parse_description(&src).unwrap();
        let p = lower(&d).unwrap();
        let m = maximal_structure(&p);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("chain", n), &n, |b, _| {
            b.iter(|| {
                let s = abb_solve(&p, &m);
                black_box(s.is_some())
            })
        });
    }
    // Tree instances stress ABB's reachability bound.
    for &d_lvl in &[3u32, 4, 5] {
        let src = gen_tree_pgraph(d_lvl);
        let d = parse_description(&src).unwrap();
        let p = lower(&d).unwrap();
        let m = maximal_structure(&p);
        let n_units = p.units.len() as u64;
        group.throughput(Throughput::Elements(n_units));
        group.bench_with_input(BenchmarkId::new("tree_depth", d_lvl), &d_lvl, |b, _| {
            b.iter(|| {
                let s = abb_solve(&p, &m);
                black_box(s.is_some())
            })
        });
    }
    group.finish();
}

/// Literature-anchored benches on the real book instances: the
/// decision-mapping SSG enumerating Example 3.3's 3465 solution-structures
/// (the case the brute 2^n SSG cannot reach), and ABB on Example 14.1.
fn bench_ssg_dm(c: &mut Criterion) {
    let relaxed = MaximalStructureOptions {
        strict_no_excess: false,
    };
    let relaxed_abb = AbbOptions {
        strict_no_excess: false,
        ..AbbOptions::default()
    };

    let mut group = c.benchmark_group("ssg_dm");

    // Example 3.3: decision-mapping SSG → 3465 structures over a 29-unit
    // relaxed maximal structure.
    let p33 = lower(&parse_description(EX3_3_SRC).unwrap()).unwrap();
    let m33 = maximal_structure_with_options(&p33, relaxed);
    group.throughput(Throughput::Elements(3465));
    group.bench_function(BenchmarkId::new("example3_3", "3465_structures"), |b| {
        b.iter(|| black_box(ssg_dm_enumerate(&p33, &m33).len()))
    });

    // Example 14.1: ABB optimum (relaxed), 12 units.
    let p14 = lower(&parse_description(EX14_1_SRC).unwrap()).unwrap();
    let m14 = maximal_structure_with_options(&p14, relaxed);
    group.bench_function(BenchmarkId::new("example14_1", "abb_optimum"), |b| {
        b.iter(|| black_box(solve_with_options(&p14, &m14, relaxed_abb.clone()).is_some()))
    });
    group.finish();
}

criterion_group! {
    name = pgraph_benches;
    config = Criterion::default()
        .sample_size(15)
        .warm_up_time(std::time::Duration::from_millis(300))
        .measurement_time(std::time::Duration::from_secs(2));
    targets = bench_parse_lower, bench_msg, bench_ssg, bench_abb, bench_ssg_dm
}
criterion_main!(pgraph_benches);
