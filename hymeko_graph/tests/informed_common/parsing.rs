//! Source ingestion measurements for the existing graph fixtures. Test client only.
//! The projection deliberately supports this fixture vocabulary, not arbitrary IR.

use std::fmt::Write;
use std::path::Path;
use std::sync::Arc;

use hymeko::ir::ir::{AnnoR, RefAtomR, SignedRefR, ValueR};
use hymeko::ir::lower::lower_to_ir;
use hymeko::module_store::module_store::{CompiledProgram, HymekoParser, ModuleStore};
use hymeko::module_store::source_provider::MemProvider;
use hymeko::resolution::intern_pass::intern_ast;
use hymeko::resolution::resolve::build_index_sym;
use hymeko::util::real_parser::RealParser;

use super::*;

type Store = ModuleStore<MemProvider, RealParser>;

struct SourceFixture {
    ir: Ir,
    source: Arc<str>,
    start: NodeId,
    goal: NodeId,
    weighted: bool,
}

impl SourceFixture {
    fn named(name: &str) -> Self {
        let (ir, start, goal) = match name {
            "path65k" | "grid256" | "sts1023" | "random1e6" => {
                let ir = shortest_fixture(name);
                let goal = NodeId::new(ir.nodes.len() - 1);
                (ir, NodeId::new(0), goal)
            }
            "sts511" => (
                workloads::steiner_sts_ir(9),
                NodeId::new(0),
                NodeId::new(510),
            ),
            "layered10" | "grid6" | "grid5" | "sts31" | "parallel16" => {
                let (ir, start, goal, _) = enumeration_fixture(name);
                let goal = goal.unwrap_or_else(|| NodeId::new(ir.nodes.len() - 1));
                (ir, start, goal)
            }
            _ => search_fixture(name),
        };
        let weighted = matches!(
            name,
            "map32" | "map128" | "map512" | "corridor128" | "transport128"
        );
        Self::new(ir, start, goal, weighted)
    }

    fn new(ir: Ir, start: NodeId, goal: NodeId, weighted: bool) -> Self {
        let source = Self::project(&ir);
        Self {
            ir,
            source: source.into(),
            start,
            goal,
            weighted,
        }
    }

    fn numeric(value: &ValueR) -> String {
        match value {
            ValueR::Num(n) => {
                assert!(n.is_finite(), "nonfinite fixture number");
                format!("{n:?}")
            }
            ValueR::List(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(Self::numeric)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => panic!("unsupported nonnumeric fixture value"),
        }
    }

    fn atom(reference: &SignedRefR) -> (&str, &RefAtomR) {
        match reference {
            SignedRefR::Plus(atom) => ("+", atom),
            SignedRefR::Minus(atom) => ("-", atom),
            SignedRefR::Neutral(atom) => ("~", atom),
        }
    }

    fn project(ir: &Ir) -> String {
        let mut source = String::from("ParsingBenchmark {}\n");
        for (i, node) in ir.nodes.iter().enumerate() {
            assert!(
                node.bases.is_empty(),
                "fixture bases require a different projection"
            );
            let anno = &ir.decl_nodes[node.decl.raw()].anno;
            let supported = AnnoR {
                value: anno.value.clone(),
                ..AnnoR::default()
            };
            assert_eq!(anno, &supported, "unsupported node annotation");
            match &anno.value {
                Some(value) => writeln!(source, "n{i} {};", Self::numeric(value)).unwrap(),
                None => writeln!(source, "n{i};").unwrap(),
            }
        }
        for (i, arc) in ir.arcs.iter().enumerate() {
            assert_eq!(arc.anno, AnnoR::default(), "unsupported arc annotation");
            write!(source, "@e{i} {{ (").unwrap();
            for (j, reference) in arc.refs.iter().enumerate() {
                let (sign, atom) = Self::atom(reference);
                assert_eq!(
                    atom.anno,
                    AnnoR::default(),
                    "unsupported incidence annotation"
                );
                assert_eq!(
                    atom.direction,
                    Default::default(),
                    "unsupported incidence direction"
                );
                let target = ir
                    .node_for_decl(atom.target)
                    .map(|(id, _)| id)
                    .expect("fixture target must be a node");
                if j != 0 {
                    source.push_str(", ");
                }
                write!(source, "{sign}n{}", target.raw()).unwrap();
                if let Some(weights) = &atom.weights {
                    write!(
                        source,
                        " [{}]",
                        weights
                            .iter()
                            .map(Self::numeric)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .unwrap();
                }
            }
            source.push_str("); }\n");
        }
        source
    }

    fn source_hash(&self) -> u64 {
        self.source.bytes().fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        })
    }

    fn compile(&self) -> (Store, Arc<CompiledProgram>) {
        let provider = MemProvider::default().with_file("benchmark.hymeko", self.source.clone());
        let mut store = ModuleStore::new(provider, RealParser);
        let compiled = store
            .compile(Path::new("benchmark.hymeko"))
            .expect("fresh source compilation");
        (store, compiled)
    }

    fn assert_equivalent(&self, ir: &Ir) {
        assert_eq!(
            dimensions(&self.ir),
            dimensions(ir),
            "parsed graph dimensions"
        );
        for (expected, actual) in self.ir.nodes.iter().zip(&ir.nodes) {
            assert_eq!(
                self.ir.decl_nodes[expected.decl.raw()].anno,
                ir.decl_nodes[actual.decl.raw()].anno
            );
        }
        for (expected, actual) in self.ir.arcs.iter().zip(&ir.arcs) {
            assert_eq!(expected.refs.len(), actual.refs.len(), "arc rank");
            for (expected, actual) in expected.refs.iter().zip(&actual.refs) {
                let (expected_sign, expected) = Self::atom(expected);
                let (actual_sign, actual) = Self::atom(actual);
                assert_eq!(expected_sign, actual_sign, "incidence sign");
                assert_eq!(
                    self.ir.node_for_decl(expected.target).map(|(id, _)| id),
                    ir.node_for_decl(actual.target).map(|(id, _)| id),
                    "incidence target"
                );
                assert_eq!(expected.weights, actual.weights, "incidence weights");
                assert_eq!(expected.direction, actual.direction, "incidence direction");
            }
        }
    }

    fn query(&self, ir: &Ir) -> Option<WeightedBergePath> {
        let index = build_incidence_index(ir).expect("parsed incidence index");
        let traversal = HyperTraversal::new(&index);
        let mut workspace = InformedSearchWorkspace::new();
        if self.weighted {
            dijkstra_berge_path(
                &traversal,
                self.start,
                self.goal,
                &IncidenceWeightCost { base: 0.0 },
                &Undirected,
                &mut workspace,
            )
        } else {
            dijkstra_berge_path(
                &traversal,
                self.start,
                self.goal,
                &UnitCost,
                &Undirected,
                &mut workspace,
            )
        }
        .expect("source query")
    }

    fn validate(&self) {
        let (_, compiled) = self.compile();
        self.assert_equivalent(&compiled.ir);
        let expected = self.query(&self.ir);
        let actual = self.query(&compiled.ir);
        assert_eq!(
            expected.as_ref().map(|p| p.total_cost),
            actual.as_ref().map(|p| p.total_cost)
        );
        assert_eq!(
            expected.as_ref().map(|p| p.hyperedge_len()),
            actual.as_ref().map(|p| p.hyperedge_len())
        );
    }

    fn measure(&self, phase: &str) -> Measurement {
        let hash = self.source_hash();
        match phase {
            "parse_ast" => {
                statistics::measure_factory(|| {
                    let ast = RealParser.parse(&self.source).expect("source parse");
                    (ast, hash)
                })
                .0
            }
            "ast_to_ir" => {
                let ast = RealParser.parse(&self.source).expect("untimed parse");
                let (measurement, ir) = statistics::measure_factory(|| {
                    let mut interned = intern_ast(&ast);
                    let index = build_index_sym(&interned.ast, &interned.interner)
                        .expect("name resolution");
                    let ir = lower_to_ir(&interned.ast, &index, &mut interned.interner)
                        .expect("AST lowering");
                    (ir, hash)
                });
                self.assert_equivalent(&ir);
                measurement
            }
            "compile" => statistics::measure_factory(|| (self.compile(), hash)).0,
            "incidence_index" => {
                let (_, compiled) = self.compile();
                statistics::measure_factory(|| {
                    (
                        build_incidence_index(&compiled.ir).expect("index construction"),
                        hash,
                    )
                })
                .0
            }
            "source_to_dijkstra" => {
                statistics::measure_factory(|| {
                    let (store, compiled) = self.compile();
                    let path = self.query(&compiled.ir);
                    ((store, compiled, path), hash)
                })
                .0
            }
            _ => panic!("unknown parsing phase {phase}"),
        }
    }
}

#[test]
#[ignore = "fresh-process parsing evidence; driven by the existing campaign.py --suite parsing"]
fn parsing_cell() {
    let run = env::var("INFORMED_BERGE_RUN").unwrap_or_else(|_| "manual".into());
    let workload = env::var("INFORMED_BERGE_WORKLOAD").expect("workload");
    let subject = env::var("INFORMED_BERGE_SUBJECT").expect("phase");
    emit_phase(&run, &workload, &subject, "process_baseline");
    let fixture = SourceFixture::named(&workload);
    fixture.validate();
    println!(
        "EVIDENCE_SOURCE\tbytes={}\tfnv1a64={}\tprojection=graph_fixture_v1\tfile_io=false\tfinal_drop=false",
        fixture.source.len(),
        fixture.source_hash()
    );
    emit_phase(&run, &workload, &subject, "source_validated");
    let measurement = fixture.measure(&subject);
    assert!(
        measurement.wall.worst < 15_000_000.0,
        "phase exceeds 15 second latency budget"
    );
    emit_measurement(
        &run,
        "parsing",
        &workload,
        &subject,
        &fixture.ir,
        &measurement,
    );
    emit_phase(&run, &workload, &subject, "complete");
}

#[test]
fn source_round_trip_preserves_signed_weights_positions_and_reachability() {
    let cases = [
        workloads::window_ir(7, 3, true, workloads::SignMode::Alternating),
        workloads::map_grid_with_transport_ir(4, 4, &HashSet::new(), 4, 0.5),
        workloads::multi_component_ir(2, 4, 3),
        workloads::dup_heavy_ir(5, 2),
    ];
    for ir in cases {
        let goal = NodeId::new(ir.nodes.len() - 1);
        let fixture = SourceFixture::new(ir, NodeId::new(0), goal, false);
        fixture.validate();
        let ast = RealParser.parse(&fixture.source).unwrap();
        let mut interned = intern_ast(&ast);
        let index = build_index_sym(&interned.ast, &interned.interner).unwrap();
        let ir = lower_to_ir(&interned.ast, &index, &mut interned.interner).unwrap();
        fixture.assert_equivalent(&ir);
    }
}

#[test]
fn malformed_source_and_semantic_corruption_are_rejected() {
    let fixture = SourceFixture::named("parallel16");
    assert!(
        RealParser
            .parse(&fixture.source.replace("; }", " }"))
            .is_err()
    );
    let (_, mut program) = fixture.compile();
    let changed = Arc::get_mut(&mut program).unwrap();
    changed.ir.arcs[0].refs.pop();
    assert!(std::panic::catch_unwind(|| fixture.assert_equivalent(&changed.ir)).is_err());
}

#[test]
fn unsupported_fixture_value_is_rejected() {
    assert!(std::panic::catch_unwind(|| SourceFixture::numeric(&ValueR::Num(f64::NAN))).is_err());
    let fixture = SourceFixture::named("parallel16");
    assert_eq!(
        fixture.source_hash(),
        SourceFixture::named("parallel16").source_hash()
    );
    assert!(std::panic::catch_unwind(|| fixture.measure("unknown")).is_err());
}

#[test]
fn every_phase_runs_real_production_code_and_returns_nine_samples() {
    let fixture = SourceFixture::named("parallel16");
    for phase in [
        "parse_ast",
        "ast_to_ir",
        "compile",
        "incidence_index",
        "source_to_dijkstra",
    ] {
        let measurement = fixture.measure(phase);
        assert_eq!(measurement.samples.len(), 9);
        assert_eq!(measurement.checksum, fixture.source_hash());
        assert!(measurement.wall.median > 0.0);
    }
}
