//! Benchmark (mission §9): the existing `close_producible` fixpoint (cell A — re-scans every unit each
//! round, `O(rounds·units)`) vs the BooleanExact worklist (cell C — each unit fires when woken, `O(work)`),
//! over the borrowed PGraphView. Prints a table (`-- --nocapture`); single host, median-of-9. The exact
//! producible set is asserted equal inline (A == C), so the speedup is a *proven* speedup.

use hymeko::util::measurement::{MeasurementRunner, Runner};

use hymeko::ir::incidence::build_incidence_index;
use hymeko::module_store::module_store::ModuleStore;
use hymeko::module_store::source_provider::MemProvider;
use hymeko::util::real_parser::RealParser;

use hymeko_pgraph::andor::{
    AndOrWorkspace, BooleanExact, PropagationConfig, UnitReliability, propagate,
};
use hymeko_pgraph::meta_resolve::lower_resolved;
use hymeko_pgraph::msg::close_producible;
use hymeko_pgraph::view::PGraphView;

const META: &str = include_str!("../data/meta_pgraph.hymeko");

fn median_us(iters: usize, f: impl FnMut()) -> f64 {
    MeasurementRunner::new(2, iters).run(f).median
}

/// A raw -> product chain of `n` materials (n-1 units); the worst case for the re-scan fixpoint.
fn chain_src(n: usize) -> String {
    let mut b = String::from(
        "R {\n    @\"meta_pgraph.hymeko\";\n    using pgraph.raw as raw;\n    using pgraph.product as product;\n    using pgraph.intermediate as inter;\n    using pgraph.process as process;\n}\nbody {\n",
    );
    b.push_str("    M0: + <isa> raw {}\n");
    for i in 1..n {
        let role = if i == n - 1 { "product" } else { "inter" };
        b.push_str(&format!("    M{i}: + <isa> {role} {{}}\n"));
    }
    for i in 1..n {
        b.push_str(&format!("    @U{i} {{ (+M{i}, -M{}); }}\n", i - 1));
    }
    b.push_str("}\n");
    b
}

#[test]
#[ignore = "performance benchmark; opt in with --ignored --exact"]
fn bench_andor_closure() {
    println!("\n=== AND/OR closure benchmark (us; median-of-9; single-host PROVISIONAL) ===");
    println!(
        "-- chain raw->product, n materials: close_producible (A) vs BooleanExact worklist (C) --"
    );
    println!(
        "{:>7} {:>8} | {:>12} | {:>12} | {:>8} | {:>10}",
        "n", "units", "A close us", "C worklist us", "C iters", "A/C x"
    );
    for &n in &[100usize, 300, 1000, 2000] {
        let src = chain_src(n);
        let provider = MemProvider::default()
            .with_file("m.hymeko", src.as_str())
            .with_file("meta_pgraph.hymeko", META);
        let mut ms = ModuleStore::new(provider, RealParser);
        let prog = ms
            .compile(std::path::Path::new("m.hymeko"))
            .expect("compile");
        let lowered = lower_resolved(&prog.ir, &ms.it).expect("lower");
        let index = build_incidence_index(&prog.ir).expect("index");
        let view = PGraphView::from_lowered(&index, &lowered);
        let cfg = PropagationConfig::boolean_exact(lowered.units.len(), lowered.materials.len());
        let mut ws = AndOrWorkspace::new();

        // correctness cross-check: A == C (exact producible set identical).
        let a_set = close_producible(&lowered, &lowered.units, &lowered.raws);
        let c = propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg).unwrap();
        assert_eq!(
            &a_set,
            c.producible(),
            "close_producible != BooleanExact worklist at n={n}"
        );
        let iters = c.iterations;

        let a_us = median_us(9, || {
            let _ = close_producible(&lowered, &lowered.units, &lowered.raws);
        });
        let c_us = median_us(9, || {
            let _ = propagate(&view, &BooleanExact, &UnitReliability, &mut ws, cfg).unwrap();
        });
        println!(
            "{:>7} {:>8} | {:>12.1} | {:>12.1} | {:>8} | {:>9.1}x",
            n,
            lowered.units.len(),
            a_us,
            c_us,
            iters,
            a_us / c_us.max(1e-9)
        );
    }
    println!();
}
