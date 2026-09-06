//! P1.4 graph/pgraph performance matrix (focused): the operations P1.3 could affect — `AxiomBundle::validate`
//! (the axiom path whose adjacency rebuilds P1.3 removed) and the BooleanExact closure — across workload
//! families, at small/medium/large. Reuses the deterministic `<isa>` source harness (no new bench framework).
//! Prints tables (`-- --nocapture`); single host, median-of-9, PROVISIONAL wall. Run this file in the P1.2
//! (A) and P1.3 (B) worktrees to obtain the A/B (exact output identical — asserted).

use std::time::Instant;

use hymeko::ir::incidence::build_incidence_index;
use hymeko::module_store::module_store::ModuleStore;
use hymeko::module_store::source_provider::MemProvider;
use hymeko::util::real_parser::RealParser;

use hymeko_pgraph::andor::{
    AndOrWorkspace, BooleanExact, PropagationConfig, UnitReliability, propagate,
};
use hymeko_pgraph::axioms::AxiomBundle;
use hymeko_pgraph::meta_resolve::lower_resolved;
use hymeko_pgraph::view::PGraphView;

const META: &str = include_str!("../data/meta_pgraph.hymeko");
const HDR: &str = "R {\n    @\"meta_pgraph.hymeko\";\n    using pgraph.raw as raw;\n    using pgraph.product as product;\n    using pgraph.intermediate as inter;\n    using pgraph.process as process;\n}\n";

fn median_us(iters: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..2 {
        f();
    }
    let mut s = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        f();
        s.push(t.elapsed().as_secs_f64() * 1e6);
    }
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[s.len() / 2]
}

/// chain: raw -> n-1 units -> product (deep A4 reachability).
fn chain(n: usize) -> String {
    let mut b = format!("{HDR}body {{\n    M0: + <isa> raw {{}}\n");
    for i in 1..n {
        let r = if i == n - 1 { "product" } else { "inter" };
        b.push_str(&format!("    M{i}: + <isa> {r} {{}}\n"));
    }
    for i in 1..n {
        b.push_str(&format!("    @U{i} {{ (+M{i}, -M{}); }}\n", i - 1));
    }
    b.push_str("}\n");
    b
}

/// wide OR fan-out: `k` units all produce the single product from one raw (many producers of one material).
fn fanout(k: usize) -> String {
    let mut b = format!("{HDR}body {{\n    A: + <isa> raw {{}}\n    P: + <isa> product {{}}\n");
    for i in 0..k {
        b.push_str(&format!("    @U{i} {{ (+P, -A); }}\n"));
    }
    b.push_str("}\n");
    b
}

/// wide AND fan-in: one unit consuming `k` raws to make a product (large ready-fold).
fn fanin(k: usize) -> String {
    let mut b = format!("{HDR}body {{\n");
    for i in 0..k {
        b.push_str(&format!("    A{i}: + <isa> raw {{}}\n"));
    }
    b.push_str("    P: + <isa> product {}\n    @U { (+P");
    for i in 0..k {
        b.push_str(&format!(", -A{i}"));
    }
    b.push_str("); }\n}\n");
    b
}

fn run(name: &str, families: &[(&str, usize)], src: impl Fn(usize) -> String) {
    println!("\n-- {name}: AxiomBundle::validate + BooleanExact closure --");
    println!(
        "{:>8} {:>8} {:>8} | {:>12} | {:>14}",
        "size", "units", "mats", "validate us", "closure us"
    );
    for &(label, sz) in families {
        let s = src(sz);
        let provider = MemProvider::default()
            .with_file("m.hymeko", s.as_str())
            .with_file("meta_pgraph.hymeko", META);
        let mut ms = ModuleStore::new(provider, RealParser);
        let prog = ms
            .compile(std::path::Path::new("m.hymeko"))
            .expect("compile");
        let lowered = lower_resolved(&prog.ir, &ms.it).expect("lower");
        let index = build_incidence_index(&prog.ir).expect("index");
        let view = PGraphView::from_lowered(&index, &lowered);
        let bundle = AxiomBundle::new(lowered.raws.iter().copied(), []);
        let cfg = PropagationConfig::boolean_exact(lowered.units.len(), lowered.materials.len());
        let mut ws = AndOrWorkspace::new();

        // Observations for comparison: validity + producible count (not an independent A/B assertion).
        let valid = bundle.validate(&lowered.schema, &lowered.products).is_ok();
        let prod = propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg)
            .unwrap()
            .producible()
            .len();
        let vus = median_us(9, || {
            let _ = bundle.validate(&lowered.schema, &lowered.products);
        });
        let cus = median_us(9, || {
            let _ = propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg).unwrap();
        });
        println!(
            "{:>8} {:>8} {:>8} | {:>12.1} | {:>14.1}   [valid={valid} prod={prod}]",
            label,
            lowered.units.len(),
            lowered.materials.len(),
            vus,
            cus
        );
    }
}

#[test]
fn bench_perf_matrix() {
    println!(
        "\n=== P1.4 pgraph query performance matrix (us; median-of-9; single-host PROVISIONAL) ==="
    );
    run(
        "chain (deep A4 reachability)",
        &[("small", 100), ("medium", 1000), ("large", 4000)],
        chain,
    );
    run(
        "OR fan-out (many producers)",
        &[("small", 50), ("medium", 500), ("large", 3000)],
        fanout,
    );
    run(
        "AND fan-in (large ready-fold)",
        &[("small", 50), ("medium", 500), ("large", 3000)],
        fanin,
    );
    println!();
}
