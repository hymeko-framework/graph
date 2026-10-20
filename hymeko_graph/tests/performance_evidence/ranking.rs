//! PageRank and traversal clients of the existing evidence sampler and canonical IR.
use super::{emit_measurement, emit_phase, ranking_support::Fixture, statistics, telemetry};
use hymeko::common::ids::{HyperArcId, NodeId};
use hymeko::generators::GraphGenerator;
use hymeko::ir::{
    incidence::{IncidenceIndex, build_incidence_index},
    ir::Ir,
};
use hymeko_graph::direct_ir::informed::{
    InformedSearchWorkspace, Undirected, UnitCost, WeightedBergePath, dijkstra_berge_path,
};
use hymeko_graph::{
    BergeIncidence, GraphicMatroidWorkspace, HyperTraversal, LeviCoord, MatroidRef, PageRank,
    PageRankConfig, PageRankModel, PageRankResult, PageRankWorkspace, TraversalWorkspace,
    connected_components, dual_reachability, levi_independent, levi_spanning_basis,
    s_connected_components,
};
use std::{
    env,
    io::{self, Write},
};

#[derive(Clone, Copy)]
enum Operation {
    Generate,
    Index,
    Levi,
    SComponents,
    Prepare,
    Solve,
    Reachable,
    Dual,
    BergePath,
    Dijkstra,
}
impl Operation {
    fn parse(value: &str) -> Self {
        match value {
            "generate_ir" => Self::Generate,
            "incidence_index" => Self::Index,
            "levi" => Self::Levi,
            "s_components2" => Self::SComponents,
            "prepare" => Self::Prepare,
            "solve" => Self::Solve,
            "reachable" => Self::Reachable,
            "dual_reachable" => Self::Dual,
            "berge_path" => Self::BergePath,
            "dijkstra" => Self::Dijkstra,
            _ => panic!("unknown ranking operation"),
        }
    }
    fn section(self) -> &'static str {
        match self {
            Self::Generate | Self::Index => "construction",
            Self::Levi | Self::SComponents => "algebra",
            Self::Prepare | Self::Solve => "pagerank",
            _ => "traversal",
        }
    }
    fn scope(self) -> &'static str {
        match self {
            Self::Generate => "core_generator_and_canonical_ir_construction_excluding_drop",
            Self::Index => "canonical_ir_to_reverse_incidence_index_excluding_drop",
            Self::Levi => "prepared_index_reused_matroid_workspace_spanning_basis",
            Self::SComponents => "prepared_index_fresh_s2_component_workspace_and_result_drop",
            Self::Prepare => "numeric_projection_and_rank_preparation",
            Self::Solve => "prepared_rank_reset_solve",
            _ => "prepared_index_reused_workspace_query_result_drop",
        }
    }
}

#[derive(Clone, Copy, Default)]
struct QueryWork {
    visited: usize,
    hops: Option<usize>,
    cost: f64,
}

struct QueryRun<'ir> {
    index: IncidenceIndex<'ir>,
    traversal: TraversalWorkspace,
    informed: InformedSearchWorkspace,
    last: QueryWork,
    matroid: GraphicMatroidWorkspace,
    basis: Vec<LeviCoord>,
    overlap_pairs: usize,
}
impl<'ir> QueryRun<'ir> {
    fn new(ir: &'ir Ir) -> Self {
        Self {
            index: build_incidence_index(ir).unwrap(),
            traversal: TraversalWorkspace::new(),
            informed: InformedSearchWorkspace::new(),
            last: QueryWork::default(),
            matroid: GraphicMatroidWorkspace::new(),
            basis: Vec::new(),
            overlap_pairs: 0,
        }
    }
    fn goal(&self) -> NodeId {
        NodeId::new(self.index.ir().nodes.len() - 1)
    }
    fn path_result(&mut self, path: Option<WeightedBergePath>, verify: bool) -> u64 {
        let Some(path) = path else {
            return if verify { 0 } else { u64::MAX };
        };
        self.last = QueryWork {
            visited: path.steps.len() + 1,
            hops: Some(path.steps.len()),
            cost: path.total_cost,
        };
        if !verify {
            return path.steps.len() as u64;
        }
        assert_eq!(path.start, NodeId::new(0));
        assert_eq!(path.total_cost, path.steps.len() as f64);
        let mut node = path.start;
        for step in &path.steps {
            assert_eq!(step.from.node, node);
            assert_eq!(
                BergeIncidence::at(self.index.ir(), step.arc, step.from.slot),
                Some(step.from)
            );
            assert_eq!(
                BergeIncidence::at(self.index.ir(), step.arc, step.to.slot),
                Some(step.to)
            );
            node = step.to.node;
        }
        assert_eq!(node, self.goal());
        path.path_hash().raw()
    }
    fn run(&mut self, operation: Operation, verify: bool) -> u64 {
        self.last = QueryWork::default();
        let traversal = HyperTraversal::new(&self.index);
        let start = NodeId::new(0);
        let goal = self.goal();
        match operation {
            Operation::Levi => {
                let rank = levi_spanning_basis(
                    MatroidRef::new(&self.index),
                    &mut self.matroid,
                    &mut self.basis,
                )
                .unwrap();
                self.last.visited = rank;
                if verify {
                    RankCase::hash(
                        self.basis
                            .iter()
                            .flat_map(|c| [c.arc.raw() as u64, u64::from(c.slot)]),
                    )
                } else {
                    rank as u64
                }
            }
            Operation::SComponents => {
                let result = s_connected_components(&self.index, 2);
                self.last.visited = result.count() as usize;
                if verify {
                    RankCase::hash(result.labels().iter().map(|&v| u64::from(v)))
                } else {
                    u64::from(result.count())
                }
            }
            Operation::Reachable => {
                let result = traversal
                    .reachable_nodes(&mut self.traversal, start)
                    .unwrap();
                self.last.visited = result.len();
                if verify {
                    RankCase::hash(result.nodes().map(|n| n.raw() as u64))
                } else {
                    result.len() as u64
                }
            }
            Operation::Dual => {
                let result =
                    dual_reachability(&self.index, &mut self.traversal, HyperArcId::new(0))
                        .unwrap();
                self.last.visited = result.len();
                if verify {
                    RankCase::hash(result.arcs().iter().map(|a| a.raw() as u64))
                } else {
                    result.len() as u64
                }
            }
            Operation::BergePath => {
                let result = traversal
                    .berge_shortest_path(&mut self.traversal, start, goal)
                    .unwrap()
                    .map(|path| WeightedBergePath {
                        total_cost: path.steps.len() as f64,
                        start: path.start,
                        steps: path.steps,
                    });
                self.path_result(result, verify)
            }
            Operation::Dijkstra => {
                let result = dijkstra_berge_path(
                    &traversal,
                    start,
                    goal,
                    &UnitCost,
                    &Undirected,
                    &mut self.informed,
                )
                .unwrap();
                self.path_result(result, verify)
            }
            _ => panic!("query operation required"),
        }
    }
    fn check(&mut self, operation: Operation) -> u64 {
        if matches!(operation, Operation::SComponents) {
            // Match the production s-line's distinct-arc work, before admitting timing.
            self.overlap_pairs = self
                .index
                .ir()
                .nodes
                .iter()
                .map(|node| {
                    let arcs = self.index.incident_arcs(node.decl);
                    let degree = arcs
                        .iter()
                        .enumerate()
                        .filter(|(i, arc)| *i == 0 || **arc != arcs[*i - 1])
                        .count();
                    degree * degree.saturating_sub(1) / 2
                })
                .sum();
            assert!(
                self.overlap_pairs <= 5_000_000,
                "s2 workload exceeds declared pair-work budget"
            );
        }
        let expected = self.run(operation, true);
        if matches!(operation, Operation::Levi) {
            let ir = self.index.ir();
            let components = connected_components(&self.index, &mut TraversalWorkspace::new())
                .unwrap()
                .count() as usize
                + ir.arcs.iter().filter(|arc| arc.refs.is_empty()).count();
            assert_eq!(
                self.last.visited,
                ir.nodes.len() + ir.arcs.len() - components
            );
            assert!(
                levi_independent(MatroidRef::new(&self.index), &self.basis, &mut self.matroid)
                    .unwrap()
            );
        }
        if matches!(operation, Operation::Dijkstra) {
            let reference = HyperTraversal::new(&self.index)
                .berge_shortest_path(&mut TraversalWorkspace::new(), NodeId::new(0), self.goal())
                .unwrap();
            assert_eq!(self.last.hops, reference.map(|p| p.steps.len()));
        }
        expected
    }
    fn emit(&self, operation: Operation, checksum: u64) {
        if matches!(operation, Operation::Levi | Operation::SComponents) {
            println!(
                "EVIDENCE_ALGEBRA\tvalue={}\tchecksum={checksum}\tverified=true\tpair_work={}\tmax_pair_work=5000000",
                self.last.visited, self.overlap_pairs
            );
            return;
        }
        let kind = match operation {
            Operation::Reachable => "nodes",
            Operation::Dual => "arcs",
            _ => "path",
        };
        let start_kind = if matches!(operation, Operation::Dual) {
            "arc"
        } else {
            "node"
        };
        let goal = if kind == "path" {
            self.goal().raw().to_string()
        } else {
            "none".into()
        };
        println!(
            "EVIDENCE_QUERY\tstart=0\tstart_kind={start_kind}\tgoal_node={goal}\tkind={kind}\tvisited={}\treachable={}\thops={}\tcost={}\tchecksum={checksum}\tflow=undirected\tweights=unit",
            self.last.visited,
            self.last.visited > 0,
            self.last.hops.unwrap_or(0),
            self.last.cost
        );
    }
}

struct RankCase {
    ir: Ir,
    fixture: Fixture,
    personalization: Vec<f64>,
    fingerprint: u64,
}
impl RankCase {
    fn new(fixture: Fixture) -> Self {
        let ir = fixture.generate().expect("Core construction");
        assert_eq!(
            (
                ir.nodes.len(),
                ir.arcs.len(),
                ir.arcs.iter().map(|a| a.refs.len()).sum::<usize>()
            ),
            (fixture.nodes, fixture.arcs, fixture.incidence_count())
        );
        let mut p = vec![0.5 / (fixture.nodes - 1) as f64; fixture.nodes];
        p[0] = 0.5; // Nonuniform input avoids a trivial uniform fixed-point benchmark.
        let fingerprint = Self::fingerprint(&ir);
        Self {
            ir,
            fixture,
            personalization: p,
            fingerprint,
        }
    }
    fn fingerprint(ir: &Ir) -> u64 {
        Self::hash(
            std::iter::once(ir.nodes.len() as u64)
                .chain(std::iter::once(ir.arcs.len() as u64))
                .chain(ir.arcs.iter().flat_map(|arc| {
                    std::iter::once(arc.refs.len() as u64)
                        .chain(arc.refs.iter().map(|r| r.target().raw() as u64))
                })),
        )
    }
    fn hash(values: impl Iterator<Item = u64>) -> u64 {
        values
            .flat_map(u64::to_le_bytes)
            .fold(0xcbf29ce484222325, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            })
    }
    fn verify(result: PageRankResult<'_>) -> u64 {
        assert!(
            result.progress.converged,
            "PageRank must attain the requested bound"
        );
        assert!(result.scores.iter().all(|v| v.is_finite() && *v >= 0.0));
        assert!((result.scores.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        Self::hash(result.scores.iter().map(|v| v.to_bits()))
    }
    fn emit_result(&self, rank: &PageRank<'_>, ws: &mut PageRankWorkspace) -> u64 {
        let result = rank
            .run(PageRankConfig::default(), ws, Some(&self.personalization))
            .expect("PageRank");
        let checksum = Self::verify(result);
        let progress = result.progress;
        let mass = result.scores.iter().sum::<f64>();
        println!(
            "EVIDENCE_RANK\titerations={}\tchange_l1={:.17e}\terror_bound_l1={:.17e}\tconverged={}\tmass={:.17e}\tscore_hash={}\tpreparation_b={}\tworkspace_b={}\tdamping=0.85\ttolerance=1e-10\tmax_iterations=200\tflow=undirected\tweights=unit\tself_return=true\tpersonalization=half_at_node0_remainder_uniform",
            progress.iterations,
            progress.change_l1,
            progress.error_bound_l1,
            progress.converged,
            mass,
            checksum,
            rank.retained_bytes(),
            ws.retained_bytes()
        );
        checksum
    }
    fn measure(
        &self,
        operation: Operation,
        run: &str,
        workload: &str,
        subject: &str,
    ) -> statistics::Measurement {
        match operation {
            Operation::Generate | Operation::Index => {
                let measurement = match operation {
                    Operation::Generate => {
                        let (measurement, ir) = statistics::measure_factory(|| {
                            let ir = self.fixture.generate().expect("Core construction");
                            assert_eq!(
                                (ir.nodes.len(), ir.arcs.len()),
                                (self.fixture.nodes, self.fixture.arcs)
                            );
                            (ir, self.fingerprint)
                        });
                        assert_eq!(Self::fingerprint(&ir), self.fingerprint);
                        measurement
                    }
                    Operation::Index => {
                        let (measurement, index) = statistics::measure_factory(|| {
                            let index = build_incidence_index(&self.ir).unwrap();
                            assert_eq!(index.incidence_count(), self.fixture.incidence_count());
                            (index, self.fingerprint)
                        });
                        index.validate().expect("exact incidence bijection");
                        measurement
                    }
                    _ => unreachable!(),
                };
                println!(
                    "EVIDENCE_CONSTRUCTION\tchecksum={}\tverified=true\tdestruction_timed=false\tfull_validation=retained_result",
                    self.fingerprint
                );
                measurement
            }
            Operation::Prepare => {
                let (measurement, rank) = statistics::measure_factory(|| {
                    (
                        PageRank::build(&self.ir, PageRankModel::default())
                            .expect("PageRank preparation"),
                        self.fingerprint,
                    )
                });
                self.emit_result(&rank, &mut PageRankWorkspace::new());
                measurement
            }
            Operation::Solve => {
                let rank = PageRank::build(&self.ir, PageRankModel::default())
                    .expect("PageRank preparation");
                let mut ws = PageRankWorkspace::new();
                let expected = self.emit_result(&rank, &mut ws);
                emit_phase(run, workload, subject, "prepared");
                let measurement = statistics::measure_calibrated_verified(|verify| {
                    let result = rank
                        .run(
                            PageRankConfig::default(),
                            &mut ws,
                            Some(&self.personalization),
                        )
                        .expect("PageRank solve");
                    if verify {
                        Self::verify(result)
                    } else {
                        result.progress.iterations as u64
                    }
                });
                assert_eq!(measurement.checksum, expected, "full rank vector drift");
                measurement
            }
            Operation::Reachable
            | Operation::Dual
            | Operation::BergePath
            | Operation::Dijkstra
            | Operation::Levi
            | Operation::SComponents => self.measure_query(operation, run, workload, subject),
        }
    }
    fn measure_query(
        &self,
        operation: Operation,
        run: &str,
        workload: &str,
        subject: &str,
    ) -> statistics::Measurement {
        let mut query = QueryRun::new(&self.ir);
        let expected = query.check(operation);
        if matches!(operation, Operation::SComponents) {
            use super::ranking_support::Membership;
            use hymeko::generators::HypergraphGenerator;
            let expected_count = match self.fixture.membership {
                Membership::Core(HypergraphGenerator::SteinerTriple { .. }) => self.fixture.arcs,
                Membership::Core(HypergraphGenerator::Sunflower { core, .. }) => {
                    if core >= 2 {
                        1
                    } else {
                        self.fixture.arcs
                    }
                }
                Membership::Core(HypergraphGenerator::CompleteUniform { r, .. }) => {
                    if r >= 3 {
                        1
                    } else {
                        self.fixture.arcs
                    }
                }
                _ => panic!("s2 comparison requires a verified combinatorial family"),
            };
            assert_eq!(
                query.last.visited, expected_count,
                "known overlap-component count"
            );
        }
        query.emit(operation, expected);
        emit_phase(run, workload, subject, "prepared");
        let measurement =
            statistics::measure_calibrated_verified(|verify| query.run(operation, verify));
        assert_eq!(
            measurement.checksum, expected,
            "complete query result drift"
        );
        measurement
    }
}

#[test]
#[ignore = "explicit PageRank/reachability performance cell; requires HYMEKO_RANK_BENCH=1"]
fn ranking_cell() {
    assert_eq!(
        env::var("HYMEKO_RANK_BENCH").as_deref(),
        Ok("1"),
        "performance opt-in required"
    );
    let run = env::var("EVIDENCE_RUN").expect("run id");
    let workload = env::var("EVIDENCE_WORKLOAD").expect("workload");
    let subject = env::var("EVIDENCE_OPERATION").expect("operation");
    let operation = Operation::parse(&subject);
    emit_phase(&run, &workload, &subject, "process_baseline");
    let fixture = Fixture::named(&workload);
    let case = RankCase::new(fixture);
    println!(
        "EVIDENCE_INPUT\tfingerprint={}\tseed={}\tarity={}\ttopology={}\tmetric_scope={}\tgeneration=core_ir_builder\tconstruction_timed={}\tindex_timed={}",
        case.fingerprint,
        fixture.seed,
        fixture.arity,
        fixture.membership.label(),
        operation.scope(),
        matches!(operation, Operation::Generate),
        matches!(operation, Operation::Index)
    );
    emit_phase(&run, &workload, &subject, "canonical_ir");
    io::stdout().flush().unwrap();
    let measurement = case.measure(operation, &run, &workload, &subject);
    assert!(
        measurement.wall.worst < 15_000_000.0,
        "15 second per-operation budget"
    );
    emit_measurement(
        &run,
        operation.section(),
        &workload,
        &subject,
        1,
        &case.ir,
        &measurement,
    );
    emit_phase(&run, &workload, &subject, "complete");
    if let Some(memory) = telemetry::memory_snapshot() {
        assert!(memory.peak_working_set_bytes < 16 * 1024usize.pow(3));
    }
}

#[test]
fn ranking_client_checks_full_scores_and_fixture_identity_without_measuring() {
    use super::ranking_support::Membership;
    let fixture = Fixture {
        nodes: 16,
        arcs: 20,
        arity: 4,
        seed: 5,
        relabel_seed: None,
        membership: Membership::Affine,
    };
    let case = RankCase::new(fixture);
    let repeated = RankCase::new(fixture);
    assert_eq!(case.fingerprint, repeated.fingerprint);
    assert_ne!(
        case.fingerprint,
        RankCase::new(Fixture { seed: 6, ..fixture }).fingerprint
    );
    let rank = PageRank::build(&case.ir, PageRankModel::default()).unwrap();
    let checksum = case.emit_result(&rank, &mut PageRankWorkspace::new());
    assert_ne!(checksum, 0);
    for subject in [
        "prepare",
        "solve",
        "reachable",
        "dual_reachable",
        "berge_path",
        "dijkstra",
    ] {
        let operation = Operation::parse(subject);
        assert!(!operation.section().is_empty() && !operation.scope().is_empty());
    }
}

#[test]
fn query_clients_match_closed_form_components_and_shortest_paths_without_timing() {
    use super::ranking_support::Case;
    use hymeko::ir::ir::IrBuilder;
    struct QueryCase {
        rows: &'static [&'static [usize]],
        nodes_reached: usize,
        arcs_reached: usize,
        hops: Option<usize>,
    }
    for case in [
        QueryCase {
            rows: &[&[0, 1], &[1, 2], &[2, 3], &[3, 4], &[4, 5]],
            nodes_reached: 6,
            arcs_reached: 5,
            hops: Some(5),
        },
        QueryCase {
            rows: &[&[0, 1], &[1, 2], &[3, 4]],
            nodes_reached: 3,
            arcs_reached: 2,
            hops: None,
        },
        QueryCase {
            rows: &[&[0, 1, 2, 3], &[3, 4, 5]],
            nodes_reached: 6,
            arcs_reached: 2,
            hops: Some(2),
        },
    ] {
        let ir = Case::unsigned(6, case.rows).build().unwrap();
        let mut query = QueryRun::new(&ir);
        for (operation, visited, hops) in [
            (Operation::Reachable, case.nodes_reached, None),
            (Operation::Dual, case.arcs_reached, None),
            (
                Operation::BergePath,
                case.hops.map_or(0, |h| h + 1),
                case.hops,
            ),
            (
                Operation::Dijkstra,
                case.hops.map_or(0, |h| h + 1),
                case.hops,
            ),
        ] {
            let checksum = query.check(operation);
            assert_eq!(query.last.visited, visited);
            assert_eq!(query.last.hops, hops);
            assert_eq!(query.last.cost, hops.unwrap_or(0) as f64);
            assert_eq!(query.run(operation, true), checksum);
            query.emit(operation, checksum);
        }
    }
}

#[test]
fn algebra_client_checks_basis_overlap_and_rejects_unbounded_pair_work_without_timing() {
    use super::ranking_support::Case;
    use hymeko::ir::ir::IrBuilder;
    for (rows, rank, components) in [
        (vec![&[0, 1][..], &[1, 2]], 4, 2),
        (vec![&[0, 1, 2][..], &[1, 2]], 4, 1),
        (vec![&[0, 0][..], &[], &[1]], 2, 3),
    ] {
        let ir = Case::unsigned(3, &rows).build().unwrap();
        let mut query = QueryRun::new(&ir);
        query.check(Operation::Levi);
        assert_eq!(query.last.visited, rank);
        query.check(Operation::SComponents);
        assert_eq!(query.last.visited, components);
    }
    let rows = vec![&[0][..]; 3_164];
    let ir = Case::unsigned(2, &rows).build().unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            QueryRun::new(&ir).check(Operation::SComponents);
        }))
        .is_err()
    );
}
