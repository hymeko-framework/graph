//! Durable graph performance-evidence client.
//!
//! The ignored cells are executed directly, one fresh process per workload/operation or
//! workload/representation. They preserve raw samples and keep operation-local CPU/wall separate
//! from whole-process CPU/RSS. Production code and the historical P3.4 harnesses are unchanged.

use std::env;
use std::hint::black_box;

use hymeko::common::ids::NodeId;
use hymeko::ir::incidence::{IncidenceIndex, build_incidence_index};
use hymeko::ir::ir::Ir;
use hymeko_graph::direct_ir::{
    HyperTraversal, IncidenceWeightCost, InformedSearchWorkspace, SearchStats, Undirected,
    dijkstra_berge_path,
};
use hymeko_graph::{
    GraphicMatroidWorkspace, MatroidRef, TraversalWorkspace, connected_components,
    levi_spanning_basis, par_batch_reachable_counts, rank2_spanning_forest,
};

#[path = "performance_evidence/representations.rs"]
mod representations;
#[path = "performance_evidence/statistics.rs"]
mod statistics;
#[path = "performance_evidence/telemetry.rs"]
mod telemetry;
#[path = "informed_common/mod.rs"]
mod workloads;

use representations::{PreparedRepresentation, RepresentationKind};
use statistics::{Measurement, measure_calibrated, measure_calibrated_verified, measure_factory};

fn dimensions(ir: &Ir) -> (usize, usize, usize) {
    (
        ir.nodes.len(),
        ir.arcs.len(),
        ir.arcs.iter().map(|arc| arc.refs.len()).sum(),
    )
}

fn evidence_workload(name: &str) -> Ir {
    match name {
        "path65k" => workloads::window_ir(65_536, 2, true, workloads::SignMode::AllPlus),
        "grid256" => workloads::grid_ir(256, 256),
        "arity2_1m" => workloads::uniform_rank_k_ir(0x51EED ^ 2, 200_000, 500_000, 2),
        "arity3_1m" => workloads::uniform_rank_k_ir(0x51EED ^ 3, 200_000, 333_333, 3),
        "arity8_1m" => workloads::uniform_rank_k_ir(0x51EED ^ 8, 200_000, 125_000, 8),
        "arity32_1m" => workloads::uniform_rank_k_ir(0x51EED ^ 32, 200_000, 31_250, 32),
        "arity128_1m" => workloads::uniform_rank_k_ir(0x51EED ^ 128, 200_000, 7_812, 128),
        other => workloads::named_workload(other),
    }
}

fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| "NA".to_string(), |value| format!("{value:.3}"))
}

fn emit_phase(run: &str, workload: &str, subject: &str, phase: &str) {
    match telemetry::memory_snapshot() {
        Some(memory) => println!(
            "EVIDENCE_PHASE\trun={run}\tworkload={workload}\tsubject={subject}\tphase={phase}\tworking_set_b={}\tself_peak_b={}",
            memory.working_set_bytes, memory.peak_working_set_bytes
        ),
        None => println!(
            "EVIDENCE_PHASE\trun={run}\tworkload={workload}\tsubject={subject}\tphase={phase}\tworking_set_b=NA\tself_peak_b=NA"
        ),
    }
}

fn emit_search_work(run: &str, workload: &str, subject: &str, stats: SearchStats) {
    println!(
        "EVIDENCE_WORK\trun={run}\tworkload={workload}\tsubject={subject}\texpanded_nodes={}\texpanded_arcs={}\tresolved_incidences={}\tpushes={}\tpops={}\tstale_pops={}\theuristic_evals={}\treopened={}",
        stats.expanded_nodes,
        stats.expanded_arcs,
        stats.resolved_incidences,
        stats.pushes,
        stats.pops,
        stats.stale_pops,
        stats.heuristic_evals,
        stats.reopened
    );
}

fn emit_measurement(
    run: &str,
    section: &str,
    workload: &str,
    subject: &str,
    threads: usize,
    ir: &Ir,
    measurement: &Measurement,
) {
    let (nodes, arcs, incidences) = dimensions(ir);
    for sample in &measurement.samples {
        println!(
            "EVIDENCE_SAMPLE\trun={run}\tsection={section}\tworkload={workload}\tsubject={subject}\tthreads={threads}\tordinal={}\trepetitions={}\twall_us={:.3}\tcpu_us={}\tchecksum={}",
            sample.ordinal,
            measurement.repetitions_per_sample,
            sample.wall_us,
            optional(sample.cpu_us),
            sample.checksum
        );
    }
    println!(
        "EVIDENCE_CELL\trun={run}\tsection={section}\tworkload={workload}\tsubject={subject}\tstatus=OK\tthreads={threads}\tnodes={nodes}\tarcs={arcs}\tincidences={incidences}\twarmups={}\tsamples={}\trepetitions={}\twall_median_us={:.3}\twall_q1_us={:.3}\twall_q3_us={:.3}\twall_worst_us={:.3}\tcpu_median_us={}\tcpu_q1_us={}\tcpu_q3_us={}\tcpu_worst_us={}\tchecksum={}",
        measurement.warmups,
        measurement.samples.len(),
        measurement.repetitions_per_sample,
        measurement.wall.median,
        measurement.wall.q1,
        measurement.wall.q3,
        measurement.wall.worst,
        optional(measurement.cpu.map(|distribution| distribution.median)),
        optional(measurement.cpu.map(|distribution| distribution.q1)),
        optional(measurement.cpu.map(|distribution| distribution.q3)),
        optional(measurement.cpu.map(|distribution| distribution.worst)),
        measurement.checksum
    );
}

fn emit_na(run: &str, workload: &str, subject: &str, threads: usize, ir: &Ir, reason: &str) {
    let (nodes, arcs, incidences) = dimensions(ir);
    println!(
        "EVIDENCE_CELL\trun={run}\tsection=operation\tworkload={workload}\tsubject={subject}\tstatus=NA\tthreads={threads}\tnodes={nodes}\tarcs={arcs}\tincidences={incidences}\twarmups=0\tsamples=0\trepetitions=0\twall_median_us=NA\twall_q1_us=NA\twall_q3_us=NA\twall_worst_us=NA\tcpu_median_us=NA\tcpu_q1_us=NA\tcpu_q3_us=NA\tcpu_worst_us=NA\tchecksum=NA\treason={reason}"
    );
}

fn measure_operation(
    run: &str,
    workload: &str,
    operation: &str,
    threads: usize,
    ir: &Ir,
    index: &IncidenceIndex<'_>,
) -> Option<Measurement> {
    let traversal = HyperTraversal::new(index);
    let node_count = ir.nodes.len();
    match operation {
        "reachable" => {
            let mut workspace = TraversalWorkspace::new();
            let expected = traversal
                .reachable_nodes(&mut workspace, NodeId::new(0))
                .expect("reachable preparation")
                .len() as u64;
            emit_phase(run, workload, operation, "prepared");
            let measurement = measure_calibrated(|| {
                traversal
                    .reachable_nodes(&mut workspace, NodeId::new(0))
                    .expect("reachable")
                    .len() as u64
            });
            assert_eq!(measurement.checksum, expected);
            Some(measurement)
        }
        "independent_query_batch" => {
            let step = (node_count / 64).max(1);
            let starts: Vec<NodeId> = (0..node_count)
                .step_by(step)
                .take(64)
                .map(NodeId::new)
                .collect();
            assert_eq!(starts.len(), 64, "fixed-work batch requires 64 starts");
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("local Rayon pool");
            let run_batch = || {
                let counts = pool
                    .install(|| par_batch_reachable_counts(index, &starts))
                    .expect("batch reachability");
                let checksum = counts.iter().map(|&count| count as u64).sum();
                black_box(counts);
                checksum
            };
            let expected = run_batch();
            emit_phase(run, workload, operation, "prepared");
            let measurement = measure_calibrated(run_batch);
            assert_eq!(measurement.checksum, expected);
            Some(measurement)
        }
        "components" => {
            let mut workspace = TraversalWorkspace::new();
            let expected = connected_components(index, &mut workspace)
                .expect("component preparation")
                .count() as u64;
            emit_phase(run, workload, operation, "prepared");
            let measurement = measure_calibrated(|| {
                connected_components(index, &mut workspace)
                    .expect("components")
                    .count() as u64
            });
            assert_eq!(measurement.checksum, expected);
            Some(measurement)
        }
        "dijkstra" => {
            let cost = IncidenceWeightCost { base: 1.0 };
            let mut workspace = InformedSearchWorkspace::new();
            let goal = NodeId::new(node_count.saturating_sub(1));
            let (expected, measurement) = {
                let mut run_query = |strong_checksum: bool| {
                    dijkstra_berge_path(
                        &traversal,
                        NodeId::new(0),
                        goal,
                        &cost,
                        &Undirected,
                        &mut workspace,
                    )
                    .expect("Dijkstra")
                    .map_or(0, |path| {
                        if strong_checksum {
                            path.path_hash().raw()
                        } else {
                            path.steps.len() as u64
                        }
                    })
                };
                let expected = run_query(true);
                emit_phase(run, workload, operation, "prepared");
                let measurement = measure_calibrated_verified(&mut run_query);
                (expected, measurement)
            };
            assert_eq!(measurement.checksum, expected);
            let measured_work = workspace.stats();
            let verification = dijkstra_berge_path(
                &traversal,
                NodeId::new(0),
                goal,
                &cost,
                &Undirected,
                &mut workspace,
            )
            .expect("Dijkstra verification")
            .map_or(0, |path| path.path_hash().raw());
            assert_eq!(verification, expected, "post-measurement path hash drift");
            assert_eq!(
                workspace.stats(),
                measured_work,
                "Dijkstra work counters must be deterministic"
            );
            emit_search_work(run, workload, operation, measured_work);
            Some(measurement)
        }
        "rank2" => {
            let mut workspace = GraphicMatroidWorkspace::new();
            let mut output = Vec::new();
            let expected =
                match rank2_spanning_forest(MatroidRef::new(index), &mut workspace, &mut output) {
                    Ok(rank) => rank as u64,
                    Err(_) => return None,
                };
            emit_phase(run, workload, operation, "prepared");
            let measurement = measure_calibrated(|| {
                rank2_spanning_forest(MatroidRef::new(index), &mut workspace, &mut output)
                    .expect("rank-2") as u64
            });
            assert_eq!(measurement.checksum, expected);
            Some(measurement)
        }
        "levi" => {
            let mut workspace = GraphicMatroidWorkspace::new();
            let mut output = Vec::new();
            let expected = levi_spanning_basis(MatroidRef::new(index), &mut workspace, &mut output)
                .expect("Levi preparation") as u64;
            emit_phase(run, workload, operation, "prepared");
            let measurement = measure_calibrated(|| {
                levi_spanning_basis(MatroidRef::new(index), &mut workspace, &mut output)
                    .expect("Levi") as u64
            });
            assert_eq!(measurement.checksum, expected);
            Some(measurement)
        }
        other => panic!("unknown evidence operation {other}"),
    }
}

#[test]
#[ignore = "fresh-process performance evidence; driven by scripts/performance_evidence/run.ps1"]
fn operation_cell() {
    let run = env::var("EVIDENCE_RUN").unwrap_or_else(|_| "manual".to_string());
    let workload = env::var("EVIDENCE_WORKLOAD").expect("EVIDENCE_WORKLOAD");
    let operation = env::var("EVIDENCE_OPERATION").expect("EVIDENCE_OPERATION");
    let threads = env::var("EVIDENCE_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    emit_phase(&run, &workload, &operation, "process_baseline");
    let ir = evidence_workload(&workload);
    emit_phase(&run, &workload, &operation, "canonical_ir");
    let index = build_incidence_index(&ir).expect("incidence index");
    emit_phase(&run, &workload, &operation, "incidence_index");
    match measure_operation(&run, &workload, &operation, threads, &ir, &index) {
        Some(measurement) => emit_measurement(
            &run,
            "operation",
            &workload,
            &operation,
            threads,
            &ir,
            &measurement,
        ),
        None => emit_na(
            &run,
            &workload,
            &operation,
            threads,
            &ir,
            "operation_not_applicable",
        ),
    }
    emit_phase(&run, &workload, &operation, "measured");
}

#[test]
#[ignore = "fresh-process representation evidence; driven by scripts/performance_evidence/run.ps1"]
fn representation_cell() {
    let run = env::var("EVIDENCE_RUN").unwrap_or_else(|_| "manual".to_string());
    let workload = env::var("EVIDENCE_WORKLOAD").expect("EVIDENCE_WORKLOAD");
    let kind_text = env::var("EVIDENCE_REPRESENTATION").expect("EVIDENCE_REPRESENTATION");
    let kind = RepresentationKind::parse(&kind_text).expect("known EVIDENCE_REPRESENTATION");
    emit_phase(&run, &workload, kind.key(), "process_baseline");
    let ir = evidence_workload(&workload);
    emit_phase(&run, &workload, kind.key(), "canonical_ir");

    let (preparation, mut prepared) = measure_factory(|| {
        let (prepared, components) = PreparedRepresentation::build(kind, &ir);
        (prepared, components as u64)
    });
    emit_phase(&run, &workload, kind.key(), "prepared");
    let expected = preparation.checksum;
    let query = measure_calibrated(|| prepared.query() as u64);
    assert_eq!(query.checksum, expected, "prepared representation parity");
    let retained = prepared.retained_bytes();

    emit_measurement(
        &run,
        "representation_preparation",
        &workload,
        kind.key(),
        1,
        &ir,
        &preparation,
    );
    emit_measurement(
        &run,
        "representation_query",
        &workload,
        kind.key(),
        1,
        &ir,
        &query,
    );
    println!(
        "EVIDENCE_RETAINED\trun={run}\tworkload={workload}\trepresentation={}\trepresentation_b={}\tworkspace_b={}\toutput_b={}\tsemantic_loss={}",
        kind.key(),
        retained.representation,
        retained
            .workspace
            .map_or_else(|| "UNMEASURED".to_string(), |bytes| bytes.to_string()),
        std::mem::size_of::<usize>(),
        PreparedRepresentation::semantic_loss(kind)
    );
    emit_phase(&run, &workload, kind.key(), "measured");
}

#[test]
fn representation_keys_are_unique_and_structural() {
    let keys: std::collections::BTreeSet<_> = RepresentationKind::ALL
        .into_iter()
        .map(RepresentationKind::key)
        .collect();
    assert_eq!(keys.len(), RepresentationKind::ALL.len());
    assert!(keys.contains("sparse_star"));
    assert!(keys.contains("clique_2section"));
}

#[test]
fn representation_queries_have_component_parity() {
    let ir = workloads::steiner_sts_ir(3);
    let mut expected = None;
    for kind in RepresentationKind::ALL {
        let (mut prepared, initial) = PreparedRepresentation::build(kind, &ir);
        assert_eq!(prepared.query(), initial);
        match expected {
            None => expected = Some(initial),
            Some(expected) => assert_eq!(initial, expected, "{} parity", kind.key()),
        }
    }
}

#[test]
fn canonical_count_control_matches_production_labeling() {
    for ir in [
        workloads::steiner_sts_ir(5),
        workloads::uniform_rank_k_ir(0xC0DEC, 256, 128, 8),
    ] {
        let index = build_incidence_index(&ir).expect("canonical index");
        let mut traversal_workspace = TraversalWorkspace::new();
        let expected = connected_components(&index, &mut traversal_workspace)
            .expect("production component labeling")
            .count() as usize;
        let (mut prepared, initial) =
            PreparedRepresentation::build(RepresentationKind::CanonicalIndex, &ir);
        assert_eq!(initial, expected);
        assert_eq!(prepared.query(), expected);
    }
}

#[test]
fn representation_controls_match_across_arities() {
    for arity in [2, 3, 8, 32, 128] {
        let edge_count = (512 / arity).max(1);
        let ir = workloads::uniform_rank_k_ir(0xA11CE ^ arity as u64, 256, edge_count, arity);
        let mut expected = None;
        for kind in RepresentationKind::ALL {
            let (mut prepared, initial) = PreparedRepresentation::build(kind, &ir);
            assert_eq!(prepared.query(), initial, "arity={arity}");
            match expected {
                None => expected = Some(initial),
                Some(expected) => {
                    assert_eq!(initial, expected, "arity={arity}, kind={}", kind.key())
                }
            }
        }
    }
}
