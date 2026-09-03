//! Independent P3.4 T/S-norm reproduction using the production propagation implementation.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fs::OpenOptions;
use std::hint::black_box;
use std::io::Write;
use std::time::Instant;

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
const HEADER: &str = "R {\n    @\"meta_pgraph.hymeko\";\n    using pgraph.raw as raw;\n    using pgraph.product as product;\n    using pgraph.intermediate as inter;\n    using pgraph.process as process;\n}\nbody {\n";

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        let _ = BYTES.try_with(|c| c.set(c.get() + layout.size() as u64));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn snapshot() -> (u64, u64) {
    (ALLOCS.with(Cell::get), BYTES.with(Cell::get))
}

fn delta(before: (u64, u64)) -> (u64, u64) {
    let after = snapshot();
    (after.0 - before.0, after.1 - before.1)
}

#[derive(Clone, Copy)]
struct Stats {
    median_us: f64,
    q1_us: f64,
    q3_us: f64,
    worst_us: f64,
}

fn measure(mut f: impl FnMut()) -> Stats {
    for _ in 0..2 {
        f();
    }
    let mut samples = Vec::with_capacity(9);
    for _ in 0..9 {
        let start = Instant::now();
        f();
        samples.push(start.elapsed().as_secs_f64() * 1e6);
    }
    samples.sort_by(f64::total_cmp);
    Stats {
        median_us: samples[4],
        q1_us: samples[2],
        q3_us: samples[6],
        worst_us: samples[8],
    }
}

fn chain_source(n: usize) -> String {
    let mut source = String::from(HEADER);
    source.push_str("    M0: + <isa> raw {}\n");
    for i in 1..n {
        let role = if i == n - 1 { "product" } else { "inter" };
        source.push_str(&format!("    M{i}: + <isa> {role} {{}}\n"));
    }
    for i in 1..n {
        source.push_str(&format!("    @U{i} {{ (+M{i}, -M{}); }}\n", i - 1));
    }
    source.push_str("}\n");
    source
}

struct Damped(f64);

impl Reliability for Damped {
    fn of(&self, _unit: DeclId) -> f64 {
        self.0
    }
}

fn assert_known_answers() {
    let b = BooleanExact;
    assert_eq!(b.t(0.3, 0.7), 0.3);
    assert_eq!(b.s(0.3, 0.7), 0.7);
    let g = Godel;
    assert_eq!(g.t(0.3, 0.7), 0.3);
    assert_eq!(g.s(0.3, 0.7), 0.7);
    let p = Product;
    assert!((p.t(0.3, 0.7) - 0.21).abs() < 1e-12);
    assert!((p.s(0.3, 0.7) - 0.79).abs() < 1e-12);
    let l = Lukasiewicz;
    assert_eq!(l.t(0.3, 0.7), 0.0);
    assert_eq!(l.s(0.3, 0.7), 1.0);
}

#[allow(clippy::too_many_arguments)]
fn measure_norm<N: AndOrNorm, R: Reliability>(
    rows: &mut Vec<String>,
    allocations: &mut Vec<String>,
    view: &PGraphView<'_, '_, '_>,
    norm: &N,
    reliability: &R,
    config: PropagationConfig,
    expected: usize,
    incidences: usize,
) {
    let mut workspace = AndOrWorkspace::new();
    let mut checksum = 0usize;
    let stats = measure(|| {
        let result = propagate(view, norm, reliability, &mut workspace, config).expect("propagate");
        checksum = result.producible().len();
        black_box(result);
    });
    assert_eq!(checksum, expected, "{} chain cardinality", N::NAME);
    rows.push(
        [
            "ts_norm".to_string(),
            "chain1500".to_string(),
            N::NAME.to_string(),
            "1500".to_string(),
            "1499".to_string(),
            incidences.to_string(),
            "2".to_string(),
            "9".to_string(),
            format!("{:.3}", stats.median_us),
            format!("{:.3}", stats.q1_us),
            format!("{:.3}", stats.q3_us),
            format!("{:.3}", stats.worst_us),
            checksum.to_string(),
            checksum.to_string(),
            "producible_count".to_string(),
            "production propagate; reliability=1.0 for BooleanExact, 0.9 for fuzzy norms"
                .to_string(),
        ]
        .join("\t"),
    );

    let before = snapshot();
    let result =
        propagate(view, norm, reliability, &mut workspace, config).expect("warm propagate");
    black_box(result);
    let d = delta(before);
    allocations.push(
        [
            "chain1500".to_string(),
            format!("{}_warm_propagate", N::NAME),
            d.0.to_string(),
            d.1.to_string(),
            "0".to_string(),
            "workspace warm; allocation is owned Availability result (Vec + BTreeSet)".to_string(),
        ]
        .join("\t"),
    );
}

#[test]
#[ignore = "independent P3.4 T/S audit; run after graph audit test to append raw TSV"]
fn independent_ts_measurements() {
    assert_known_answers();

    let before = snapshot();
    let source = chain_source(1500);
    let source_delta = delta(before);

    let before = snapshot();
    let provider = MemProvider::default()
        .with_file("m.hymeko", source.as_str())
        .with_file("meta_pgraph.hymeko", META);
    let mut store = ModuleStore::new(provider, RealParser);
    let program = store
        .compile(std::path::Path::new("m.hymeko"))
        .expect("compile chain");
    let compile_delta = delta(before);

    let before = snapshot();
    let lowered = lower_resolved(&program.ir, &store.it).expect("lower chain");
    let lower_delta = delta(before);

    let before = snapshot();
    let index = build_incidence_index(&program.ir).expect("index chain");
    let index_delta = delta(before);

    let view = PGraphView::from_lowered(&index, &lowered);
    let incidences: usize = program.ir.arcs.iter().map(|a| a.refs.len()).sum();
    let exact = PropagationConfig::boolean_exact(lowered.units.len(), lowered.materials.len());
    let fuzzy = PropagationConfig::fuzzy(1e-9, exact.max_iters);
    let damped = Damped(0.9);
    let mut rows = Vec::new();
    let mut allocations = Vec::new();

    measure_norm(
        &mut rows,
        &mut allocations,
        &view,
        &BooleanExact,
        &UnitReliability,
        exact,
        1500,
        incidences,
    );
    measure_norm(
        &mut rows,
        &mut allocations,
        &view,
        &Godel,
        &damped,
        fuzzy,
        1500,
        incidences,
    );
    measure_norm(
        &mut rows,
        &mut allocations,
        &view,
        &Product,
        &damped,
        fuzzy,
        197,
        incidences,
    );
    measure_norm(
        &mut rows,
        &mut allocations,
        &view,
        &Lukasiewicz,
        &damped,
        fuzzy,
        10,
        incidences,
    );

    allocations.splice(
        0..0,
        [
            [
                "chain1500",
                "source_generation",
                &source_delta.0.to_string(),
                &source_delta.1.to_string(),
                "0",
                "HyMeKo source String generation only",
            ]
            .join("\t"),
            [
                "chain1500",
                "parse_resolve_ir",
                &compile_delta.0.to_string(),
                &compile_delta.1.to_string(),
                "0",
                "ModuleStore compile; includes parse, resolve, and canonical Ir",
            ]
            .join("\t"),
            [
                "chain1500",
                "pgraph_lowering",
                &lower_delta.0.to_string(),
                &lower_delta.1.to_string(),
                "0",
                "LoweredPGraph construction",
            ]
            .join("\t"),
            [
                "chain1500",
                "incidence_index",
                &index_delta.0.to_string(),
                &index_delta.1.to_string(),
                &index.retained_bytes().to_string(),
                "IncidenceIndex build only",
            ]
            .join("\t"),
        ],
    );

    let report_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../reports/data");
    let rerun = report_dir.join("2026-09-03-codex-p34-rerun.tsv");
    let alloc = report_dir.join("2026-09-03-codex-p34-allocations.tsv");
    let mut rerun_file = OpenOptions::new()
        .append(true)
        .open(&rerun)
        .expect("graph audit rerun TSV must exist first");
    writeln!(rerun_file, "{}", rows.join("\n")).expect("append T/S rows");
    let mut alloc_file = OpenOptions::new()
        .append(true)
        .open(&alloc)
        .expect("graph audit allocation TSV must exist first");
    writeln!(alloc_file, "{}", allocations.join("\n")).expect("append T/S allocations");
    println!("appended T/S audit rows to {}", rerun.display());
    println!("appended T/S allocation rows to {}", alloc.display());
}
