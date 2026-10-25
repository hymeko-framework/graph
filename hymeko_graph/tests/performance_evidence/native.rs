//! Matched native API clients; canonical construction and algorithms stay in their owners.
use super::{
    emit_measurement, emit_phase,
    ranking_support::{Case, Fixture, VertexPermutation},
    statistics,
};
use hymeko::{
    generators::GraphGenerator,
    ir::{
        incidence::{IncidenceDomain, IncidenceIndex, IncidenceProjection, build_incidence_index},
        ir::{Ir, IrBuilder},
    },
    util::measurement::Distribution,
};
use hymeko_graph::{
    PageRank, PageRankConfig, PageRankModel, PageRankSolver, PageRankWorkspace, TraversalWorkspace,
    connected_components,
};
use std::{
    env,
    io::{self, Write},
};

#[path = "native/library.rs"]
mod library;
#[cfg(target_os = "linux")]
#[path = "native/memory.rs"]
mod memory;

#[derive(Clone, Copy)]
enum Operation {
    Rank,
    Components,
}
#[derive(Clone, Copy)]
enum Backend {
    Hymeko,
    Phases,
    Ordered,
    Gmres,
    Prepared,
    Hypergraphz,
}
struct Selection {
    operation: Operation,
    backend: Backend,
    workers: usize,
}
impl Selection {
    fn parse(value: &str) -> Self {
        let (operation, backend) = value.split_once('_').expect("operation_backend");
        Self {
            workers: 1,
            operation: match operation {
                "rank" => Operation::Rank,
                "components" => Operation::Components,
                _ => panic!("unknown operation"),
            },
            backend: match backend {
                "hymeko" => Backend::Hymeko,
                "phases" if operation == "rank" => Backend::Phases,
                "ordered" if operation == "rank" => Backend::Ordered,
                "gmres" if operation == "rank" => Backend::Gmres,
                "prepared" => Backend::Prepared,
                "hypergraphz" => Backend::Hypergraphz,
                _ => panic!("unknown backend"),
            },
        }
    }
    fn scope(&self) -> &'static str {
        match (self.operation, self.backend) {
            (Operation::Rank, Backend::Prepared) => "prepared_rank_reset_solve_copy",
            (Operation::Rank, _) => "rank_prepare_allocate_solve_copy_drop",
            (Operation::Components, _) => {
                "prepared_index_fresh_workspace_components_normalize_drop"
            }
        }
    }
    fn solver(&self) -> PageRankSolver {
        match self.backend {
            Backend::Gmres => PageRankSolver::Gmres { restart: 20 },
            Backend::Ordered => PageRankSolver::OrderedPowerIteration,
            _ => PageRankSolver::PowerIteration,
        }
    }
}

struct Input {
    ir: Ir,
    offsets: Vec<u32>,
    members: Vec<u32>,
}

struct InputStats {
    arity: Distribution,
    degree: Distribution,
    isolates: usize,
    incidences: usize,
}
impl InputStats {
    fn new(input: &Input) -> Self {
        let counts = input.degrees();
        let isolates = counts.iter().filter(|&&count| count == 0).count();
        let mut degrees: Vec<_> = counts.into_iter().map(|n| n as f64).collect();
        let mut arities: Vec<_> = input
            .offsets
            .windows(2)
            .map(|p| f64::from(p[1] - p[0]))
            .collect();
        // Empty domains have zero-valued descriptors; actual dimensions remain
        // in EVIDENCE_CELL, so this sentinel never invents a vertex or edge.
        if degrees.is_empty() {
            degrees.push(0.0);
        }
        if arities.is_empty() {
            arities.push(0.0);
        }
        Self {
            arity: Distribution::summarize(&mut arities),
            degree: Distribution::summarize(&mut degrees),
            isolates,
            incidences: input.members.len(),
        }
    }
    fn emit(&self) {
        println!(
            "EVIDENCE_SHAPE\tarity_min={}\tarity_q1={}\tarity_median={}\tarity_q3={}\tarity_max={}\tdegree_min={}\tdegree_q1={}\tdegree_median={}\tdegree_q3={}\tdegree_max={}\tisolated_nodes={}\tdegree_sum={}",
            self.arity.min,
            self.arity.q1,
            self.arity.median,
            self.arity.q3,
            self.arity.worst,
            self.degree.min,
            self.degree.q1,
            self.degree.median,
            self.degree.q3,
            self.degree.worst,
            self.isolates,
            self.incidences,
        );
    }
}
impl Input {
    fn rank_with_phases(
        &self,
        scores: &mut [f64],
        workers: usize,
        mut boundary: impl FnMut(usize),
    ) -> usize {
        let projection = IncidenceProjection::build(&self.ir, IncidenceDomain::Nodes).unwrap();
        boundary(0);
        let rank = PageRank::from_projection(projection, PageRankModel::default()).unwrap();
        boundary(1);
        let iterations = Self::solve(&rank, &mut PageRankWorkspace::new(), scores, workers);
        boundary(2);
        drop(rank);
        boundary(3);
        iterations
    }
    fn relabeling_parity(&self, fixture: Fixture) {
        let permutation = VertexPermutation::new(fixture.nodes, fixture.relabel_seed);
        let mapping_hash = Self::hash((0..fixture.nodes).map(|n| permutation.target(n) as u64));
        let Some(seed) = fixture.relabel_seed else {
            println!(
                "EVIDENCE_RELABEL\tlayout=identity\tseed=none\tmapping_hash={mapping_hash}\tcanonical_fingerprint={}\texact_topology=true\tmapped_rank_l1=0",
                self.fingerprint()
            );
            return;
        };
        let original = Self::new(
            Fixture {
                relabel_seed: None,
                ..fixture
            }
            .generate()
            .unwrap(),
        );
        assert_eq!(original.offsets, self.offsets);
        for (&source, &target) in original.members.iter().zip(&self.members) {
            assert_eq!(permutation.target(source as usize), target as usize);
        }
        let degrees = self.degrees();
        for (original_node, degree) in original.degrees().into_iter().enumerate() {
            assert_eq!(degree, degrees[permutation.target(original_node)]);
        }
        let mut scores = vec![0.0; fixture.nodes];
        let mut mapped = scores.clone();
        for (input, output) in [(&original, &mut scores), (self, &mut mapped)] {
            Self::solve(
                &PageRank::build(&input.ir, PageRankModel::default()).unwrap(),
                &mut PageRankWorkspace::new(),
                output,
                1,
            );
        }
        let difference = scores
            .iter()
            .enumerate()
            .map(|(node, score)| (score - mapped[permutation.target(node)]).abs())
            .sum::<f64>();
        assert!(
            difference <= 2e-10,
            "inverse-mapped rank mismatch: {difference:e}"
        );
        println!(
            "EVIDENCE_RELABEL\tlayout=permuted\tseed={seed}\tmapping_hash={mapping_hash}\tcanonical_fingerprint={}\texact_topology=true\tmapped_rank_l1={difference:.17e}",
            original.fingerprint()
        );
    }

    fn new(ir: Ir) -> Self {
        let mut offsets = vec![0];
        let mut members = Vec::new();
        for arc in &ir.arcs {
            let start = members.len();
            members.extend(
                arc.refs
                    .iter()
                    .map(|r| ir.as_node(r.target()).expect("node").raw() as u32),
            );
            let mut distinct = members[start..].to_vec();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(
                distinct.len(),
                members.len() - start,
                "matched comparison requires distinct incidence"
            );
            offsets.push(u32::try_from(members.len()).unwrap());
        }
        Self {
            ir,
            offsets,
            members,
        }
    }
    fn hash(values: impl Iterator<Item = u64>) -> u64 {
        values
            .flat_map(u64::to_le_bytes)
            .fold(0xcbf29ce484222325, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
            })
    }
    fn fingerprint(&self) -> u64 {
        Self::hash(
            std::iter::once(self.ir.nodes.len() as u64).chain(
                self.offsets
                    .iter()
                    .chain(&self.members)
                    .map(|&x| u64::from(x)),
            ),
        )
    }
    fn rank_hash(scores: &[f64]) -> u64 {
        assert!(scores.iter().all(|v| v.is_finite() && *v >= 0.0));
        assert!((scores.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        Self::hash(scores.iter().map(|v| v.to_bits()))
    }
    fn degrees(&self) -> Vec<usize> {
        let mut degree = vec![0; self.ir.nodes.len()];
        for &member in &self.members {
            degree[member as usize] += 1;
        }
        degree
    }
    fn residual_bound(&self, scores: &[f64]) -> f64 {
        // Independent post-solve residual check outside every measurement boundary.
        let n = scores.len();
        let degree = self.degrees();
        let dangling: f64 = scores
            .iter()
            .zip(&degree)
            .filter(|(_, d)| **d == 0)
            .map(|(s, _)| s)
            .sum();
        let mut next = vec![(0.15 + 0.85 * dangling) / n as f64; n];
        for bounds in self.offsets.windows(2) {
            let row = &self.members[bounds[0] as usize..bounds[1] as usize];
            if row.is_empty() {
                continue;
            }
            let mass: f64 = row
                .iter()
                .map(|&v| scores[v as usize] / degree[v as usize] as f64)
                .sum();
            for &v in row {
                next[v as usize] += 0.85 * mass / row.len() as f64;
            }
        }
        next.iter()
            .zip(scores)
            .map(|(a, b)| (a - b).abs())
            .sum::<f64>()
            / 0.15
    }
    fn solve(
        rank: &PageRank<'_>,
        workspace: &mut PageRankWorkspace,
        scores: &mut [f64],
        workers: usize,
    ) -> usize {
        Self::solve_using(rank, workspace, scores, workers, PageRankSolver::default())
    }
    fn solve_using(
        rank: &PageRank<'_>,
        workspace: &mut PageRankWorkspace,
        scores: &mut [f64],
        workers: usize,
        solver: PageRankSolver,
    ) -> usize {
        workspace.set_workers(workers).expect("rank worker budget");
        assert_eq!(workspace.worker_count(), workers);
        let result = rank
            .run_with_solver(PageRankConfig::default(), workspace, None, solver)
            .expect("rank solve");
        assert!(result.progress.converged);
        assert!(result.progress.error_bound_l1 <= 1e-10);
        scores.copy_from_slice(result.scores);
        result.progress.iterations
    }
    fn components(index: &IncidenceIndex<'_>, labels: &mut [u32]) -> u32 {
        let partition =
            connected_components(index, &mut TraversalWorkspace::new()).expect("components");
        let mut minima = vec![u32::MAX; partition.count() as usize];
        for (vertex, &component) in partition.labels().iter().enumerate() {
            minima[component as usize] = minima[component as usize].min(vertex as u32);
        }
        for (label, &component) in labels.iter_mut().zip(partition.labels()) {
            *label = minima[component as usize];
        }
        partition.count()
    }
    fn parity(&self, selection: &Selection, native: &mut library::Graph<'_>) -> u64 {
        match selection.operation {
            Operation::Rank => {
                let rank = PageRank::build(&self.ir, PageRankModel::default()).unwrap();
                let projection =
                    IncidenceProjection::build(&self.ir, IncidenceDomain::Nodes).unwrap();
                if matches!(selection.backend, Backend::Ordered) {
                    println!(
                        "EVIDENCE_PLAN\tstrategy=ordered_rows\tsolver={:?}",
                        selection.solver()
                    );
                } else if selection.workers > 1 {
                    println!(
                        "EVIDENCE_PLAN\tstrategy=parallel_pull\tworkers={}\treduction_order=canonical",
                        selection.workers
                    );
                } else if let Some(plan) = rank.plan_stats() {
                    println!(
                        "EVIDENCE_PLAN\tstrategy=component_ranges\tcomponents={}\tisolates={}\trange_rows={}\tranges={}\tdirect_work={}\tselected_work={}\tsearch_states={}\tsearch_complete={}\tordering={:?}\tinput_ranges={}",
                        plan.components,
                        plan.isolated_nodes,
                        plan.range_rows,
                        plan.ranges,
                        plan.direct_work,
                        plan.selected_work,
                        plan.search_states,
                        plan.search_complete,
                        plan.ordering,
                        plan.input_ranges
                    );
                } else if rank.incoming_term_bound().is_some_and(|n| n <= 2) {
                    println!(
                        "EVIDENCE_PLAN\tstrategy=bounded_pair_scatter\tincoming_bound={}",
                        rank.incoming_term_bound().unwrap()
                    );
                } else {
                    println!("EVIDENCE_PLAN\tstrategy=ordered_rows");
                }
                let mut scores = vec![0.0; self.ir.nodes.len()];
                let mut other = scores.clone();
                let own_iterations = Self::solve_using(
                    &rank,
                    &mut PageRankWorkspace::new(),
                    &mut scores,
                    selection.workers,
                    selection.solver(),
                );
                println!(
                    "EVIDENCE_LAYOUT\thymeko_prepared_bytes={}\tprojection_ranges={}",
                    rank.retained_bytes(),
                    projection.range_count()
                );
                let foreign_iterations = native.rank(&mut other);
                let difference = scores
                    .iter()
                    .zip(&other)
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f64>();
                assert!(
                    difference <= 2e-10,
                    "full rank vector mismatch: {difference:e}"
                );
                let own_bound = self.residual_bound(&scores);
                let foreign_bound = self.residual_bound(&other);
                assert!(
                    own_bound <= 1.01e-10 && foreign_bound <= 1.01e-10,
                    "residual bound"
                );
                let own_hash = Self::rank_hash(&scores);
                let foreign_hash = Self::rank_hash(&other);
                let hash = match selection.backend {
                    Backend::Hypergraphz => foreign_hash,
                    _ => own_hash,
                };
                println!(
                    "EVIDENCE_NATIVE\toperation=rank\tparity=true\tvector_l1={difference:.17e}\thymeko_iterations={own_iterations}\thypergraphz_iterations={foreign_iterations}\thymeko_bound={own_bound:.17e}\thypergraphz_bound={foreign_bound:.17e}\thymeko_hash={own_hash}\thypergraphz_hash={foreign_hash}\tchecksum={hash}\tdamping=0.85\ttolerance=1e-10\tmax_iterations=200\tpersonalization=uniform\tweights=unit\tflow=undirected\tself_return=true"
                );
                hash
            }
            Operation::Components => {
                let index = build_incidence_index(&self.ir).unwrap();
                let mut labels = vec![u32::MAX; self.ir.nodes.len()];
                let mut other = labels.clone();
                let own_count = Self::components(&index, &mut labels);
                let foreign_count = native.components(&mut other);
                assert_eq!(own_count, foreign_count);
                assert_eq!(labels, other, "full normalized partition mismatch");
                let hash = Self::hash(labels.iter().map(|&x| u64::from(x)));
                println!(
                    "EVIDENCE_NATIVE\toperation=components\tparity=true\tcomponents={own_count}\tchecksum={hash}\tpartition=minimum_vertex_ordinal\tdirection=weak"
                );
                hash
            }
        }
    }
    fn measure(
        &self,
        selection: &Selection,
        native: &mut library::Graph<'_>,
    ) -> statistics::Measurement {
        match selection.operation {
            Operation::Rank => {
                let mut scores = vec![0.0; self.ir.nodes.len()];
                let prepared = matches!(selection.backend, Backend::Prepared)
                    .then(|| PageRank::build(&self.ir, PageRankModel::default()).unwrap());
                let mut workspace = PageRankWorkspace::new();
                let mut phases = statistics::PhaseSamples::default();
                let measurement = statistics::measure_calibrated_verified(|verify| {
                    let iterations = match selection.backend {
                        Backend::Hypergraphz => native.rank(&mut scores),
                        Backend::Prepared => Self::solve(
                            prepared.as_ref().unwrap(),
                            &mut workspace,
                            &mut scores,
                            selection.workers,
                        ),
                        Backend::Hymeko | Backend::Gmres | Backend::Ordered => {
                            let rank = PageRank::build(&self.ir, PageRankModel::default()).unwrap();
                            Self::solve_using(
                                &rank,
                                &mut PageRankWorkspace::new(),
                                &mut scores,
                                selection.workers,
                                selection.solver(),
                            )
                        }
                        Backend::Phases => {
                            if verify {
                                self.rank_with_phases(&mut scores, selection.workers, |_| {})
                            } else {
                                let epoch = std::time::Instant::now();
                                phases.observe(
                                    || epoch.elapsed().as_nanos(),
                                    |boundary| {
                                        self.rank_with_phases(
                                            &mut scores,
                                            selection.workers,
                                            boundary,
                                        )
                                    },
                                )
                            }
                        }
                    };
                    if verify {
                        Self::rank_hash(&scores)
                    } else {
                        iterations as u64
                    }
                });
                if matches!(selection.backend, Backend::Phases) {
                    for (ordinal, values) in phases.normalized(&measurement).iter().enumerate() {
                        println!(
                            "EVIDENCE_STAGE_SAMPLE\tordinal={ordinal}\trepetitions={}\tincidence_us={:.6}\tpreparation_us={:.6}\tsolve_us={:.6}\tcleanup_us={:.6}\tinner_total_us={:.6}\tboundary=same_call_partition",
                            measurement.repetitions_per_sample,
                            values[0],
                            values[1],
                            values[2],
                            values[3],
                            values.iter().sum::<f64>()
                        );
                    }
                }
                measurement
            }
            Operation::Components => {
                let index = build_incidence_index(&self.ir).unwrap();
                let mut labels = vec![u32::MAX; self.ir.nodes.len()];
                statistics::measure_calibrated_verified(|verify| {
                    let count = match selection.backend {
                        Backend::Hypergraphz => native.components(&mut labels),
                        Backend::Hymeko => Self::components(&index, &mut labels),
                        Backend::Prepared | Backend::Phases | Backend::Gmres | Backend::Ordered => {
                            panic!("components require the matched fresh-workspace API")
                        }
                    };
                    if verify {
                        Self::hash(labels.iter().map(|&x| u64::from(x)))
                    } else {
                        u64::from(count)
                    }
                })
            }
        }
    }
}

#[test]
#[ignore = "explicit native comparison requires HYMEKO_NATIVE_BENCH=1 and pinned DLL"]
fn native_cell() {
    assert_eq!(env::var("HYMEKO_NATIVE_BENCH").as_deref(), Ok("1"));
    let run = env::var("EVIDENCE_RUN").unwrap();
    let workload = env::var("EVIDENCE_WORKLOAD").unwrap();
    let subject = env::var("EVIDENCE_OPERATION").unwrap();
    let mut selection = Selection::parse(&subject);
    if matches!(selection.backend, Backend::Phases) {
        assert_eq!(
            env::var("HYMEKO_PHASE_BENCH").as_deref(),
            Ok("1"),
            "phase timing disabled"
        );
    }
    selection.workers = env::var("EVIDENCE_THREADS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .expect("positive worker count");
    assert!(selection.workers > 0);
    if matches!(selection.backend, Backend::Hypergraphz) {
        assert_eq!(selection.workers, 1, "native upstream is single-threaded");
    }
    let fixture = Fixture::named(&workload);
    let input = Input::new(fixture.generate().unwrap());
    assert_eq!(input.members.len(), fixture.incidence_count());
    input.relabeling_parity(fixture);
    let library = library::Library::load();
    let mut native = library.build(input.ir.nodes.len(), &input.offsets, &input.members);
    println!(
        "EVIDENCE_INPUT\tfingerprint={}\tseed={}\tarity_cap={}\ttopology={}\tshape_reported=true\tconstruction_timed=false\tindex_timed=false\tmetric_scope={}\tprocess_memory=includes_both_graphs_and_parity",
        input.fingerprint(),
        fixture.seed,
        fixture.arity,
        fixture.membership.label(),
        selection.scope()
    );
    InputStats::new(&input).emit();
    let expected = input.parity(&selection, &mut native);
    emit_phase(&run, &workload, &subject, "prepared_and_verified");
    io::stdout().flush().unwrap();
    let measurement = input.measure(&selection, &mut native);
    assert_eq!(
        measurement.checksum, expected,
        "result drift after full parity"
    );
    assert!(
        measurement.wall.worst < 15_000_000.0,
        "15-second operation budget"
    );
    emit_measurement(
        &run,
        "native",
        &workload,
        &subject,
        selection.workers,
        &input.ir,
        &measurement,
    );
    emit_phase(&run, &workload, &subject, "complete");
}

#[test]
#[ignore = "explicit upstream parity requires the pinned native DLL, no measurements"]
fn native_parity() {
    let library = library::Library::load();
    for fixture in [
        Case::unsigned(4, &[&[0, 1, 2]]),
        Case::unsigned(5, &[&[], &[0], &[1, 2], &[2, 3]]),
        Case::unsigned(4, &[]),
        Case::unsigned(8, &[&[0, 1], &[1, 2, 3], &[4, 5], &[6]]),
    ] {
        let input = Input::new(fixture.build().unwrap());
        let mut native = library.build(input.ir.nodes.len(), &input.offsets, &input.members);
        for name in ["rank_hymeko", "rank_hypergraphz"] {
            input.parity(&Selection::parse(name), &mut native);
        }
        if input.offsets.windows(2).any(|p| p[1] - p[0] == 1) {
            assert_eq!(
                native.try_components(&mut vec![u32::MAX; input.ir.nodes.len()]),
                Err(2)
            );
            println!(
                "UPSTREAM_LIMITATION: singleton components are guarded; upstream indexes a short window"
            );
        } else {
            input.parity(&Selection::parse("components_hymeko"), &mut native);
        }
    }
    use super::ranking_support::Membership;
    for fixture in [
        Fixture {
            nodes: 32,
            arcs: 24,
            arity: 8,
            seed: 19,
            relabel_seed: None,
            membership: Membership::Mixed(&[2, 3, 8]),
        },
        Fixture {
            nodes: 32,
            arcs: 24,
            arity: 4,
            seed: 19,
            relabel_seed: None,
            membership: Membership::Hub,
        },
        Fixture {
            nodes: 40,
            arcs: 32,
            arity: 4,
            seed: 19,
            relabel_seed: None,
            membership: Membership::Communities {
                groups: 4,
                active: 32,
            },
        },
        Fixture {
            nodes: 32,
            arcs: 31,
            arity: 2,
            seed: 19,
            relabel_seed: None,
            membership: Membership::Chain,
        },
    ] {
        let input = Input::new(fixture.generate().unwrap());
        let mut native = library.build(input.ir.nodes.len(), &input.offsets, &input.members);
        for operation in ["rank_hymeko", "rank_hypergraphz", "components_hymeko"] {
            input.parity(&Selection::parse(operation), &mut native);
        }
    }
}

#[test]
fn rank_phase_boundaries_preserve_full_vector_without_reading_a_clock() {
    for case in [
        Case::unsigned(5, &[&[0, 1, 2], &[2, 3]]),
        Case::unsigned(4, &[&[0, 1], &[0, 1], &[1, 2]]),
        Case::unsigned(3, &[]),
    ] {
        let input = Input::new(case.build().unwrap());
        let mut expected = vec![0.0; input.ir.nodes.len()];
        let expected_iterations = Input::solve(
            &PageRank::build(&input.ir, PageRankModel::default()).unwrap(),
            &mut PageRankWorkspace::new(),
            &mut expected,
            1,
        );
        let mut actual = expected.clone();
        let mut observed = Vec::new();
        let iterations = input.rank_with_phases(&mut actual, 1, |phase| observed.push(phase));
        assert_eq!(observed, [0, 1, 2, 3]);
        assert_eq!(iterations, expected_iterations);
        assert_eq!(actual, expected);
        assert!(matches!(
            Selection::parse("rank_phases").backend,
            Backend::Phases
        ));
    }
}

#[test]
fn rank_solver_selection_and_independent_residuals_require_no_native_library_or_clock() {
    let input = Input::new(
        Case::unsigned(7, &[&[], &[0, 1, 2], &[2, 3], &[3, 4]])
            .build()
            .unwrap(),
    );
    let rank = PageRank::build(&input.ir, PageRankModel::default()).unwrap();
    for name in ["rank_hymeko", "rank_ordered", "rank_gmres"] {
        let selection = Selection::parse(name);
        let mut scores = vec![0.0; 7];
        Input::solve_using(
            &rank,
            &mut PageRankWorkspace::new(),
            &mut scores,
            1,
            selection.solver(),
        );
        assert!(input.residual_bound(&scores) <= 1e-10);
    }
}

#[test]
#[ignore = "large canonical input correctness, no timing or external library"]
fn rank_solver_full_catalog_correctness() {
    for base in ["chain65k", "rank2_1m", "hub32_1m", "components8_1m"] {
        for suffix in ["", "@17"] {
            let name = format!("{base}{suffix}");
            let fixture = Fixture::named(&name);
            let input = Input::new(fixture.generate().unwrap());
            let rank = PageRank::build(&input.ir, PageRankModel::default()).unwrap();
            let mut reference = vec![0.0; rank.node_count()];
            let mut candidate = reference.clone();
            let mut ws = PageRankWorkspace::new();
            Input::solve(&rank, &mut ws, &mut reference, 1);
            for solver in [
                PageRankSolver::OrderedPowerIteration,
                PageRankSolver::Gmres { restart: 20 },
            ] {
                let steps = Input::solve_using(&rank, &mut ws, &mut candidate, 1, solver);
                let bound = input.residual_bound(&candidate);
                let difference: f64 = reference
                    .iter()
                    .zip(&candidate)
                    .map(|(a, b)| (a - b).abs())
                    .sum();
                assert!(
                    bound <= 1.01e-10 && difference <= 2e-10,
                    "{name} {solver:?}: {bound}, {difference}"
                );
                println!(
                    "SOLVER_CORRECTNESS workload={name} solver={solver:?} steps={steps} residual_bound={bound:e} reference_l1={difference:e}"
                );
            }
        }
    }
}

#[test]
fn rank_phase_requires_an_additional_opt_in_before_fixture_loading() {
    // An invalid fixture also prevents any graph work if the guard regresses.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["native::native_cell", "--exact", "--ignored", "--nocapture"])
        .env("HYMEKO_NATIVE_BENCH", "1")
        .env_remove("HYMEKO_PHASE_BENCH")
        .env("EVIDENCE_RUN", "untimed-opt-in-contract")
        .env("EVIDENCE_WORKLOAD", "deliberately-invalid-fixture")
        .env("EVIDENCE_OPERATION", "rank_phases")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(messages.contains("phase timing disabled"), "{messages}");
}

#[test]
fn seeded_vertex_relabeling_is_bijective_and_preserves_topology_and_rank() {
    use super::ranking_support::Membership;
    for nodes in [0, 1, 2, 17, 1024] {
        let identity = VertexPermutation::new(nodes, None);
        for seed in [17, 29, 43] {
            let a = VertexPermutation::new(nodes, Some(seed));
            let b = VertexPermutation::new(nodes, Some(seed));
            let mut targets: Vec<_> = (0..nodes).map(|node| a.target(node)).collect();
            for node in 0..nodes {
                assert_eq!(identity.target(node), node);
                assert_eq!(a.target(node), b.target(node));
            }
            targets.sort_unstable();
            assert_eq!(targets, (0..nodes).collect::<Vec<_>>());
        }
    }
    let a = VertexPermutation::new(64, Some(17));
    let b = VertexPermutation::new(64, Some(29));
    assert!((0..64).any(|node| a.target(node) != b.target(node)));
    for name in ["chain65k", "rank2_1m", "hub32_1m", "components8_1m"] {
        let mut fixture = Fixture::named(name);
        fixture.nodes = 64;
        fixture.arcs = 48;
        fixture.arity = 8;
        match fixture.membership {
            Membership::Chain => {
                fixture.arcs = 63;
                fixture.arity = 2;
            }
            Membership::Communities { .. } => {
                fixture.membership = Membership::Communities {
                    groups: 4,
                    active: 48,
                };
            }
            _ => {}
        }
        for seed in [None, Some(17), Some(29), Some(43)] {
            fixture.relabel_seed = seed;
            Input::new(fixture.generate().unwrap()).relabeling_parity(fixture);
        }
        assert_eq!(Fixture::named(&format!("{name}@29")).relabel_seed, Some(29));
        assert!(std::panic::catch_unwind(|| Fixture::named(&format!("{name}@bad"))).is_err());
    }
}

#[test]
fn native_input_and_selection_contracts_without_loading_external_code() {
    let input = Input::new(Case::unsigned(3, &[&[2, 0], &[], &[1]]).build().unwrap());
    assert_eq!(input.offsets, [0, 2, 2, 3]);
    assert_eq!(input.members, [2, 0, 1]);
    assert_ne!(input.fingerprint(), 0);
    let stats = InputStats::new(&input);
    stats.emit();
    assert_eq!((stats.arity.min, stats.arity.worst), (0.0, 2.0));
    assert_eq!((stats.degree.min, stats.degree.worst), (1.0, 1.0));
    assert_eq!(stats.isolates, 0);
    assert_eq!(stats.incidences, 3);
    let stats = InputStats::new(&Input::new(
        Case::unsigned(4, &[&[0, 1], &[1, 2]]).build().unwrap(),
    ));
    assert_eq!(stats.isolates, 1);
    assert_eq!(
        (stats.degree.min, stats.degree.median, stats.degree.worst),
        (0.0, 1.0, 2.0)
    );
    let stats = InputStats::new(&Input::new(Case::unsigned(0, &[]).build().unwrap()));
    assert_eq!(stats.arity.worst, 0.0);
    assert_eq!(stats.degree.worst, 0.0);
    for value in [
        "rank_hymeko",
        "rank_prepared",
        "rank_hypergraphz",
        "components_hymeko",
        "components_hypergraphz",
    ] {
        assert!(!Selection::parse(value).scope().is_empty());
    }
}
