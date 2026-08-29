//! std-only probe for the two performance axes of the P-graph lowerer (F.13).
//!
//! Not part of the production API and adds no dependency. It measures, per
//! workload:
//!
//! * **end-to-end** — generated source → parse → lower → `meta_resolve`;
//! * **resident**    — a pre-parsed canonical `Ir` → `meta_resolve` only.
//!
//! Source generation, parsing (for the resident case) and every output/hash
//! check happen **outside** the timed region. The generated source is held for
//! the lifetime the parser borrows it. Run:
//!
//! ```text
//! cargo run --release -p hymeko_pgraph --example meta_resolve_bench
//! ```
//!
//! It prints one line per (workload, axis) with source bytes, a deterministic
//! source fingerprint, the structural oracle, and the median wall time.

use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Instant;

use hymeko::ir::ir::Ir;
use hymeko::module_store::module_store::{HymekoParser, ModuleStore};
use hymeko::module_store::source_provider::MemProvider;
use hymeko_pgraph::lower_resolved;
use parser::ast::AstStr;

#[path = "../tests/workload/mod.rs"]
mod workload;
use workload::{GeneratedHymeko, WorkloadFamily, WorkloadScale, WorkloadSpec, generate_hymeko};

/// Local thin parser (mirrors the crate-private one in `meta_resolve`).
struct LocalParser;
impl HymekoParser for LocalParser {
    fn parse<'a>(&self, src: &'a str) -> Result<AstStr<'a>, String> {
        parser::parse_description(src).map_err(|e| format!("{e:?}"))
    }
}

const META: &str = include_str!("../data/meta_pgraph.hymeko");

/// A parsed workload held resident: the `Ir` plus the `ModuleStore` that owns
/// its interner. The store is kept alive so `&store.it` can be borrowed in the
/// timed region without cloning the interner (which is not `Clone`).
struct Resident {
    ir: Ir,
    store: ModuleStore<MemProvider, LocalParser>,
}

fn resident_ir(g: &GeneratedHymeko) -> Resident {
    let mut provider = MemProvider::default();
    provider.insert_file(
        std::path::PathBuf::from("workload.hymeko"),
        g.source.clone(),
    );
    provider.insert_file(
        std::path::PathBuf::from("meta_pgraph.hymeko"),
        META.to_string(),
    );
    let mut store = ModuleStore::new(provider, LocalParser);
    let ir = store
        .compile(Path::new("workload.hymeko"))
        .expect("workload compiles")
        .ir
        .clone();
    Resident { ir, store }
}

fn fingerprint(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn bench(name: &str, family: WorkloadFamily) {
    let spec = WorkloadSpec {
        family,
        scale: WorkloadScale::Medium,
        seed: 0,
    };
    // Generation is OUTSIDE every timed region.
    let g = generate_hymeko(&spec).expect("generate");
    let bytes = g.source.len();
    let fp = fingerprint(&g.source);

    // Resident: parse ONCE outside the timed region, then time meta_resolve.
    let res = resident_ir(&g);
    let iters = 25usize;
    let mut resident = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        let lowered = lower_resolved(&res.ir, &res.store.it).expect("resident lower");
        let dt = t.elapsed().as_secs_f64() * 1e6;
        std::hint::black_box(&lowered);
        resident.push(dt);
    }

    // End-to-end: the same source bytes each iteration; hashing/asserts are
    // outside the timed region (there are none inside).
    let mut e2e = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        let g2 = hymeko_pgraph::compile_sources(
            "workload.hymeko",
            &[
                ("workload.hymeko", g.source.as_str()),
                ("meta_pgraph.hymeko", META),
            ],
        )
        .expect("e2e lower");
        let dt = t.elapsed().as_secs_f64() * 1e6;
        std::hint::black_box(&g2);
        e2e.push(dt);
    }

    println!(
        "{name:22} bytes={bytes:>7} fp={fp:016x} mats={:>5} units={:>5} inc={:>6} | resident={:>9.2}us e2e={:>10.2}us",
        g.expected.materials,
        g.expected.units,
        g.expected.incidences,
        median(resident),
        median(e2e),
    );
}

fn main() {
    println!("# meta_resolve two-axis probe (median of 25, release)");
    bench(
        "sunflower_medium",
        WorkloadFamily::Sunflower {
            core: 4,
            petal: 4,
            petals: 40,
        },
    );
    bench(
        "spine_leaf_medium",
        WorkloadFamily::SpineLeaf {
            spines: 8,
            leaves: 16,
        },
    );
    bench(
        "robot_chain_medium",
        WorkloadFamily::RobotChain { joints: 200 },
    );
    bench("grotzsch", WorkloadFamily::MycielskiGrotzsch);
}
