//! std-only benchmark probe for the P-graph lowerer (F.13 / S).
//!
//! Not part of the production API; no dependency. Per workload it splits four
//! phases, each timed with source generation, parsing (for resident) and every
//! hash check **outside** the timed region:
//!
//! * source generation (diagnostic only);
//! * parse / resolve / lower;
//! * resident `meta_resolve` (pre-parsed `Ir`);
//! * end-to-end (source → … → `LoweredPGraph`).
//!
//! It prints source bytes, a **stable FNV-1a-64** source fingerprint, the
//! structural oracle and per-phase medians. Topologies are reported separately —
//! never averaged into one number. Run:
//!
//! ```text
//! cargo run --release -p hymeko_pgraph --example meta_resolve_bench
//! ```

use std::path::Path;
use std::time::Instant;

use hymeko::ir::ir::Ir;
use hymeko::module_store::module_store::{HymekoParser, ModuleStore};
use hymeko::module_store::source_provider::MemProvider;
use hymeko_pgraph::lower_resolved;
use parser::ast::AstStr;

#[path = "../tests/workload/mod.rs"]
mod workload;
use workload::{Built, WorkloadFamily, build, source_fingerprint};

struct LocalParser;
impl HymekoParser for LocalParser {
    fn parse<'a>(&self, src: &'a str) -> Result<AstStr<'a>, String> {
        parser::parse_description(src).map_err(|e| format!("{e:?}"))
    }
}

const META: &str = include_str!("../data/meta_pgraph.hymeko");

struct Resident {
    ir: Ir,
    store: ModuleStore<MemProvider, LocalParser>,
}

fn resident_ir(b: &Built) -> Resident {
    let mut provider = MemProvider::default();
    provider.insert_file(
        std::path::PathBuf::from("workload.hymeko"),
        b.source.clone(),
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

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn bench(name: &str, family: WorkloadFamily, iters: usize) {
    let t = Instant::now();
    let b = build(family).expect("generate");
    let gen_us = t.elapsed().as_secs_f64() * 1e6;
    let bytes = b.source.len();
    let fp = source_fingerprint(&b.source);

    let res = resident_ir(&b);
    let mut resident = Vec::with_capacity(iters);
    let mut e2e = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        let g = lower_resolved(&res.ir, &res.store.it).expect("resident");
        resident.push(t.elapsed().as_secs_f64() * 1e6);
        std::hint::black_box(&g);
    }
    for _ in 0..iters {
        let t = Instant::now();
        let g = hymeko_pgraph::compile_sources(
            "workload.hymeko",
            &[
                ("workload.hymeko", b.source.as_str()),
                ("meta_pgraph.hymeko", META),
            ],
        )
        .expect("e2e");
        e2e.push(t.elapsed().as_secs_f64() * 1e6);
        std::hint::black_box(&g);
    }
    let rmed = median(resident);
    let inc = b.oracle.incidences.max(1) as f64;
    println!(
        "{name:26} bytes={bytes:>8} fp={fp:016x} mat={:>6} unit={:>6} inc={:>7} maxcard={:>4} | gen={:>8.1}us resident={:>9.2}us e2e={:>10.2}us ns/inc={:>7.1}",
        b.oracle.materials,
        b.oracle.units,
        b.oracle.incidences,
        b.max_cardinality,
        gen_us,
        rmed,
        median(e2e),
        rmed * 1000.0 / inc,
    );
}

fn main() {
    use WorkloadFamily as F;
    println!("# meta_resolve benchmark survey (median, release) — topologies reported separately");

    println!("## control (original four resident cells)");
    bench(
        "sunflower_medium",
        F::Sunflower {
            core: 4,
            petal: 4,
            petals: 40,
        },
        25,
    );
    bench(
        "spine_leaf_medium",
        F::SpineLeaf {
            spines: 8,
            leaves: 16,
        },
        25,
    );
    bench("robot_chain_medium", F::RobotChain { joints: 200 }, 25);
    bench("grotzsch", F::MycielskiGrotzsch, 50);
    bench("fano", F::Fano, 50);
    bench("mycielski_c5", F::MycielskiC5, 50);

    println!("## survey");
    bench("steiner_sts15", F::Steiner { m: 4 }, 50);
    bench("steiner_sts31", F::Steiner { m: 5 }, 25);
    bench("steiner_sts127", F::Steiner { m: 7 }, 5);
    bench("mycielski_iter3", F::Mycielski { iterations: 3 }, 25);
    bench("mycielski_iter5", F::Mycielski { iterations: 5 }, 5);
    bench(
        "sunflower_skew",
        F::Sunflower {
            core: 2,
            petal: 20,
            petals: 200,
        },
        10,
    );
    bench(
        "random_uniform",
        F::RandomUniform {
            nodes: 2000,
            edges: 4000,
            card: 5,
            seed: 1,
        },
        10,
    );
    bench(
        "random_degree",
        F::RandomDegree {
            nodes: 2000,
            edges: 4000,
            min_card: 2,
            max_card: 8,
            seed: 1,
        },
        10,
    );
    bench(
        "random_hub",
        F::RandomHub {
            nodes: 2000,
            edges: 4000,
            hubs: 20,
            hub_ratio_num: 1,
            hub_ratio_den: 2,
            card: 5,
            seed: 1,
        },
        10,
    );
    bench(
        "vlan_failure_domain",
        F::Vlan {
            switches: 64,
            hosts_per_switch: 32,
            vlans: 16,
            power_domains: 8,
        },
        10,
    );
    bench(
        "service_dependency",
        F::ServiceDependency {
            services: 200,
            replicas: 3,
            shared_groups: 12,
            dep_card: 4,
            seed: 1,
        },
        10,
    );
    bench(
        "humanoid",
        F::Humanoid {
            arm_joints: 7,
            leg_joints: 6,
            sensors_per_joint: 3,
            actuator_redundancy: 2,
        },
        25,
    );
    bench(
        "hexapod",
        F::Hexapod {
            legs: 6,
            joints_per_leg: 3,
            sensors_per_joint: 2,
            actuator_redundancy: 2,
        },
        25,
    );
    bench("chicken_physiology", F::ChickenPhysiology, 50);
    bench(
        "chicken_flock",
        F::ChickenFlockContact {
            birds: 500,
            coops: 8,
            feeders: 16,
            drinkers: 16,
            group: 10,
            windows: 100,
            seed: 1,
        },
        10,
    );
}
