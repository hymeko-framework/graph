//! P3.4 §9 — the T-norm/S-norm performance matrix over `hymeko_pgraph::andor`. Reuses the production
//! `propagate` entry point and the four supported norm families (each bundling its T-norm/AND and
//! S-norm/OR): `BooleanExact` (min/max on {0,1}), `Godel` (min/max), `Product` (a·b / a+b−ab),
//! `Lukasiewicz` (max(0,a+b−1) / min(1,a+b)). It does **not** modify any production PGraph code — it is a
//! new test. BooleanExact runs exact (unit reliability, {0,1}); the fuzzy families run with a damped
//! reliability (0.9) so their T/S combinators do genuine fuzzy work (distinct values + convergence),
//! kept entirely separate from A*.
//!
//! Run: cargo test -p hymeko_pgraph --release --test andor_ts_matrix ts_matrix -- --ignored --nocapture

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use hymeko::common::ids::DeclId;
use hymeko::ir::incidence::build_incidence_index;
use hymeko::module_store::module_store::ModuleStore;
use hymeko::module_store::source_provider::MemProvider;
use hymeko::util::real_parser::RealParser;

use hymeko_pgraph::andor::{
    AndOrNorm, AndOrWorkspace, BooleanExact, Godel, Lukasiewicz, Product, PropagationConfig,
    Reliability, UnitReliability, propagate,
};
use hymeko_pgraph::meta_resolve::lower_resolved;
use hymeko_pgraph::view::PGraphView;

const META: &str = include_str!("../data/meta_pgraph.hymeko");

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

/// A damped reliability so the fuzzy norms do real work (availability decays along production paths).
struct Damped(f64);
impl Reliability for Damped {
    fn of(&self, _u: DeclId) -> f64 {
        self.0
    }
}

// ---- P-graph source topologies ---------------------------------------------------------------------

const HEADER: &str = "R {\n    @\"meta_pgraph.hymeko\";\n    using pgraph.raw as raw;\n    using pgraph.product as product;\n    using pgraph.intermediate as inter;\n    using pgraph.process as process;\n}\nbody {\n";

fn chain_src(n: usize) -> String {
    let mut b = String::from(HEADER);
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

fn fanout_src(n: usize) -> String {
    let mut b = String::from(HEADER);
    b.push_str("    M0: + <isa> raw {}\n");
    for i in 1..n {
        b.push_str(&format!("    M{i}: + <isa> product {{}}\n"));
    }
    for i in 1..n {
        b.push_str(&format!("    @U{i} {{ (+M{i}, -M0); }}\n"));
    }
    b.push_str("}\n");
    b
}

fn fanin_src(n: usize) -> String {
    let mut b = String::from(HEADER);
    b.push_str("    M0: + <isa> product {}\n");
    for i in 1..n {
        b.push_str(&format!("    M{i}: + <isa> raw {{}}\n"));
    }
    b.push_str("    @U0 { (+M0");
    for i in 1..n {
        b.push_str(&format!(", -M{i}"));
    }
    b.push_str("); }\n}\n");
    b
}

/// A binary AND tree: `n` raw leaves combined pairwise up to a root (mixed depth + fan-in).
fn mixed_src(n: usize) -> String {
    let mut b = String::from(HEADER);
    for i in 0..n {
        b.push_str(&format!("    M{i}: + <isa> raw {{}}\n"));
    }
    let mut next = n;
    let mut level: Vec<usize> = (0..n).collect();
    let mut u = 0;
    while level.len() > 1 {
        let mut up = Vec::new();
        for pair in level.chunks(2) {
            if pair.len() == 2 {
                let role = "inter";
                b.push_str(&format!("    M{next}: + <isa> {role} {{}}\n"));
                b.push_str(&format!(
                    "    @U{u} {{ (+M{next}, -M{}, -M{}); }}\n",
                    pair[0], pair[1]
                ));
                up.push(next);
                next += 1;
                u += 1;
            } else {
                up.push(pair[0]);
            }
        }
        level = up;
    }
    b.push_str("}\n");
    b
}

/// A sparse random DAG: material `i` produced from 1–3 earlier materials (deterministic LCG).
fn random_src(n: usize, seed: u64) -> String {
    let mut b = String::from(HEADER);
    b.push_str("    M0: + <isa> raw {}\n");
    for i in 1..n {
        let role = if i % 7 == 0 { "product" } else { "inter" };
        b.push_str(&format!("    M{i}: + <isa> {role} {{}}\n"));
    }
    let mut s = seed;
    let mut nxt = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        s >> 33
    };
    for i in 1..n {
        let k = 1 + (nxt() as usize % 3);
        b.push_str(&format!("    @U{i} {{ (+M{i}"));
        for _ in 0..k {
            let j = (nxt() as usize) % i; // an earlier material
            b.push_str(&format!(", -M{j}"));
        }
        b.push_str("); }\n");
    }
    b.push_str("}\n");
    b
}

fn run_norm<N: AndOrNorm, R: Reliability>(
    view: &PGraphView<'_, '_, '_>,
    norm: &N,
    rel: &R,
    cfg: PropagationConfig,
) -> (f64, f64, usize, u64, u64, i64) {
    let mut ws = AndOrWorkspace::new();
    // cold prep run (first propagate populates the workspace) + its allocation bytes.
    let b0 = snap();
    let cold_wall = {
        let t = std::time::Instant::now();
        let _ = propagate(view, norm, rel, &mut ws, cfg).unwrap();
        t.elapsed().as_secs_f64() * 1e6
    };
    let cold_bytes = snap().1 - b0.1;

    // warm timing + warm allocations (should be ~0 — the workspace is reused).
    let a0 = snap();
    let mut iters = 0;
    let mut checksum = 0i64;
    let warm_wall = {
        let mut best = f64::INFINITY;
        for _ in 0..7 {
            let t = std::time::Instant::now();
            let r = propagate(view, norm, rel, &mut ws, cfg).unwrap();
            best = best.min(t.elapsed().as_secs_f64() * 1e6);
            iters = r.iterations;
            checksum = r.producible().len() as i64;
        }
        best
    };
    let warm_allocs = snap().0 - a0.0;
    (
        cold_wall,
        warm_wall,
        iters,
        warm_allocs,
        cold_bytes,
        checksum,
    )
}

#[test]
#[ignore = "T/S-norm matrix; --release --ignored --nocapture"]
fn ts_matrix() {
    let n = 1500;
    let topos: Vec<(&str, String)> = vec![
        ("chain", chain_src(n)),
        ("fanout", fanout_src(n)),
        ("fanin", fanin_src(n)),
        ("mixed", mixed_src(n)),
        ("random", random_src(n, 0xA5A5_1234)),
    ];
    let mut rows = vec![
        [
            "topology",
            "norm",
            "prep_us",
            "warm_us",
            "iterations",
            "incidences",
            "warm_allocs",
            "cold_alloc_b",
            "checksum",
        ]
        .join("\t"),
    ];
    println!("\n# P3.4 §9 T-norm/S-norm matrix (n={n} materials per topology)");
    println!(
        "{:<8} {:<12} {:>10} {:>10} {:>7} {:>8} {:>6}",
        "topology", "norm", "prep_us", "warm_us", "iters", "inc", "chk"
    );

    for (tname, src) in &topos {
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
        let incidences: usize = prog.ir.arcs.iter().map(|a| a.refs.len()).sum();
        let cfg_exact =
            PropagationConfig::boolean_exact(lowered.units.len(), lowered.materials.len());
        let cfg_fuzzy = PropagationConfig::fuzzy(1e-9, cfg_exact.max_iters);
        let damped = Damped(0.9);

        let mut emit = |norm_name: &str, r: (f64, f64, usize, u64, u64, i64)| {
            println!(
                "{tname:<8} {norm_name:<12} {:>10.2} {:>10.2} {:>7} {incidences:>8} {:>6}",
                r.0, r.1, r.2, r.5
            );
            rows.push(
                [
                    tname.to_string(),
                    norm_name.to_string(),
                    format!("{:.3}", r.0),
                    format!("{:.3}", r.1),
                    r.2.to_string(),
                    incidences.to_string(),
                    r.3.to_string(),
                    r.4.to_string(),
                    r.5.to_string(),
                ]
                .join("\t"),
            );
        };
        emit(
            "BooleanExact",
            run_norm(&view, &BooleanExact, &UnitReliability, cfg_exact),
        );
        emit("Godel", run_norm(&view, &Godel, &damped, cfg_fuzzy));
        emit("Product", run_norm(&view, &Product, &damped, cfg_fuzzy));
        emit(
            "Lukasiewicz",
            run_norm(&view, &Lukasiewicz, &damped, cfg_fuzzy),
        );
    }
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../reports/data/2026-09-03-p34-ts-norm.tsv"
    );
    std::fs::write(path, rows.join("\n") + "\n").unwrap();
    println!("\nwrote {path}");
}
