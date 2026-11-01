//! Opt-in resident-memory client; generation and PageRank keep their existing owners.
#[cfg(all(feature = "static-link", feature = "dynamic-link"))]
compile_error!("Select exactly one linkage.");
#[cfg(not(any(feature = "static-link", feature = "dynamic-link")))]
compile_error!("Select static-link or dynamic-link.");
#[cfg(feature = "static-link")]
extern crate graph_static as hymeko_graph;
#[cfg(feature = "dynamic-link")]
extern crate graph_dynamic as hymeko_graph;

#[path = "../../../hymeko_graph/tests/pagerank_support/mod.rs"]
mod fixtures;
use hymeko::generators::GraphGenerator;
use hymeko_graph::{PageRank, PageRankConfig, PageRankModel, PageRankWorkspace};
use std::io::{self, Write};

struct ResidentClient;
impl ResidentClient {
    fn checkpoint(phase: &str, nodes: usize, incidences: usize, checksum: u64, steps: usize) {
        println!("{{\"phase\":\"{phase}\",\"pid\":{},\"nodes\":{nodes},\"incidences\":{incidences},\"checksum\":\"{checksum}\",\"steps\":{steps}}}", std::process::id());
        io::stdout().flush().unwrap();
        let mut command = String::new();
        assert!(io::stdin().read_line(&mut command).unwrap() > 0, "controller disconnected");
        assert_eq!(command.trim(), "continue", "unexpected controller command");
    }
    fn run() {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() != 2 || args[0] != "--enable-memory-measurement" {
            eprintln!("explicit --enable-memory-measurement CASE required");
            std::process::exit(2);
        }
        assert!(matches!(args[1].as_str(), "chain65k" | "hub32_1m"));
        Self::checkpoint("loaded", 0, 0, 0, 0);
        let ir = fixtures::Fixture::named(&args[1]).generate().unwrap();
        let nodes = ir.nodes.len();
        let incidences: usize = ir.arcs.iter().map(|a| a.refs.len()).sum();
        Self::checkpoint("graph", nodes, incidences, 0, 0);
        let rank = PageRank::build(&ir, PageRankModel::default()).unwrap();
        let mut workspace = PageRankWorkspace::new();
        Self::checkpoint("prepared", nodes, incidences, 0, 0);
        let mut expected = None;
        for _ in 0..2 {
            let result = rank.run(PageRankConfig::default(), &mut workspace, None).unwrap();
            assert!(result.progress.converged);
            assert_eq!(result.scores.len(), nodes);
            let checksum = result.scores.iter().fold(0xcbf29ce484222325u64, |h, x| {
                assert!(x.is_finite() && *x >= 0.0);
                (h ^ x.to_bits()).wrapping_mul(0x100000001b3)
            });
            let witness = (checksum, result.progress.iterations);
            if let Some(previous) = expected { assert_eq!(previous, witness); }
            expected = Some(witness);
        }
        let (checksum, steps) = expected.unwrap();
        Self::checkpoint("resident", nodes, incidences, checksum, steps);
        std::hint::black_box((&ir, &rank, &workspace));
    }
}
fn main() { ResidentClient::run(); }
